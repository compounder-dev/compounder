//! A backtest: a strategy walked day by day over a history.
//!
//! The run starts with its capital in cash on the first day the whole
//! strategy can be evaluated. Each day, at the close:
//!
//! 1. The book is valued at today's closes.
//! 2. It is decided whether to trade. The first day always does. A calendar
//!    strategy trades on the first trading day of each period. A threshold
//!    strategy evaluates the tree every day and trades when a holding has
//!    drifted as far as its corridor.
//! 3. On a trading day the tree is evaluated at today's close and the book
//!    is traded to those targets at that same close.
//! 4. The value after trading and costs is recorded, with the weights the
//!    book ends the day on.
//!
//! So the first day earns nothing and already carries its costs, and between
//! trading days nothing is evaluated and holdings drift.
//!
//! The answer is the raw series. Statistics are worked out from it
//! elsewhere. The same strategy, history and run always give the same
//! answer, to the last bit.

mod orders;

use crate::evaluate::{day_text, Evaluator, Warning};
use crate::history::History;
use crate::rules::{Asset, Issue, IssueCode, Rebalance, RuleStrategy};
use crate::schedule;
use compounder_prices::day::Day;
use orders::{Book, Traded};
use serde::Serialize;

/// What to run a strategy on, besides the strategy and its prices.
#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    /// The first day wanted. The run starts on the first trading day from
    /// here on which the strategy can be evaluated. `None` asks for as early
    /// as that is.
    pub start: Option<Day>,
    /// The last day wanted, included. `None` asks for as late as prices go.
    pub end: Option<Day>,
    /// Dollars to start with. Above 0.
    pub capital: f64,
    /// The cost of trading, as a fraction of every dollar bought or sold:
    /// 0.0015 is 15 basis points. From 0 to below 1.
    pub slippage: f64,
    /// An order under this many dollars is not placed.
    pub min_order: f64,
    /// Give carried readings the history the reference conventions give
    /// them. Only a recorded case sets this. Off, a reading depends on its
    /// subject alone.
    pub run_in: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Backtest {
    #[serde(serialize_with = "day_text")]
    pub first_day: Day,
    #[serde(serialize_with = "day_text")]
    pub last_day: Day,
    /// The assets the strategy may hold. Every list of weights and of shares
    /// below is in this order.
    pub assets: Vec<Asset>,
    /// One entry for each trading day from the first to the last.
    pub days: Vec<DayResult>,
    /// What is held at the end of the last day.
    pub holdings: Holdings,
    /// What the strategy picks on the last day's closes: its target
    /// weights, which are not the holdings unless it traded that day.
    pub picks: Vec<f64>,
    pub warnings: Vec<Warning>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayResult {
    #[serde(serialize_with = "day_text")]
    pub day: Day,
    /// The value at this day's close, after this day's trades and costs.
    pub value: f64,
    /// Each asset's share of that value. Cash left over is the rest.
    pub weights: Vec<f64>,
    /// Whether the book was brought to its targets this day.
    pub traded: bool,
    /// Dollars of assets bought and sold, each before its cost.
    pub bought: f64,
    pub sold: f64,
    /// What trading cost this day.
    pub cost: f64,
}

/// Shares can be any fraction.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Holdings {
    pub shares: Vec<f64>,
    pub cash: f64,
}

impl Run {
    /// What is wrong with the settings, if anything.
    fn issues(&self) -> Vec<Issue> {
        let mut issues = Vec::new();
        let mut refuse = |field: &str, message: String, fix: &str| {
            issues.push(Issue { code: IssueCode::InvalidRun, path: format!("run.{field}"), message, fix: fix.to_string() });
        };
        if !self.capital.is_finite() || self.capital <= 0.0 {
            refuse("capital", format!("The capital is {}, which is not an amount above 0.", self.capital), "Start with some money, such as 10000.");
        }
        if !self.slippage.is_finite() || !(0.0..1.0).contains(&self.slippage) {
            refuse("slippage", format!("The slippage is {}, which is not a fraction from 0 to below 1.", self.slippage), "Give it as a fraction, such as 0.0015 for 15 basis points.");
        }
        if !self.min_order.is_finite() || self.min_order < 0.0 {
            refuse("min_order", format!("The minimum order is {}, which is not an amount of 0 or more.", self.min_order), "Give it in dollars, such as 1.");
        }
        issues
    }
}

/// Run a strategy over a history. Every problem with the strategy, the
/// prices or the settings comes back instead when there is one.
pub fn backtest(strategy: &RuleStrategy, history: &History, run: &Run) -> Result<Backtest, Vec<Issue>> {
    let issues = run.issues();
    if !issues.is_empty() {
        return Err(issues);
    }
    let evaluator = Evaluator::new(strategy, history, run.run_in)?;
    let window = evaluator.window(run.start, run.end)?;
    let calendar = history.calendar();
    let assets = evaluator.assets();
    // A window's two ends are trading days.
    let (Some(first), Some(last)) = (calendar.position(window.first), calendar.position(window.last)) else { return Err(no_price(window.first)) };

    let mut book = Book { shares: vec![0.0; assets.len()], cash: run.capital };
    let mut last_traded = None;
    let mut days = Vec::with_capacity(last - first + 1);
    for day in calendar.days()[first..=last].iter().copied() {
        let prices = evaluator.closes(day).ok_or_else(|| no_price(day))?;
        let before = book.value(&prices);
        let targets = match strategy.rebalance {
            // A strategy that has not traded yet is due, so the first day trades.
            Rebalance::Calendar(frequency) if schedule::due(frequency, day, last_traded) => Some(evaluator.on(day)?.weights),
            Rebalance::Calendar(_) => None,
            // The target is read afresh every day, and one breach brings
            // every holding to it. The first day trades whatever the corridor.
            Rebalance::Threshold { corridor } => {
                let targets = evaluator.on(day)?.weights;
                let held: Vec<f64> = book.shares.iter().zip(&prices).map(|(shares, price)| shares * price / before).collect();
                Some(targets).filter(|targets| last_traded.is_none() || schedule::breached(corridor, &held, targets))
            }
        };
        let traded = match &targets {
            Some(targets) => {
                last_traded = Some(day);
                book.trade(&prices, targets, run.slippage, run.min_order)
            }
            None => Traded::default(),
        };
        let value = book.value(&prices);
        let weights = book.shares.iter().zip(&prices).map(|(shares, price)| shares * price / value).collect();
        days.push(DayResult { day, value, weights, traded: targets.is_some(), bought: traded.bought, sold: traded.sold, cost: traded.cost });
    }
    let picks = evaluator.on(window.last)?.weights;
    Ok(Backtest {
        first_day: window.first,
        last_day: window.last,
        assets: assets.to_vec(),
        days,
        holdings: Holdings { shares: book.shares, cash: book.cash },
        picks,
        warnings: window.warnings,
    })
}

/// The evaluator only gives a window in which every asset has a price, so
/// this says a rule of the engine is broken, not that the caller erred.
fn no_price(day: Day) -> Vec<Issue> {
    vec![Issue {
        code: IssueCode::MissingHistory,
        path: "history".to_string(),
        message: format!("An asset the strategy may hold has no price on {day}."),
        fix: "Load its prices for every trading day of the run.".to_string(),
    }]
}
