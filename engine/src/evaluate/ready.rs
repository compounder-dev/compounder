//! When each part of a plan first has a value.
//!
//! An asset has a price from its first bar. A reading has a value once its
//! window is full: `first_index` closes into its subject's series. A block
//! can be evaluated once everything in it can, the branches it never takes
//! included, so no day ever sees half a tree. A block's own series starts on
//! that day, and a reading on the block fills its window from there. Warm-up
//! adds up through nesting for that reason and no other.
//!
//! Days here are calendar positions. Nothing is walked: each answer is the
//! latest of a few sums.

use super::plan::{Block, Plan, Subject};
use crate::rules::Indicator;

/// A day before any calendar starts: where the latest of no days stands.
const LONG_AGO: i64 = i64::MIN / 2;

/// The first day each block and each measure has a value.
pub(crate) struct Ready {
    pub blocks: Vec<i64>,
    pub measures: Vec<i64>,
}

impl Ready {
    /// `start` is the first day each asset has a price. `skip` is how many
    /// of an asset's first closes its carried readings are not fed, which is
    /// none unless the run-in convention is on.
    pub fn of(plan: &Plan, start: &[i64], skip: &[usize]) -> Ready {
        let mut ready = Ready { blocks: Vec::with_capacity(plan.blocks.len()), measures: vec![LONG_AGO; plan.measures.len()] };
        // A block's readings rest on assets and on earlier blocks, so one pass in order settles everything.
        for block in &plan.blocks {
            let mut latest = match block {
                Block::Asset(asset) => start[*asset],
                _ => LONG_AGO,
            };
            block.for_each_measure(&mut |place| {
                let measure = &plan.measures[place];
                let series = match measure.subject {
                    Subject::Asset(asset) => start[asset] + late(measure.indicator, skip[asset]),
                    Subject::Block(block) => ready.blocks[block],
                };
                ready.measures[place] = series + measure.indicator.first_index() as i64;
                latest = latest.max(ready.measures[place]);
            });
            for child in block.inside() {
                latest = latest.max(ready.blocks[child]);
            }
            ready.blocks.push(latest);
        }
        ready
    }
}

/// Only a carried reading is started late under the run-in convention.
fn late(indicator: Indicator, skip: usize) -> i64 {
    if indicator.carries() {
        skip as i64
    } else {
        0
    }
}

/// For each asset, the first day it alone would let the whole tree be
/// evaluated: its first bar, plus the longest run of windows that rests on
/// it. The latest of these is the day the tree is ready. Each is what a
/// warning about a late start is dated with.
pub(crate) fn waits(plan: &Plan, start: &[i64], skip: &[usize]) -> Vec<i64> {
    // How many closes of warm-up lie between each block and the root, going
    // down the longest way. The root comes last, so parents are met first.
    let mut above = vec![LONG_AGO; plan.blocks.len()];
    above[plan.root()] = 0;
    let mut longest = vec![LONG_AGO; plan.assets.len()];
    for (place, block) in plan.blocks.iter().enumerate().rev() {
        let here = above[place];
        if let Block::Asset(asset) = block {
            longest[*asset] = longest[*asset].max(here);
        }
        block.for_each_measure(&mut |measure| {
            let measure = &plan.measures[measure];
            let through = here + measure.indicator.first_index() as i64;
            match measure.subject {
                Subject::Asset(asset) => longest[asset] = longest[asset].max(through + late(measure.indicator, skip[asset])),
                Subject::Block(block) => above[block] = above[block].max(through),
            }
        });
        for child in block.inside() {
            above[child] = above[child].max(here);
        }
    }
    longest.iter().zip(start).map(|(longest, start)| start + longest).collect()
}

/// How many of each asset's first closes its carried readings are not fed
/// under the run-in convention.
///
/// The reference conventions do not give a carried reading its subject's
/// whole history. They start it a fixed number of closes before `anchor`,
/// the first day the whole strategy can be evaluated. That number is the
/// longest any reading on the asset asks for. A reading on a window asks for
/// its window. A carried one adds the closes its seed takes to fade below a
/// set size, a thousand at most.
pub(crate) fn run_in(plan: &Plan, start: &[i64], anchor: i64) -> Vec<usize> {
    let mut reach = vec![0_i64; plan.assets.len()];
    for measure in &plan.measures {
        if let Subject::Asset(asset) = measure.subject {
            reach[asset] = reach[asset].max((measure.indicator.first_index() + fade(measure.indicator)) as i64);
        }
    }
    reach.iter().zip(start).map(|(reach, start)| (anchor - start - reach).max(0) as usize).collect()
}

/// The closes a carried reading is run before it counts, by the reference
/// conventions: until its seed weighs less than 1e-8 for relative strength
/// and 1e-6 for the exponential average.
fn fade(indicator: Indicator) -> usize {
    const MOST: f64 = 1000.0;
    let (kept_each_close, small) = match indicator {
        Indicator::RelativeStrength { window } => (1.0 - 1.0 / f64::from(window), 1e-8_f64),
        Indicator::ExponentialAveragePrice { window } => (1.0 - 2.0 / (f64::from(window) + 1.0), 1e-6_f64),
        _ => return 0,
    };
    // A one day average keeps nothing of its seed.
    if kept_each_close <= 0.0 {
        return 0;
    }
    (small.ln() / kept_each_close.ln()).ceil().min(MOST) as usize
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Asset, Comparator, Comparison, Condition, Frequency, Market, Node, Operand, Reading, Rebalance, RuleStrategy, Take, Weighting};

    fn listed(symbol: &str) -> Asset {
        Asset::Listed { market: Market::Equities, symbol: symbol.to_string() }
    }

    #[test]
    fn each_asset_holds_the_tree_back_by_its_start_and_the_longest_run_of_windows_on_it() {
        let asset = |symbol: &str| Node::Asset(listed(symbol));
        let reading = |indicator: Indicator, symbol: &str| Reading { indicator, asset: listed(symbol) };
        // A block that reads a 14 day strength of YOUNG, a pair of MID and OLD, and OLD, ranked by a 20 day return.
        let strong = Condition::Compare(Comparison { left: reading(Indicator::RelativeStrength { window: 14 }, "YOUNG"), comparator: Comparator::Above, right: Operand::Fixed(0.5) });
        let switching = Node::If { condition: strong, then: vec![asset("YOUNG")], otherwise: vec![asset("OLD")] };
        let pair = Node::Weight { name: None, weighting: Weighting::Equal, children: vec![asset("MID"), asset("OLD")] };
        let ranked = Node::Filter { sort_by: Indicator::CumulativeReturn { window: 20 }, take: Take::Top, count: 1, children: vec![switching, pair, asset("OLD")] };
        // All of it under a condition on MID's 50 day average.
        let trend = Condition::Compare(Comparison { left: reading(Indicator::Price, "MID"), comparator: Comparator::Above, right: Operand::Reading(reading(Indicator::AveragePrice { window: 50 }, "MID")) });
        let root = Node::If { condition: trend, then: vec![ranked], otherwise: vec![asset("OLD")] };
        let plan = Plan::of(&RuleStrategy { name: "Test".to_string(), rebalance: Rebalance::Calendar(Frequency::Daily), root });
        assert_eq!(plan.assets, ["MID", "OLD", "YOUNG"].map(listed));

        // MID's first bar is day 100, OLD's day 0 and YOUNG's day 300.
        let start = [100, 0, 300];
        // MID: its 50 day average, which is later than a 20 day return on the pair.
        // OLD: a 20 day return, on itself or on a block that holds it.
        // YOUNG: 14 closes for the strength, then 20 of the block that reads it.
        assert_eq!(waits(&plan, &start, &[0, 0, 0]), [100 + 49, 20, 300 + 14 + 20]);
        let ready = Ready::of(&plan, &start, &[0, 0, 0]);
        assert_eq!(ready.blocks[plan.root()], 334);
        // The block that reads YOUNG starts its own series on day 314, and the pair on day 100.
        let block = |path: &str| ready.blocks[plan.paths.iter().position(|at| at == path).unwrap()];
        assert_eq!((block("root.then[0].children[0]"), block("root.then[0].children[1]"), block("root.then[0]")), (314, 100, 334));

        // A carried reading that is started late is ready that much later. Readings on a window are not moved.
        let skip = [7, 7, 5];
        assert_eq!(waits(&plan, &start, &skip), [149, 20, 339]);
        assert_eq!(Ready::of(&plan, &start, &skip).blocks[plan.root()], 339);
        // The strength on YOUNG is the longest thing asked of it: 14 closes and 249 for its seed. MID is asked for 49, OLD for 20.
        assert_eq!(run_in(&plan, &start, 1_000), [1_000 - 100 - 49, 1_000 - 20, 1_000 - 300 - 263]);
        assert_eq!(run_in(&plan, &start, 334), [334 - 100 - 49, 334 - 20, 0]);
    }

    #[test]
    fn a_seed_is_given_time_to_fade_and_a_thousand_closes_at_most() {
        // Closes before the anchor, the window included.
        let reach = |indicator: Indicator| indicator.first_index() + fade(indicator);
        assert_eq!(reach(Indicator::RelativeStrength { window: 14 }), 263);
        assert_eq!(reach(Indicator::RelativeStrength { window: 50 }), 962);
        assert_eq!(reach(Indicator::RelativeStrength { window: 100 }), 1_100);
        assert_eq!(reach(Indicator::RelativeStrength { window: 252 }), 1_252);
        assert_eq!(reach(Indicator::ExponentialAveragePrice { window: 30 }), 237);
        assert_eq!(reach(Indicator::ExponentialAveragePrice { window: 100 }), 790);
        assert_eq!(reach(Indicator::ExponentialAveragePrice { window: 252 }), 1_251);
        assert_eq!(reach(Indicator::ExponentialAveragePrice { window: 1 }), 0);
        // A reading on a window asks for its window and no more.
        assert_eq!(reach(Indicator::AveragePrice { window: 200 }), 199);
        assert_eq!(reach(Indicator::CumulativeReturn { window: 20 }), 20);
        assert_eq!(reach(Indicator::Price), 0);
    }
}
