//! The backtest route: a strategy and a few settings in, its daily values
//! and its statistics out.
//!
//! ```text
//! { "strategy": <document>,        or "import": <block-tree document>
//!   "start": "2015-01-02", "end": "2025-12-31",   both optional
//!   "capital": 10000, "slippage": 0.0015,          both optional
//!   "benchmark": "SPY",                            optional
//!   "trades": true }                               optional: list every trading day
//! ```
//!
//! The answer repeats the capital and the slippage it ran on, so a figure can
//! be shown beside what it cost to make.
//!
//! Money is rounded here and nowhere before: a value to the cent, a weight
//! to a millionth.

use compounder_engine::{
    backtest::{backtest, Backtest, Run},
    history::History,
    rules::{self, Asset as Held, Market, RuleStrategy},
};
use compounder_prices::{calendar::Calendar, day::Day};
use serde_json::{json, Value};

use crate::{
    assets,
    prices::{Listing, Prices},
    ApiError,
};

/// What a run starts with when the request does not say.
const CAPITAL: f64 = 10_000.0;
/// What a trade costs when the request does not say: 15 basis points of
/// every dollar bought or sold.
const SLIPPAGE: f64 = 0.0015;
/// An order under a dollar is not placed.
const MIN_ORDER: f64 = 1.0;
const FIELDS: [&str; 8] = ["strategy", "import", "start", "end", "capital", "slippage", "benchmark", "trades"];

/// One request, read and ready to run.
pub struct Ask {
    strategy: RuleStrategy,
    run: Run,
    benchmark: Option<String>,
    trades: bool,
}

impl Ask {
    pub fn read(body: &str) -> Result<Ask, ApiError> {
        let request = rules::parse_json(body).map_err(|issues| ApiError::invalid_request(issues[0].message.clone()))?;
        let Some(fields) = request.as_object() else {
            return Err(ApiError::invalid_request("the request is not a JSON object"));
        };
        if let Some(unknown) = fields.keys().find(|key| !FIELDS.contains(&key.as_str())) {
            return Err(ApiError::invalid_request(format!("the request cannot have a field called {unknown:?}. It takes: {}", FIELDS.join(", "))));
        }
        let given = |key: &str| fields.get(key).filter(|value| !value.is_null());
        let strategy = match (given("strategy"), given("import")) {
            (Some(document), None) => RuleStrategy::from_document(document),
            (None, Some(document)) => rules::import(document),
            _ => return Err(ApiError::invalid_request("send the strategy as \"strategy\", or a block-tree document as \"import\", and not both")),
        }
        .map_err(ApiError::invalid_strategy)?;
        let day = |key: &str| match given(key) {
            None => Ok(None),
            Some(value) => match value.as_str().and_then(|text| text.parse::<Day>().ok()) {
                Some(day) => Ok(Some(day)),
                None => Err(ApiError::invalid_request(format!("\"{key}\" is {value}, which is not a day written like \"2020-01-31\""))),
            },
        };
        let number = |key: &str, otherwise: f64| match given(key) {
            None => Ok(otherwise),
            Some(value) => value.as_f64().ok_or_else(|| ApiError::invalid_request(format!("\"{key}\" is {value}, which is not a number"))),
        };
        let benchmark = match given("benchmark") {
            None => None,
            Some(value) => match value.as_str() {
                Some(symbol) => Some(symbol.to_string()),
                None => return Err(ApiError::invalid_request(format!("\"benchmark\" is {value}, which is not a ticker such as \"SPY\""))),
            },
        };
        let trades = match given("trades") {
            None => false,
            Some(value) => value.as_bool().ok_or_else(|| ApiError::invalid_request(format!("\"trades\" is {value}, which is not true or false")))?,
        };
        let run = Run { start: day("start")?, end: day("end")?, capital: number("capital", CAPITAL)?, slippage: number("slippage", SLIPPAGE)?, min_order: MIN_ORDER, run_in: false };
        Ok(Ask { strategy, run, benchmark, trades })
    }

    /// A strategy over all the days it has, on the settings a request gets when it names none.
    pub fn plain(strategy: RuleStrategy, benchmark: &str) -> Ask {
        let run = Run { start: None, end: None, capital: CAPITAL, slippage: SLIPPAGE, min_order: MIN_ORDER, run_in: false };
        Ask { strategy, run, benchmark: Some(benchmark.to_string()), trades: false }
    }
}

/// The ticker whose price history stands for an asset, when there is one: a
/// listed share's own symbol, or the share a token tracks.
fn ticker(asset: &Held) -> Option<String> {
    match asset {
        Held::Listed { market: Market::Equities, symbol } => Some(symbol.clone()),
        Held::Listed { .. } => None,
        Held::Token { id } => assets().get(id).and_then(|token| token.tracks.clone()),
    }
}

/// An asset with what the app shows for it.
fn described(asset: &Held) -> Value {
    let (symbol, name) = match asset {
        Held::Listed { symbol, .. } => (Some(symbol.clone()), None),
        Held::Token { id } => {
            let known = assets().get(id);
            (known.map(|token| token.symbol.clone()), known.map(|token| token.name.clone()))
        }
    };
    json!({ "asset": asset, "symbol": symbol, "name": name })
}

fn rounded(number: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (number * scale).round() / scale
}

fn cents(dollars: f64) -> f64 {
    rounded(dollars, 2)
}

fn weights(weights: &[f64]) -> Vec<f64> {
    weights.iter().map(|weight| rounded(*weight, 6)).collect()
}

/// Run what was asked for on the prices in the store.
pub fn run(prices: &Prices, ask: &Ask) -> Result<Value, ApiError> {
    let needed = ask.strategy.assets();
    let tickers: Vec<Option<String>> = needed.iter().map(ticker).collect();
    let mut wanted: Vec<&str> = tickers.iter().flatten().map(String::as_str).collect();
    wanted.extend(ask.benchmark.as_deref());
    let (calendar, mut listings) = prices.of(&wanted).map_err(ApiError::broken)?;
    // The benchmark was asked for last.
    let benchmark = match &ask.benchmark {
        Some(symbol) => match listings.pop().flatten() {
            Some(listing) => Some((symbol, listing)),
            None => return Err(ApiError::invalid_request(format!("there is no price history for the benchmark {symbol}"))),
        },
        None => None,
    };

    // An asset with no ticker, or with one the store does not have, is left
    // out of the history. The engine then names it as having no prices.
    let mut history = History::new((*calendar).clone());
    let mut warnings = Vec::new();
    let mut found = listings.into_iter();
    for (asset, ticker) in needed.iter().zip(&tickers) {
        let Some(listing) = ticker.as_ref().and_then(|_| found.next().flatten()) else { continue };
        history.add(asset.clone(), &listing.series);
        if let Some(why) = &listing.dropped_first_bar {
            let message = format!("The first day of {}'s history was left out: {why}.", ticker.as_deref().unwrap_or_default());
            warnings.push(json!({ "code": "DROPPED_FIRST_BAR", "asset": asset, "message": message }));
        }
    }

    let result = backtest(&ask.strategy, &history, &ask.run).map_err(ApiError::invalid_strategy)?;
    let stats = result.statistics().map_err(ApiError::broken)?;
    warnings.extend(result.warnings.iter().map(|warning| json!(warning)));
    let against = match &benchmark {
        Some((symbol, listing)) => Some(compared(&result, &calendar, symbol, listing, &mut warnings)?),
        None => None,
    };

    let traded: Vec<_> = result.days.iter().filter(|day| day.traded).collect();
    let total = |amount: fn(&compounder_engine::backtest::DayResult) -> f64| cents(traded.iter().map(|day| amount(day)).sum());
    let last = result.days.last().ok_or_else(|| ApiError::broken("a backtest came back with no days".to_string()))?;
    let mut answer = json!({
        "hash": ask.strategy.hash(),
        "capital": result.capital,
        "slippage": ask.run.slippage,
        "first_day": result.first_day.to_string(),
        "last_day": result.last_day.to_string(),
        "assets": result.assets.iter().map(described).collect::<Vec<_>>(),
        "days": result.days.iter().map(|day| day.day.to_string()).collect::<Vec<_>>(),
        "values": result.days.iter().map(|day| cents(day.value)).collect::<Vec<_>>(),
        "trading_days": traded.len(),
        "bought": total(|day| day.bought),
        "sold": total(|day| day.sold),
        "cost": total(|day| day.cost),
        "holdings": { "weights": weights(&last.weights), "shares": result.holdings.shares, "cash": cents(result.holdings.cash) },
        "picks": weights(&result.picks),
        "stats": stats,
        "benchmark": against,
        "warnings": warnings,
    });
    if ask.trades {
        answer["trades"] = traded
            .iter()
            .map(|day| json!({ "day": day.day.to_string(), "weights": weights(&day.weights), "bought": cents(day.bought), "sold": cents(day.sold), "cost": cents(day.cost) }))
            .collect();
    }
    Ok(answer)
}

/// The benchmark beside the run, over the days both have.
///
/// A benchmark can have less history than the run covers. Its figures are
/// then for its own days only, so the answer says which days those are and
/// what the strategy made over the same ones: the two returns that may be
/// set side by side. Its line starts level with the strategy on the first
/// of those days and has no value outside them.
fn compared(result: &Backtest, calendar: &Calendar, symbol: &str, listing: &Listing, warnings: &mut Vec<Value>) -> Result<Value, ApiError> {
    let figures = result.against(listing.series.days(), listing.series.prices()).map_err(|reason| ApiError::invalid_request(format!("the benchmark {symbol} cannot be compared: {reason}")))?;
    let prices = calendar.align(&listing.series);
    let on_run_days: Vec<Option<f64>> = result.days.iter().map(|day| calendar.position(day.day).and_then(|position| prices.at(position))).collect();
    let (Some(first), Some(last)) = (on_run_days.iter().position(Option::is_some), on_run_days.iter().rposition(Option::is_some)) else {
        return Err(ApiError::invalid_request(format!("the benchmark {symbol} has no prices on the days of the run")));
    };
    // Measured from the capital when both start together, as the run's own figures are.
    let level = if first == 0 { result.capital } else { result.days[first].value };
    let values: Vec<Option<f64>> = on_run_days.iter().map(|price| Some(cents(level * (*price)? / on_run_days[first]?))).collect();
    let (first_day, last_day) = (result.days[first].day, result.days[last].day);
    if first > 0 || last + 1 < result.days.len() {
        let message = format!("{symbol} has prices from {first_day} to {last_day}, which is less than the run covers. Its figures, and the strategy's return beside them, are for those days only.");
        warnings.push(json!({ "code": "BENCHMARK_SHORTER", "message": message }));
    }
    let mut answer = json!(figures);
    answer["symbol"] = json!(symbol);
    answer["first_day"] = json!(first_day.to_string());
    answer["last_day"] = json!(last_day.to_string());
    answer["strategy_return"] = json!(result.days[last].value / level - 1.0);
    answer["values"] = json!(values);
    Ok(answer)
}
