//! What a block holds at one close.
//!
//! A block's weights are its share of each asset, summing to 1. An asset is
//! all itself. A weight block splits its share among its children. An if
//! holds one branch or the other. A filter ranks its children and keeps the
//! first few. Shares multiply down the tree, and an asset reached twice adds
//! up.
//!
//! Everything is a pure function of the closes up to the day. Within one day
//! each distinct block is weighed once and each distinct reading taken once,
//! however often the tree asks.

use super::compare;
use super::plan::{Block, Plan, Side, Split, Subject, Test};
use super::tracks::Tracks;
use crate::indicator;
use crate::rules::Take;
use compounder_prices::calendar::Aligned;
use std::cmp::Ordering;
use std::rc::Rc;

/// A block's share of each asset it holds, in asset order, each asset once.
pub(crate) type Weights = Rc<[(usize, f64)]>;

/// What an evaluation reads: the plan, each asset's prices and each block's
/// own series.
pub(crate) struct Ground<'a> {
    pub plan: &'a Plan,
    pub prices: &'a [&'a Aligned],
    pub tracks: &'a Tracks,
}

impl Ground<'_> {
    /// The value of a measure at the close of `day`, when it has one.
    fn value(&self, place: usize, day: usize) -> Option<f64> {
        let measure = &self.plan.measures[place];
        if measure.indicator.carries() {
            let carried = self.tracks.carried[place].as_ref()?;
            return carried.values.get(day.checked_sub(carried.start)?).copied();
        }
        let (start, series) = match measure.subject {
            Subject::Asset(asset) => (self.prices[asset].range().start, self.prices[asset].prices()),
            Subject::Block(block) => {
                let level = self.tracks.levels[block].as_ref()?;
                (level.start, &level.values[..])
            }
        };
        indicator::at(measure.indicator, series.get(..=day.checked_sub(start)?)?)
    }
}

/// One day's work, kept so nothing is done twice.
pub(crate) struct Memo {
    day: usize,
    weights: Vec<Option<Weights>>,
    values: Vec<Option<f64>>,
    /// The measures read so far this day, in the order they were first read.
    pub read: Vec<usize>,
    /// Room to add shares up in, by asset. All zero and all false between uses.
    sums: Vec<f64>,
    touched: Vec<bool>,
}

impl Memo {
    pub fn new(plan: &Plan) -> Memo {
        Memo {
            day: usize::MAX,
            weights: vec![None; plan.blocks.len()],
            values: vec![None; plan.measures.len()],
            read: Vec::new(),
            sums: vec![0.0; plan.assets.len()],
            touched: vec![false; plan.assets.len()],
        }
    }

    /// What the block holds at the close of `day`. Nothing when a reading it
    /// needs has no value yet.
    pub fn weigh(&mut self, ground: &Ground, block: usize, day: usize) -> Option<Weights> {
        if self.day != day {
            self.day = day;
            self.weights.fill(None);
            self.values.fill(None);
            self.read.clear();
        }
        self.block(ground, block)
    }

    /// The value of a measure this day, as it was read.
    pub fn value(&self, place: usize) -> Option<f64> {
        self.values[place]
    }

    fn block(&mut self, ground: &Ground, place: usize) -> Option<Weights> {
        if let Some(known) = &self.weights[place] {
            return Some(known.clone());
        }
        let weights = match &ground.plan.blocks[place] {
            Block::Asset(asset) => Rc::from([(*asset, 1.0)]),
            Block::Weight { split, children } => {
                let shares = match split {
                    Split::Equal => vec![1.0 / children.len() as f64; children.len()],
                    Split::Specified(shares) => {
                        // Shares are divided by their sum, so 60 and 30 hold two thirds and one third.
                        let total: f64 = shares.iter().sum();
                        shares.iter().map(|share| share / total).collect()
                    }
                    Split::InverseVolatility(measures) => inverse_volatility(&self.measures(ground, measures)?),
                };
                self.mix(ground, children.iter().copied().zip(shares))?
            }
            Block::If { test, then, otherwise } => {
                let branch = if self.holds(ground, test)? { then } else { otherwise };
                // Several blocks in one branch share it equally.
                self.mix(ground, branch.iter().map(|child| (*child, 1.0 / branch.len() as f64)))?
            }
            Block::Filter { scores, take, count, children } => {
                let scores = self.measures(ground, scores)?;
                let mut order: Vec<usize> = (0..children.len()).collect();
                // By the raw value, largest first for top and smallest first
                // for bottom. The sort is stable, so ties keep list order.
                let rising = |first: &usize, second: &usize| scores[*first].partial_cmp(&scores[*second]).unwrap_or(Ordering::Equal);
                order.sort_by(|first, second| match take {
                    Take::Top => rising(second, first),
                    Take::Bottom => rising(first, second),
                });
                // Asked to keep more than it holds, it keeps them all.
                order.truncate(*count);
                self.mix(ground, order.iter().map(|kept| (children[*kept], 1.0 / order.len() as f64)))?
            }
        };
        self.weights[place] = Some(weights.clone());
        Some(weights)
    }

    /// The holdings of several blocks, each scaled by its share and added up.
    /// A block with no share is not looked into.
    fn mix(&mut self, ground: &Ground, parts: impl Iterator<Item = (usize, f64)>) -> Option<Weights> {
        let mut inner = Vec::new();
        for (block, share) in parts {
            if share != 0.0 {
                inner.push((self.block(ground, block)?, share));
            }
        }
        let mut assets = Vec::new();
        for (weights, share) in &inner {
            for (asset, weight) in weights.iter() {
                if !std::mem::replace(&mut self.touched[*asset], true) {
                    assets.push(*asset);
                }
                self.sums[*asset] += share * weight;
            }
        }
        assets.sort_unstable();
        Some(assets.iter().map(|asset| (*asset, std::mem::take(&mut self.sums[*asset]))).inspect(|(asset, _)| self.touched[*asset] = false).collect())
    }

    fn measures(&mut self, ground: &Ground, places: &[usize]) -> Option<Vec<f64>> {
        places.iter().map(|place| self.measure(ground, *place)).collect()
    }

    fn measure(&mut self, ground: &Ground, place: usize) -> Option<f64> {
        if self.values[place].is_none() {
            self.values[place] = Some(ground.value(place, self.day)?);
            self.read.push(place);
        }
        self.values[place]
    }

    /// Every member of a group is read, even once the answer is settled, so
    /// a day's readings do not depend on which member came first.
    fn holds(&mut self, ground: &Ground, test: &Test) -> Option<bool> {
        Some(match test {
            Test::Compare { left, comparator, right } => {
                let left = self.measure(ground, *left)?;
                let right = match right {
                    Side::Fixed(value) => *value,
                    Side::Measure(right) => self.measure(ground, *right)?,
                };
                compare::holds(*comparator, left, right)
            }
            Test::Any(parts) => parts.iter().map(|part| self.holds(ground, part)).collect::<Option<Vec<bool>>>()?.contains(&true),
            Test::All(parts) => !parts.iter().map(|part| self.holds(ground, part)).collect::<Option<Vec<bool>>>()?.contains(&false),
        })
    }
}

/// Each child's share from the deviation of its daily returns: in proportion
/// to one over it.
///
/// A child that has not moved has a deviation of exactly 0, and one over
/// that is no number. It is the steadiest thing in the block, so it takes
/// the whole block. Several such children share it equally. The rest get
/// nothing.
fn inverse_volatility(deviations: &[f64]) -> Vec<f64> {
    // One over a deviation too small to divide by is as good as none at all.
    let is_still = |deviation: f64| !(1.0 / deviation).is_finite();
    let still = deviations.iter().filter(|deviation| is_still(**deviation)).count();
    if still > 0 {
        return deviations.iter().map(|deviation| if is_still(*deviation) { 1.0 / still as f64 } else { 0.0 }).collect();
    }
    let total: f64 = deviations.iter().map(|deviation| 1.0 / deviation).sum();
    deviations.iter().map(|deviation| 1.0 / deviation / total).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close_to(got: &[f64], expected: &[f64]) -> bool {
        got.len() == expected.len() && got.iter().zip(expected).all(|(got, expected)| (got - expected).abs() <= 1e-15)
    }

    #[test]
    fn a_deviation_too_small_to_divide_by_counts_as_none() {
        // One over 1e-320 is past the largest number there is.
        assert_eq!(inverse_volatility(&[1e-320, 0.02]), [1.0, 0.0]);
        assert_eq!(inverse_volatility(&[1e-320, 0.0, 0.02]), [0.5, 0.5, 0.0]);
        assert!(inverse_volatility(&[1e-300, 0.02]).iter().all(|weight| weight.is_finite()));
    }

    #[test]
    fn the_steadier_child_gets_more() {
        // One over 0.01, 0.02 and 0.04 is 100, 50 and 25.
        assert!(close_to(&inverse_volatility(&[0.01, 0.02, 0.04]), &[4.0 / 7.0, 2.0 / 7.0, 1.0 / 7.0]));
        assert!(close_to(&inverse_volatility(&[0.03, 0.03]), &[0.5, 0.5]));
        assert_eq!(inverse_volatility(&[0.03]), [1.0]);
    }

    #[test]
    fn a_child_that_has_not_moved_takes_the_whole_block() {
        assert_eq!(inverse_volatility(&[0.01, 0.0, 0.04]), [0.0, 1.0, 0.0]);
        assert_eq!(inverse_volatility(&[0.0, 0.02, 0.0]), [0.5, 0.0, 0.5]);
        assert_eq!(inverse_volatility(&[0.0, 0.0, 0.0]), [1.0 / 3.0; 3]);
        // A residue is movement, however small.
        assert!(close_to(&inverse_volatility(&[1e-11, 1e-11]), &[0.5, 0.5]));
    }
}
