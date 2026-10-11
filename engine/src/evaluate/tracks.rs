//! A block's own daily series, and the readings that carry a value from day
//! to day.
//!
//! A filter or an inverse volatility weight may hold blocks as well as
//! assets, and then reads an indicator on a block as if it were one asset.
//! The block's series is what makes that possible:
//!
//! ```text
//! level[d0] = 1                      d0 is the first day the block can be evaluated
//! ret(d)    = sum over assets of  w(d-1)[i] * (p_i[d] / p_i[d-1] - 1)
//! level[d]  = level[d-1] * (1 + ret(d))
//! ```
//!
//! Each day the block earns the return of the weights it chose at the close
//! before. It is evaluated afresh every trading day, whatever the strategy's
//! own rebalance rule says, and nothing is charged inside it. Only ratios of
//! the level matter. The series depends on the block and its prices alone,
//! so it runs from long before any backtest starts.

use super::plan::{Plan, Subject};
use super::ready::Ready;
use super::weigh::{Ground, Memo, Weights};
use crate::indicator::Running;
use compounder_prices::calendar::Aligned;

/// A value for each trading day of a stretch. `start` is the calendar
/// position of the first.
pub(crate) struct Daily {
    pub start: usize,
    pub values: Vec<f64>,
}

pub(crate) struct Tracks {
    /// The own series of each block some measure reads, by block.
    pub levels: Vec<Option<Daily>>,
    /// The values of each carried measure from its first one on, by measure.
    pub carried: Vec<Option<Daily>>,
}

impl Tracks {
    /// Every series up to `last`, the last day every asset has a price.
    /// `skip` is how many of each asset's first closes a carried reading on
    /// it is not fed.
    pub fn build(plan: &Plan, prices: &[&Aligned], ready: &Ready, skip: &[usize], last: usize) -> Tracks {
        let mut tracks = Tracks { levels: plan.blocks.iter().map(|_| None).collect(), carried: plan.measures.iter().map(|_| None).collect() };
        for (place, measure) in plan.measures.iter().enumerate() {
            if let (true, Subject::Asset(asset)) = (measure.indicator.carries(), measure.subject) {
                let from = prices[asset].range().start + skip[asset];
                let mut run = Daily { start: from + measure.indicator.first_index(), values: Vec::new() };
                if let (Some(mut running), Some(closes)) = (Running::new(measure.indicator), prices[asset].prices().get(skip[asset]..)) {
                    run.values.extend(closes.iter().filter_map(|close| running.push(*close)));
                }
                tracks.carried[place] = Some(run);
            }
        }

        // The blocks that need a series, each with the day it starts and the
        // measures taken on it. A block that is not ready by `last` has none.
        let measured = plan.measures_on_blocks();
        let mut walks = Vec::new();
        for (block, measures) in measured.iter().enumerate().filter(|(_, measures)| !measures.is_empty()) {
            let Some(start) = usize::try_from(ready.blocks[block]).ok().filter(|start| *start <= last) else { continue };
            tracks.levels[block] = Some(Daily { start, values: Vec::new() });
            let mut carried = Vec::new();
            for place in measures {
                if let Some(running) = Running::new(plan.measures[*place].indicator) {
                    tracks.carried[*place] = Some(Daily { start, values: Vec::new() });
                    carried.push((*place, running));
                }
            }
            walks.push(Walk { block, start, held: None, carried });
        }

        let mut memo = Memo::new(plan);
        let first = walks.iter().map(|walk| walk.start).min().unwrap_or(last + 1);
        for day in first..=last {
            // In order, so a block inside another has its level for the day
            // before the outer one is weighed on it.
            for walk in walks.iter_mut().filter(|walk| walk.start <= day) {
                let level = match &walk.held {
                    // Its first day.
                    None if day == walk.start => Some(1.0),
                    // It could not be weighed at some close, and has no series from then on.
                    None => None,
                    Some(held) => earned(held, prices, day).zip(tracks.level(walk.block, day - 1)).map(|(earned, before)| before * (1.0 + earned)),
                };
                let (Some(level), Some(levels)) = (level, tracks.levels[walk.block].as_mut()) else { continue };
                levels.values.push(level);
                for (place, running) in &mut walk.carried {
                    if let (Some(value), Some(run)) = (running.push(level), tracks.carried[*place].as_mut()) {
                        if run.values.is_empty() {
                            run.start = day;
                        }
                        run.values.push(value);
                    }
                }
                walk.held = memo.weigh(&Ground { plan, prices, tracks: &tracks }, walk.block, day);
            }
        }
        tracks
    }

    fn level(&self, block: usize, day: usize) -> Option<f64> {
        let run = self.levels[block].as_ref()?;
        run.values.get(day.checked_sub(run.start)?).copied()
    }
}

/// A block being walked forward.
struct Walk {
    block: usize,
    start: usize,
    /// What it chose at the last close.
    held: Option<Weights>,
    /// Each carried measure on the block, with its state.
    carried: Vec<(usize, Running)>,
}

/// What these weights earned from the close before `day` to the close of `day`.
fn earned(held: &Weights, prices: &[&Aligned], day: usize) -> Option<f64> {
    let mut sum = 0.0;
    for (asset, weight) in held.iter() {
        sum += weight * (prices[*asset].at(day)? / prices[*asset].at(day.checked_sub(1)?)? - 1.0);
    }
    Some(sum)
}
