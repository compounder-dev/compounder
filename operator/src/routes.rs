//! Market routes: where to trade a token for cash, or cash for a token, and
//! the exact call that does it. Routes come from a public swap-routing
//! service and are executed unchanged by the settlement contract, which
//! checks the outcome itself.

use alloy::primitives::{Address, Bytes, U256};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};

#[derive(Clone)]
pub struct Route {
    pub router: Address,
    pub call: Bytes,
    /// What the route expects to return.
    pub amount_out: U256,
    /// The least it may return before the router itself reverts.
    pub min_out: U256,
    pub venues: Vec<String>,
}

fn text<'a>(value: &'a Value, path: &[&str]) -> Result<&'a str> {
    let mut at = value;
    for key in path {
        at = at.get(key).ok_or_else(|| anyhow!("the routing service's answer has no `{}`", path.join(".")))?;
    }
    at.as_str().ok_or_else(|| anyhow!("`{}` is not text", path.join(".")))
}

/// A route the service has found, not yet turned into a call.
pub struct Found {
    summary: Value,
    /// What the route is expected to return. The firm route built from it returns the same.
    pub amount_out: U256,
}

/// Find the best route that spends exactly `amount_in` of `token_in`.
/// `only_venues` limits the route to a comma-separated list of venues.
pub async fn find(
    http: &reqwest::Client,
    api: &str,
    token_in: Address,
    token_out: Address,
    amount_in: U256,
    only_venues: Option<&str>,
) -> Result<Found> {
    let mut query = vec![("tokenIn", format!("{token_in:#x}")), ("tokenOut", format!("{token_out:#x}")), ("amountIn", amount_in.to_string())];
    if let Some(venues) = only_venues {
        query.push(("includedSources", venues.to_string()));
    }
    let found: Value = http
        .get(format!("{api}/routes"))
        .query(&query)
        .send()
        .await
        .context("the routing service did not answer")?
        .error_for_status()
        .context("the routing service refused the request")?
        .json()
        .await?;
    let summary = found.pointer("/data/routeSummary").ok_or_else(|| anyhow!("no route between these tokens"))?.clone();
    let amount_out = text(&summary, &["amountOut"])?.parse().context("amountOut is not a number")?;
    Ok(Found { summary, amount_out })
}

/// Turn a found route into the exact call, built for `taker` to both pay and receive.
pub async fn build(http: &reqwest::Client, api: &str, found: &Found, taker: Address, slippage_bps: u32) -> Result<Route> {
    let built: Value = http
        .post(format!("{api}/route/build"))
        .json(&json!({
            "routeSummary": found.summary,
            "sender": format!("{taker:#x}"),
            "recipient": format!("{taker:#x}"),
            "slippageTolerance": slippage_bps,
        }))
        .send()
        .await?
        .error_for_status()
        .context("the routing service could not build the route")?
        .json()
        .await?;

    let amount_out: U256 = text(&built, &["data", "amountOut"])?.parse().context("amountOut is not a number")?;
    let venues = found.summary["route"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|path| path.as_array().into_iter().flatten())
        .filter_map(|hop| hop["exchange"].as_str().map(str::to_string))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    Ok(Route {
        router: text(&built, &["data", "routerAddress"])?.parse().context("routerAddress is not an address")?,
        call: text(&built, &["data", "data"])?.parse().context("route data is not hex")?,
        amount_out,
        min_out: amount_out * U256::from(10_000 - slippage_bps) / U256::from(10_000u32),
        venues,
    })
}
