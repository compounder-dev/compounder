//! Local web backend: the strategy engine behind an HTTP API, plus the built
//! web app served from one port.

use std::{collections::BTreeMap, net::SocketAddr, path::PathBuf, sync::OnceLock};

use axum::{
    extract::{rejection::JsonRejection, Request},
    http::{header::CACHE_CONTROL, HeaderValue, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use compounder_engine::strategy::{Issue, Strategy};
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
    issues: Vec<Issue>,
}

impl ApiError {
    fn invalid_strategy(issues: Vec<Issue>) -> Self {
        let count = issues.len();
        let noun = if count == 1 { "problem" } else { "problems" };
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            code: "invalid_strategy",
            message: format!("strategy has {count} {noun}"),
            issues,
        }
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
        out.push(json!({
            "id": starter.id,
            "name": starter.name,
            "summary": starter.summary,
            "accent": starter.accent,
            "strategy": starter.strategy,
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

fn app() -> Router {
    let dist = web_dist();
    // Unknown paths fall back to the app shell so client-side routes reload cleanly.
    let web = ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html")));
    let api = Router::new()
        .route("/assets", get(list_assets))
        .route("/starters", get(list_starters))
        .route("/strategy/validate", post(validate_strategy))
        .route("/strategy/targets", post(strategy_targets))
        .fallback(unknown_api_route);
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
    println!("backend on http://{address}");
    axum::serve(listener, app()).await.expect("server stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::header;
    use tower::ServiceExt;

    async fn call(method: &str, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        let body = match body {
            Some(json) => {
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(json.to_string())
            }
            None => Body::empty(),
        };
        let response = app().oneshot(request.body(body).unwrap()).await.unwrap();
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
            let response = app().oneshot(request).await.unwrap();
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
}
