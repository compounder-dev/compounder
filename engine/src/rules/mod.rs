//! Rule-based strategies: a tree of blocks that decides what to hold from
//! market data, afresh at every rebalance.
//!
//! This module is the language only: the typed tree, an importer from the
//! block-tree JSON that strategy editors write, validation, and a canonical
//! form with a hash. Nothing here reads a price or picks a holding. The tree
//! is shaped so an evaluator can be built on top without changing it.
//!
//! The tree is the meaning of a strategy and nothing more. It has no block
//! ids and no display state, and each fact has one spelling: a window is a
//! whole number, a share is a fraction, and a fixed value sits in the scale
//! of the indicator it is compared with. Store the original document beside
//! it: the tree cannot give it back.

use serde::Serialize;

pub(crate) mod canonical;
mod fixed;
mod import;
mod validate;

pub use import::{import, import_json};

/// The node limit to pass `validate` when no tighter one applies.
pub const MAX_NODES: usize = 15_000;
/// Deepest nesting of blocks a strategy may have, counting its first block.
pub const MAX_DEPTH: usize = 100;
/// Most conditions one any/all group may hold.
pub const MAX_CONDITIONS: usize = 50;
/// Deepest nesting of any/all groups in one condition.
pub const MAX_CONDITION_DEPTH: usize = 20;
/// Longest window an indicator may look back over, in trading days.
pub const MAX_WINDOW: u32 = 1000;

#[derive(Debug, Clone, PartialEq)]
pub struct RuleStrategy {
    pub name: String,
    pub rebalance: Rebalance,
    pub root: Node,
}

/// When a portfolio that follows the strategy is brought back to its targets.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Rebalance {
    /// On a fixed schedule.
    Calendar(Frequency),
    /// Once a holding has drifted this far from its target. A fraction: 0.05 is 5%.
    Threshold { corridor: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Quarterly,
    Yearly,
}

impl Frequency {
    const ALL: [Frequency; 5] = [Frequency::Daily, Frequency::Weekly, Frequency::Monthly, Frequency::Quarterly, Frequency::Yearly];

    fn word(self) -> &'static str {
        match self {
            Frequency::Daily => "daily",
            Frequency::Weekly => "weekly",
            Frequency::Monthly => "monthly",
            Frequency::Quarterly => "quarterly",
            Frequency::Yearly => "yearly",
        }
    }

    fn named(word: &str) -> Option<Frequency> {
        Self::ALL.into_iter().find(|frequency| frequency.word() == word)
    }
}

/// One block of the tree. Every block resolves to holdings, so any block can
/// sit wherever a block is allowed.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Asset(Asset),
    /// Splits its share among its children. A named equal weight is what an
    /// editor calls a group: the name changes nothing that is traded.
    Weight { name: Option<String>, weighting: Weighting, children: Vec<Node> },
    /// Holds `then` when the condition is true and `otherwise` when it is not.
    /// How several blocks in one branch share it is the evaluator's rule.
    If { condition: Condition, then: Vec<Node>, otherwise: Vec<Node> },
    /// Ranks its children by an indicator and keeps `count` of them. A child
    /// that is a block is ranked on the value of its own holdings over time.
    Filter { sort_by: Indicator, take: Take, count: u32, children: Vec<Node> },
}

/// Something a strategy can hold, or read an indicator on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Asset {
    /// A security or coin by its market symbol, such as `SPY` or `BRK/B`.
    Listed { market: Market, symbol: String },
    /// A token on a chain: `0x<token address>:<chain id>`, in lower case so
    /// that one token is one value.
    Token { id: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Market {
    Equities,
    Crypto,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Weighting {
    /// Every child gets the same share.
    Equal,
    /// Child `i` gets `shares[i]` of this block, as a fraction.
    Specified { shares: Vec<f64> },
    /// A child's share is in proportion to one over the volatility of its
    /// daily returns across the last `window` trading days.
    InverseVolatility { window: u32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Take {
    /// Keep the largest values.
    Top,
    /// Keep the smallest values.
    Bottom,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Condition {
    Compare(Comparison),
    /// True when at least one of the conditions is.
    Any(Vec<Condition>),
    /// True when every one of the conditions is.
    All(Vec<Condition>),
}

/// The left side is always an indicator, and it sets the scale the two sides
/// are compared in.
#[derive(Debug, Clone, PartialEq)]
pub struct Comparison {
    pub left: Reading,
    pub comparator: Comparator,
    pub right: Operand,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Comparator {
    Above,
    AtLeast,
    /// How close counts as equal is the evaluator's rule.
    Equal,
    Below,
    AtMost,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// A number in the scale of the left indicator: see `Unit`.
    Fixed(f64),
    Reading(Reading),
}

/// An indicator read on one asset's price history.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub indicator: Indicator,
    pub asset: Asset,
}

/// A number worked out from a daily series of closing prices. Windows are in
/// trading days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Indicator {
    /// The latest close.
    Price,
    /// The mean of the closes.
    AveragePrice { window: u32 },
    /// The mean of the closes, weighted toward the recent ones.
    ExponentialAveragePrice { window: u32 },
    /// The standard deviation of the closes.
    PriceDeviation { window: u32 },
    /// The total return across the window.
    CumulativeReturn { window: u32 },
    /// The mean of the daily returns.
    AverageReturn { window: u32 },
    /// The standard deviation of the daily returns.
    ReturnDeviation { window: u32 },
    /// The largest fall from a peak inside the window.
    MaxDrawdown { window: u32 },
    /// The relative strength index, from 0 to 1.
    RelativeStrength { window: u32 },
}

/// What an indicator's value is a measure of.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    /// A ratio, held as a fraction: 0.79 is 79%.
    Percent,
    /// Dollars per share or per token.
    Dollars,
}

impl Indicator {
    pub fn unit(&self) -> Unit {
        match self {
            Indicator::Price
            | Indicator::AveragePrice { .. }
            | Indicator::ExponentialAveragePrice { .. }
            | Indicator::PriceDeviation { .. } => Unit::Dollars,
            Indicator::CumulativeReturn { .. }
            | Indicator::AverageReturn { .. }
            | Indicator::ReturnDeviation { .. }
            | Indicator::MaxDrawdown { .. }
            | Indicator::RelativeStrength { .. } => Unit::Percent,
        }
    }

    /// How many trading days it looks back over. The latest price has none.
    pub fn window(&self) -> Option<u32> {
        match *self {
            Indicator::Price => None,
            Indicator::AveragePrice { window }
            | Indicator::ExponentialAveragePrice { window }
            | Indicator::PriceDeviation { window }
            | Indicator::CumulativeReturn { window }
            | Indicator::AverageReturn { window }
            | Indicator::ReturnDeviation { window }
            | Indicator::MaxDrawdown { window }
            | Indicator::RelativeStrength { window } => Some(window),
        }
    }

    /// The shortest window it has a meaning over. One value has no
    /// deviation, one close has no fall inside it, and relative strength
    /// needs a change to average.
    pub fn least_window(&self) -> u32 {
        match self {
            Indicator::PriceDeviation { .. } | Indicator::ReturnDeviation { .. } | Indicator::MaxDrawdown { .. } | Indicator::RelativeStrength { .. } => 2,
            _ => 1,
        }
    }
}

impl RuleStrategy {
    /// What the strategy counts as against a node limit: one for the root,
    /// one for each block, and two for each if (one per branch). The
    /// comparisons inside a condition are not counted.
    pub fn node_count(&self) -> usize {
        1 + self.root.node_count()
    }
}

impl Node {
    fn node_count(&self) -> usize {
        let count = |nodes: &[Node]| nodes.iter().map(Node::node_count).sum::<usize>();
        match self {
            Node::Asset(_) => 1,
            Node::Weight { children, .. } | Node::Filter { children, .. } => 1 + count(children),
            Node::If { then, otherwise, .. } => 2 + count(then) + count(otherwise),
        }
    }
}

/// Is this a market symbol: capital letters, with a slash before a share class?
fn is_symbol(symbol: &str) -> bool {
    symbol.split('/').all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_uppercase()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IssueCode {
    InvalidJson,
    InvalidBlockType,
    InvalidStructure,
    UnknownField,
    MissingField,
    InvalidField,
    UnsupportedBlock,
    IncompleteStrategy,
    IncompleteBlock,
    IncompleteCondition,
    InvalidCondition,
    InvalidIndicator,
    InvalidWindow,
    InvalidAsset,
    InvalidWeightValue,
    InvalidWeightTotal,
    InconsistentWeights,
    InvalidFilter,
    InvalidFixedValue,
    InvalidRebalance,
    ExcessiveNodes,
    ExcessiveDepth,
    ExcessiveConditions,
    /// An asset in the strategy has no prices to evaluate it on.
    MissingHistory,
    /// There is no trading day on which the strategy can be evaluated.
    NoTradingDay,
    /// A backtest was asked for with settings it cannot run on.
    InvalidRun,
}

/// One problem, tied to the place it is in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Issue {
    pub code: IssueCode,
    /// The way from the root to the problem, such as `root.children[1].then[0]`.
    /// An import problem is placed in the document that was read, a
    /// validation problem in the tree.
    pub path: String,
    /// What is wrong, in plain words.
    pub message: String,
    /// What to change.
    pub fix: String,
}

/// Where a walk has got to. Kept as steps, so a path is only written out
/// when there is a problem to report.
#[derive(Default)]
struct Trail {
    steps: Vec<(&'static str, Option<usize>)>,
    issues: Vec<Issue>,
}

impl Trail {
    fn enter(&mut self, field: &'static str) {
        self.steps.push((field, None));
    }

    fn enter_item(&mut self, field: &'static str, index: usize) {
        self.steps.push((field, Some(index)));
    }

    fn leave(&mut self) {
        self.steps.pop();
    }

    fn report(&mut self, code: IssueCode, message: impl Into<String>, fix: impl Into<String>) {
        let mut path = String::from("root");
        for (field, index) in &self.steps {
            path.push('.');
            path.push_str(field);
            if let Some(index) = index {
                path.push_str(&format!("[{index}]"));
            }
        }
        self.report_at(&path, code, message, fix);
    }

    fn report_at(&mut self, path: &str, code: IssueCode, message: impl Into<String>, fix: impl Into<String>) {
        self.issues.push(Issue { code, path: path.to_string(), message: message.into(), fix: fix.into() });
    }
}
