//! Shared by the tests that run a strategy on made-up prices.
#![allow(dead_code)]

use compounder_engine::history::History;
use compounder_engine::rules::{Asset, Comparator, Comparison, Condition, Frequency, Indicator, Market, Node, Operand, Reading, Rebalance, RuleStrategy, Take, Weighting};
use compounder_prices::{
    adjust::{adjust, Bar},
    calendar::Calendar,
    day::Day,
};

pub fn day(text: &str) -> Day {
    text.parse().expect("a day written as 2026-10-09")
}

/// Every weekday from `first` on, `count` of them. A made-up calendar has no
/// holidays unless a test leaves a day out itself.
pub fn weekdays(first: &str, count: usize) -> Vec<Day> {
    // 1970-01-01 was a Thursday, so days 2 and 3 of each seven are a weekend.
    (day(first).number()..).map(Day::from_number).filter(|day| !matches!(day.number().rem_euclid(7), 2 | 3)).take(count).collect()
}

pub fn listed(symbol: &str) -> Asset {
    Asset::Listed { market: Market::Equities, symbol: symbol.to_string() }
}

/// A history on the given trading days. Each asset's closes start on the
/// trading day at `offset` and run one a day from there.
pub fn history(days: &[Day], assets: &[(&str, usize, &[f64])]) -> History {
    let mut history = History::new(Calendar::new(days.iter().copied()).expect("some days"));
    for (symbol, offset, closes) in assets {
        let bars: Vec<Bar> = closes.iter().enumerate().map(|(index, close)| Bar::new(days[offset + index], *close)).collect();
        history.add(listed(symbol), &adjust(&bars).expect("a sound series"));
    }
    history
}

pub fn asset(symbol: &str) -> Node {
    Node::Asset(listed(symbol))
}

pub fn equal(children: Vec<Node>) -> Node {
    Node::Weight { name: None, weighting: Weighting::Equal, children }
}

pub fn specified(shares: &[f64], children: Vec<Node>) -> Node {
    Node::Weight { name: None, weighting: Weighting::Specified { shares: shares.to_vec() }, children }
}

pub fn inverse_volatility(window: u32, children: Vec<Node>) -> Node {
    Node::Weight { name: None, weighting: Weighting::InverseVolatility { window }, children }
}

pub fn filter(sort_by: Indicator, take: Take, count: u32, children: Vec<Node>) -> Node {
    Node::Filter { sort_by, take, count, children }
}

pub fn when(condition: Condition, then: Vec<Node>, otherwise: Vec<Node>) -> Node {
    Node::If { condition, then, otherwise }
}

pub fn reading(indicator: Indicator, symbol: &str) -> Reading {
    Reading { indicator, asset: listed(symbol) }
}

/// The indicator on an asset, held against a fixed value.
pub fn against(indicator: Indicator, symbol: &str, comparator: Comparator, fixed: f64) -> Condition {
    Condition::Compare(Comparison { left: reading(indicator, symbol), comparator, right: Operand::Fixed(fixed) })
}

/// One reading held against another.
pub fn between(left: Reading, comparator: Comparator, right: Reading) -> Condition {
    Condition::Compare(Comparison { left, comparator, right: Operand::Reading(right) })
}

pub fn daily(root: Node) -> RuleStrategy {
    scheduled(Frequency::Daily, root)
}

pub fn scheduled(frequency: Frequency, root: Node) -> RuleStrategy {
    RuleStrategy { name: "Test".to_string(), rebalance: Rebalance::Calendar(frequency), root }
}

pub fn threshold(corridor: f64, root: Node) -> RuleStrategy {
    RuleStrategy { name: "Test".to_string(), rebalance: Rebalance::Threshold { corridor }, root }
}

pub fn close(got: f64, expected: f64, tolerance: f64) -> bool {
    (got - expected).abs() <= tolerance * expected.abs().max(1.0)
}

pub fn all_close(got: &[f64], expected: &[f64], tolerance: f64) -> bool {
    got.len() == expected.len() && got.iter().zip(expected).all(|(got, expected)| close(*got, *expected, tolerance))
}
