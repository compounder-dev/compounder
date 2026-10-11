//! A made-up strategy the size of the largest real ones: about nine
//! thousand nodes, built by copying a few dozen branches under many
//! conditions, the way such strategies grow. It reads about two hundred
//! distinct readings and holds sixty distinct groups.
//!
//! The test checks that it runs and says how long it took. For the time
//! that matters:
//!
//! `cargo test --release -p compounder-engine --test giant -- --nocapture`

mod support;

use compounder_engine::backtest::{backtest, Run};
use compounder_engine::evaluate::Evaluator;
use compounder_engine::rules::{Comparator, Condition, Indicator, Node, Operand, Take, Weighting, MAX_NODES};
use std::collections::BTreeSet;
use std::time::Instant;
use support::*;

const ASSETS: usize = 120;
/// Ten years of trading days, and two more before them to fill the longest windows.
const TEN_YEARS: usize = 2_520;
const WARM_UP: usize = 504;

/// A small generator, so the same giant is built every time.
struct Dice(u64);

impl Dice {
    fn roll(&mut self, sides: usize) -> usize {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % sides as u64) as usize
    }
}

fn symbol(index: usize) -> String {
    [index / 676, index / 26 % 26, index % 26].iter().map(|letter| (b'A' + *letter as u8) as char).collect()
}

/// A made-up price path: steps of whole basis points, up a little more often than down.
fn path(dice: &mut Dice, count: usize) -> Vec<f64> {
    let mut price = 20.0 + dice.roll(180) as f64;
    (0..count)
        .map(|_| {
            let close = (price * 1e6_f64).round() / 1e6;
            price = (price * (1.0 + (dice.roll(301) as f64 - 148.0) / 10_000.0)).max(1.0);
            close
        })
        .collect()
}

/// How many distinct conditions the giant draws on.
const CONDITIONS: usize = 90;

/// One condition, on its own asset and window.
fn condition(index: usize) -> Condition {
    let asset = symbol(index % ASSETS);
    let window = [5, 10, 14, 20, 21, 30, 50, 60, 100, 200][index / 6 % 10];
    match index % 6 {
        0 => against(Indicator::RelativeStrength { window }, &asset, Comparator::Above, 0.45 + (index % 5) as f64 * 0.03),
        1 => against(Indicator::CumulativeReturn { window }, &asset, Comparator::Below, 0.0),
        2 => between(reading(Indicator::Price, &asset), Comparator::Above, reading(Indicator::AveragePrice { window }, &asset)),
        3 => against(Indicator::MaxDrawdown { window: window.max(10) }, &asset, Comparator::Above, 0.08),
        4 => between(reading(Indicator::ExponentialAveragePrice { window }, &asset), Comparator::AtLeast, reading(Indicator::AveragePrice { window: window * 2 }, &asset)),
        _ => against(Indicator::ReturnDeviation { window: window.max(10) }, &asset, Comparator::Below, 0.012),
    }
}

/// One of sixty groups of five to seven assets, weighted four ways. The two
/// ways that read their assets draw them from the first forty.
fn group(index: usize, dice: &mut Dice) -> Node {
    let mut assets = |count: usize, among: usize| (0..count).map(|_| asset(&symbol(dice.roll(among)))).collect::<Vec<Node>>();
    let name = Some(format!("Group {index}"));
    match index % 4 {
        0 => Node::Weight { name, weighting: Weighting::Equal, children: assets(6, ASSETS) },
        1 => Node::Weight { name, weighting: Weighting::Specified { shares: vec![0.3, 0.25, 0.2, 0.15, 0.1] }, children: assets(5, ASSETS) },
        2 => Node::Weight { name, weighting: Weighting::InverseVolatility { window: 20 }, children: assets(5, 40) },
        _ => filter(Indicator::CumulativeReturn { window: 20 }, Take::Top, 2, assets(7, 40)),
    }
}

/// Ifs nested `depth` deep, with a block from `leaves` at each end.
fn nest(depth: usize, leaves: &[Node], dice: &mut Dice) -> Node {
    if depth == 0 {
        return leaves[dice.roll(leaves.len())].clone();
    }
    // One condition in four is a group of three.
    let test = match dice.roll(8) {
        0 => Condition::Any((0..3).map(|_| condition(dice.roll(CONDITIONS))).collect()),
        1 => Condition::All((0..3).map(|_| condition(dice.roll(CONDITIONS))).collect()),
        _ => condition(dice.roll(CONDITIONS)),
    };
    when(test, vec![nest(depth - 1, leaves, dice)], vec![nest(depth - 1, leaves, dice)])
}

fn giant() -> Node {
    let mut dice = Dice(2_030);
    let groups: Vec<Node> = (0..60).map(|index| group(index, &mut dice)).collect();
    // Forty branches of fifteen ifs each, then copied under sixty-three more.
    let branches: Vec<Node> = (0..40).map(|_| nest(4, &groups, &mut dice)).collect();
    let tree = nest(6, &branches, &mut dice);
    // Beside it, groups ranked and weighed as if each were one asset.
    let ranked = filter(Indicator::CumulativeReturn { window: 20 }, Take::Top, 2, groups[..6].to_vec());
    let weighed = inverse_volatility(20, groups[6..9].to_vec());
    specified(&[0.8, 0.1, 0.1], vec![tree, ranked, weighed])
}

/// Every distinct reading a tree takes on an asset.
fn readings(node: &Node, found: &mut BTreeSet<String>) {
    fn of(condition: &Condition, found: &mut BTreeSet<String>) {
        match condition {
            Condition::Compare(comparison) => {
                found.insert(format!("{:?}", comparison.left));
                if let Operand::Reading(right) = &comparison.right {
                    found.insert(format!("{right:?}"));
                }
            }
            Condition::Any(parts) | Condition::All(parts) => parts.iter().for_each(|part| of(part, found)),
        }
    }
    let on_assets = |indicator: Indicator, children: &[Node], found: &mut BTreeSet<String>| {
        for child in children {
            if let Node::Asset(asset) = child {
                found.insert(format!("{indicator:?} {asset:?}"));
            }
        }
    };
    match node {
        Node::Asset(_) => {}
        Node::Weight { weighting, children, .. } => {
            if let Weighting::InverseVolatility { window } = weighting {
                on_assets(Indicator::ReturnDeviation { window: *window }, children, found);
            }
            children.iter().for_each(|child| readings(child, found));
        }
        Node::If { condition, then, otherwise } => {
            of(condition, found);
            then.iter().chain(otherwise).for_each(|child| readings(child, found));
        }
        Node::Filter { sort_by, children, .. } => {
            on_assets(*sort_by, children, found);
            children.iter().for_each(|child| readings(child, found));
        }
    }
}

#[test]
fn a_strategy_of_nine_thousand_nodes_runs_ten_years_of_daily_evaluation() {
    let strategy = daily(giant());
    let nodes = strategy.node_count();
    assert!((8_500..=9_500).contains(&nodes), "{nodes} nodes");
    assert!(strategy.validate(MAX_NODES).is_empty());
    let mut conditions = BTreeSet::new();
    readings(&strategy.root, &mut conditions);
    assert!((180..=220).contains(&conditions.len()), "{} distinct readings", conditions.len());

    let days = weekdays("2014-01-01", WARM_UP + TEN_YEARS);
    let mut dice = Dice(7);
    let paths: Vec<(String, Vec<f64>)> = (0..ASSETS).map(|index| (symbol(index), path(&mut dice, days.len()))).collect();
    let assets: Vec<(&str, usize, &[f64])> = paths.iter().map(|(symbol, closes)| (symbol.as_str(), 0, &closes[..])).collect();
    let history = history(&days, &assets);

    let run = Run { start: Some(days[WARM_UP]), end: None, capital: 10_000.0, slippage: 0.0015, min_order: 1.0, run_in: false };
    let started = Instant::now();
    let evaluator = Evaluator::new(&strategy, &history, false).expect("a sound strategy");
    let prepared = started.elapsed();
    let evaluated = Instant::now();
    for day in &days[WARM_UP..] {
        let evaluation = evaluator.on(*day).expect("a day it can be evaluated on");
        assert!(close(evaluation.weights.iter().sum::<f64>(), 1.0, 1e-12));
    }
    let evaluated = evaluated.elapsed();
    let whole = Instant::now();
    let result = backtest(&strategy, &history, &run).unwrap_or_else(|issues| panic!("{issues:#?}"));
    let whole = whole.elapsed();

    assert_eq!((result.first_day, result.days.len()), (days[WARM_UP], TEN_YEARS));
    assert!(result.days.iter().all(|day| day.traded && day.value.is_finite() && day.value > 0.0));
    assert!(result.warnings.is_empty());
    // It moves between groups: a strategy stuck in one branch would prove little.
    let held: BTreeSet<Vec<bool>> = result.days.iter().map(|day| day.weights.iter().map(|weight| *weight > 1e-9).collect()).collect();
    assert!(held.len() > 200, "it only ever held {} different sets of assets", held.len());
    println!(
        "A strategy of {nodes} nodes, {} distinct readings on assets and 60 groups, over {} assets and {} trading days: \
         prepared in {prepared:.1?}, ten years of daily evaluation in {evaluated:.1?}, the whole backtest in {whole:.1?}.",
        conditions.len(),
        result.assets.len(),
        days.len(),
    );
}
