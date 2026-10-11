//! Evaluating a rule strategy: prices in, target weights out.
//!
//! The tree is a pure function of the closes up to and including the day it
//! is evaluated on. Evaluating it at a close gives one target weight for
//! each asset it may hold, and the weights sum to 1. A decision made on a
//! day reads that day's close.
//!
//! `Evaluator` does the work that does not depend on the day once: it checks
//! the strategy, lays the tree flat, finds when each part first has a value
//! and builds the series of every block an indicator reads. `on` then
//! answers for any one day. A backtest and a live run call the same `on`, so
//! what was tested is what trades.

mod compare;
mod plan;
mod ready;
mod span;
mod tracks;
mod weigh;

pub use compare::{holds, TOLERANCE};
pub(crate) use span::day_text;
pub use span::{Warning, WarningCode, Window};

use crate::history::History;
use crate::rules::{Asset, Indicator, Issue, IssueCode, RuleStrategy, MAX_NODES};
use compounder_prices::{calendar::Aligned, day::Day};
use plan::{Plan, Subject};
use ready::Ready;
use tracks::Tracks;
use weigh::{Ground, Memo};

/// The most work one evaluation may take, counted in closes gone over: a
/// reading on a window goes over the window, a reading or a block that keeps
/// a series for every day counts as 40, and each comparison as 6. A backtest
/// evaluates a strategy on every day of thirty years, so this is what keeps
/// one run to about a second. The largest of 137 real strategies measured
/// comes to 29,000.
pub const MAX_WORK: usize = 50_000;

/// What a strategy wants to hold at one close, and what it read to decide.
#[derive(Debug, Clone, PartialEq)]
pub struct Evaluation {
    pub day: Day,
    /// The target weight of each asset the strategy may hold, in the order
    /// of `Evaluator::assets`. They sum to 1.
    pub weights: Vec<f64>,
    /// Every reading taken to reach the weights, in the order first taken.
    /// A reading on a branch that was not held is not here.
    pub readings: Vec<Read>,
}

/// One reading and the value it had.
#[derive(Debug, Clone, PartialEq)]
pub struct Read {
    pub indicator: Indicator,
    pub on: ReadOn,
    pub value: f64,
}

/// What a reading was taken on.
#[derive(Debug, Clone, PartialEq)]
pub enum ReadOn {
    /// An asset's adjusted closes.
    Asset(Asset),
    /// A block's own series. The block is named by where it first appears
    /// in the tree, such as `root.children[1]`.
    Block(String),
}

/// A strategy made ready to be evaluated on one history.
pub struct Evaluator<'a> {
    history: &'a History,
    plan: Plan,
    /// Each asset's prices, by asset.
    prices: Vec<&'a Aligned>,
    tracks: Tracks,
    /// The assets the strategy may hold, in order, and where each sits among
    /// all the plan's assets.
    assets: Vec<Asset>,
    held: Vec<usize>,
    /// The first day the whole tree has a value, as a calendar position. It
    /// can lie past the calendar's end.
    ready: i64,
    /// The last day every asset has a price.
    last: usize,
    /// For each asset, the first day it alone would let the tree be evaluated.
    waits: Vec<i64>,
}

impl<'a> Evaluator<'a> {
    /// Check the strategy and prepare it on this history. `run_in` turns on
    /// the reference convention for the history a carried reading is given.
    /// It is for checking recorded cases and is off everywhere else: without
    /// it a carried reading starts at the first close of its subject and its
    /// value depends on the subject alone.
    pub fn new(strategy: &RuleStrategy, history: &'a History, run_in: bool) -> Result<Evaluator<'a>, Vec<Issue>> {
        let issues = strategy.validate(MAX_NODES);
        if !issues.is_empty() {
            return Err(issues);
        }
        Evaluator::prepare(strategy, history, run_in)
    }

    /// Prepare a strategy that has been checked.
    fn prepare(strategy: &RuleStrategy, history: &'a History, run_in: bool) -> Result<Evaluator<'a>, Vec<Issue>> {
        let plan = Plan::of(strategy);
        let work = plan.work();
        if work > MAX_WORK {
            return Err(vec![Issue {
                code: IssueCode::ExcessiveWork,
                path: "root".to_string(),
                message: format!("The strategy takes {work} steps to work out each day; the most allowed is {MAX_WORK}. A step is one close that a reading goes over."),
                fix: "Use fewer distinct readings or conditions, or shorter windows.".to_string(),
            }]);
        }
        let mut prices = Vec::with_capacity(plan.assets.len());
        let mut missing = Vec::new();
        for asset in &plan.assets {
            match history.of(asset) {
                Some(aligned) => prices.push(aligned),
                None => missing.push(Issue {
                    code: IssueCode::MissingHistory,
                    path: "history".to_string(),
                    message: format!("{} has no price history.", span::name(asset)),
                    fix: "Load its prices, or take it out of the strategy.".to_string(),
                }),
            }
        }
        if !missing.is_empty() {
            return Err(missing);
        }
        let start: Vec<i64> = prices.iter().map(|aligned| aligned.range().start as i64).collect();
        let last = prices.iter().map(|aligned| aligned.range().end - 1).min().unwrap_or(0);
        let mut skip = vec![0; plan.assets.len()];
        let mut ready = Ready::of(&plan, &start, &skip);
        if run_in {
            skip = ready::run_in(&plan, &start, ready.blocks[plan.root()]);
            ready = Ready::of(&plan, &start, &skip);
        }
        let tracks = Tracks::build(&plan, &prices, &ready, &skip, last);
        let waits = ready::waits(&plan, &start, &skip);
        let held: Vec<usize> = (0..plan.assets.len()).filter(|asset| plan.held[*asset]).collect();
        let assets = held.iter().map(|asset| plan.assets[*asset].clone()).collect();
        Ok(Evaluator { history, ready: ready.blocks[plan.root()], plan, prices, tracks, assets, held, last, waits })
    }

    /// The assets the strategy may hold, in order. Weights are in this order.
    pub fn assets(&self) -> &[Asset] {
        &self.assets
    }

    /// The first and the last day the strategy can be evaluated on. Nothing
    /// when there is no such day.
    pub fn days(&self) -> Option<(Day, Day)> {
        let calendar = self.history.calendar();
        let first = usize::try_from(self.ready).ok().filter(|first| *first <= self.last)?;
        Some((calendar.day(first), calendar.day(self.last)))
    }

    /// The close of each asset the strategy may hold on a trading day, in
    /// the order of `assets`. Nothing when one of them has no price that day.
    pub fn closes(&self, day: Day) -> Option<Vec<f64>> {
        let position = self.history.calendar().position(day)?;
        self.held.iter().map(|asset| self.prices[*asset].at(position)).collect()
    }

    /// What the strategy wants to hold at the close of `day`.
    pub fn on(&self, day: Day) -> Result<Evaluation, Vec<Issue>> {
        let cannot = |message: String, fix: &str| vec![Issue { code: IssueCode::NoTradingDay, path: "day".to_string(), message, fix: fix.to_string() }];
        let Some((first, last)) = self.days() else {
            return Err(cannot("There is no day on which every reading in the strategy has a value.".to_string(), "Load more price history, or shorten the windows."));
        };
        let position = self.history.calendar().position(day).filter(|_| first <= day && day <= last);
        let Some(position) = position else {
            return Err(cannot(format!("The strategy cannot be evaluated on {day}. It can be from {first} to {last}, on trading days."), "Pick a trading day in that range."));
        };
        let mut memo = Memo::new(&self.plan);
        let ground = Ground { plan: &self.plan, prices: &self.prices, tracks: &self.tracks };
        let Some(shares) = memo.weigh(&ground, self.plan.root(), position) else {
            return Err(cannot(format!("A reading in the strategy has no value on {day}."), "Load more price history."));
        };
        let mut weights = vec![0.0; self.assets.len()];
        for (asset, weight) in shares.iter() {
            // Every asset a block holds is one the strategy may hold.
            if let Ok(column) = self.held.binary_search(asset) {
                weights[column] = *weight;
            }
        }
        let readings = memo
            .read
            .iter()
            .filter_map(|place| {
                let measure = &self.plan.measures[*place];
                let on = match measure.subject {
                    Subject::Asset(asset) => ReadOn::Asset(self.plan.assets[asset].clone()),
                    Subject::Block(block) => ReadOn::Block(self.plan.paths[block].clone()),
                };
                Some(Read { indicator: measure.indicator, on, value: memo.value(*place)? })
            })
            .collect();
        Ok(Evaluation { day, weights, readings })
    }
}

/// What a strategy wants to hold at the close of `day`, and every reading it
/// took to decide. The one step a live run needs: nothing in it knows about
/// money, holdings or the days before.
pub fn evaluate(strategy: &RuleStrategy, history: &History, day: Day) -> Result<Evaluation, Vec<Issue>> {
    Evaluator::new(strategy, history, false)?.on(day)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicator;
    use crate::rules::{Comparator, Comparison, Condition, Frequency, Market, Node, Operand, Reading, Rebalance, Take, Weighting};
    use compounder_prices::adjust::{adjust, Bar};
    use compounder_prices::calendar::Calendar;

    fn listed(symbol: &str) -> Asset {
        Asset::Listed { market: Market::Equities, symbol: symbol.to_string() }
    }

    fn asset(symbol: &str) -> Node {
        Node::Asset(listed(symbol))
    }

    fn daily(root: Node) -> RuleStrategy {
        RuleStrategy { name: "Test".to_string(), rebalance: Rebalance::Calendar(Frequency::Daily), root }
    }

    /// Made-up closes that wander without repeating: quarters of a dollar.
    fn closes(start: f64, step: usize, count: usize) -> Vec<f64> {
        (0..count).map(|index| start + ((index * step) % 23) as f64 * 0.25 + index as f64 * 0.125).collect()
    }

    /// A history on consecutive days. Each asset's closes start `offset` days in.
    fn history(days: usize, assets: &[(&str, usize, &[f64])]) -> History {
        let day = |index: usize| Day::from_number(19_000 + index as i32);
        let mut history = History::new(Calendar::new((0..days).map(day)).unwrap());
        for (symbol, offset, closes) in assets {
            let bars: Vec<Bar> = closes.iter().enumerate().map(|(index, close)| Bar::new(day(offset + index), *close)).collect();
            history.add(listed(symbol), &adjust(&bars).unwrap());
        }
        history
    }

    fn weights(evaluator: &Evaluator, position: usize) -> Vec<f64> {
        evaluator.on(Day::from_number(19_000 + position as i32)).expect("a day it can be evaluated on").weights
    }

    // Validation refuses the two shapes below in every strategy today. An
    // imported one is to be let through with a warning, and the rule for it
    // is already here.

    #[test]
    fn shares_are_divided_by_their_sum() {
        let flat = [10.0; 3];
        let history = history(3, &[("AAA", 0, &flat), ("BBB", 0, &flat), ("CCC", 0, &flat)]);
        let specified = |shares: &[f64], children: Vec<Node>| Node::Weight { name: None, weighting: Weighting::Specified { shares: shares.to_vec() }, children };
        // 60 and 30 hold two thirds and one third. 70 and 70 hold half each.
        let short = daily(specified(&[0.6, 0.3], vec![asset("AAA"), asset("BBB")]));
        assert!(!short.validate(MAX_NODES).is_empty());
        let weights_of = |strategy: &RuleStrategy| weights(&Evaluator::prepare(strategy, &history, false).unwrap(), 0);
        let (two_thirds, third) = (0.6 / (0.6 + 0.3), 0.3 / (0.6 + 0.3));
        assert_eq!(weights_of(&short), [two_thirds, third]);
        assert!((two_thirds - 2.0 / 3.0).abs() < 1e-15 && (third - 1.0 / 3.0).abs() < 1e-15);
        let long = daily(specified(&[0.7, 0.7], vec![asset("AAA"), asset("BBB")]));
        assert_eq!(weights_of(&long), [0.5, 0.5]);
        // Thirds written to two decimals of a percent.
        let thirds = daily(specified(&[0.3333, 0.3333, 0.3333], vec![asset("AAA"), asset("BBB"), asset("CCC")]));
        assert_eq!(weights_of(&thirds), [0.3333 / (0.3333 + 0.3333 + 0.3333); 3]);
    }

    #[test]
    fn a_filter_asked_to_keep_more_than_it_holds_keeps_them_all() {
        let (up, down) = ([10.0, 11.0, 12.0], [10.0, 9.0, 8.0]);
        let history = history(3, &[("AAA", 0, &up), ("BBB", 0, &down)]);
        let over = daily(Node::Filter { sort_by: Indicator::CumulativeReturn { window: 1 }, take: Take::Top, count: 5, children: vec![asset("AAA"), asset("BBB")] });
        assert!(!over.validate(MAX_NODES).is_empty());
        assert_eq!(weights(&Evaluator::prepare(&over, &history, false).unwrap(), 2), [0.5, 0.5]);
    }

    /// Hold OLD while its 5 day relative strength is above a half, else YOUNG.
    fn strength_of_old() -> RuleStrategy {
        let left = Reading { indicator: Indicator::RelativeStrength { window: 5 }, asset: listed("OLD") };
        let condition = Condition::Compare(Comparison { left, comparator: Comparator::Above, right: Operand::Fixed(0.5) });
        daily(Node::If { condition, then: vec![asset("OLD")], otherwise: vec![asset("YOUNG")] })
    }

    #[test]
    fn a_strategy_that_takes_too_much_working_out_is_refused() {
        let prices = closes(40.0, 7, 30);
        let history = history(30, &[("OLD", 0, &prices), ("YOUNG", 0, &prices)]);
        // Each distinct average over a window of 1,000 goes over 1,000 closes.
        let averages = |count: u32| {
            let parts = (0..count).map(|index| {
                let left = Reading { indicator: Indicator::AveragePrice { window: 1000 - index }, asset: listed("OLD") };
                Condition::Compare(Comparison { left, comparator: Comparator::Above, right: Operand::Fixed(1.0) })
            });
            // A group holds 50 conditions at most, so they go in groups of 30.
            let groups = parts.collect::<Vec<_>>().chunks(30).map(|group| Condition::All(group.to_vec())).collect();
            daily(Node::If { condition: Condition::All(groups), then: vec![asset("OLD")], otherwise: vec![asset("YOUNG")] })
        };
        assert!(Evaluator::new(&averages(40), &history, false).is_ok());
        let issues = Evaluator::new(&averages(60), &history, false).err().expect("too much work");
        assert_eq!(issues.iter().map(|issue| issue.code).collect::<Vec<_>>(), [IssueCode::ExcessiveWork]);
        // Asking the same thing many times is one reading, and costs little more than once.
        let same = Condition::All((0..50).map(|_| Condition::Compare(Comparison { left: Reading { indicator: Indicator::AveragePrice { window: 1000 }, asset: listed("OLD") }, comparator: Comparator::Above, right: Operand::Fixed(1.0) })).collect());
        assert!(Evaluator::new(&daily(Node::If { condition: same, then: vec![asset("OLD")], otherwise: vec![asset("YOUNG")] }), &history, false).is_ok());
    }

    #[test]
    fn a_carried_reading_starts_at_the_first_close_of_its_subject() {
        let (old, young) = (closes(40.0, 7, 600), closes(15.0, 5, 100));
        let history = history(600, &[("OLD", 0, &old), ("YOUNG", 500, &young)]);
        let evaluator = Evaluator::new(&strength_of_old(), &history, false).unwrap();
        // The strategy starts with YOUNG, on day 500, and the reading has run since day 0.
        assert_eq!(evaluator.days().map(|(first, _)| first), Some(Day::from_number(19_500)));
        for position in [500, 501, 550, 599] {
            let evaluation = evaluator.on(Day::from_number(19_000 + position)).unwrap();
            let whole = indicator::at(Indicator::RelativeStrength { window: 5 }, &old[..=position as usize]);
            assert_eq!(evaluation.readings.iter().map(|read| Some(read.value)).collect::<Vec<_>>(), [whole], "{position}");
        }
    }

    #[test]
    fn under_the_run_in_convention_a_carried_reading_starts_a_set_time_before_the_first_day() {
        let (old, young) = (closes(40.0, 7, 600), closes(15.0, 5, 100));
        let history = history(600, &[("OLD", 0, &old), ("YOUNG", 500, &young)]);
        let evaluator = Evaluator::new(&strength_of_old(), &history, true).unwrap();
        assert_eq!(evaluator.days().map(|(first, _)| first), Some(Day::from_number(19_500)));
        // The first day is 500. A 5 day strength is given its 5 closes and 83 more for
        // its seed to fade below 1e-8, since 0.8 to the 83rd is the first power under it.
        assert!(0.8_f64.powi(83) < 1e-8 && 0.8_f64.powi(82) > 1e-8);
        let from = 500 - 5 - 83;
        for position in [500, 501, 550, 599] {
            let evaluation = evaluator.on(Day::from_number(19_000 + position)).unwrap();
            let cut = indicator::at(Indicator::RelativeStrength { window: 5 }, &old[from..=position as usize]).unwrap();
            let whole = indicator::at(Indicator::RelativeStrength { window: 5 }, &old[..=position as usize]).unwrap();
            assert_eq!(evaluation.readings[0].value, cut, "{position}");
            // The two differ by less than the seed's weight, and after a hundred closes more by nothing.
            assert!((cut - whole).abs() < 1e-8 && (cut != whole) == (position < 599), "{position}: {cut} {whole}");
        }
    }

    #[test]
    fn the_run_in_reaches_as_far_as_the_longest_reading_on_the_asset_asks() {
        // A 200 day average beside the 5 day strength: both on OLD, so the strength is given the 199 closes too.
        let (old, young) = (closes(40.0, 7, 600), closes(15.0, 5, 100));
        let history = history(600, &[("OLD", 0, &old), ("YOUNG", 500, &young)]);
        let strength = Reading { indicator: Indicator::RelativeStrength { window: 5 }, asset: listed("OLD") };
        let average = Reading { indicator: Indicator::AveragePrice { window: 200 }, asset: listed("OLD") };
        let both = Condition::All(vec![
            Condition::Compare(Comparison { left: strength, comparator: Comparator::Above, right: Operand::Fixed(0.5) }),
            Condition::Compare(Comparison { left: Reading { indicator: Indicator::Price, asset: listed("OLD") }, comparator: Comparator::Above, right: Operand::Reading(average) }),
        ]);
        let strategy = daily(Node::If { condition: both, then: vec![asset("OLD")], otherwise: vec![asset("YOUNG")] });
        let evaluation = Evaluator::new(&strategy, &history, true).unwrap().on(Day::from_number(19_500)).unwrap();
        let cut = indicator::at(Indicator::RelativeStrength { window: 5 }, &old[500 - 199..=500]);
        assert_eq!(Some(evaluation.readings[0].value), cut);
        // A reading on a window is the same wherever its series is cut.
        assert_eq!(Some(evaluation.readings[2].value), indicator::at(Indicator::AveragePrice { window: 200 }, &old[..=500]));
    }
}
