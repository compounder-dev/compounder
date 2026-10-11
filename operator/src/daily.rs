//! The daily run: plan every fund from the chain, then settle in rounds.
//!
//! A round is one transaction. It holds as many tokens as can be priced
//! before the first quote among them runs short, because the settlement
//! contract refuses a quote that has expired. Whatever a round does not
//! reach is planned again, from fresh chain state, in the next one.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    time::Duration,
};

use alloy::{
    network::EthereumWallet,
    primitives::{Address, U256},
    providers::{Provider, ProviderBuilder},
    signers::local::PrivateKeySigner,
};
use anyhow::{anyhow, bail, Context, Result};
use compounder_engine::{
    batch::{self, Book, Leg, Order, Prices, TokenBatch, Want},
    rebalance::{self, Position, Rules, Side},
    strategy::TradingSettings,
};

use crate::{
    asset_id,
    market::{self, Market, Priced, Settings, FETCH_SECONDS, RESERVE_SECONDS},
    percent, percent_from, show, targets, token_of, unit, within, Config, IPriceFeed, ISettlement, IToken, Send, BASIS_POINTS, DECIMALS,
};

/// The most gas one transaction may use on this chain.
const MAX_GAS: u64 = 16_000_000;
/// A run that needs more rounds than this stops and says so.
const MAX_ROUNDS: usize = 40;
/// How often a round is put together again after its prices went stale.
const MAX_RETRIES: usize = 3;
/// A whole run that takes longer than this is stuck.
const RUN_LIMIT: Duration = Duration::from_secs(30 * 60);

/// What every fund holds and wants, read from the chain at one moment.
struct Plan {
    orders: Vec<Order>,
    /// The public feed's price for each token any fund lists.
    prices: BTreeMap<Address, U256>,
    symbols: BTreeMap<Address, String>,
    cash_held: BTreeMap<String, U256>,
    names: BTreeMap<String, String>,
}

/// The settlement contract's two price bands, in basis points.
struct Bands {
    /// How far a batch price may sit from the issuer's quote.
    quote: u32,
    /// How far the issuer's quote may sit from the public feed.
    feed: u32,
}

struct Built {
    token: Address,
    trade: ISettlement::Trade,
    line: String,
}

/// One transaction's worth of trades, already simulated.
struct Round {
    built: Vec<Built>,
    notes: Vec<String>,
    /// Tokens not reached before time ran short. They go in the next round.
    waiting: usize,
}

/// Seconds left on the quote that expires first.
fn oldest(built: &[Built]) -> Option<i64> {
    built.iter().map(|one| market::seconds_left(&one.trade.quote)).min()
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

/// A quote expired between being fetched and being checked. That is nobody's fault: fetch again.
fn went_stale(error: &alloy::contract::Error) -> bool {
    matches!(error.as_decoded_interface_error::<ISettlement::ISettlementErrors>(), Some(ISettlement::ISettlementErrors::QuoteNotLive(_)))
}

pub async fn run(config: Config, send: Send) -> Result<()> {
    tokio::time::timeout(RUN_LIMIT, rounds(config, send)).await.map_err(|_| anyhow!("the run took more than half an hour and was stopped"))?
}

async fn rounds(config: Config, send: Send) -> Result<()> {
    let provider = ProviderBuilder::new().connect_http(config.rpc_url.parse().context("rpc_url is not a URL")?);
    let chain = provider.get_chain_id().await.context("the chain did not answer")?;
    let settlement = ISettlement::new(config.settlement, &provider);

    let bands = Bands {
        quote: settlement.quoteBandBps().call().await.context("the settlement contract did not answer")?.to::<u32>(),
        feed: settlement.feedBandBps().call().await?.to::<u32>(),
    };
    // The cushion has to cover everything the price may do between planning
    // and settling: the route's slippage and the contract's band around the quote.
    if config.cushion_bps < bands.quote + config.slippage_bps || config.cushion_bps >= BASIS_POINTS {
        bail!("cushion_bps must be at least slippage_bps plus the contract's price band of {}, and under 10000", bands.quote);
    }
    let mut market = Market::open(
        &provider,
        Settings {
            routing_api: &config.routing_api,
            issuer: settlement.issuer().call().await?,
            cash: config.cash,
            taker: config.settlement,
            slippage_bps: config.slippage_bps,
            venues: config.route_venues.as_deref(),
            quote_probe_dollars: config.quote_probe_dollars,
        },
    )
    .await?;

    // Tokens that cannot settle today, each for a reason already printed.
    let mut refused: BTreeSet<Address> = BTreeSet::new();
    let mut routers: HashMap<Address, bool> = HashMap::new();
    let mut undone = 0;
    for round in 1..=MAX_ROUNDS {
        let plan = plan_funds(&provider, &config, chain, round == 1).await?;
        let orders: Vec<Order> =
            plan.orders.iter().filter(|order| order.token.parse().is_ok_and(|token: Address| !refused.contains(&token))).cloned().collect();
        if orders.is_empty() {
            println!("{}", if round == 1 && refused.is_empty() { "Every fund is on target. Nothing to do." } else { "Nothing more to settle." });
            return Ok(());
        }
        if round > 1 {
            println!("Round {round}: planned again from the chain.");
        }
        let Some(batch) = assemble(&provider, &mut market, &config, &plan, &orders, &bands, &mut refused, &mut routers).await? else {
            println!("Nothing {}can settle this run.", if round == 1 { "" } else { "more " });
            return Ok(());
        };
        println!("One transaction settles {} token{}:", batch.built.len(), if batch.built.len() == 1 { "" } else { "s" });
        for one in &batch.built {
            println!("{}", one.line);
        }
        for note in &batch.notes {
            println!("{note}");
        }
        if batch.waiting > 0 {
            println!("    {} more wait{} for the next round: a price is only good for seconds.", batch.waiting, if batch.waiting == 1 { "s" } else { "" });
        }
        println!("Simulation passed.");
        if send == Send::No {
            println!("Dry run only. Nothing was sent.");
            return Ok(());
        }
        if !send_round(&provider, &config, &send, &batch).await? {
            undone += 1;
            if undone > MAX_RETRIES {
                bail!("the settlement transaction kept being undone");
            }
            market.forget();
        }
    }
    println!("Stopped after {MAX_ROUNDS} rounds with trades still to settle. Run again.");
    Ok(())
}

/// Read every fund and plan the trades that bring it back to its strategy.
async fn plan_funds(provider: &impl Provider, config: &Config, chain: u64, show_funds: bool) -> Result<Plan> {
    let cash_id = asset_id(config.cash, chain);
    let settlement = ISettlement::new(config.settlement, provider);
    let feed = IPriceFeed::new(config.price_feed, provider);
    let min_trade = U256::from(config.min_trade_dollars) * unit();
    let mut plan = Plan { orders: Vec::new(), prices: BTreeMap::new(), symbols: BTreeMap::new(), cash_held: BTreeMap::new(), names: BTreeMap::new() };

    for fund in &config.funds {
        let (weights, tokens) = targets(fund, &cash_id)?;
        let account = format!("{:#x}", fund.account);
        plan.names.insert(account.clone(), fund.name.clone());

        // The fund must have opted in with exactly what its strategy says.
        if !settlement.accounts(fund.account).call().await?.open {
            if show_funds {
                println!("{}: has not opted in. Skipped.", fund.name);
            }
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
            if show_funds {
                println!("{}: the targets it opted in with are not what its strategy says. Skipped.", fund.name);
            }
            continue;
        }

        let cash = IToken::new(config.cash, provider).balanceOf(fund.account).call().await?;
        plan.cash_held.insert(account.clone(), cash);
        let mut positions = vec![Position { asset_id: cash_id.clone(), amount: cash, decimals: DECIMALS, price: unit() }];
        for token in &listed {
            if !plan.prices.contains_key(token) {
                let posted = feed.prices(*token).call().await.with_context(|| format!("no public price for {token:#x}"))?;
                plan.prices.insert(*token, posted.price);
                let symbol = IToken::new(*token, provider).symbol().call().await.unwrap_or_else(|_| format!("{token:#x}"));
                plan.symbols.insert(*token, symbol);
            }
            let amount = IToken::new(*token, provider).balanceOf(fund.account).call().await?;
            positions.push(Position { asset_id: asset_id(*token, chain), amount, decimals: DECIMALS, price: plan.prices[token] });
        }

        let trigger_bps = match &fund.strategy.trading_settings {
            TradingSettings::Threshold { trigger_percentage } => (trigger_percentage * 100.0).round() as u32,
            TradingSettings::Calendar { .. } => config.trigger_bps,
        };
        let rules = Rules { quote_asset_id: cash_id.clone(), trigger_bps, min_trade_usd: min_trade };
        let planned = rebalance::plan(&weights, &positions, &rules).map_err(|reason| anyhow!("{}: {reason}", fund.name))?;

        if show_funds {
            let by_asset: BTreeMap<&str, &rebalance::Trade> = planned.trades.iter().map(|trade| (trade.asset_id.as_str(), trade)).collect();
            println!("{}: worth ${}, cash ${}", fund.name, show(planned.total_value_usd), show(cash));
            for position in positions.iter().skip(1) {
                let token = token_of(&position.asset_id)?;
                let share = weights.get(&position.asset_id).copied().unwrap_or(0);
                let value = position.amount * position.price / unit();
                let action = match by_asset.get(position.asset_id.as_str()) {
                    Some(trade) if trade.side == Side::Sell => format!("sell ${}", show(trade.value_usd)),
                    Some(trade) => format!("buy ${}", show(trade.value_usd)),
                    None => "on target".into(),
                };
                println!(
                    "    {:<8} holds {:>6}, target {:>6}  {action}",
                    plan.symbols[&token],
                    percent(value, planned.total_value_usd),
                    percent(U256::from(share), U256::from(rebalance::WEIGHT_SCALE))
                );
            }
        }

        let keep = U256::from(BASIS_POINTS - config.cushion_bps);
        for trade in &planned.trades {
            let want = match trade.side {
                // Leaving a token altogether sells all of it. Any other sell stops just short of the target.
                Side::Sell if weights.get(&trade.asset_id).copied().unwrap_or(0) == 0 => Want::Sell { tokens: trade.amount },
                Side::Sell => Want::Sell { tokens: trade.amount * keep / U256::from(BASIS_POINTS) },
                Side::Buy => Want::Buy { cash: trade.quote_amount },
            };
            plan.orders.push(Order { account: account.clone(), token: format!("{:#x}", token_of(&trade.asset_id)?), want });
        }
    }
    Ok(plan)
}

/// What the market charges for each book's token right now. The public price
/// is posted rarely, so the market can sit some way from it, and trades are
/// sized at the market's price. A token with no market is refused.
async fn estimate(
    market: &mut Market<'_, impl Provider>,
    config: &Config,
    plan: &Plan,
    books: &BTreeMap<String, Book>,
    refused: &mut BTreeSet<Address>,
) -> Result<BTreeMap<String, U256>> {
    let mut expected = BTreeMap::new();
    for book in books.values() {
        let token: Address = book.token.parse()?;
        let public = plan.prices[&token];
        let nothing = || anyhow!("the market offers nothing for it");
        let price = match batch::leg(book, Prices::same(public), config.cushion_bps).map_err(|reason| anyhow!(reason))? {
            Leg::Matched => Ok(public),
            Leg::Buy { cash_in } => match market.expect(config.cash, token, cash_in).await {
                Ok(out) if out.is_zero() => Err(nothing()),
                Ok(out) => Ok(cash_in * unit() / out),
                Err(reason) => Err(reason),
            },
            Leg::Sell { tokens_in } => match market.expect(token, config.cash, tokens_in).await {
                Ok(out) if out.is_zero() => Err(nothing()),
                Ok(out) => Ok(out * unit() / tokens_in),
                Err(reason) => Err(reason),
            },
        };
        match price {
            Ok(price) => {
                expected.insert(book.token.clone(), price);
            }
            Err(reason) => {
                println!("{}: left for the next run, {reason:#}", plan.symbols[&token]);
                refused.insert(token);
            }
        }
    }
    Ok(expected)
}

/// The market leg and the fills for one book, with the route that carries
/// it and the issuer quote its price is held to.
async fn build(market: &mut Market<'_, impl Provider>, config: &Config, book: &Book, mut at: Prices, bands: &Bands) -> Result<(TokenBatch, Priced)> {
    let token: Address = book.token.parse()?;
    let mut resized = false;
    loop {
        let (token_in, token_out, amount_in) = match batch::leg(book, at, config.cushion_bps).map_err(|reason| anyhow!(reason))? {
            Leg::Matched => bail!("buyers and sellers cancel out exactly, so there is no market trade to set the price"),
            Leg::Buy { cash_in } => (config.cash, token, cash_in),
            Leg::Sell { tokens_in } => (token, config.cash, tokens_in),
        };
        let priced = market.route(token_in, token_out, amount_in).await?;
        let route = &priced.route;
        let buying = token_in == config.cash;
        let charged = if buying { amount_in * unit() / route.amount_out } else { route.amount_out * unit() / amount_in };
        // The trade was sized at an earlier estimate. If the route itself is
        // priced far enough from it to matter, size the trade once more at the route's price.
        if !resized && !within(charged, at.market, config.cushion_bps / 4) {
            at.market = charged;
            resized = true;
            continue;
        }

        // The contract undoes a batch priced outside its bands. Seeing that here
        // saves simulating a trade that cannot settle.
        let quoted = priced.quote.price;
        let side = |value: U256, base: U256| if value > base { "above" } else { "below" };
        if !within(charged, quoted, bands.quote) {
            bail!(
                "the market {} it {:.2}% {} the issuer's own price, and the contract allows {:.2}%",
                if buying { "sells" } else { "buys" },
                percent_from(charged, quoted).abs(),
                side(charged, quoted),
                bands.quote as f64 / 100.0
            );
        }
        if !within(quoted, at.public, bands.feed) {
            bail!(
                "the issuer quotes it {:.2}% {} its public price, and the contract allows {:.2}%",
                percent_from(quoted, at.public).abs(),
                side(quoted, at.public),
                bands.feed as f64 / 100.0
            );
        }
        let batch = if buying {
            batch::fill_net_buy(book, at.public, amount_in, route.min_out, route.amount_out)
        } else {
            batch::fill_net_sell(book, at, config.cushion_bps, route.min_out)
        }
        .map_err(|reason| anyhow!(reason))?;
        return Ok((batch, priced));
    }
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

/// Match the orders token by token, route what is left over, and put together
/// one transaction that the contract accepts in simulation. A token that
/// cannot settle today is refused with its reason. A token there was no time
/// for is left waiting.
#[allow(clippy::too_many_arguments)]
async fn assemble<P: Provider>(
    provider: &P,
    market: &mut Market<'_, P>,
    config: &Config,
    plan: &Plan,
    orders: &[Order],
    bands: &Bands,
    refused: &mut BTreeSet<Address>,
    routers: &mut HashMap<Address, bool>,
) -> Result<Option<Round>> {
    let settlement = ISettlement::new(config.settlement, provider);
    let min_trade = U256::from(config.min_trade_dollars) * unit();
    let books = batch::books(orders).map_err(|reason| anyhow!(reason))?;
    let expected = estimate(market, config, plan, &books, refused).await?;

    let mut stale = 0;
    loop {
        let remaining: Vec<Book> = books.values().filter(|book| book.token.parse().is_ok_and(|token: Address| !refused.contains(&token))).cloned().collect();
        if remaining.is_empty() {
            return Ok(None);
        }
        // Sellers are counted on for a little less than the market offers now.
        let schedule = batch::schedule(remaining, &plan.cash_held, &expected, config.cushion_bps, min_trade).map_err(|reason| anyhow!(reason))?;

        let mut built: Vec<Built> = Vec::new();
        let mut waiting = 0;
        let mut failed = false;
        for (index, book) in schedule.books.iter().enumerate() {
            // Another token takes seconds to price. Stop while the first quote can still be used.
            if oldest(&built).is_some_and(|left| left < RESERVE_SECONDS + FETCH_SECONDS) {
                waiting = schedule.books.len() - index;
                break;
            }
            let token: Address = book.token.parse()?;
            let at = Prices { public: plan.prices[&token], market: expected[&book.token] };
            let result = match build(market, config, book, at, bands).await {
                Ok((batch, priced)) => {
                    let router = priced.route.router;
                    let approved = match routers.get(&router) {
                        Some(approved) => *approved,
                        None => {
                            let approved = settlement.isRouter(router).call().await?;
                            routers.insert(router, approved);
                            approved
                        }
                    };
                    if approved { Ok((batch, priced)) } else { Err(anyhow!("its route goes through {router:#x}, which is not an approved route")) }
                }
                Err(reason) => Err(reason),
            };
            match result {
                Ok((batch, priced)) => {
                    let line = format!(
                        "    {:<8} {} buying, {} selling. The market {} ${}, {} ${}, via {}. Issuer quotes ${}",
                        plan.symbols[&token],
                        batch.buys.len(),
                        batch.sells.len(),
                        if batch.net_buy { "buys" } else { "sells" },
                        show(batch.net_quantity * expected[&book.token] / unit()),
                        if batch.net_buy { "paying at most" } else { "for at least" },
                        show(batch.cash_limit),
                        priced.route.venues.join(", "),
                        show(priced.quote.price)
                    );
                    built.push(Built { token, trade: to_trade(token, &batch, &priced)?, line });
                }
                Err(reason) => {
                    println!("{}: left for the next run, {reason:#}", plan.symbols[&token]);
                    refused.insert(token);
                    // Buyers who stay out keep their cash, so the order still holds.
                    // Sellers who stay out leave later buyers short: schedule again.
                    if !book.sells.is_empty() {
                        failed = true;
                        break;
                    }
                }
            }
        }
        if failed {
            continue;
        }
        if built.is_empty() {
            return Ok(None);
        }
        let mut start_over = |market: &mut Market<'_, P>| -> Result<()> {
            stale += 1;
            if stale > MAX_RETRIES {
                bail!("prices went stale faster than a batch could be put together");
            }
            market.forget();
            Ok(())
        };
        if oldest(&built).is_some_and(|left| left < RESERVE_SECONDS) {
            start_over(market)?;
            continue;
        }

        let trades: Vec<ISettlement::Trade> = built.iter().map(|one| one.trade.clone()).collect();
        let whole = match settlement.settle(trades.clone()).from(config.operator).call().await {
            Ok(_) => {
                let mut notes = Vec::new();
                for late in &schedule.deferred {
                    let token: Address = late.token.parse()?;
                    if late.given.is_zero() {
                        notes.push(format!("    {} waits a round to buy {}: its cash arrives later in this batch.", plan.names[&late.account], plan.symbols[&token]));
                    }
                }
                return Ok(Some(Round { built, notes, waiting }));
            }
            Err(error) => error,
        };
        // Find the first token whose trade undoes the batch.
        let mut culprit = (built.len() - 1, whole);
        for count in 1..built.len() {
            if let Err(error) = settlement.settle(trades[..count].to_vec()).from(config.operator).call().await {
                culprit = (count - 1, error);
                break;
            }
        }
        if went_stale(&culprit.1) {
            start_over(market)?;
            continue;
        }
        println!("{}: left for the next run, it would be undone: {}", plan.symbols[&built[culprit.0].token], explain(&culprit.1));
        println!("{}", built[culprit.0].line.replacen("    ", "    (was) ", 1));
        refused.insert(built[culprit.0].token);
    }
}

/// Send one round and wait for it. False means it did not settle and the round should be planned again.
async fn send_round(provider: &impl Provider, config: &Config, send: &Send, batch: &Round) -> Result<bool> {
    let trades: Vec<ISettlement::Trade> = batch.built.iter().map(|one| one.trade.clone()).collect();
    let settlement = ISettlement::new(config.settlement, provider);
    // Routes take different paths when gas runs low, so the bare estimate can
    // fall short. Unused gas is not charged.
    let estimate = match settlement.settle(trades.clone()).from(config.operator).estimate_gas().await {
        Ok(estimate) => estimate,
        Err(error) if went_stale(&error) => {
            println!("A price expired before the transaction could be sent. Trying again.");
            return Ok(false);
        }
        Err(error) => bail!("could not estimate gas: {}", explain(&error)),
    };
    if estimate > MAX_GAS {
        bail!("this round needs {estimate} gas and one transaction may use {MAX_GAS}");
    }
    let gas = (estimate * 3 / 2).min(MAX_GAS);
    if oldest(&batch.built).is_some_and(|left| left < 2) {
        println!("A price is about to expire. Trying again.");
        return Ok(false);
    }
    let receipt = match send {
        Send::No => unreachable!("a dry run never sends"),
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
        println!("The transaction was undone: {:#x}. Trying again.", receipt.transaction_hash);
        return Ok(false);
    }
    println!("Settled in transaction {:#x}, gas used {}.", receipt.transaction_hash, receipt.gas_used);
    Ok(true)
}
