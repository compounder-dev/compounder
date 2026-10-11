//! The operator: the daily run over every fund.
//!
//! It reads each fund's holdings and the public prices, plans the trades that
//! bring it back to its strategy, matches all funds' orders token by token,
//! and fetches a market route for what is left over together with the
//! issuer's live quote that the price is held to. A quote lives for seconds,
//! so the run settles in rounds: each round is one transaction holding as
//! many tokens as can be priced inside a quote's life, and each round plans
//! afresh from the chain. Without a send flag it only simulates round one.
//!
//!   compounder-operator targets <config.json>   the shares each fund opts in with
//!   compounder-operator run <config.json>       plan and simulate
//!   compounder-operator run <config.json> --send            settle, signing with OPERATOR_KEY
//!   compounder-operator run <config.json> --send-unlocked   settle through a development node that holds the key
//!   compounder-operator route <request.json>    one route and its issuer quote, as JSON

mod daily;
mod market;
mod quote;
mod routes;

use std::collections::BTreeMap;

use alloy::{
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    sol,
};
use anyhow::{anyhow, bail, Context, Result};
use compounder_engine::{rebalance, strategy::Strategy};
use serde::Deserialize;

use market::{Market, Priced, Settings};

sol! {
    #[sol(rpc)]
    #[derive(Debug)]
    interface ISettlement {
        struct Fill { address account; uint256 quantity; }
        struct Quote {
            uint256 chainId;
            uint256 attestationId;
            bytes32 userId;
            address asset;
            uint256 price;
            uint256 quantity;
            uint256 expiration;
            uint8 side;
            bytes32 additionalData;
        }
        struct Trade {
            address token;
            bool netBuy;
            uint256 netQuantity;
            uint256 cashLimit;
            address router;
            bytes routerCall;
            Fill[] buys;
            Fill[] sells;
            Quote quote;
            bytes quoteSignature;
        }

        error AccountNotOpen(address account);
        error TokenNotListed(address account, address token);
        error PastTarget(address account, address token, uint256 valueAfter, uint256 target);
        error DailyLimitExceeded(address account, uint256 value, uint256 left);
        error RouterNotAllowed(address router);
        error NetQuantityMismatch(uint256 expected, uint256 stated);
        error TokenTwice(address token);
        error EmptyFill();
        error PriceUnavailable(address token);
        error PriceOutOfBand(address token, uint256 price, uint256 lowest, uint256 highest);
        error QuoteMismatch(address token);
        error QuoteNotLive(address token, uint256 expiration);
        error QuoteNotSigned(address token);
        error QuoteOutOfBand(address token, uint256 quoted, uint256 feed);
        error MarketLegShort(uint256 expected, uint256 got);
        error LeftShort(address token, uint256 before, uint256 afterwards);
        error TokenCallFailed(address token);
        error NotOperator();

        function settle(Trade[] calldata trades) external;
        function isRouter(address router) external view returns (bool);
        function issuer() external view returns (address);
        function quoteBandBps() external view returns (uint256);
        function feedBandBps() external view returns (uint256);
        function accounts(address account) external view returns (bool open, uint32 epoch, uint64 limitAt, uint256 dailyLimit, uint256 limitLeft);
        function tokensOf(address account) external view returns (address[] memory);
        function targetOf(address account, address token) external view returns (bool isListed, uint256 targetShare);
    }

    #[sol(rpc)]
    interface IToken {
        function balanceOf(address account) external view returns (uint256);
        function symbol() external view returns (string memory);
    }

    #[sol(rpc)]
    #[derive(Debug)]
    interface IIssuer {
        function getDomainSeparator() external view returns (bytes32);
        function hasRole(bytes32 role, address account) external view returns (bool);
    }

    #[sol(rpc)]
    interface IPriceFeed {
        function prices(address token) external view returns (uint256 price, uint64 lastUpdated, uint32 maxTimeDelay, uint16 deviationBps);
    }
}

#[derive(Deserialize)]
pub(crate) struct Fund {
    pub name: String,
    pub account: Address,
    pub strategy: Strategy,
}

#[derive(Deserialize)]
pub(crate) struct Config {
    pub rpc_url: String,
    pub routing_api: String,
    pub settlement: Address,
    pub price_feed: Address,
    pub cash: Address,
    /// The address `settle` is simulated and sent from.
    pub operator: Address,
    pub funds: Vec<Fund>,
    /// Used for a fund whose strategy does not set its own trigger: how far
    /// from target, in basis points of the target, a holding must be to trade.
    pub trigger_bps: u32,
    /// Trades under this many whole dollars are skipped.
    pub min_trade_dollars: u64,
    /// Every trade stops this far short of its target, so a small price move
    /// between planning and settling cannot carry it past.
    pub cushion_bps: u32,
    pub slippage_bps: u32,
    /// Trade only through these venues, comma separated. Any venue when absent.
    #[serde(default)]
    pub route_venues: Option<String>,
    /// A small trade's route does not pass through the issuer, so it carries
    /// no issuer quote. The quote is then read from a larger route, which is
    /// asked for and never used. This many dollars is the smallest tried.
    #[serde(default = "default_quote_probe")]
    pub quote_probe_dollars: u64,
}

fn default_quote_probe() -> u64 {
    2_000
}

/// What the `route` command is asked for.
#[derive(Deserialize)]
struct RouteRequest {
    rpc_url: String,
    routing_api: String,
    issuer: Address,
    cash: Address,
    /// Who the route is built for: it both pays and receives.
    taker: Address,
    token_in: Address,
    token_out: Address,
    amount_in: U256,
    slippage_bps: u32,
    #[serde(default = "default_quote_probe")]
    quote_probe_dollars: u64,
}

pub(crate) const DECIMALS: u8 = 18;
pub(crate) const BASIS_POINTS: u32 = 10_000;

pub(crate) fn unit() -> U256 {
    U256::from(10u8).pow(U256::from(DECIMALS))
}

/// A token amount or dollar value as text with two decimals.
pub(crate) fn show(value: U256) -> String {
    let cents = value / U256::from(10u8).pow(U256::from(DECIMALS - 2));
    format!("{}.{:0>2}", cents / U256::from(100u8), (cents % U256::from(100u8)).to_string())
}

/// `part` as a percentage of `whole`, with one decimal.
pub(crate) fn percent(part: U256, whole: U256) -> String {
    if whole.is_zero() {
        return "0.0%".into();
    }
    let tenths = (part * U256::from(1_000u32) / whole).to::<u64>();
    format!("{}.{}%", tenths / 10, tenths % 10)
}

pub(crate) fn asset_id(token: Address, chain: u64) -> String {
    format!("{token:#x}:{chain}")
}

pub(crate) fn token_of(asset_id: &str) -> Result<Address> {
    asset_id.split(':').next().unwrap_or_default().parse().with_context(|| format!("{asset_id} is not an asset id"))
}

/// A fund's target shares out of one billion, and the stock tokens among them.
pub(crate) fn targets(fund: &Fund, cash_id: &str) -> Result<(BTreeMap<String, u64>, BTreeMap<Address, u64>)> {
    let fractions = fund.strategy.targets().map_err(|issues| anyhow!("{}: the strategy has problems: {issues:?}", fund.name))?;
    let weights = rebalance::fix_weights(&fractions).map_err(|reason| anyhow!("{}: {reason}", fund.name))?;
    let tokens = weights.iter().filter(|(id, _)| id.as_str() != cash_id).map(|(id, share)| Ok((token_of(id)?, *share))).collect::<Result<_>>()?;
    Ok((weights, tokens))
}

fn read_config<T: serde::de::DeserializeOwned>(path: &str) -> Result<T> {
    serde_json::from_str(&std::fs::read_to_string(path).with_context(|| format!("cannot read {path}"))?)
        .with_context(|| format!("{path} is not a valid config"))
}

/// How far `value` sits from `base`, as a percentage of `base`.
pub(crate) fn percent_from(value: U256, base: U256) -> f64 {
    let as_number = |value: U256| value.to_string().parse::<f64>().unwrap_or(f64::NAN);
    (as_number(value) / as_number(base) - 1.0) * 100.0
}

pub(crate) fn within(value: U256, base: U256, band_bps: u32) -> bool {
    let scale = U256::from(BASIS_POINTS);
    value * scale >= base * U256::from(BASIS_POINTS - band_bps) && value * scale <= base * U256::from(BASIS_POINTS + band_bps)
}

#[derive(PartialEq)]
pub(crate) enum Send {
    No,
    WithKey,
    Unlocked,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "usage: compounder-operator <targets|run|route> <config.json> [--send | --send-unlocked]";
    match (args.first().map(String::as_str), args.get(1), args.get(2).map(String::as_str)) {
        (Some("route"), Some(path), None) => print_route(read_config(path)?).await,
        (Some("targets"), Some(path), None) => print_targets(read_config(path)?).await,
        (Some("run"), Some(path), None) => daily::run(read_config(path)?, Send::No).await,
        (Some("run"), Some(path), Some("--send")) => daily::run(read_config(path)?, Send::WithKey).await,
        (Some("run"), Some(path), Some("--send-unlocked")) => daily::run(read_config(path)?, Send::Unlocked).await,
        _ => bail!(usage),
    }
}

/// Print, for each fund, the tokens and shares it opts in with. These are the
/// same whole numbers the settlement contract checks every fill against. A
/// token the fund still holds from an earlier strategy is listed with a share
/// of zero, so it can be sold.
async fn print_targets(config: Config) -> Result<()> {
    let provider = ProviderBuilder::new().connect_http(config.rpc_url.parse().context("rpc_url is not a URL")?);
    let cash_id = asset_id(config.cash, provider.get_chain_id().await.context("the chain did not answer")?);
    let settlement = ISettlement::new(config.settlement, &provider);
    let mut out = Vec::new();
    for fund in &config.funds {
        let (_, mut tokens) = targets(fund, &cash_id)?;
        for token in settlement.tokensOf(fund.account).call().await? {
            if !tokens.contains_key(&token) && !IToken::new(token, &provider).balanceOf(fund.account).call().await?.is_zero() {
                tokens.insert(token, 0);
            }
        }
        out.push(serde_json::json!({
            "name": fund.name,
            "account": format!("{:#x}", fund.account),
            "tokens": tokens.iter().map(|(token, _)| format!("{token:#x}")).collect::<Vec<_>>(),
            "shares": tokens.iter().map(|(_, share)| share.to_string()).collect::<Vec<_>>(),
        }));
    }
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

/// Print one route and the issuer quote its price is held to, for checking
/// the settlement contract against them by hand.
async fn print_route(request: RouteRequest) -> Result<()> {
    let provider = ProviderBuilder::new().connect_http(request.rpc_url.parse().context("rpc_url is not a URL")?);
    let mut market = Market::open(
        &provider,
        Settings {
            routing_api: &request.routing_api,
            issuer: request.issuer,
            cash: request.cash,
            taker: request.taker,
            slippage_bps: request.slippage_bps,
            venues: None,
            quote_probe_dollars: request.quote_probe_dollars,
        },
    )
    .await?;
    let Priced { route, quote, signature } = market.route(request.token_in, request.token_out, request.amount_in).await?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "router": format!("{:#x}", route.router),
            "data": route.call.to_string(),
            "amountIn": request.amount_in.to_string(),
            "amountOut": route.amount_out.to_string(),
            "minAmountOut": route.min_out.to_string(),
            "venues": route.venues.join(", "),
            "quote": {
                "chainId": quote.chainId.to_string(),
                "attestationId": quote.attestationId.to_string(),
                "userId": quote.userId.to_string(),
                "asset": format!("{:#x}", quote.asset),
                "price": quote.price.to_string(),
                "quantity": quote.quantity.to_string(),
                "expiration": quote.expiration.to_string(),
                "side": quote.side,
                "additionalData": quote.additionalData.to_string(),
            },
            "signature": signature.to_string(),
        }))?
    );
    Ok(())
}

