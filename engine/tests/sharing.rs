//! The evaluator lays a tree flat, weighs each distinct block once a day and
//! works out when everything is ready without walking the days. None of that
//! may change an answer. Here a few hundred made-up trees, full of repeated
//! branches and of blocks read as assets, are evaluated twice: by the
//! engine, and the long way, straight from the rules, sharing nothing.

mod support;

use compounder_engine::evaluate::{holds, Evaluator};
use compounder_engine::history::History;
use compounder_engine::indicator;
use compounder_engine::rules::{Asset, Comparator, Comparison, Condition, Indicator, Node, Operand, Reading, RuleStrategy, Take, Weighting, MAX_NODES};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use support::*;

const DAYS: usize = 70;
/// Six assets. The fourth and fifth are listed late, and the sixth does not move for a stretch.
const SYMBOLS: [(&str, usize); 6] = [("AAA", 0), ("BBB", 0), ("CCC", 0), ("DDD", 6), ("EEE", 13), ("FFF", 0)];

struct Dice(u64);

impl Dice {
    fn roll(&mut self, sides: usize) -> usize {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % sides as u64) as usize
    }
}

// ---- made-up prices and trees ----

/// Closes in steps of a quarter, by symbol, each from its own first day.
fn prices(dice: &mut Dice) -> BTreeMap<String, (usize, Vec<f64>)> {
    let mut all = BTreeMap::new();
    for (symbol, offset) in SYMBOLS {
        let mut price = 20.0 + dice.roll(60) as f64;
        let closes = (offset..DAYS)
            .map(|day| {
                // FFF stands still from day 30 to day 40.
                if !(symbol == "FFF" && (30..=40).contains(&day)) {
                    price = (price + (dice.roll(9) as f64 - 4.0) * 0.25).max(1.0);
                }
                price
            })
            .collect();
        all.insert(symbol.to_string(), (offset, closes));
    }
    all
}

fn percent_indicator(dice: &mut Dice) -> Indicator {
    let window = 2 + dice.roll(4) as u32;
    match dice.roll(5) {
        0 => Indicator::CumulativeReturn { window },
        1 => Indicator::AverageReturn { window },
        2 => Indicator::ReturnDeviation { window },
        3 => Indicator::MaxDrawdown { window },
        _ => Indicator::RelativeStrength { window },
    }
}

fn any_indicator(dice: &mut Dice) -> Indicator {
    let window = 2 + dice.roll(4) as u32;
    match dice.roll(9) {
        0 => Indicator::Price,
        1 => Indicator::AveragePrice { window },
        2 => Indicator::ExponentialAveragePrice { window },
        3 => Indicator::PriceDeviation { window },
        _ => percent_indicator(dice),
    }
}

fn some_symbol(dice: &mut Dice) -> &'static str {
    SYMBOLS[dice.roll(SYMBOLS.len())].0
}

fn comparison(dice: &mut Dice) -> Condition {
    let indicator = any_indicator(dice);
    let comparator = [Comparator::Above, Comparator::AtLeast, Comparator::Equal, Comparator::AtMost, Comparator::Below][dice.roll(5)];
    // Against a reading of the same kind on another asset, or a fixed value near where such readings sit.
    let right = match (dice.roll(2), indicator) {
        (0, _) => Operand::Reading(reading(indicator, some_symbol(dice))),
        (_, Indicator::Price | Indicator::AveragePrice { .. } | Indicator::ExponentialAveragePrice { .. }) => Operand::Fixed(30.0 + dice.roll(30) as f64),
        (_, Indicator::RelativeStrength { .. }) => Operand::Fixed(0.3 + dice.roll(5) as f64 * 0.1),
        (_, Indicator::PriceDeviation { .. }) => Operand::Fixed(0.5),
        _ => Operand::Fixed([0.0, 0.01, -0.01, 0.02][dice.roll(4)]),
    };
    Condition::Compare(Comparison { left: reading(indicator, some_symbol(dice)), comparator, right })
}

fn test(dice: &mut Dice) -> Condition {
    match dice.roll(5) {
        0 => Condition::Any(vec![comparison(dice), comparison(dice)]),
        1 => Condition::All(vec![comparison(dice), Condition::Any(vec![comparison(dice), comparison(dice)])]),
        _ => comparison(dice),
    }
}

/// A made-up block. One in three is a copy of a block made before, so trees repeat themselves.
fn block(depth: usize, made: &mut Vec<Node>, dice: &mut Dice) -> Node {
    if depth == 0 || dice.roll(4) == 0 {
        return asset(some_symbol(dice));
    }
    if !made.is_empty() && dice.roll(3) == 0 {
        return made[dice.roll(made.len())].clone();
    }
    let children = |count: usize, made: &mut Vec<Node>, dice: &mut Dice| (0..count).map(|_| block(depth - 1, made, dice)).collect::<Vec<Node>>();
    let node = match dice.roll(6) {
        0 => equal(children(2 + dice.roll(2), made, dice)),
        1 => specified(&[[0.5, 0.5, 0.0], [0.2, 0.3, 0.5], [1.0, 0.0, 0.0]][dice.roll(3)], children(3, made, dice)),
        2 => inverse_volatility(2 + dice.roll(4) as u32, children(2 + dice.roll(2), made, dice)),
        3 | 4 => when(test(dice), children(1 + dice.roll(2), made, dice), children(1 + dice.roll(2), made, dice)),
        _ => {
            let kept = children(2 + dice.roll(3), made, dice);
            let take = [Take::Top, Take::Bottom][dice.roll(2)];
            filter(percent_indicator(dice), take, 1 + dice.roll(kept.len()) as u32, kept)
        }
    };
    made.push(node.clone());
    node
}

// ---- the long way ----

/// Evaluates a tree straight from the rules. Each place in the tree is
/// worked out on its own: equal blocks in two places are worked out twice,
/// and a block's series is walked from the first day the block is ready.
/// An answer is kept by the place it was asked of, only so the test ends.
struct LongWay<'a> {
    prices: &'a BTreeMap<String, (usize, Vec<f64>)>,
    last: usize,
    ready: RefCell<HashMap<(usize, usize), bool>>,
    held: RefCell<HashMap<(usize, usize), Option<Held>>>,
    first: RefCell<HashMap<usize, Option<usize>>>,
}

type Held = BTreeMap<String, f64>;

/// Where a block sits in memory, which names its place in the tree.
fn place(node: &Node) -> usize {
    std::ptr::from_ref(node) as usize
}

fn symbol(asset: &Asset) -> &str {
    match asset {
        Asset::Listed { symbol, .. } => symbol,
        Asset::Token { id } => id,
    }
}

impl LongWay<'_> {
    fn close(&self, symbol: &str, day: usize) -> Option<f64> {
        let (offset, closes) = &self.prices[symbol];
        closes.get(day.checked_sub(*offset)?).copied()
    }

    /// An indicator on an asset at a day: over its closes from its first one.
    fn on_asset(&self, indicator: Indicator, symbol: &str, day: usize) -> Option<f64> {
        let (offset, closes) = &self.prices[symbol];
        indicator::at(indicator, closes.get(..=day.checked_sub(*offset)?)?)
    }

    /// An indicator on any block at a day: an asset on its prices, another block on its own series.
    fn on_block(&self, indicator: Indicator, node: &Node, day: usize) -> Option<f64> {
        if let Node::Asset(asset) = node {
            return self.on_asset(indicator, symbol(asset), day);
        }
        // The series starts at one on the first day the block can be evaluated,
        // and each day earns the return of what the block held at the close before.
        let known = self.first.borrow().get(&place(node)).copied();
        let first = known.unwrap_or_else(|| {
            let first = (0..=self.last).find(|day| self.ready(node, *day));
            self.first.borrow_mut().insert(place(node), first);
            first
        })?;
        if day < first {
            return None;
        }
        let mut levels = vec![1.0];
        for today in first + 1..=day {
            let mut earned = 0.0;
            for (symbol, weight) in self.held(node, today - 1)? {
                earned += weight * (self.close(&symbol, today)? / self.close(&symbol, today - 1)? - 1.0);
            }
            levels.push(levels[levels.len() - 1] * (1.0 + earned));
        }
        indicator::at(indicator, &levels)
    }

    fn compared(&self, comparison: &Comparison, day: usize) -> Option<bool> {
        let left = self.on_asset(comparison.left.indicator, symbol(&comparison.left.asset), day)?;
        let right = match &comparison.right {
            Operand::Fixed(value) => *value,
            Operand::Reading(Reading { indicator, asset }) => self.on_asset(*indicator, symbol(asset), day)?,
        };
        Some(holds(comparison.comparator, left, right))
    }

    /// Whether a condition holds. Nothing when any reading in it, on either side of any member, has no value.
    fn holds(&self, condition: &Condition, day: usize) -> Option<bool> {
        match condition {
            Condition::Compare(comparison) => self.compared(comparison, day),
            Condition::Any(parts) => Some(parts.iter().map(|part| self.holds(part, day)).collect::<Option<Vec<bool>>>()?.contains(&true)),
            Condition::All(parts) => Some(!parts.iter().map(|part| self.holds(part, day)).collect::<Option<Vec<bool>>>()?.contains(&false)),
        }
    }

    /// Whether everything in a block has a value on a day: every asset a price,
    /// every reading a value, in the branches it would not take as much as in the one it would.
    fn ready(&self, node: &Node, day: usize) -> bool {
        if let Some(known) = self.ready.borrow().get(&(place(node), day)) {
            return *known;
        }
        let all = |nodes: &[Node]| nodes.iter().all(|child| self.ready(child, day));
        let ready = match node {
            Node::Asset(asset) => self.close(symbol(asset), day).is_some(),
            Node::Weight { weighting, children, .. } => {
                let measured = match weighting {
                    Weighting::InverseVolatility { window } => children.iter().all(|child| self.on_block(Indicator::ReturnDeviation { window: *window }, child, day).is_some()),
                    _ => true,
                };
                measured && all(children)
            }
            Node::If { condition, then, otherwise } => self.holds(condition, day).is_some() && all(then) && all(otherwise),
            Node::Filter { sort_by, children, .. } => children.iter().all(|child| self.on_block(*sort_by, child, day).is_some()) && all(children),
        };
        self.ready.borrow_mut().insert((place(node), day), ready);
        ready
    }

    /// What a block holds at a close, when it is ready.
    fn held(&self, node: &Node, day: usize) -> Option<Held> {
        if let Some(known) = self.held.borrow().get(&(place(node), day)) {
            return known.clone();
        }
        let held = self.weigh(node, day);
        self.held.borrow_mut().insert((place(node), day), held.clone());
        held
    }

    fn weigh(&self, node: &Node, day: usize) -> Option<Held> {
        if day > self.last || !self.ready(node, day) {
            return None;
        }
        let parts: Vec<(&Node, f64)> = match node {
            Node::Asset(asset) => return Some(Held::from([(symbol(asset).to_string(), 1.0)])),
            Node::Weight { weighting, children, .. } => {
                let shares: Vec<f64> = match weighting {
                    Weighting::Equal => vec![1.0 / children.len() as f64; children.len()],
                    Weighting::Specified { shares } => shares.iter().map(|share| share / shares.iter().sum::<f64>()).collect(),
                    Weighting::InverseVolatility { window } => {
                        let deviations = children.iter().map(|child| self.on_block(Indicator::ReturnDeviation { window: *window }, child, day)).collect::<Option<Vec<f64>>>()?;
                        let still = deviations.iter().filter(|deviation| **deviation == 0.0).count();
                        let total: f64 = deviations.iter().map(|deviation| 1.0 / deviation).sum();
                        deviations.iter().map(|deviation| if still > 0 { f64::from(u8::from(*deviation == 0.0)) / still as f64 } else { 1.0 / deviation / total }).collect()
                    }
                };
                children.iter().zip(shares).collect()
            }
            Node::If { condition, then, otherwise } => {
                let branch = if self.holds(condition, day)? { then } else { otherwise };
                branch.iter().map(|child| (child, 1.0 / branch.len() as f64)).collect()
            }
            Node::Filter { sort_by, take, count, children } => {
                let mut scored = children.iter().map(|child| self.on_block(*sort_by, child, day).map(|score| (score, child))).collect::<Option<Vec<(f64, &Node)>>>()?;
                // Stable, so ties keep list order.
                scored.sort_by(|first, second| match take {
                    Take::Top => second.0.partial_cmp(&first.0).unwrap(),
                    Take::Bottom => first.0.partial_cmp(&second.0).unwrap(),
                });
                scored.truncate(*count as usize);
                scored.iter().map(|(_, child)| (*child, 1.0 / scored.len() as f64)).collect()
            }
        };
        let mut held = Held::new();
        for (child, share) in parts {
            if share != 0.0 {
                for (symbol, weight) in self.held(child, day)? {
                    *held.entry(symbol).or_insert(0.0) += share * weight;
                }
            }
        }
        Some(held)
    }
}

#[test]
fn laying_a_tree_flat_changes_no_answer() {
    let days = weekdays("2025-01-06", DAYS);
    let mut dice = Dice(2_024);
    let all = prices(&mut dice);
    let series: Vec<(&str, usize, &[f64])> = all.iter().map(|(symbol, (offset, closes))| (symbol.as_str(), *offset, &closes[..])).collect();
    let history: History = history(&days, &series);

    let (mut trees, mut evaluated, mut never) = (0, 0, 0);
    while trees < 300 {
        let mut made = Vec::new();
        let strategy: RuleStrategy = daily(block(4, &mut made, &mut dice));
        assert!(strategy.validate(MAX_NODES).is_empty(), "{:#?}", strategy.root);
        trees += 1;
        // Places are only the same within one tree, so each tree starts with nothing kept.
        let long_way = LongWay { prices: &all, last: DAYS - 1, ready: RefCell::default(), held: RefCell::default(), first: RefCell::default() };
        let evaluator = Evaluator::new(&strategy, &history, false).expect("a sound strategy");
        let first = (0..DAYS).find(|day| long_way.ready(&strategy.root, *day));
        assert_eq!(evaluator.days().map(|(first, _)| first), first.map(|first| days[first]), "tree {trees}: {:#?}", strategy.root);
        let Some(first) = first else {
            never += 1;
            continue;
        };
        for day in (first..DAYS).step_by(3) {
            let expected = long_way.held(&strategy.root, day).expect("ready from its first day on");
            let evaluation = evaluator.on(days[day]).unwrap_or_else(|issues| panic!("tree {trees}, day {day}: {issues:#?}"));
            for (asset, weight) in evaluator.assets().iter().zip(&evaluation.weights) {
                let wanted = expected.get(symbol(asset)).copied().unwrap_or(0.0);
                assert!((weight - wanted).abs() <= 1e-12, "tree {trees}, day {day}: {} is held at {weight}, not {wanted}\n{:#?}", symbol(asset), strategy.root);
            }
            evaluated += 1;
        }
    }
    // Most trees can be evaluated on most days, or the test proves little.
    assert!(evaluated > 3_000 && never < 30, "{evaluated} days evaluated, {never} trees never ready");
}
