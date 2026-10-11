//! Local web backend: the strategy engine behind an HTTP API, plus the built
//! web app served from one port.

mod backtest;
mod prices;

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

use axum::{
    body::Bytes,
    extract::{rejection::JsonRejection, Request, State},
    http::{header::CACHE_CONTROL, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use compounder_engine::{rules::RuleStrategy, strategy::Strategy};
use prices::Prices;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tower_http::services::{ServeDir, ServeFile};

const BIND: ([u8; 4], u16) = ([127, 0, 0, 1], 8787);

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

#[derive(Deserialize)]
struct Starter {
    id: String,
    name: String,
    summary: String,
    accent: String,
    strategy: Strategy,
}

#[derive(Deserialize)]
struct StarterFile {
    starters: Vec<Starter>,
}

fn assets() -> &'static BTreeMap<String, Asset> {
    static ASSETS: OnceLock<BTreeMap<String, Asset>> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let file: AssetFile = serde_json::from_str(include_str!("../data/assets.json")).expect("data/assets.json is valid");
        file.assets.into_iter().map(|asset| (asset.id.clone(), asset)).collect()
    })
}

fn starters() -> &'static Vec<Starter> {
    static STARTERS: OnceLock<Vec<Starter>> = OnceLock::new();
    STARTERS.get_or_init(|| {
        let file: StarterFile = serde_json::from_str(include_str!("../data/starters.json")).expect("data/starters.json is valid");
        file.starters
    })
}

/// One line of a resolved portfolio: an asset and its share.
#[derive(Serialize)]
struct Holding {
    asset_id: String,
    /// Null when the asset is not in the registry.
    symbol: Option<String>,
    name: Option<String>,
    weight: f64,
}

/// Largest share first; ties in asset-id order so the list is stable.
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
    rows.sort_by(|a, b| b.weight.total_cmp(&a.weight).then_with(|| a.asset_id.cmp(&b.asset_id)));
    rows
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

async fn list_starters() -> ApiResult {
    let mut out = Vec::new();
    for starter in starters() {
        let targets = starter.strategy.targets().map_err(ApiError::invalid_strategy)?;
        let rules = RuleStrategy::from_fixed(&starter.strategy).map_err(ApiError::invalid_strategy)?;
        out.push(json!({
            "id": starter.id,
            "name": starter.name,
            "summary": starter.summary,
            "accent": starter.accent,
            "strategy": starter.strategy,
            // The same fund as a rule document, which is what a backtest takes.
            "rules": rules.document(),
            "holdings": holdings(targets),
        }));
    }
    Ok(Json(json!({ "starters": out })))
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

/// A backtest is plain arithmetic and can take a while, so it runs off the
/// threads that answer requests.
async fn backtest_strategy(State(prices): State<Option<Arc<Prices>>>, body: Bytes) -> ApiResult {
    let text = std::str::from_utf8(&body).map_err(|_| ApiError::invalid_request("the request is not text"))?;
    let ask = backtest::Ask::read(text)?;
    let Some(prices) = prices else {
        return Err(ApiError::unavailable("this server has no price history to run a backtest on"));
    };
    let answer = tokio::task::spawn_blocking(move || backtest::run(&prices, &ask)).await.map_err(|error| ApiError::broken(error.to_string()))?;
    Ok(Json(answer?))
}

async fn unknown_api_route() -> ApiError {
    ApiError {
        status: StatusCode::NOT_FOUND,
        code: "not_found",
        message: "no such route".into(),
        issues: Vec::new(),
    }
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

/// The whole app. `prices` is the price store backtests run on, when there is one.
fn app(prices: Option<Arc<Prices>>) -> Router {
    let dist = web_dist();
    // Unknown paths fall back to the app shell so client-side routes reload cleanly.
    let web = ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html")));
    let api = Router::new()
        .route("/assets", get(list_assets))
        .route("/starters", get(list_starters))
        .route("/strategy/validate", post(validate_strategy))
        .route("/strategy/targets", post(strategy_targets))
        .route("/backtest", post(backtest_strategy))
        .fallback(unknown_api_route)
        .with_state(prices);
    Router::new()
        .route("/health", get(health))
        .nest("/v1", api)
        // A missing asset is a 404, never the app shell under an asset's name.
        .nest_service("/assets", ServeDir::new(dist.join("assets")))
        .fallback_service(web)
        .layer(middleware::from_fn(cache_policy))
}

#[tokio::main]
async fn main() {
    let address = SocketAddr::from(BIND);
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
        let response = app(prices).oneshot(request.body(body).unwrap()).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
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
        let (status, body) = call("GET", "/v1/starters", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let starters = body["starters"].as_array().unwrap();
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
        let policy = |path: &'static str| async move {
            let request = Request::builder().uri(path).body(Body::empty()).unwrap();
            let response = app(None).oneshot(request).await.unwrap();
            response.headers().get(CACHE_CONTROL).map(|value| value.to_str().unwrap().to_string())
        };
        assert_eq!(policy("/v1/starters").await.as_deref(), Some("no-cache"));
        assert_eq!(policy("/").await.as_deref(), Some("no-cache"));
        // A missing asset must not be pinned in a browser's cache.
        assert_eq!(policy("/assets/missing-0000.js").await.as_deref(), Some("no-cache"));
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
    }

    #[tokio::test]
    async fn with_no_price_store_a_backtest_is_unavailable() {
        let request = json!({ "strategy": equal_monthly(vec![listed("SPY")]) });
        let (status, body) = call("POST", "/v1/backtest", Some(request)).await;
        assert_eq!((status, body["error"]["code"].as_str()), (StatusCode::SERVICE_UNAVAILABLE, Some("unavailable")));
    }

    #[tokio::test]
    async fn every_starter_comes_with_the_rule_document_a_backtest_takes() {
        let (_, body) = call("GET", "/v1/starters", None).await;
        for starter in body["starters"].as_array().unwrap() {
            let rules = RuleStrategy::from_document(&starter["rules"]).expect("a sound document");
            assert_eq!(rules.name, starter["name"].as_str().unwrap());
            assert!(rules.validate(compounder_engine::rules::MAX_NODES).is_empty());
        }
    }

    /// With `PRICE_STORE` naming a real store, every starter and one rule
    /// strategy are run on it and what each came to is printed. Skipped
    /// quietly otherwise.
    #[tokio::test]
    async fn a_real_store_runs_every_starter_and_a_rule_strategy() {
        let Some(dir) = std::env::var_os("PRICE_STORE") else { return };
        let prices = Some(Arc::new(Prices::at(dir)));
        let (_, starters) = call("GET", "/v1/starters", None).await;
        let mut strategies: Vec<Value> = starters["starters"].as_array().unwrap().iter().map(|starter| starter["rules"].clone()).collect();
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
