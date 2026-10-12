//! Local web backend: the strategy engine behind an HTTP API, plus the built
//! web app served from one port. Every address of the app is answered with a
//! whole page: the first screen's words and numbers are in the first response.

mod page;

mod backtest;
mod catalogue;
mod outline;
mod prices;
mod tested;

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use axum::{
    body::Bytes,
    extract::{rejection::JsonRejection, DefaultBodyLimit, Path, Request, State},
    handler::HandlerWithoutStateExt,
    http::{
        header::{CACHE_CONTROL, CONTENT_TYPE},
        HeaderValue, StatusCode, Uri,
    },
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use catalogue::Entry;
use compounder_engine::strategy::Strategy;
use prices::Prices;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Semaphore;
use tested::{Figures, Tested};
use tower_http::{compression::CompressionLayer, services::ServeDir};

/// The port to listen on, unless `PORT` names another.
const DEFAULT_PORT: u16 = 8787;

/// A token the app knows about. Symbol, name and decimals were read from the
/// token contract itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Asset {
    id: String,
    chain_id: u64,
    address: String,
    symbol: String,
    name: String,
    decimals: u8,
    /// The listed share whose price history stands for this token's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tracks: Option<String>,
}

#[derive(Deserialize)]
struct AssetFile {
    assets: Vec<Asset>,
}

fn assets() -> &'static BTreeMap<String, Asset> {
    static ASSETS: OnceLock<BTreeMap<String, Asset>> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let file: AssetFile = serde_json::from_str(include_str!("../data/assets.json")).expect("data/assets.json is valid");
        file.assets.into_iter().map(|asset| (asset.id.clone(), asset)).collect()
    })
}

/// One line of a resolved portfolio: an asset and its share.
#[derive(Clone, Serialize, Deserialize)]
struct Holding {
    asset_id: String,
    /// Null when the asset is not in the registry.
    symbol: Option<String>,
    name: Option<String>,
    weight: f64,
}

/// The holdings a fixed strategy aims for, each with what the registry knows of it.
fn holdings(targets: BTreeMap<String, f64>) -> Vec<Holding> {
    let mut rows: Vec<Holding> = targets
        .into_iter()
        .map(|(asset_id, weight)| {
            let known = assets().get(&asset_id);
            Holding {
                symbol: known.map(|asset| asset.symbol.clone()),
                name: known.map(|asset| asset.name.clone()),
                asset_id,
                weight,
            }
        })
        .collect();
    largest_first(&mut rows);
    rows
}

/// Largest share first; ties in asset-id order so the list is stable.
fn largest_first(rows: &mut [Holding]) {
    rows.sort_by(|a, b| b.weight.total_cmp(&a.weight).then_with(|| a.asset_id.cmp(&b.asset_id)));
}

/// A fund as the app is sent it: its words, its rules, what its test came to and what it holds.
#[derive(Serialize, Deserialize)]
struct StrategyView {
    id: String,
    name: String,
    summary: String,
    accent: String,
    /// "basket" when it holds named assets in set shares, "rules" when what it holds follows from prices.
    kind: String,
    /// One of the funds the home page leads with.
    #[serde(default)]
    starter: bool,
    /// The fixed document the money side runs and a browser can check. Only a starter has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fixed: Option<Strategy>,
    /// The fund as a rule document, which is what a backtest takes.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    rules: Value,
    /// When it trades, in words.
    trades: String,
    /// Its rules in plain words. A starter has none: what it holds is all its rules say.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    outline: Vec<outline::Line>,
    /// What its test over past prices came to, where there are prices to test it on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    test: Option<Figures>,
    /// A starter's targets. For any other fund, what its test held on its last day.
    holdings: Vec<Holding>,
}

fn strategy_view(entry: &Entry, tested: Option<&Tested>) -> Result<StrategyView, ApiError> {
    let test = tested.and_then(|tested| tested.of(&entry.id, &entry.rules));
    let holdings = match &entry.fixed {
        Some(fixed) => holdings(fixed.targets().map_err(ApiError::invalid_strategy)?),
        None => test.as_ref().map(|test| test.holdings.clone()).unwrap_or_default(),
    };
    Ok(StrategyView {
        id: entry.id.clone(),
        name: entry.name.clone(),
        summary: entry.summary.clone(),
        accent: entry.accent.clone(),
        kind: if entry.is_basket() { "basket" } else { "rules" }.to_string(),
        starter: entry.is_starter(),
        fixed: entry.fixed.clone(),
        rules: entry.rules.document(),
        trades: outline::trades(&entry.rules.rebalance),
        outline: if entry.is_starter() { Vec::new() } else { outline::lines(&entry.rules) },
        test: test.map(|test| test.figures.clone()),
        holdings,
    })
}

fn strategy_views(tested: Option<&Tested>) -> Result<Vec<StrategyView>, ApiError> {
    catalogue::entries().iter().map(|entry| strategy_view(entry, tested)).collect()
}

fn find_strategy(id: &str) -> Result<&'static Entry, ApiError> {
    catalogue::find(id).ok_or_else(|| ApiError::not_found("no such strategy"))
}

/// What every route can reach: the price store, when there is one, and the catalogue's tests on it.
#[derive(Clone)]
struct Shared {
    prices: Option<Arc<Prices>>,
    tested: Option<Arc<Tested>>,
}

impl Shared {
    fn over(prices: Option<Arc<Prices>>) -> Shared {
        Shared { tested: prices.clone().map(|prices| Arc::new(Tested::on(prices))), prices }
    }
}

/// A test is plain arithmetic and can take a while, so anything that may run one
/// is worked out off the threads that answer requests.
async fn aside<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(work).await.map_err(|error| ApiError::broken(error.to_string()))
}

/// Every non-2xx response has this shape.
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
    issues: Vec<Value>,
}

impl ApiError {
    /// The problems of a strategy, whichever kind of strategy it is.
    fn invalid_strategy(issues: Vec<impl Serialize>) -> Self {
        let count = issues.len();
        let noun = if count == 1 { "problem" } else { "problems" };
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "invalid_strategy",
            message: format!("strategy has {count} {noun}"),
            issues: issues.iter().map(|issue| json!(issue)).collect(),
        }
    }

    /// The server cannot do this yet, through no fault of the request.
    fn unavailable(message: impl Into<String>) -> Self {
        Self { status: StatusCode::SERVICE_UNAVAILABLE, code: "unavailable", message: message.into(), issues: Vec::new() }
    }

    /// Something of ours went wrong. The reason goes to the log, not to the caller.
    fn broken(reason: String) -> Self {
        eprintln!("internal error: {reason}");
        Self { status: StatusCode::INTERNAL_SERVER_ERROR, code: "internal", message: "something went wrong on the server".into(), issues: Vec::new() }
    }

    fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "invalid_request",
            message: message.into(),
            issues: Vec::new(),
        }
    }

    fn not_found(message: &str) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
            issues: Vec::new(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "error": {
            "code": self.code,
            "message": self.message,
            "issues": self.issues,
        }});
        (self.status, Json(body)).into_response()
    }
}

/// A body that is not valid JSON for the route is the caller's error, said plainly.
impl From<JsonRejection> for ApiError {
    fn from(rejection: JsonRejection) -> Self {
        ApiError::invalid_request(rejection.body_text())
    }
}

type ApiResult = Result<Json<Value>, ApiError>;

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "version": env!("CARGO_PKG_VERSION") }))
}

async fn list_assets() -> Json<Value> {
    Json(json!({ "assets": assets().values().collect::<Vec<_>>() }))
}

async fn list_strategies(State(shared): State<Shared>) -> ApiResult {
    let views = aside(move || strategy_views(shared.tested.as_deref())).await??;
    Ok(Json(json!({ "strategies": views })))
}

async fn one_strategy(State(shared): State<Shared>, Path(id): Path<String>) -> ApiResult {
    let entry = find_strategy(&id)?;
    let view = aside(move || strategy_view(entry, shared.tested.as_deref())).await??;
    Ok(Json(json!({ "strategy": view })))
}

/// A fund's whole test, day by day, beside its benchmark: what the chart on its page draws.
async fn strategy_test(State(shared): State<Shared>, Path(id): Path<String>) -> Result<Response, ApiError> {
    let entry = find_strategy(&id)?;
    let Some(tested) = shared.tested else {
        return Err(ApiError::unavailable("this server has no price history to test a strategy on"));
    };
    let test = aside(move || tested.of(&entry.id, &entry.rules)).await?.ok_or_else(|| ApiError::unavailable("this strategy cannot be tested just now"))?;
    Ok(([(CONTENT_TYPE, HeaderValue::from_static("application/json"))], test.body.clone()).into_response())
}

/// Always 200: an invalid document is a normal answer here, with its problems listed.
async fn validate_strategy(body: Result<Json<Strategy>, JsonRejection>) -> ApiResult {
    let Json(strategy) = body?;
    let issues = strategy.validate();
    Ok(Json(json!({ "valid": issues.is_empty(), "issues": issues })))
}

async fn strategy_targets(body: Result<Json<Strategy>, JsonRejection>) -> ApiResult {
    let Json(strategy) = body?;
    let targets = strategy.targets().map_err(ApiError::invalid_strategy)?;
    Ok(Json(json!({ "holdings": holdings(targets) })))
}

async fn backtest_strategy(State(shared): State<Shared>, body: Bytes) -> ApiResult {
    let text = std::str::from_utf8(&body).map_err(|_| ApiError::invalid_request("the request is not text"))?;
    let ask = backtest::Ask::read(text)?;
    let Some(prices) = shared.prices else {
        return Err(ApiError::unavailable("this server has no price history to run a backtest on"));
    };
    // No more run at once than there are processors. The rest wait their turn.
    let _turn = turns().acquire().await.map_err(|error| ApiError::broken(error.to_string()))?;
    Ok(Json(aside(move || backtest::run(&prices, &ask)).await??))
}

fn turns() -> &'static Semaphore {
    static TURNS: OnceLock<Semaphore> = OnceLock::new();
    TURNS.get_or_init(|| Semaphore::new(std::thread::available_parallelism().map_or(1, usize::from)))
}

/// The named events the app may log at this step. A name outside the list is refused.
const EVENTS: [&str; 4] = ["home_seen", "explore_seen", "strategy_opened", "pricing_seen"];

/// One line of the app's own log: what happened, on which screen, to which strategy or fund.
/// There is no field for who, and an unknown field is refused, so none can creep in.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Event {
    name: String,
    screen: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
}

/// The line an event is logged as, or why it is refused. Every part is a short plain word,
/// so nothing a caller sends can break the log or hide in it.
fn event_line(event: &Event) -> Result<String, ApiError> {
    let plain = |text: &str| !text.is_empty() && text.len() <= 64 && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if !EVENTS.contains(&event.name.as_str()) {
        return Err(ApiError::invalid_request("no such event"));
    }
    if !plain(&event.screen) || !event.id.as_deref().is_none_or(plain) {
        return Err(ApiError::invalid_request("an event's screen and id are short plain words"));
    }
    Ok(format!("event {}", json!(event)))
}

async fn log_event(body: Result<Json<Event>, JsonRejection>) -> Result<StatusCode, ApiError> {
    let Json(event) = body?;
    println!("{}", event_line(&event)?);
    Ok(StatusCode::NO_CONTENT)
}

async fn unknown_api_route() -> ApiError {
    ApiError::not_found("no such route")
}

/// Built assets carry a content hash in their name and never change, so they
/// are cached for good. Everything else is revalidated on each load, so a new
/// build or new data is picked up at once.
async fn cache_policy(request: Request, next: Next) -> Response {
    let hashed_asset = request.uri().path().starts_with("/assets/");
    let mut response = next.run(request).await;
    let policy = if hashed_asset && response.status().is_success() {
        "public, max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    response.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static(policy));
    response
}

fn web_dist() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../web/dist")
}

/// What an address shows, looked up before a word of the page is written.
enum Lookup {
    Home(Vec<StrategyView>),
    Explore(Vec<StrategyView>),
    Pricing,
    Strategy(Box<StrategyView>),
    NotFound,
    /// The data for the page could not be made.
    Broke,
}

fn look_up(path: &str, tested: Option<&Tested>) -> Lookup {
    match page::route(path) {
        page::Route::Home => strategy_views(tested).map_or(Lookup::Broke, Lookup::Home),
        page::Route::Explore => strategy_views(tested).map_or(Lookup::Broke, Lookup::Explore),
        page::Route::Pricing => Lookup::Pricing,
        page::Route::Strategy(id) => match catalogue::find(&id) {
            Some(entry) => strategy_view(entry, tested).map_or(Lookup::Broke, |view| Lookup::Strategy(Box::new(view))),
            None => Lookup::NotFound,
        },
        page::Route::NotFound => Lookup::NotFound,
    }
}

/// The whole page for an address and the status that tells a machine what a person will read.
fn respond(shell: &str, path: &str, lookup: &Lookup) -> (StatusCode, Html<String>) {
    let (status, screen) = match lookup {
        Lookup::Home(views) => (StatusCode::OK, page::Screen::Home(views)),
        Lookup::Explore(views) => (StatusCode::OK, page::Screen::Explore(views)),
        Lookup::Pricing => (StatusCode::OK, page::Screen::Pricing),
        Lookup::Strategy(view) => (StatusCode::OK, page::Screen::Strategy(view)),
        Lookup::NotFound => (StatusCode::NOT_FOUND, page::Screen::NotFound),
        Lookup::Broke => (StatusCode::INTERNAL_SERVER_ERROR, page::Screen::Broke),
    };
    (status, Html(page::document(shell, path, &screen)))
}

/// Every address that is not a file or an API route: one of the app's screens, or the not-found page.
/// The shell is read on each request, so a new build of the web app is picked up at once.
async fn page_for(shell_file: PathBuf, tested: Option<Arc<Tested>>, uri: Uri) -> (StatusCode, Html<String>) {
    let built = tokio::fs::read_to_string(shell_file).await.unwrap_or_default();
    let path = uri.path().to_string();
    let lookup = aside(move || look_up(&path, tested.as_deref())).await.unwrap_or(Lookup::Broke);
    respond(page::shell_or_bare(&built), uri.path(), &lookup)
}

/// The API and the web app built into `dist`, on one router. `prices` is the price store
/// backtests run on, when there is one.
fn router(dist: PathBuf, prices: Option<Arc<Prices>>) -> Router {
    let shared = Shared::over(prices);
    let shell_file = dist.join("index.html");
    let tested = shared.tested.clone();
    let pages = move |uri: Uri| page_for(shell_file.clone(), tested.clone(), uri);
    // Files the build put beside the shell, such as the icon. Anything else is a page.
    let web = ServeDir::new(&dist).append_index_html_on_directories(false).fallback(pages.into_service());
    // The build leaves a compressed copy beside each asset, and a browser that asks for one gets it.
    let assets = ServeDir::new(dist.join("assets")).precompressed_br().precompressed_gzip();
    let api = Router::new()
        .route("/assets", get(list_assets))
        .route("/strategies", get(list_strategies))
        .route("/strategies/{id}", get(one_strategy))
        .route("/strategies/{id}/test", get(strategy_test))
        .route("/strategy/validate", post(validate_strategy))
        .route("/strategy/targets", post(strategy_targets))
        .route("/backtest", post(backtest_strategy))
        // An event is a few words. Anything larger is not one.
        .route("/events", post(log_event).layer(DefaultBodyLimit::max(512)))
        .fallback(unknown_api_route)
        .with_state(shared);
    Router::new()
        .route("/health", get(health))
        .nest("/v1", api)
        // A missing asset is a 404, never a page under an asset's name.
        .nest_service("/assets", assets)
        .fallback_service(web)
        // Pages and answers are sent compressed to a browser that asks. An asset already is.
        .layer(CompressionLayer::new())
        .layer(middleware::from_fn(cache_policy))
}

fn app(prices: Option<Arc<Prices>>) -> Router {
    router(web_dist(), prices)
}

#[tokio::main]
async fn main() {
    let port = std::env::var("PORT").ok().and_then(|port| port.parse().ok()).unwrap_or(DEFAULT_PORT);
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .unwrap_or_else(|e| panic!("cannot bind {address}: {e}"));
    // Backtests need daily prices. Without a store the rest of the app still runs.
    let prices = std::env::var_os("PRICE_STORE").map(|dir| Arc::new(Prices::at(dir)));
    println!("backend on http://{address}, {}", if prices.is_some() { "with a price store" } else { "with no price store: set PRICE_STORE to run backtests" });
    axum::serve(listener, app(prices)).await.expect("server stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::header;
    use tower::ServiceExt;

    /// A web build made for one test, so no test depends on what happens to be in `web/dist`.
    struct Build(PathBuf);

    impl Build {
        fn new(name: &str, files: &[(&str, &str)]) -> Self {
            let dir = std::env::temp_dir().join(format!("compounder-server-test-{}-{name}", std::process::id()));
            for (file, text) in files {
                let path = dir.join(file);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, text).unwrap();
            }
            Build(dir)
        }

        async fn get(&self, path: &str, accept_encoding: Option<&str>) -> Response {
            let mut request = Request::builder().uri(path);
            if let Some(encoding) = accept_encoding {
                request = request.header(header::ACCEPT_ENCODING, encoding);
            }
            router(self.0.clone(), None).oneshot(request.body(Body::empty()).unwrap()).await.unwrap()
        }
    }

    impl Drop for Build {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The server with no web build at all: pages come in the bare shell.
    fn unbuilt(prices: Option<Arc<Prices>>) -> Router {
        router(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("no-web-build-here"), prices)
    }

    async fn text_of(response: Response) -> String {
        String::from_utf8(axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap()
    }

    async fn call(method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        call_with(None, method, path, body).await
    }

    async fn call_with(prices: Option<Arc<Prices>>, method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        let body = match body {
            Some(json) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = unbuilt(prices).oneshot(request.body(body).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    /// A page as a browser gets it: the status, the type and the text.
    async fn page(path: &str) -> (StatusCode, String, String) {
        let request = Request::builder().uri(path).body(Body::empty()).unwrap();
        let response = unbuilt(None).oneshot(request).await.unwrap();
        let status = response.status();
        let kind = response.headers().get(header::CONTENT_TYPE).map(|value| value.to_str().unwrap().to_string()).unwrap_or_default();
        (status, kind, text_of(response).await)
    }

    /// The data a page carries for the app, read back out of the page.
    fn seed_in(text: &str) -> Value {
        let data = text.split(r#"<script type="application/json" id="seed">"#).nth(1).expect("the page carries its data");
        serde_json::from_str(data.split("</script>").next().unwrap()).expect("the data is valid")
    }

    fn two_asset_strategy(first: &str, second: &str) -> Value {
        json!({
            "entry": {
                "blockType": "topLevel", "name": "Test", "blockIdForInstance": "top", "parentNodeId": "root",
                "children": {
                    "blockType": "weight", "weightType": "specified-percentage", "weightings": [first, second],
                    "blockIdForInstance": "w", "parentNodeId": "top",
                    "children": [
                        { "blockType": "asset", "assetId": "0xa9ee28c80f960b889dfbd1902055218cba016f75:56", "blockIdForInstance": "a" },
                        { "blockType": "asset", "assetId": "0x00000000000000000000000000000000000000aa:56", "blockIdForInstance": "b" }
                    ]
                }
            },
            "version": "1.0.0",
            "tradingSettings": { "type": "threshold", "triggerPercentage": 10 }
        })
    }

    #[tokio::test]
    async fn health_reports_ok() {
        let (status, body) = call("GET", "/health", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["ok"], true);
    }

    #[tokio::test]
    async fn the_registry_lists_tokens_with_their_on_chain_identity() {
        let (status, body) = call("GET", "/v1/assets", None).await;
        assert_eq!(status, StatusCode::OK);
        let assets = body["assets"].as_array().unwrap();
        assert!(assets.len() > 100);
        for asset in assets {
            let id = asset["id"].as_str().unwrap();
            assert_eq!(id, format!("{}:{}", asset["address"].as_str().unwrap(), asset["chain_id"]));
            assert!(!asset["symbol"].as_str().unwrap().is_empty());
        }
    }

    #[tokio::test]
    async fn every_starter_is_fully_allocated_to_known_tokens() {
        let (status, body) = call("GET", "/v1/strategies", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let starters: Vec<&Value> = body["strategies"].as_array().unwrap().iter().filter(|strategy| strategy["starter"] == true).collect();
        assert_eq!(starters.len(), 3);
        for starter in starters {
            let holdings = starter["holdings"].as_array().unwrap();
            let total: f64 = holdings.iter().map(|h| h["weight"].as_f64().unwrap()).sum();
            assert!((total - 1.0).abs() < 1e-9, "{}: {total}", starter["id"]);
            for holding in holdings {
                assert!(holding["symbol"].is_string(), "{}: unknown token {}", starter["id"], holding["asset_id"]);
            }
            let weights: Vec<f64> = holdings.iter().map(|h| h["weight"].as_f64().unwrap()).collect();
            assert!(weights.windows(2).all(|pair| pair[0] >= pair[1]), "{}: not largest first", starter["id"]);
        }
    }

    #[tokio::test]
    async fn targets_name_known_tokens_and_leave_unknown_ones_unnamed() {
        let (status, body) = call("POST", "/v1/strategy/targets", Some(two_asset_strategy("70", "30"))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let holdings = body["holdings"].as_array().unwrap();
        assert_eq!(holdings[0]["symbol"], "NVDAon");
        assert_eq!(holdings[0]["weight"], 0.7);
        assert!(holdings[1]["symbol"].is_null());
    }

    #[tokio::test]
    async fn a_broken_strategy_returns_its_problems_with_block_ids() {
        let (status, body) = call("POST", "/v1/strategy/targets", Some(two_asset_strategy("70", "20"))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["code"], "invalid_strategy");
        assert_eq!(body["error"]["issues"][0]["code"], "INVALID_WEIGHT_TOTAL");
        assert_eq!(body["error"]["issues"][0]["block_id"], "w");
    }

    #[tokio::test]
    async fn validate_answers_ok_for_a_broken_strategy_and_lists_why() {
        let (status, body) = call("POST", "/v1/strategy/validate", Some(two_asset_strategy("70", "20"))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["valid"], false);
        assert_eq!(body["issues"].as_array().unwrap().len(), 1);
        let (_, body) = call("POST", "/v1/strategy/validate", Some(two_asset_strategy("70", "30"))).await;
        assert_eq!(body["valid"], true);
    }

    #[tokio::test]
    async fn a_malformed_body_is_an_invalid_request_not_a_crash() {
        let (status, body) = call("POST", "/v1/strategy/validate", Some(json!({ "entry": 7 }))).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn only_hashed_assets_are_cached_for_good() {
        let build = Build::new("cache", &[("assets/app-1a2b.js", "let a")]);
        let dist = || build.0.clone();
        let policy = |path: &'static str| async move {
            let request = Request::builder().uri(path).body(Body::empty()).unwrap();
            let response = router(dist(), None).oneshot(request).await.unwrap();
            response.headers().get(CACHE_CONTROL).map(|value| value.to_str().unwrap().to_string())
        };
        assert_eq!(policy("/v1/strategies").await.as_deref(), Some("no-cache"));
        assert_eq!(policy("/").await.as_deref(), Some("no-cache"));
        assert_eq!(policy("/s/hard-assets").await.as_deref(), Some("no-cache"));
        assert_eq!(policy("/assets/app-1a2b.js").await.as_deref(), Some("public, max-age=31536000, immutable"));
        // A missing asset must not be pinned in a browser's cache.
        assert_eq!(policy("/assets/missing-0000.js").await.as_deref(), Some("no-cache"));
    }

    #[tokio::test]
    async fn pages_are_written_into_the_built_shell_and_files_beside_it_are_served() {
        let shell = r#"<html><head><title>Compounder</title><link rel="stylesheet" href="/assets/app-1a2b.css"></head><body><div id="root"><!--page--></div><!--seed--><script type="module" src="/assets/app-1a2b.js"></script></body></html>"#;
        let build = Build::new("shell", &[("index.html", shell), ("icon.svg", "<svg/>"), ("assets/app-1a2b.js", "let a")]);
        let page = text_of(build.get("/s/hard-assets", None).await).await;
        assert!(page.contains(r#"<link rel="stylesheet" href="/assets/app-1a2b.css">"#));
        assert!(page.contains(r#"<div id="root"><a class="skip-link""#));
        assert!(page.contains(r#"<script type="application/json" id="seed">{"resources":{"/v1/strategies/hard-assets":"#));
        assert_eq!(text_of(build.get("/icon.svg", None).await).await, "<svg/>");
        // A folder of the build is not a page of the app.
        assert_eq!(build.get("/assets", None).await.status(), StatusCode::NOT_FOUND);
        assert_eq!(build.get("/assets/", None).await.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_shell_from_an_older_build_still_gets_the_words() {
        let build = Build::new("old-shell", &[("index.html", r#"<html><head><title>Compounder</title></head><body><div id="root"></div></body></html>"#)]);
        let response = build.get("/", None).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(text_of(response).await.contains("Build your own wealth fund."));
    }

    #[tokio::test]
    async fn an_asset_is_sent_compressed_to_a_browser_that_asks() {
        let build = Build::new("compressed", &[("assets/app-1a2b.js", "plain"), ("assets/app-1a2b.js.br", "smaller"), ("assets/app-1a2b.js.gz", "small")]);
        let encoding = |response: &Response| response.headers().get(header::CONTENT_ENCODING).map(|value| value.to_str().unwrap().to_string());

        let response = build.get("/assets/app-1a2b.js", Some("gzip, br")).await;
        assert_eq!(encoding(&response).as_deref(), Some("br"));
        assert_eq!(response.headers()[header::CONTENT_TYPE], "text/javascript");
        assert_eq!(text_of(response).await, "smaller");

        let response = build.get("/assets/app-1a2b.js", Some("gzip")).await;
        assert_eq!(encoding(&response).as_deref(), Some("gzip"));

        let response = build.get("/assets/app-1a2b.js", None).await;
        assert_eq!(encoding(&response), None);
        assert_eq!(text_of(response).await, "plain");
    }

    #[tokio::test]
    async fn one_fund_is_served_by_its_id() {
        let (status, body) = call("GET", "/v1/strategies/hard-assets", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["strategy"]["name"], "Hard Assets");
        let (_, list) = call("GET", "/v1/strategies", None).await;
        let listed = list["strategies"].as_array().unwrap().iter().find(|starter| starter["id"] == "hard-assets").unwrap();
        assert_eq!(&body["strategy"], listed);

        let (status, body) = call("GET", "/v1/strategies/no-such-fund", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "not_found");
    }

    #[tokio::test]
    async fn every_fund_has_an_id_an_address_can_carry_and_one_of_the_five_colours() {
        for starter in catalogue::entries() {
            assert_eq!(page::route(&format!("/s/{}", starter.id)), page::Route::Strategy(starter.id.clone()));
            assert!(["blue", "orange", "teal", "violet", "magenta"].contains(&starter.accent.as_str()), "{}: {}", starter.id, starter.accent);
        }
    }

    #[tokio::test]
    async fn the_home_page_has_every_starter_in_its_first_response() {
        let (status, kind, text) = page("/").await;
        assert_eq!(status, StatusCode::OK);
        assert!(kind.starts_with("text/html"), "{kind}");
        assert!(text.contains("<title>Compounder</title>"));
        assert!(text.contains("Build your own wealth fund."));
        for entry in catalogue::entries() {
            // The home page leads with the starters. The rest of the catalogue is on Explore.
            let listed = text.contains(&format!(r#"<a class="card-link" href="/s/{}">{}</a>"#, entry.id, entry.name));
            assert_eq!(listed, entry.is_starter(), "{}", entry.id);
        }
        // The same answer the app would otherwise have to ask for.
        let (_, asked) = call("GET", "/v1/strategies", None).await;
        assert_eq!(seed_in(&text)["resources"]["/v1/strategies"], asked);
        assert!(!text.contains(page::PAGE_PLACE) && !text.contains(page::SEED_PLACE));
    }

    #[tokio::test]
    async fn a_starter_page_has_its_words_and_numbers_in_its_first_response() {
        let (status, _, text) = page("/s/hard-assets").await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("<title>Hard Assets · Compounder</title>"));
        assert!(text.contains(r#"<span class="fund-dot" aria-hidden="true"></span>Hard Assets</h1>"#));
        assert!(text.contains("When a holding drifts 10% from its target"));
        assert!(text.contains("Adding money is not open yet."));
        let (_, asked) = call("GET", "/v1/strategies/hard-assets", None).await;
        let holdings = asked["strategy"]["holdings"].as_array().unwrap();
        assert!(text.contains(&format!(r#"<dt>Holdings</dt><dd><span class="number">{}</span></dd>"#, holdings.len())));
        // One row for each holding, each with its share.
        assert_eq!(text.matches(r#"<li class="row row-asset">"#).count(), holdings.len());
        let largest = page::percent(holdings[0]["weight"].as_f64().unwrap(), false);
        assert!(text.contains(&format!(r#"<span class="number">{largest}</span>"#)), "{largest}");
        assert_eq!(seed_in(&text)["resources"]["/v1/strategies/hard-assets"], asked);
    }

    #[tokio::test]
    async fn the_pricing_page_says_what_it_costs_in_its_first_response() {
        let (status, kind, text) = page("/pricing").await;
        assert_eq!(status, StatusCode::OK);
        assert!(kind.starts_with("text/html"), "{kind}");
        assert!(text.contains("<title>Pricing · Compounder</title>"), "{text}");
        assert!(text.contains("$20") && text.contains("a month") && text.contains("Run one fund"), "{text}");
        // The home page says the price as well, with the way to this page.
        let (_, _, home) = page("/").await;
        assert!(home.contains("$20 a month") && home.contains(r#"href="/pricing""#), "{home}");
    }

    #[tokio::test]
    async fn explore_lists_every_fund_and_a_rule_funds_page_says_its_rules() {
        let (status, _, text) = page("/explore").await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("<title>Explore · Compounder</title>"), "{text}");
        for entry in catalogue::entries() {
            assert!(text.contains(&format!(r#"<a class="card-link" href="/s/{}">{}</a>"#, entry.id, entry.name)), "{}", entry.id);
        }
        let (_, asked) = call("GET", "/v1/strategies", None).await;
        assert_eq!(seed_in(&text)["resources"]["/v1/strategies"], asked);

        let (status, _, text) = page("/s/trend-switch").await;
        assert_eq!(status, StatusCode::OK);
        assert!(text.contains("If SPY&#x27;s price is above its 200-day average price"), "{text}");
        assert!(text.contains("<dt>Trades</dt><dd>Checked every trading day</dd>"), "{text}");
        // With no prices there is no test, so the page says nothing of results or holdings.
        assert!(!text.contains("strategy-results") && !text.contains("strategy-holdings"), "{text}");
    }

    #[tokio::test]
    async fn an_address_that_matches_nothing_is_the_not_found_page_with_a_404() {
        for path in ["/nope", "/s/no-such-fund", "/s/", "/s/hard-assets/more", "/s/hard%2Dassets", "/explore/more"] {
            let (status, kind, text) = page(path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert!(kind.starts_with("text/html"), "{path}: {kind}");
            assert!(text.contains("That page does not exist."), "{path}");
            assert!(text.contains("<title>Page not found · Compounder</title>"), "{path}");
        }
        // The app is told the starter is not there, so it does not ask again.
        let (_, _, text) = page("/s/no-such-fund").await;
        assert!(seed_in(&text)["resources"]["/v1/strategies/no-such-fund"].is_null());
        assert_eq!(seed_in(&text)["resources"].as_object().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_page_that_could_not_be_made_says_so_with_a_500() {
        let (status, Html(text)) = respond(page::BARE_SHELL, "/s/hard-assets", &Lookup::Broke);
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert!(text.contains("Something went wrong on our side."));
        assert!(text.contains(r#"<a class="button button-primary" href="/s/hard-assets">Try again</a>"#));
        assert_eq!(seed_in(&text)["broke"], true);
    }

    #[tokio::test]
    async fn a_named_event_is_logged_and_anything_else_is_refused() {
        let (status, _) = call("POST", "/v1/events", Some(json!({ "name": "home_seen", "screen": "home" }))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = call("POST", "/v1/events", Some(json!({ "name": "strategy_opened", "screen": "strategy", "id": "hard-assets" }))).await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let refused = [
            json!({ "name": "made_up", "screen": "home" }),
            json!({ "name": "home_seen" }),
            json!({ "name": "home_seen", "screen": "" }),
            json!({ "name": "home_seen", "screen": "home\nevent forged" }),
            json!({ "name": "strategy_opened", "screen": "strategy", "id": "a b" }),
            json!({ "name": "home_seen", "screen": "home", "email": "someone@example.com" }),
            json!({ "name": "home_seen", "screen": "x".repeat(65) }),
        ];
        for body in refused {
            let (status, answer) = call("POST", "/v1/events", Some(body.clone())).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
            assert_eq!(answer["error"]["code"], "invalid_request", "{body}");
        }
        // A body that would be a fine event, padded past what an event can be.
        let padded = format!(r#"{{"name":"home_seen","screen":"home"}}{}"#, " ".repeat(600));
        let request = Request::builder().method("POST").uri("/v1/events").header(header::CONTENT_TYPE, "application/json").body(Body::from(padded)).unwrap();
        assert_eq!(unbuilt(None).oneshot(request).await.unwrap().status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn an_event_is_one_line_with_no_room_for_who() {
        let event = Event { name: "strategy_opened".into(), screen: "strategy".into(), id: Some("hard-assets".into()) };
        assert_eq!(event_line(&event).ok().as_deref(), Some(r#"event {"id":"hard-assets","name":"strategy_opened","screen":"strategy"}"#));
        let event = Event { name: "home_seen".into(), screen: "home".into(), id: None };
        assert_eq!(event_line(&event).ok().as_deref(), Some(r#"event {"name":"home_seen","screen":"home"}"#));
    }

    #[tokio::test]
    async fn an_unknown_api_route_is_a_json_404() {
        let (status, body) = call("GET", "/v1/nope", None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "not_found");
    }

    /// A price store of made-up prices in a folder of its own, taken away
    /// again when the test is done.
    struct MadeUp {
        dir: PathBuf,
    }

    impl MadeUp {
        /// 300 weekdays from the start of 2024. SPY, KO and NVDA climb and TLT falls.
        fn new(name: &str) -> MadeUp {
            use compounder_prices::{adjust::Bar, day::Day, store::Store};
            let dir = std::env::temp_dir().join(format!("compounder-server-{}-{name}", std::process::id()));
            let mut store = Store::open(&dir).unwrap();
            // 1970-01-01 was a Thursday, so days 2 and 3 of each seven are a weekend.
            let first = Day::from_ymd(2024, 1, 1).unwrap().number();
            let days: Vec<Day> = (first..).map(Day::from_number).filter(|day| !matches!(day.number().rem_euclid(7), 2 | 3)).take(300).collect();
            for (ticker, start, step) in [("SPY", 100.0, 0.5), ("KO", 50.0, 0.125), ("NVDA", 20.0, 1.0), ("TLT", 200.0, -0.25)] {
                let bars: Vec<Bar> = days.iter().enumerate().map(|(index, day)| Bar::new(*day, start + step * index as f64)).collect();
                store.save(ticker, &bars).unwrap();
            }
            MadeUp { dir }
        }

        /// The same store with a climbing price for every share a fund in the catalogue needs, so each can be tested.
        fn with_every_share_a_fund_needs(name: &str) -> MadeUp {
            use compounder_prices::{adjust::Bar, store::Store};
            let made = MadeUp::new(name);
            let mut store = Store::open(&made.dir).unwrap();
            let days: Vec<_> = store.find("SPY").unwrap().unwrap().iter().map(|bar| bar.day).collect();
            let needed: std::collections::BTreeSet<String> = catalogue::entries().iter().flat_map(|entry| entry.rules.assets()).filter_map(|asset| backtest::ticker(&asset)).collect();
            for (place, ticker) in needed.iter().filter(|ticker| !["SPY", "KO", "NVDA", "TLT"].contains(&ticker.as_str())).enumerate() {
                // Each share climbs at its own pace, so a rule that ranks them has something to rank.
                let bars: Vec<Bar> = days.iter().enumerate().map(|(index, day)| Bar::new(*day, 40.0 + (0.05 + 0.01 * place as f64) * index as f64)).collect();
                store.save(ticker, &bars).unwrap();
            }
            made
        }

        fn prices(&self) -> Option<Arc<Prices>> {
            Some(Arc::new(Prices::at(&self.dir)))
        }
    }

    impl Drop for MadeUp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn listed(symbol: &str) -> Value {
        json!({ "block": "asset", "asset": { "market": "equities", "symbol": symbol } })
    }

    fn equal_monthly(children: Vec<Value>) -> Value {
        json!({ "name": "Test", "rebalance": { "type": "calendar", "frequency": "monthly" }, "root": { "block": "weight", "weighting": { "type": "equal" }, "children": children } })
    }

    #[tokio::test]
    async fn a_backtest_gives_daily_values_statistics_and_a_benchmark() {
        let store = MadeUp::new("values");
        let request = json!({ "strategy": equal_monthly(vec![listed("SPY"), listed("TLT")]), "benchmark": "SPY", "trades": true });
        let (status, body) = call_with(store.prices(), "POST", "/v1/backtest", Some(request)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!((body["first_day"].as_str(), body["last_day"].as_str()), (Some("2024-01-01"), Some("2025-02-21")));
        let values = body["values"].as_array().unwrap();
        assert_eq!((body["days"].as_array().unwrap().len(), values.len()), (300, 300));
        // The first day buys with the whole capital and pays 15 basis points on it.
        let first = values[0].as_f64().unwrap();
        assert!((first - 10_000.0 / 1.0015).abs() < 0.01, "{first}");
        // One trading day for each month from January 2024 to February 2025.
        assert_eq!(body["trading_days"], 14);
        assert_eq!(body["trades"].as_array().unwrap().len(), 14);
        assert_eq!(body["trades"][0]["weights"], json!([0.5, 0.5]));
        assert_eq!(body["assets"][0]["symbol"], "SPY");
        assert_eq!(body["hash"].as_str().map(str::len), Some(64));
        assert!(body["stats"]["total_return"].is_number() && body["stats"]["max_drawdown"].is_number());
        // The benchmark starts level with the run's capital and has its own figures.
        assert_eq!(body["benchmark"]["symbol"], "SPY");
        assert_eq!(body["benchmark"]["values"][0], 10_000.0);
        assert!(body["benchmark"]["beta"].is_number() && body["benchmark"]["stats"]["total_return"].is_number());
        // Without being asked, the trading days are counted and not listed.
        let (_, brief) = call_with(store.prices(), "POST", "/v1/backtest", Some(json!({ "strategy": equal_monthly(vec![listed("SPY")]), "capital": 500 }))).await;
        assert!(brief.get("trades").is_none() && brief["benchmark"].is_null());
        assert_eq!(brief["capital"], 500.0);
    }

    #[tokio::test]
    async fn a_benchmark_with_less_history_is_compared_over_its_own_days_and_says_so() {
        use compounder_prices::{adjust::Bar, day::Day, store::Store};
        let store = MadeUp::new("shorter");
        // LATE starts on the run's 101st day at 10 and gains a tenth of a dollar a day.
        let first = Day::from_ymd(2024, 1, 1).unwrap().number();
        let days: Vec<Day> = (first..).map(Day::from_number).filter(|day| !matches!(day.number().rem_euclid(7), 2 | 3)).take(300).collect();
        let bars: Vec<Bar> = days[100..].iter().enumerate().map(|(index, day)| Bar::new(*day, 10.0 + 0.1 * index as f64)).collect();
        Store::open(&store.dir).unwrap().save("LATE", &bars).unwrap();

        let request = json!({ "strategy": equal_monthly(vec![listed("SPY")]), "benchmark": "LATE", "slippage": 0 });
        let (status, body) = call_with(store.prices(), "POST", "/v1/backtest", Some(request)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let benchmark = &body["benchmark"];
        assert_eq!((benchmark["first_day"].as_str(), benchmark["last_day"].as_str()), (Some(days[100].to_string().as_str()), Some("2025-02-21")));
        // No value before its first day, and level with the strategy on it.
        assert!(benchmark["values"][99].is_null());
        assert_eq!(benchmark["values"][100], body["values"][100]);
        // SPY goes from 150 to 249.5 over those days and LATE from 10 to 29.9.
        assert!((benchmark["strategy_return"].as_f64().unwrap() - (249.5 / 150.0 - 1.0)).abs() < 1e-9);
        assert!((benchmark["stats"]["total_return"].as_f64().unwrap() - 1.99).abs() < 1e-9);
        assert_eq!(body["warnings"][0]["code"], "BENCHMARK_SHORTER");
        // A benchmark that covers the whole run needs no such warning.
        let (_, whole) = call_with(store.prices(), "POST", "/v1/backtest", Some(json!({ "strategy": equal_monthly(vec![listed("SPY")]), "benchmark": "KO" }))).await;
        assert_eq!(whole["warnings"], json!([]));
        assert_eq!(whole["benchmark"]["strategy_return"], whole["stats"]["total_return"]);
    }

    #[tokio::test]
    async fn a_token_is_tested_on_the_share_it_tracks() {
        let store = MadeUp::new("token");
        let token = assets().values().find(|asset| asset.tracks.as_deref() == Some("NVDA")).expect("a token that tracks NVDA");
        let strategy = equal_monthly(vec![json!({ "block": "asset", "asset": { "token": token.id } })]);
        let (status, body) = call_with(store.prices(), "POST", "/v1/backtest", Some(json!({ "strategy": strategy }))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["assets"][0]["symbol"], token.symbol.as_str());
        // NVDA goes from 20 to 319 over the run.
        assert!(body["stats"]["total_return"].as_f64().unwrap() > 14.0);
    }

    #[tokio::test]
    async fn a_block_tree_document_can_be_tested_as_it_is() {
        let store = MadeUp::new("import");
        let document = json!({ "step": "root", "name": "Two", "rebalance": "daily", "children": [
            { "step": "wt-cash-equal", "children": [{ "step": "asset", "ticker": "SPY" }, { "step": "asset", "ticker": "TLT" }] }
        ] });
        let (status, body) = call_with(store.prices(), "POST", "/v1/backtest", Some(json!({ "import": document }))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["trading_days"], 300);
    }

    #[tokio::test]
    async fn an_asset_with_no_prices_is_named() {
        let store = MadeUp::new("missing");
        let request = json!({ "strategy": equal_monthly(vec![listed("SPY"), listed("ZZZZ")]) });
        let (status, body) = call_with(store.prices(), "POST", "/v1/backtest", Some(request)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"]["code"], "invalid_strategy");
        assert_eq!(body["error"]["issues"][0]["code"], "MISSING_HISTORY");
        assert!(body["error"]["issues"][0]["message"].as_str().unwrap().contains("ZZZZ"));
    }

    #[tokio::test]
    async fn a_backtest_request_that_is_wrong_says_how() {
        let store = MadeUp::new("wrong");
        let strategy = equal_monthly(vec![listed("SPY")]);
        for (request, said) in [
            (json!({ "strategy": strategy, "leverage": 2 }), "leverage"),
            (json!({ "strategy": strategy, "import": {} }), "not both"),
            (json!({}), "not both"),
            (json!({ "strategy": strategy, "start": "last year" }), "start"),
            (json!({ "strategy": strategy, "capital": "lots" }), "capital"),
            (json!({ "strategy": strategy, "benchmark": "ZZZZ" }), "ZZZZ"),
            (json!([1, 2]), "not a JSON object"),
        ] {
            let (status, body) = call_with(store.prices(), "POST", "/v1/backtest", Some(request)).await;
            assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_request")), "{body}");
            assert!(body["error"]["message"].as_str().unwrap().contains(said), "{body}");
        }
        // A strategy with a problem is a problem of the strategy, listed with its place.
        let (status, body) = call_with(store.prices(), "POST", "/v1/backtest", Some(json!({ "strategy": equal_monthly(vec![]) }))).await;
        assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("invalid_strategy")));
        // And so is a run it cannot be given.
        let (status, body) = call_with(store.prices(), "POST", "/v1/backtest", Some(json!({ "strategy": strategy, "capital": -5 }))).await;
        assert_eq!((status, body["error"]["issues"][0]["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("INVALID_RUN")));
        let (status, body) = call_with(store.prices(), "POST", "/v1/backtest", Some(json!({ "strategy": strategy, "capital": 1e308 }))).await;
        assert_eq!((status, body["error"]["issues"][0]["code"].as_str()), (StatusCode::UNPROCESSABLE_ENTITY, Some("INVALID_RUN")));
    }

    #[tokio::test]
    async fn with_no_price_store_a_backtest_is_unavailable() {
        let request = json!({ "strategy": equal_monthly(vec![listed("SPY")]) });
        let (status, body) = call("POST", "/v1/backtest", Some(request)).await;
        assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("unavailable")));
    }

    #[tokio::test]
    async fn every_fund_comes_with_the_rule_document_a_backtest_takes() {
        let (_, body) = call("GET", "/v1/strategies", None).await;
        for starter in body["strategies"].as_array().unwrap() {
            let rules = compounder_engine::rules::RuleStrategy::from_document(&starter["rules"]).expect("a sound document");
            assert_eq!(rules.name, starter["name"].as_str().unwrap());
            assert!(rules.validate(compounder_engine::rules::MAX_NODES).is_empty());
        }
    }

    #[tokio::test]
    async fn a_fund_carries_what_its_test_came_to_where_there_are_prices() {
        let store = MadeUp::with_every_share_a_fund_needs("figures");
        let (status, body) = call_with(store.prices(), "GET", "/v1/strategies", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        for strategy in body["strategies"].as_array().unwrap() {
            let test = &strategy["test"];
            assert_eq!(test["last_day"], "2025-02-21", "{strategy}");
            assert!(test["per_year"].as_f64().unwrap() > 0.0, "{test}");
            assert!(test["worst_drop"].as_f64().unwrap() >= 0.0, "{test}");
            assert_eq!(test["cost"], 0.0015);
            if strategy["starter"] == true {
                // A basket of tokens starts on the store's first day and stands beside the benchmark over the same days.
                assert_eq!(test["first_day"], "2024-01-01", "{strategy}");
                assert!(test["sharpe"].is_number() && test["benchmark_per_year"].is_number(), "{test}");
                assert!(strategy.get("outline").is_none() && strategy["fixed"].is_object(), "{strategy}");
            } else {
                // Any other fund says its rules in words and shows what its test held on the last day.
                assert!(!strategy["outline"].as_array().unwrap().is_empty() && strategy.get("fixed").is_none(), "{strategy}");
                let held: f64 = strategy["holdings"].as_array().unwrap().iter().map(|holding| holding["weight"].as_f64().unwrap()).sum();
                assert!((held - 1.0).abs() < 0.01, "{}: holds {held}", strategy["id"]);
            }
        }
        let id = body["strategies"][0]["id"].as_str().unwrap();
        let (_, one) = call_with(store.prices(), "GET", &format!("/v1/strategies/{id}"), None).await;
        assert_eq!(one["strategy"]["test"], body["strategies"][0]["test"]);
        // The page has the figures in its first response as well.
        let request = Request::builder().uri(format!("/s/{id}")).body(Body::empty()).unwrap();
        let page = text_of(unbuilt(store.prices()).oneshot(request).await.unwrap()).await;
        assert_eq!(seed_in(&page)["resources"][format!("/v1/strategies/{id}")]["strategy"]["test"], body["strategies"][0]["test"]);
    }

    #[tokio::test]
    async fn a_fund_with_no_prices_to_test_it_on_is_listed_without_figures() {
        for prices in [None, MadeUp::new("untested").prices()] {
            let (status, body) = call_with(prices, "GET", "/v1/strategies", None).await;
            assert_eq!(status, StatusCode::OK);
            assert!(body["strategies"].as_array().unwrap().iter().all(|starter| starter.get("test").is_none()), "{body}");
        }
    }

    #[tokio::test]
    async fn a_funds_whole_test_is_served_for_its_chart() {
        let store = MadeUp::with_every_share_a_fund_needs("series");
        let id = &catalogue::entries()[0].id;
        let (status, body) = call_with(store.prices(), "GET", &format!("/v1/strategies/{id}/test"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["days"].as_array().unwrap().len(), 300);
        assert_eq!(body["values"].as_array().unwrap().len(), 300);
        assert_eq!(body["benchmark"]["values"].as_array().unwrap().len(), 300);
        assert_eq!((body["benchmark"]["symbol"].as_str(), body["slippage"].as_f64()), (Some("SPY"), Some(0.0015)));

        let (status, body) = call_with(store.prices(), "GET", "/v1/strategies/no-such-fund/test", None).await;
        assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::NOT_FOUND, Some("not_found")));
        let (status, body) = call("GET", &format!("/v1/strategies/{id}/test"), None).await;
        assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("unavailable")));
        let (status, body) = call_with(MadeUp::new("series-short").prices(), "GET", &format!("/v1/strategies/{id}/test"), None).await;
        assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("unavailable")));
    }

    #[tokio::test]
    async fn an_answer_is_sent_compressed_to_a_browser_that_asks() {
        let request = |encoding: Option<&str>| {
            let mut request = Request::builder().uri("/v1/strategies");
            if let Some(encoding) = encoding {
                request = request.header(header::ACCEPT_ENCODING, encoding);
            }
            unbuilt(None).oneshot(request.body(Body::empty()).unwrap())
        };
        let plain = request(None).await.unwrap();
        assert!(plain.headers().get(header::CONTENT_ENCODING).is_none());
        let plain = axum::body::to_bytes(plain.into_body(), usize::MAX).await.unwrap();
        for encoding in ["gzip", "br"] {
            let packed = request(Some(encoding)).await.unwrap();
            assert_eq!(packed.headers().get(header::CONTENT_ENCODING).unwrap(), encoding);
            let packed = axum::body::to_bytes(packed.into_body(), usize::MAX).await.unwrap();
            assert!(packed.len() * 2 < plain.len(), "{encoding}: {} of {}", packed.len(), plain.len());
        }
    }

    /// With `PRICE_STORE` naming a real store, every starter and one rule
    /// strategy are run on it and what each came to is printed. Skipped
    /// quietly otherwise.
    #[tokio::test]
    async fn a_real_store_runs_every_starter_and_a_rule_strategy() {
        let Some(dir) = std::env::var_os("PRICE_STORE") else { return };
        let prices = Some(Arc::new(Prices::at(dir)));
        let (_, starters) = call("GET", "/v1/strategies", None).await;
        let mut strategies: Vec<Value> = starters["strategies"].as_array().unwrap().iter().map(|starter| starter["rules"].clone()).collect();
        // Hold the fast fund while the broad one is above its 200 day average and not overbought, else bills.
        let reading = |indicator: &str, window: u32, symbol: &str| json!({ "indicator": indicator, "window": window, "asset": { "market": "equities", "symbol": symbol } });
        strategies.push(json!({ "name": "Trend with a brake", "rebalance": { "type": "calendar", "frequency": "daily" }, "root": { "block": "if",
            "condition": { "all": [
                { "compare": "above", "left": { "indicator": "price", "asset": { "market": "equities", "symbol": "SPY" } }, "right": reading("average_price", 200, "SPY") },
                { "compare": "below", "left": reading("relative_strength", 10, "TQQQ"), "right": { "fixed": 0.79 } }
            ] },
            "then": [listed("TQQQ")], "else": [listed("BIL")] } }));
        for strategy in strategies {
            let started = std::time::Instant::now();
            let (status, body) = call_with(prices.clone(), "POST", "/v1/backtest", Some(json!({ "strategy": strategy, "benchmark": "SPY" }))).await;
            assert_eq!(status, StatusCode::OK, "{}: {body}", strategy["name"]);
            println!(
                "{:28} {} to {}  {:5} days  return {:8.1}%  a year {:6.1}%  worst fall {:5.1}%  SPY {:7.1}%  trades {:5}  cost {:9.2}  {} warnings  {:?}",
                strategy["name"].as_str().unwrap(),
                body["first_day"].as_str().unwrap(),
                body["last_day"].as_str().unwrap(),
                body["days"].as_array().unwrap().len(),
                body["stats"]["total_return"].as_f64().unwrap() * 100.0,
                body["stats"]["annual_return"].as_f64().unwrap() * 100.0,
                body["stats"]["max_drawdown"].as_f64().unwrap() * 100.0,
                body["benchmark"]["stats"]["total_return"].as_f64().unwrap() * 100.0,
                body["trading_days"],
                body["cost"].as_f64().unwrap(),
                body["warnings"].as_array().unwrap().len(),
                started.elapsed(),
            );
        }
    }
}
