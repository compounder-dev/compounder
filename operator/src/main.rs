//! The operator: one pass over every fund.
//!
//! It reads each fund's holdings and the public prices, plans the trades that
//! bring it back to its strategy, matches all funds' orders token by token,
//! fetches a market route for what is left over together with the issuer's
//! live quote that the price is held to, and settles everything in one
//! transaction. Without a send flag it only simulates that transaction.
//!
//!   compounder-operator targets <config.json>   the shares each fund opts in with
//!   compounder-operator run <config.json>       plan and simulate
//!   compounder-operator run <config.json> --send            settle, signing with OPERATOR_KEY
//!   compounder-operator run <config.json> --send-unlocked   settle through a development node that holds the key
//!   compounder-operator route <request.json>    one route and its issuer quote, as JSON

mod quote;
mod routes;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use alloy::{
    network::EthereumWallet,
    primitives::{Address, Bytes, B256, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
    sol,
};
use anyhow::{anyhow, bail, Context, Result};
use compounder_engine::{
    batch::{self, Book, Leg, Order, Prices, TokenBatch, Want},
    rebalance::{self, Position, Rules, Side},
    strategy::{Strategy, TradingSettings},
};
use serde::Deserialize;

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
        error PriceOutOfBand(uint256 price, uint256 lowest, uint256 highest);
        error QuoteMismatch(address token);
        error QuoteNotLive(uint256 expiration);
        error QuoteNotSigned();
        error QuoteOutOfBand(uint256 quoted, uint256 feed);
        error MarketLegShort(uint256 expected, uint256 got);
        error LeftShort(address token, uint256 before, uint256 afterwards);
        error TokenCallFailed(address token);
        error NotOperator();

        function settle(Trade[] calldata trades) external;
        function isRouter(address router) external view returns (bool);
        function issuer() external view returns (address);
        function quoteBandBps() external view returns (uint256);
        function maxDeviationBps() external view returns (uint256);
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
struct Fund {
    name: String,
    account: Address,
    strategy: Strategy,
}

#[derive(Deserialize)]
struct Config {
    rpc_url: String,
    routing_api: String,
    settlement: Address,
    price_feed: Address,
    cash: Address,
    /// The address `settle` is simulated and sent from.
    operator: Address,
    funds: Vec<Fund>,
    /// Used for a fund whose strategy does not set its own trigger: how far
    /// from target, in basis points of the target, a holding must be to trade.
    trigger_bps: u32,
    /// Trades under this many whole dollars are skipped.
    min_trade_dollars: u64,
    /// Every trade stops this far short of its target, so a small price move
    /// between planning and settling cannot carry it past.
    cushion_bps: u32,
    slippage_bps: u32,
    /// Trade only through these venues, comma separated. Any venue when absent.
    #[serde(default)]
    route_venues: Option<String>,
    /// A small trade's route does not pass through the issuer, so it carries
    /// no issuer quote. The quote is then read from a route of this many
    /// dollars, which is asked for and never used.
    #[serde(default = "default_quote_probe")]
    quote_probe_dollars: u64,
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

const DECIMALS: u8 = 18;
const BASIS_POINTS: u32 = 10_000;
/// The most gas one transaction may use on this chain.
const MAX_GAS: u64 = 16_000_000;

fn unit() -> U256 {
    U256::from(10u8).pow(U256::from(DECIMALS))
}

/// A token amount or dollar value as text with two decimals.
fn show(value: U256) -> String {
    let cents = value / U256::from(10u8).pow(U256::from(DECIMALS - 2));
    format!("{}.{:0>2}", cents / U256::from(100u8), (cents % U256::from(100u8)).to_string())
}

/// `part` as a percentage of `whole`, with one decimal.
fn percent(part: U256, whole: U256) -> String {
    if whole.is_zero() {
        return "0.0%".into();
    }
    let tenths = (part * U256::from(1_000u32) / whole).to::<u64>();
    format!("{}.{}%", tenths / 10, tenths % 10)
}

fn asset_id(token: Address, chain: u64) -> String {
    format!("{token:#x}:{chain}")
}

fn token_of(asset_id: &str) -> Result<Address> {
    asset_id.split(':').next().unwrap_or_default().parse().with_context(|| format!("{asset_id} is not an asset id"))
}

/// A fund's target shares out of one billion, and the stock tokens among them.
fn targets(fund: &Fund, cash_id: &str) -> Result<(BTreeMap<String, u64>, BTreeMap<Address, u64>)> {
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
fn percent_from(value: U256, base: U256) -> f64 {
    let as_number = |value: U256| value.to_string().parse::<f64>().unwrap_or(f64::NAN);
    (as_number(value) / as_number(base) - 1.0) * 100.0
}

fn within(value: U256, base: U256, band_bps: u32) -> bool {
    let scale = U256::from(BASIS_POINTS);
    value * scale >= base * U256::from(BASIS_POINTS - band_bps) && value * scale <= base * U256::from(BASIS_POINTS + band_bps)
}

/// A route, and the issuer's quote its price is held to.
#[derive(Clone)]
struct Priced {
    route: routes::Route,
    quote: ISettlement::Quote,
    signature: Bytes,
}

/// Where routes and issuer quotes come from.
struct Market<'a, P> {
    http: reqwest::Client,
    provider: &'a P,
    api: &'a str,
    chain: u64,
    cash: Address,
    taker: Address,
    slippage_bps: u32,
    venues: Option<&'a str>,
    probe: U256,
    issuer: Address,
    domain: B256,
    /// Whether the issuer lets each address seen so far sign its quotes.
    signers: HashMap<Address, bool>,
    routes: HashMap<(Address, Address, U256), Priced>,
}

impl<'a, P: Provider> Market<'a, P> {
    async fn open(
        provider: &'a P,
        api: &'a str,
        issuer: Address,
        cash: Address,
        taker: Address,
        slippage_bps: u32,
        venues: Option<&'a str>,
        probe_dollars: u64,
    ) -> Result<Self> {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("x-client-id", reqwest::header::HeaderValue::from_static("compounder"));
        Ok(Market {
            http: reqwest::Client::builder().user_agent("compounder-operator").default_headers(headers).build()?,
            provider,
            api,
            chain: provider.get_chain_id().await.context("the chain did not answer")?,
            cash,
            taker,
            slippage_bps,
            venues,
            probe: U256::from(probe_dollars) * unit(),
            issuer,
            domain: IIssuer::new(issuer, provider).getDomainSeparator().call().await.context("the issuer's contract did not answer")?,
            signers: HashMap::new(),
            routes: HashMap::new(),
        })
    }

    /// The routing service allows about two requests a second.
    async fn pace(&self) {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    /// What the market gives for `amount_in` right now, without building a route.
    async fn expect(&self, token_in: Address, token_out: Address, amount_in: U256) -> Result<U256> {
        self.pace().await;
        Ok(routes::find(&self.http, self.api, token_in, token_out, amount_in, self.venues).await?.amount_out)
    }

    async fn build(&self, token_in: Address, token_out: Address, amount_in: U256) -> Result<routes::Route> {
        self.pace().await;
        let found = routes::find(&self.http, self.api, token_in, token_out, amount_in, self.venues).await?;
        self.pace().await;
        routes::build(&self.http, self.api, &found, self.taker, self.slippage_bps).await
    }

    /// The issuer's quote inside a route's call, if it has one the issuer really signed.
    async fn quote_in(&mut self, call: &[u8], token: Address, issuer_sells: bool) -> Result<Option<(ISettlement::Quote, Bytes)>> {
        for candidate in quote::candidates(call, self.chain, token, issuer_sells, self.domain) {
            let allowed = match self.signers.get(&candidate.signer) {
                Some(allowed) => *allowed,
                None => {
                    let role = alloy::primitives::keccak256("ATTESTATION_SIGNER_ROLE");
                    let allowed = IIssuer::new(self.issuer, self.provider).hasRole(role, candidate.signer).call().await?;
                    self.signers.insert(candidate.signer, allowed);
                    allowed
                }
            };
            if allowed {
                return Ok(Some((candidate.quote, candidate.signature)));
            }
        }
        Ok(None)
    }

    /// A route that spends exactly `amount_in`, with the issuer's live quote
    /// for the stock token in the same direction.
    async fn route(&mut self, token_in: Address, token_out: Address, amount_in: U256) -> Result<Priced> {
        if let Some(priced) = self.routes.get(&(token_in, token_out, amount_in)) {
            return Ok(priced.clone());
        }
        let buying = token_in == self.cash;
        let token = if buying { token_out } else { token_in };
        let route = self.build(token_in, token_out, amount_in).await?;
        if route.amount_out.is_zero() || route.min_out.is_zero() {
            bail!("the market offers nothing for it");
        }
        let mut quoted = self.quote_in(&route.call, token, buying).await?;
        if quoted.is_none() {
            // The same trade, sized to pass through the issuer. Only its quote is kept.
            let size = if buying { self.probe } else { self.probe * amount_in / route.amount_out };
            if size > amount_in {
                let larger = self.build(token_in, token_out, size).await?;
                quoted = self.quote_in(&larger.call, token, buying).await?;
            }
        }
        let (quote, signature) = quoted.ok_or_else(|| anyhow!("the issuer is not quoting it, so there is no price to hold the trade to"))?;
        let priced = Priced { route, quote, signature };
        self.routes.insert((token_in, token_out, amount_in), priced.clone());
        Ok(priced)
    }
}

#[derive(PartialEq)]
enum Send {
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
        (Some("run"), Some(path), None) => run(read_config(path)?, Send::No).await,
        (Some("run"), Some(path), Some("--send")) => run(read_config(path)?, Send::WithKey).await,
        (Some("run"), Some(path), Some("--send-unlocked")) => run(read_config(path)?, Send::Unlocked).await,
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
        &request.routing_api,
        request.issuer,
        request.cash,
        request.taker,
        request.slippage_bps,
        None,
        request.quote_probe_dollars,
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

/// The market leg and the fills for one book, with the route that carries
/// it and the issuer quote its price is held to.
async fn build(
    market: &mut Market<'_, impl Provider>,
    config: &Config,
    book: &Book,
    prices: Prices,
    quote_band_bps: u32,
    feed_band_bps: u32,
) -> Result<(TokenBatch, Priced)> {
    let token: Address = book.token.parse()?;
    let (token_in, token_out, amount_in) = match batch::leg(book, prices, config.cushion_bps).map_err(|reason| anyhow!(reason))? {
        Leg::Matched => bail!("buyers and sellers cancel out exactly, so there is no market trade to set the price"),
        Leg::Buy { cash_in } => (config.cash, token, cash_in),
        Leg::Sell { tokens_in } => (token, config.cash, tokens_in),
    };
    let priced = market.route(token_in, token_out, amount_in).await?;
    let route = &priced.route;
    // The contract undoes a batch priced outside its bands. Seeing that here
    // saves simulating a trade that cannot settle.
    let buying = token_in == config.cash;
    let charged = if buying { amount_in * unit() / route.amount_out } else { route.amount_out * unit() / amount_in };
    let quoted = priced.quote.price;
    let side = |value: U256, base: U256| if value > base { "above" } else { "below" };
    if !within(charged, quoted, quote_band_bps) {
        bail!(
            "the market {} it {:.2}% {} the issuer's own price, and the contract allows {:.2}%",
            if buying { "sells" } else { "buys" },
            percent_from(charged, quoted).abs(),
            side(charged, quoted),
            quote_band_bps as f64 / 100.0
        );
    }
    if !within(quoted, prices.public, feed_band_bps) {
        bail!(
            "the issuer quotes it {:.2}% {} its public price, and the contract allows {:.2}%",
            percent_from(quoted, prices.public).abs(),
            side(quoted, prices.public),
            feed_band_bps as f64 / 100.0
        );
    }
    let batch = if buying {
        batch::fill_net_buy(book, amount_in, route.min_out)
    } else {
        batch::fill_net_sell(book, prices, config.cushion_bps, route.min_out)
    }
    .map_err(|reason| anyhow!(reason))?;
    Ok((batch, priced))
}

fn to_trade(token: Address, batch: &TokenBatch, priced: &Priced) -> Result<ISettlement::Trade> {
    let fills = |list: &[batch::Fill]| -> Result<Vec<ISettlement::Fill>> {
        list.iter().map(|fill| Ok(ISettlement::Fill { account: fill.account.parse()?, quantity: fill.quantity })).collect()
    };
    Ok(ISettlement::Trade {
        token,
        netBuy: batch.net_buy,
        netQuantity: batch.net_quantity,
        cashLimit: batch.cash_limit,
        router: priced.route.router,
        routerCall: priced.route.call.clone(),
        buys: fills(&batch.buys)?,
        sells: fills(&batch.sells)?,
        quote: priced.quote.clone(),
        quoteSignature: priced.signature.clone(),
    })
}

/// Why the contract would undo a call, in its own words where it gives them.
fn explain(error: &alloy::contract::Error) -> String {
    match error.as_decoded_interface_error::<ISettlement::ISettlementErrors>() {
        // Printed as `Name(Name { fields })`; the inner part says it all.
        Some(reason) => {
            let text = format!("{reason:?}");
            text.split_once('(').map(|(_, inner)| inner.trim_end_matches(')').to_string()).unwrap_or(text)
        }
        None => error.to_string(),
    }
}

async fn run(config: Config, send: Send) -> Result<()> {
    let provider = ProviderBuilder::new().connect_http(config.rpc_url.parse().context("rpc_url is not a URL")?);
    let chain = provider.get_chain_id().await.context("the chain did not answer")?;
    let cash_id = asset_id(config.cash, chain);
    let settlement = ISettlement::new(config.settlement, &provider);
    let feed = IPriceFeed::new(config.price_feed, &provider);

    let quote_band_bps = settlement.quoteBandBps().call().await.context("the settlement contract did not answer")?.to::<u32>();
    let feed_band_bps = settlement.maxDeviationBps().call().await?.to::<u32>();
    // The cushion has to cover everything the price may do between planning
    // and settling: the route's slippage and the contract's band around the quote.
    if config.cushion_bps < quote_band_bps + config.slippage_bps || config.cushion_bps >= BASIS_POINTS {
        bail!(
            "cushion_bps must be at least slippage_bps plus the contract's price band of {quote_band_bps}, and under 10000"
        );
    }
    let min_trade = U256::from(config.min_trade_dollars) * unit();

    // ---- plan every fund ----
    let mut prices: BTreeMap<Address, U256> = BTreeMap::new();
    let mut symbols: BTreeMap<Address, String> = BTreeMap::new();
    let mut cash_held: BTreeMap<String, U256> = BTreeMap::new();
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    let mut orders: Vec<Order> = Vec::new();
    for fund in &config.funds {
        let (weights, tokens) = targets(fund, &cash_id)?;
        let account = format!("{:#x}", fund.account);
        names.insert(account.clone(), fund.name.clone());

        // The fund must have opted in with exactly what its strategy says.
        if !settlement.accounts(fund.account).call().await?.open {
            println!("{}: has not opted in. Skipped.", fund.name);
            continue;
        }
        // A listed token the strategy does not name must have a target of zero.
        let listed: BTreeSet<Address> = settlement.tokensOf(fund.account).call().await?.into_iter().collect();
        let mut matches = tokens.keys().all(|token| listed.contains(token));
        for token in &listed {
            let on_chain = settlement.targetOf(fund.account, *token).call().await?;
            matches &= on_chain.targetShare == U256::from(tokens.get(token).copied().unwrap_or(0));
        }
        if !matches {
            println!("{}: the targets it opted in with are not what its strategy says. Skipped.", fund.name);
            continue;
        }

        let cash = IToken::new(config.cash, &provider).balanceOf(fund.account).call().await?;
        cash_held.insert(account.clone(), cash);
        let mut positions = vec![Position { asset_id: cash_id.clone(), amount: cash, decimals: DECIMALS, price: unit() }];
        for token in &listed {
            if !prices.contains_key(token) {
                let quote = feed.prices(*token).call().await.with_context(|| format!("no public price for {token:#x}"))?;
                prices.insert(*token, quote.price);
                let symbol = IToken::new(*token, &provider).symbol().call().await.unwrap_or_else(|_| format!("{token:#x}"));
                symbols.insert(*token, symbol);
            }
            let amount = IToken::new(*token, &provider).balanceOf(fund.account).call().await?;
            positions.push(Position { asset_id: asset_id(*token, chain), amount, decimals: DECIMALS, price: prices[token] });
        }

        let trigger_bps = match &fund.strategy.trading_settings {
            TradingSettings::Threshold { trigger_percentage } => (trigger_percentage * 100.0).round() as u32,
            TradingSettings::Calendar { .. } => config.trigger_bps,
        };
        let rules = Rules { quote_asset_id: cash_id.clone(), trigger_bps, min_trade_usd: min_trade };
        let plan = rebalance::plan(&weights, &positions, &rules).map_err(|reason| anyhow!("{}: {reason}", fund.name))?;
        let planned: BTreeMap<&str, &rebalance::Trade> = plan.trades.iter().map(|trade| (trade.asset_id.as_str(), trade)).collect();

        println!("{}: worth ${}, cash ${}", fund.name, show(plan.total_value_usd), show(cash));
        for position in positions.iter().skip(1) {
            let token = token_of(&position.asset_id)?;
            let share = weights.get(&position.asset_id).copied().unwrap_or(0);
            let value = position.amount * position.price / unit();
            let action = match planned.get(position.asset_id.as_str()) {
                Some(trade) if trade.side == Side::Sell => format!("sell ${}", show(trade.value_usd)),
                Some(trade) => format!("buy ${}", show(trade.value_usd)),
                None => "on target".into(),
            };
            println!(
                "    {:<8} holds {:>6}, target {:>6}  {action}",
                symbols[&token],
                percent(value, plan.total_value_usd),
                percent(U256::from(share), U256::from(rebalance::WEIGHT_SCALE))
            );
        }

        let keep = U256::from(BASIS_POINTS - config.cushion_bps);
        for trade in &plan.trades {
            let want = match trade.side {
                // Leaving a token altogether sells all of it. Any other sell stops just short of the target.
                Side::Sell if weights.get(&trade.asset_id).copied().unwrap_or(0) == 0 => Want::Sell { tokens: trade.amount },
                Side::Sell => Want::Sell { tokens: trade.amount * keep / U256::from(BASIS_POINTS) },
                Side::Buy => Want::Buy { cash: trade.quote_amount },
            };
            orders.push(Order { account: account.clone(), token: format!("{:#x}", token_of(&trade.asset_id)?), want });
        }
    }
    if orders.is_empty() {
        println!("Every fund is on target. Nothing to do.");
        return Ok(());
    }

    // ---- what the market charges for each token today ----
    let books = batch::books(&orders).map_err(|reason| anyhow!(reason))?;
    let mut market = Market::open(
        &provider,
        &config.routing_api,
        settlement.issuer().call().await?,
        config.cash,
        config.settlement,
        config.slippage_bps,
        config.route_venues.as_deref(),
        config.quote_probe_dollars,
    )
    .await?;
    // The public price is posted rarely, so the market can sit some way from
    // it. Trades are sized at what the market charges now.
    let mut expected: BTreeMap<String, U256> = BTreeMap::new();
    let mut set_aside: BTreeSet<String> = BTreeSet::new();
    for book in books.values() {
        let token: Address = book.token.parse()?;
        let at_public = match batch::leg(book, Prices::same(prices[&token]), config.cushion_bps).map_err(|reason| anyhow!(reason))? {
            Leg::Matched => Ok(prices[&token]),
            Leg::Buy { cash_in } => market.expect(config.cash, token, cash_in).await.and_then(|out| {
                if out.is_zero() { Err(anyhow!("the market offers nothing for it")) } else { Ok(cash_in * unit() / out) }
            }),
            Leg::Sell { tokens_in } => market.expect(token, config.cash, tokens_in).await.and_then(|out| {
                if out.is_zero() { Err(anyhow!("the market offers nothing for it")) } else { Ok(out * unit() / tokens_in) }
            }),
        };
        match at_public {
            Ok(price) => {
                expected.insert(book.token.clone(), price);
            }
            Err(reason) => {
                println!("{}: left for the next round, {reason:#}", symbols[&token]);
                set_aside.insert(book.token.clone());
            }
        }
    }

    // ---- match orders token by token, route what is left, and simulate ----
    let trades = loop {
        let remaining = books.values().filter(|book| !set_aside.contains(&book.token)).cloned();
        // Sellers are counted on for a little less than the market offers now.
        let schedule = batch::schedule(remaining, &cash_held, &expected, config.cushion_bps, min_trade).map_err(|reason| anyhow!(reason))?;

        let mut built: Vec<(String, ISettlement::Trade, String)> = Vec::new();
        let mut failed = None;
        for book in &schedule.books {
            let token: Address = book.token.parse()?;
            let at = Prices { public: prices[&token], market: expected[&book.token] };
            let result = match build(&mut market, &config, book, at, quote_band_bps, feed_band_bps).await {
                Ok((_, priced)) if !settlement.isRouter(priced.route.router).call().await? => {
                    Err(anyhow!("its route goes through {:#x}, which is not an approved route", priced.route.router))
                }
                other => other,
            };
            match result {
                Ok((batch, priced)) => {
                    let line = format!(
                        "    {:<8} {} buying, {} selling. The market {} ${}, {} ${}, via {}. Issuer quotes ${}",
                        symbols[&token],
                        batch.buys.len(),
                        batch.sells.len(),
                        if batch.net_buy { "buys" } else { "sells" },
                        show(batch.net_quantity * expected[&book.token] / unit()),
                        if batch.net_buy { "paying at most" } else { "for at least" },
                        show(batch.cash_limit),
                        priced.route.venues.join(", "),
                        show(priced.quote.price)
                    );
                    built.push((book.token.clone(), to_trade(token, &batch, &priced)?, line));
                }
                Err(reason) => {
                    println!("{}: left for the next round, {reason:#}", symbols[&token]);
                    failed = Some(book.token.clone());
                    break;
                }
            }
        }
        if let Some(token) = failed {
            set_aside.insert(token);
            continue;
        }
        if built.is_empty() {
            println!("Nothing can settle this round.");
            return Ok(());
        }

        let trades: Vec<ISettlement::Trade> = built.iter().map(|(_, trade, _)| trade.clone()).collect();
        let whole = match settlement.settle(trades.clone()).from(config.operator).call().await {
            Ok(_) => {
                println!("One transaction settles {} token{}:", built.len(), if built.len() == 1 { "" } else { "s" });
                for (_, _, line) in &built {
                    println!("{line}");
                }
                for late in &schedule.deferred {
                    let token: Address = late.token.parse()?;
                    if late.given.is_zero() {
                        println!("    {} waits a round to buy {}: its cash arrives later in this batch.", names[&late.account], symbols[&token]);
                    }
                }
                break trades;
            }
            Err(error) => error,
        };
        // Find the first token whose trade undoes the batch and set it aside.
        let mut culprit = (built.len() - 1, explain(&whole));
        for count in 1..built.len() {
            if let Err(error) = settlement.settle(trades[..count].to_vec()).from(config.operator).call().await {
                culprit = (count - 1, explain(&error));
                break;
            }
        }
        let token: Address = built[culprit.0].0.parse()?;
        println!("{}: left for the next round, it would be undone: {}", symbols[&token], culprit.1);
        println!("{}", built[culprit.0].2.replacen("    ", "    (was) ", 1));
        set_aside.insert(built[culprit.0].0.clone());
    };
    println!("Simulation passed.");

    if send == Send::No {
        println!("Dry run only. Nothing was sent.");
        return Ok(());
    }
    // Routes take different paths when gas runs low, so the bare estimate can
    // fall short. Unused gas is not charged.
    let estimate = settlement.settle(trades.clone()).from(config.operator).estimate_gas().await.context("could not estimate gas")?;
    let gas = (estimate * 3 / 2).min(MAX_GAS);
    let receipt = match send {
        Send::No => unreachable!("handled above"),
        Send::Unlocked => settlement.settle(trades).from(config.operator).gas(gas).send().await?.get_receipt().await?,
        Send::WithKey => {
            let signer: PrivateKeySigner =
                std::env::var("OPERATOR_KEY").context("set OPERATOR_KEY to send")?.parse().context("OPERATOR_KEY is not a private key")?;
            if signer.address() != config.operator {
                bail!("OPERATOR_KEY belongs to {:#x}, not the configured operator", signer.address());
            }
            let signing = ProviderBuilder::new().wallet(EthereumWallet::from(signer)).connect_http(config.rpc_url.parse()?);
            ISettlement::new(config.settlement, &signing).settle(trades).gas(gas).send().await?.get_receipt().await?
        }
    };
    if !receipt.status() {
        bail!("the settlement transaction was undone: {:#x}", receipt.transaction_hash);
    }
    println!("Settled in transaction {:#x}, gas used {}.", receipt.transaction_hash, receipt.gas_used);
    Ok(())
}
