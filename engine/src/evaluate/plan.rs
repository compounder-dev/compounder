//! The tree laid flat: every distinct block once, after the blocks inside it.
//!
//! Real strategies repeat themselves. One group of holdings sits under
//! hundreds of branches, and one reading is asked for thousands of times.
//! Here each distinct sub-tree becomes one block and each distinct reading
//! one measure, so a day's work is done once however often the tree says it.
//! Two sub-trees are one block when their canonical forms are the same text.

use crate::rules::{canonical, Asset, Comparator, Condition, Indicator, Node, Operand, Reading, RuleStrategy, Take, Weighting};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};

/// What taking one reading costs beyond going over its closes, counted in closes.
const MEASURE_STEPS: usize = 6;
/// What keeping a series for every day costs, spread over the days.
const SERIES_STEPS: usize = 40;

pub(crate) struct Plan {
    /// Every asset the tree holds or reads, in order.
    pub assets: Vec<Asset>,
    /// Whether the tree may hold each asset, or only reads it.
    pub held: Vec<bool>,
    /// Every distinct block. A block comes after the blocks inside it, and
    /// the last one is the root.
    pub blocks: Vec<Block>,
    /// Where each block first appears in the tree, such as `root.then[0]`.
    pub paths: Vec<String>,
    /// Every distinct reading.
    pub measures: Vec<Measure>,
}

/// Blocks, assets and measures are named by their place in the plan.
pub(crate) enum Block {
    Asset(usize),
    Weight { split: Split, children: Vec<usize> },
    If { test: Test, then: Vec<usize>, otherwise: Vec<usize> },
    /// `scores[i]` is the measure that ranks `children[i]`.
    Filter { scores: Vec<usize>, take: Take, count: usize, children: Vec<usize> },
}

pub(crate) enum Split {
    Equal,
    Specified(Vec<f64>),
    /// The measure of each child's deviation of returns, in child order.
    InverseVolatility(Vec<usize>),
}

pub(crate) enum Test {
    Compare { left: usize, comparator: Comparator, right: Side },
    Any(Vec<Test>),
    All(Vec<Test>),
}

pub(crate) enum Side {
    Fixed(f64),
    Measure(usize),
}

/// An indicator on an asset's prices, or on a block's own series.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Measure {
    pub indicator: Indicator,
    pub subject: Subject,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Subject {
    Asset(usize),
    Block(usize),
}

impl Block {
    /// The blocks directly inside this one, in order.
    pub fn inside(&self) -> impl Iterator<Item = usize> + '_ {
        let (first, second): (&[usize], &[usize]) = match self {
            Block::Asset(_) => (&[], &[]),
            Block::Weight { children, .. } | Block::Filter { children, .. } => (children, &[]),
            Block::If { then, otherwise, .. } => (then, otherwise),
        };
        first.iter().chain(second).copied()
    }

    /// Every measure this block reads to decide what it holds. Every member
    /// of an any/all group counts, whichever of them decides the answer.
    pub fn for_each_measure(&self, visit: &mut dyn FnMut(usize)) {
        match self {
            Block::Weight { split: Split::InverseVolatility(measures), .. } | Block::Filter { scores: measures, .. } => measures.iter().copied().for_each(visit),
            Block::If { test, .. } => test.for_each_measure(visit),
            Block::Asset(_) | Block::Weight { .. } => {}
        }
    }
}

impl Test {
    fn for_each_measure(&self, visit: &mut dyn FnMut(usize)) {
        match self {
            Test::Compare { left, right, .. } => {
                visit(*left);
                if let Side::Measure(right) = right {
                    visit(*right);
                }
            }
            Test::Any(parts) | Test::All(parts) => parts.iter().for_each(|part| part.for_each_measure(visit)),
        }
    }
}

impl Test {
    /// How many comparisons the test makes.
    fn comparisons(&self) -> usize {
        match self {
            Test::Compare { .. } => 1,
            Test::Any(parts) | Test::All(parts) => parts.iter().map(Test::comparisons).sum(),
        }
    }
}

impl Plan {
    /// How much there is to do each time the strategy is evaluated, counted
    /// in closes gone over. A reading on a window goes over the window. One
    /// that carries its value from day to day keeps a series for every day
    /// there is, and so does each block that a reading is taken on. Each
    /// comparison and each distinct block is a step of its own.
    pub fn work(&self) -> usize {
        let read: usize = self
            .measures
            .iter()
            .map(|measure| match measure.indicator.carries() {
                true => SERIES_STEPS,
                false => MEASURE_STEPS + measure.indicator.window().map_or(1, |window| window as usize),
            })
            .sum();
        let tracked = self.measures_on_blocks().iter().filter(|measures| !measures.is_empty()).count();
        let compared: usize = self.blocks.iter().map(|block| if let Block::If { test, .. } = block { test.comparisons() } else { 0 }).sum();
        read + tracked * SERIES_STEPS + compared * MEASURE_STEPS + self.blocks.len()
    }

    /// Lay out a strategy that `validate` has passed.
    pub fn of(strategy: &RuleStrategy) -> Plan {
        let (mut held, mut read) = (BTreeSet::new(), BTreeSet::new());
        gather(&strategy.root, &mut held, &mut read);
        let assets: Vec<Asset> = held.union(&read).map(|asset| (*asset).clone()).collect();
        let places = assets.iter().enumerate().map(|(place, asset)| (asset, place)).collect();
        let mut layout = Layout { places, known: HashMap::new(), measured: HashMap::new(), blocks: Vec::new(), paths: Vec::new(), measures: Vec::new() };
        layout.block(&strategy.root, "root");
        let Layout { blocks, paths, measures, .. } = layout;
        let held = assets.iter().map(|asset| held.contains(asset)).collect();
        Plan { assets, held, blocks, paths, measures }
    }

    pub fn root(&self) -> usize {
        self.blocks.len() - 1
    }

    /// The measures taken on each block's own series, by block. Only a block
    /// with some needs a series.
    pub fn measures_on_blocks(&self) -> Vec<Vec<usize>> {
        let mut on = vec![Vec::new(); self.blocks.len()];
        for (place, measure) in self.measures.iter().enumerate() {
            if let Subject::Block(block) = measure.subject {
                on[block].push(place);
            }
        }
        on
    }
}

/// Every asset a tree may hold, and every asset one of its conditions reads.
fn gather<'a>(node: &'a Node, held: &mut BTreeSet<&'a Asset>, read: &mut BTreeSet<&'a Asset>) {
    match node {
        Node::Asset(asset) => {
            held.insert(asset);
        }
        Node::Weight { children, .. } | Node::Filter { children, .. } => children.iter().for_each(|child| gather(child, held, read)),
        Node::If { condition, then, otherwise } => {
            gather_read(condition, read);
            then.iter().chain(otherwise).for_each(|child| gather(child, held, read));
        }
    }
}

fn gather_read<'a>(condition: &'a Condition, read: &mut BTreeSet<&'a Asset>) {
    match condition {
        Condition::Compare(comparison) => {
            read.insert(&comparison.left.asset);
            if let Operand::Reading(right) = &comparison.right {
                read.insert(&right.asset);
            }
        }
        Condition::Any(parts) | Condition::All(parts) => parts.iter().for_each(|part| gather_read(part, read)),
    }
}

struct Layout<'a> {
    places: HashMap<&'a Asset, usize>,
    /// The place of each block met so far, by its canonical text with the
    /// blocks inside it written as their places.
    known: HashMap<String, usize>,
    measured: HashMap<Measure, usize>,
    blocks: Vec<Block>,
    paths: Vec<String>,
    measures: Vec<Measure>,
}

impl Layout<'_> {
    /// The place of this block, laid out now if it has not been met before.
    fn block(&mut self, node: &Node, path: &str) -> usize {
        let mut inner = Vec::new();
        let form = canonical::block(node, &mut |child| {
            let (field, index) = match node {
                Node::If { then, .. } if inner.len() >= then.len() => ("else", inner.len() - then.len()),
                Node::If { .. } => ("then", inner.len()),
                _ => ("children", inner.len()),
            };
            let place = self.block(child, &format!("{path}.{field}[{index}]"));
            inner.push(place);
            Value::from(place)
        });
        let key = canonical::text(&form);
        if let Some(place) = self.known.get(&key) {
            return *place;
        }
        let block = match node {
            Node::Asset(asset) => Block::Asset(self.places[asset]),
            Node::Weight { weighting, .. } => {
                let split = match weighting {
                    Weighting::Equal => Split::Equal,
                    Weighting::Specified { shares } => Split::Specified(shares.clone()),
                    Weighting::InverseVolatility { window } => Split::InverseVolatility(self.measures_on(Indicator::ReturnDeviation { window: *window }, &inner)),
                };
                Block::Weight { split, children: inner }
            }
            Node::If { condition, then, .. } => {
                let otherwise = inner.split_off(then.len());
                Block::If { test: self.test(condition), then: inner, otherwise }
            }
            Node::Filter { sort_by, take, count, .. } => Block::Filter { scores: self.measures_on(*sort_by, &inner), take: *take, count: *count as usize, children: inner },
        };
        self.blocks.push(block);
        self.paths.push(path.to_string());
        self.known.insert(key, self.blocks.len() - 1);
        self.blocks.len() - 1
    }

    /// One indicator on each of these blocks. An asset is measured on its
    /// prices and any other block on its own series.
    fn measures_on(&mut self, indicator: Indicator, blocks: &[usize]) -> Vec<usize> {
        blocks
            .iter()
            .map(|block| {
                let subject = match self.blocks[*block] {
                    Block::Asset(asset) => Subject::Asset(asset),
                    _ => Subject::Block(*block),
                };
                self.measure(Measure { indicator, subject })
            })
            .collect()
    }

    fn measure(&mut self, measure: Measure) -> usize {
        *self.measured.entry(measure).or_insert_with(|| {
            self.measures.push(measure);
            self.measures.len() - 1
        })
    }

    fn reading(&mut self, reading: &Reading) -> usize {
        let subject = Subject::Asset(self.places[&reading.asset]);
        self.measure(Measure { indicator: reading.indicator, subject })
    }

    fn test(&mut self, condition: &Condition) -> Test {
        match condition {
            Condition::Compare(comparison) => {
                let left = self.reading(&comparison.left);
                let right = match &comparison.right {
                    Operand::Fixed(value) => Side::Fixed(*value),
                    Operand::Reading(reading) => Side::Measure(self.reading(reading)),
                };
                Test::Compare { left, comparator: comparison.comparator, right }
            }
            Condition::Any(parts) => Test::Any(parts.iter().map(|part| self.test(part)).collect()),
            Condition::All(parts) => Test::All(parts.iter().map(|part| self.test(part)).collect()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Comparison, Frequency, Market, Rebalance};

    fn listed(symbol: &str) -> Asset {
        Asset::Listed { market: Market::Equities, symbol: symbol.to_string() }
    }

    fn asset(symbol: &str) -> Node {
        Node::Asset(listed(symbol))
    }

    fn equal(name: Option<&str>, children: Vec<Node>) -> Node {
        Node::Weight { name: name.map(str::to_string), weighting: Weighting::Equal, children }
    }

    fn above(indicator: Indicator, symbol: &str, fixed: f64) -> Condition {
        Condition::Compare(Comparison { left: Reading { indicator, asset: listed(symbol) }, comparator: Comparator::Above, right: Operand::Fixed(fixed) })
    }

    fn plan(root: Node) -> Plan {
        Plan::of(&RuleStrategy { name: "Test".to_string(), rebalance: Rebalance::Calendar(Frequency::Daily), root })
    }

    #[test]
    fn equal_sub_trees_become_one_block_whatever_they_are_called() {
        let strength = Indicator::RelativeStrength { window: 10 };
        let pair = |name: &str| equal(Some(name), vec![asset("TLT"), asset("GLD")]);
        let inner = Node::If { condition: above(strength, "QQQ", 0.7), then: vec![pair("One")], otherwise: vec![asset("SPY")] };
        let root = Node::If { condition: above(strength, "SPY", 0.8), then: vec![pair("Another")], otherwise: vec![inner, pair("A third"), asset("SPY")] };
        let plan = plan(root);
        // GLD, QQQ, SPY and TLT. QQQ is read and never held.
        assert_eq!(plan.assets, ["GLD", "QQQ", "SPY", "TLT"].map(listed));
        assert_eq!(plan.held, [true, false, true, true]);
        // TLT, GLD, the pair, SPY, the inner if and the root: six blocks for ten in the tree.
        assert_eq!(plan.blocks.len(), 6);
        assert_eq!(plan.paths, ["root.then[0].children[0]", "root.then[0].children[1]", "root.then[0]", "root.else[0].else[0]", "root.else[0]", "root"]);
        assert!(matches!(&plan.blocks[plan.root()], Block::If { then, otherwise, .. } if then == &[2] && otherwise == &[4, 2, 3]));
        assert_eq!(plan.measures.len(), 2);
    }

    #[test]
    fn a_reading_asked_for_many_times_is_one_measure() {
        let return_20 = Indicator::CumulativeReturn { window: 20 };
        let group = || equal(None, vec![asset("SPY"), asset("TLT")]);
        let filter = |count| Node::Filter { sort_by: return_20, take: Take::Top, count, children: vec![asset("QQQ"), group(), asset("GLD")] };
        let weighed = Node::Weight { name: None, weighting: Weighting::InverseVolatility { window: 20 }, children: vec![group(), asset("QQQ")] };
        let root = Node::If { condition: above(return_20, "QQQ", 0.0), then: vec![filter(1), filter(2)], otherwise: vec![weighed] };
        let plan = plan(root);
        // The return of QQQ, of the group and of GLD, then the deviation of the group and of QQQ.
        assert_eq!(plan.measures.len(), 5);
        let group_at = plan.paths.iter().position(|path| path == "root.then[0].children[1]").unwrap();
        assert_eq!(plan.measures[1], Measure { indicator: return_20, subject: Subject::Block(group_at) });
        assert_eq!(plan.measures[3], Measure { indicator: Indicator::ReturnDeviation { window: 20 }, subject: Subject::Block(group_at) });
        let on_blocks = plan.measures_on_blocks();
        assert_eq!(on_blocks[group_at], [1, 3]);
        assert_eq!(on_blocks.iter().filter(|measures| !measures.is_empty()).count(), 1);
        // The two filters differ only in how many they keep, and share their scores.
        let scores: Vec<&Vec<usize>> = plan.blocks.iter().filter_map(|block| if let Block::Filter { scores, .. } = block { Some(scores) } else { None }).collect();
        assert_eq!(scores, [&vec![0, 1, 2], &vec![0, 1, 2]]);
    }
}
