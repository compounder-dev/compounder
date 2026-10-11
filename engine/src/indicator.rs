//! The nine indicators, worked out over one daily series of closes.
//!
//! A series is oldest first, with one close for every trading day of its
//! subject's life. A value at a close reads that close and the ones before
//! it, never a later one. A window counts closes. A window of prices uses
//! `window` of them. A window of returns uses `window` returns, so one close
//! more.
//!
//! Returns are simple, never log. Deviations divide by one less than the
//! count and are not annualised. Percent indicators are fractions: 0.05 is
//! 5%, and relative strength runs from 0 to 1.
//!
//! Every window is summed afresh, oldest to newest, in plain `f64`, with no
//! fused multiply-add. The same closes then give the same bits on every
//! machine, the browser build included. A running sum would be quicker and
//! would leave the noise of every close it ever held in today's value.

use crate::rules::Indicator;

impl Indicator {
    /// The index of the first close at which it has a value. A window of
    /// prices is full at `window - 1`, a window of returns at `window`.
    pub fn first_index(&self) -> usize {
        match *self {
            Indicator::Price => 0,
            Indicator::AveragePrice { window }
            | Indicator::ExponentialAveragePrice { window }
            | Indicator::PriceDeviation { window }
            | Indicator::MaxDrawdown { window } => (window as usize).saturating_sub(1),
            Indicator::CumulativeReturn { window }
            | Indicator::AverageReturn { window }
            | Indicator::ReturnDeviation { window }
            | Indicator::RelativeStrength { window } => window as usize,
        }
    }

    /// Whether each value is built on the one before it. The other seven
    /// read a window and nothing older.
    pub fn carries(&self) -> bool {
        matches!(self, Indicator::ExponentialAveragePrice { .. } | Indicator::RelativeStrength { .. })
    }
}

/// The indicator at the last close of `closes`, which are a series from its
/// first close on. Nothing when there are too few closes, or when the window
/// is shorter than the indicator has a meaning over.
pub fn at(indicator: Indicator, closes: &[f64]) -> Option<f64> {
    let last = closes.len().checked_sub(1)?;
    if last < indicator.first_index() || indicator.window().is_some_and(|window| window < indicator.least_window()) {
        return None;
    }
    if indicator.carries() {
        let mut running = Running::new(indicator)?;
        return closes.iter().fold(None, |_, close| running.push(*close));
    }
    let window = indicator.window().unwrap_or(1) as usize;
    // The closes of a price window, and the closes its returns are taken between.
    let prices = &closes[closes.len() - window..];
    let ends = &closes[last - indicator.first_index()..];
    let change = |index: usize| ends[index + 1] / ends[index] - 1.0;
    Some(match indicator {
        Indicator::Price => closes[last],
        Indicator::AveragePrice { .. } => mean(window, |index| prices[index]),
        Indicator::PriceDeviation { .. } => deviation(window, |index| prices[index]),
        Indicator::CumulativeReturn { .. } => closes[last] / ends[0] - 1.0,
        Indicator::AverageReturn { .. } => mean(window, change),
        Indicator::ReturnDeviation { .. } => deviation(window, change),
        Indicator::MaxDrawdown { .. } => drawdown(prices),
        Indicator::ExponentialAveragePrice { .. } | Indicator::RelativeStrength { .. } => return None,
    })
}

/// The mean of `count` values, added up oldest to newest.
fn mean(count: usize, value: impl Fn(usize) -> f64) -> f64 {
    let mut sum = 0.0;
    for index in 0..count {
        sum += value(index);
    }
    sum / count as f64
}

/// The sample deviation of `count` values, two at least.
///
/// A window whose values are all the same has none, and gives exactly 0.
/// That is said outright because the sums do not promise it: the mean of
/// equal floats can miss them by a last digit, and a residue of that size
/// must not read as movement.
fn deviation(count: usize, value: impl Fn(usize) -> f64) -> f64 {
    let first = value(0);
    if (1..count).all(|index| value(index) == first) {
        return 0.0;
    }
    let mean = mean(count, &value);
    let mut squares = 0.0;
    for index in 0..count {
        let gap = value(index) - mean;
        squares += gap * gap;
    }
    (squares / (count - 1) as f64).sqrt()
}

/// The largest fall from a peak inside these closes, as a positive fraction.
fn drawdown(prices: &[f64]) -> f64 {
    let (mut peak, mut worst) = (prices[0], 0.0_f64);
    for price in prices {
        peak = peak.max(*price);
        worst = worst.max(1.0 - price / peak);
    }
    worst
}

/// One of the two indicators that carry a value from close to close, fed a
/// series one close at a time from its first close.
///
/// Where the series starts is part of the value: the first `window` closes
/// seed it with a plain mean, and every later value leans on that seed a
/// little less.
#[derive(Debug, Clone)]
pub struct Running {
    window: usize,
    /// Closes fed so far.
    seen: usize,
    state: State,
}

#[derive(Debug, Clone)]
enum State {
    /// The exponential average. Until the window is full `value` is the sum
    /// of the closes so far.
    Average { value: f64 },
    /// Relative strength, by Wilder's smoothing. Until the window is full
    /// `gain` and `loss` are the sums of the rises and the falls so far.
    Strength { before: f64, gain: f64, loss: f64 },
}

impl Running {
    /// Nothing for the seven indicators that carry no state, or for a window
    /// shorter than the indicator has a meaning over.
    pub fn new(indicator: Indicator) -> Option<Running> {
        let window = indicator.window().filter(|window| *window >= indicator.least_window())? as usize;
        let state = match indicator {
            Indicator::ExponentialAveragePrice { .. } => State::Average { value: 0.0 },
            Indicator::RelativeStrength { .. } => State::Strength { before: 0.0, gain: 0.0, loss: 0.0 },
            _ => return None,
        };
        Some(Running { window, seen: 0, state })
    }

    /// Feed the next close. Gives the value at that close, once there is one.
    pub fn push(&mut self, close: f64) -> Option<f64> {
        self.seen += 1;
        let (window, count) = (self.window, self.window as f64);
        match &mut self.state {
            State::Average { value } => {
                if self.seen <= window {
                    *value += close;
                    if self.seen < window {
                        return None;
                    }
                    *value /= count;
                } else {
                    let weight = 2.0 / (count + 1.0);
                    *value = weight * close + (1.0 - weight) * *value;
                }
                Some(*value)
            }
            State::Strength { before, gain, loss } => {
                // A price difference, not a return.
                let change = close - *before;
                *before = close;
                if self.seen == 1 {
                    return None;
                }
                let (up, down) = (change.max(0.0), (-change).max(0.0));
                // One change fewer than closes.
                if self.seen - 1 <= window {
                    *gain += up;
                    *loss += down;
                    if self.seen - 1 < window {
                        return None;
                    }
                    *gain /= count;
                    *loss /= count;
                } else {
                    *gain = (*gain * (count - 1.0) + up) / count;
                    *loss = (*loss * (count - 1.0) + down) / count;
                }
                Some(strength(*gain, *loss))
            }
        }
    }
}

/// Relative strength from the average rise and the average fall. With no
/// fall it is 1. With no fall and no rise the series has not moved, which is
/// neither strong nor weak, so it sits in the middle and trips neither an
/// overbought rule nor an oversold one.
fn strength(gain: f64, loss: f64) -> f64 {
    if loss > 0.0 {
        1.0 - 1.0 / (1.0 + gain / loss)
    } else if gain > 0.0 {
        1.0
    } else {
        0.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Thirty made-up closes. Every expected number below was worked out
    /// from them apart from this code.
    const CLOSES: [f64; 30] = [
        100.0, 101.5, 99.75, 102.25, 103.0, 101.0, 104.5, 106.25, 105.0, 107.75, 108.5, 106.0, 109.25, 111.0, 110.5, 112.75, 109.0, 113.5, 115.25, 114.0, 116.75,
        118.5, 117.0, 119.25, 121.0, 118.75, 122.5, 124.25, 123.0, 125.75,
    ];

    fn close_to(got: Option<f64>, expected: f64) -> bool {
        got.is_some_and(|got| (got - expected).abs() <= 1e-12 * expected.abs().max(1.0))
    }

    /// The indicator at every close of a series.
    fn series(indicator: Indicator, closes: &[f64]) -> Vec<Option<f64>> {
        (0..closes.len()).map(|index| at(indicator, &closes[..=index])).collect()
    }

    /// The index of the first close each one has a value at, for a window of 5.
    #[test]
    fn each_indicator_starts_when_its_window_is_full() {
        let firsts = [
            (Indicator::Price, 0),
            (Indicator::AveragePrice { window: 5 }, 4),
            (Indicator::ExponentialAveragePrice { window: 5 }, 4),
            (Indicator::PriceDeviation { window: 5 }, 4),
            (Indicator::MaxDrawdown { window: 5 }, 4),
            (Indicator::CumulativeReturn { window: 5 }, 5),
            (Indicator::AverageReturn { window: 5 }, 5),
            (Indicator::ReturnDeviation { window: 5 }, 5),
            (Indicator::RelativeStrength { window: 5 }, 5),
        ];
        for (indicator, first) in firsts {
            assert_eq!(indicator.first_index(), first, "{indicator:?}");
            let values = series(indicator, &CLOSES);
            assert_eq!(values.iter().position(Option::is_some), Some(first), "{indicator:?}");
            assert!(values[first..].iter().all(Option::is_some), "{indicator:?}");
        }
        assert_eq!(at(Indicator::Price, &[]), None);
    }

    #[test]
    fn the_seven_window_indicators_follow_their_formulas() {
        let cases = [
            (Indicator::Price, 29, 125.75),
            (Indicator::AveragePrice { window: 5 }, 4, 101.3),
            (Indicator::AveragePrice { window: 5 }, 29, 122.85),
            (Indicator::AveragePrice { window: 1 }, 17, 113.5),
            (Indicator::PriceDeviation { window: 5 }, 4, 1.407_568_826_025_924_7),
            (Indicator::PriceDeviation { window: 5 }, 29, 2.613_666_007_737_025),
            (Indicator::PriceDeviation { window: 2 }, 1, 1.060_660_171_779_821_2),
            (Indicator::CumulativeReturn { window: 5 }, 5, 0.010_000_000_000_000_009),
            (Indicator::CumulativeReturn { window: 5 }, 29, 0.039_256_198_347_107_36),
            (Indicator::CumulativeReturn { window: 1 }, 2, -0.017_241_379_310_344_862),
            (Indicator::AverageReturn { window: 5 }, 5, 0.002_147_752_985_657_436),
            (Indicator::AverageReturn { window: 5 }, 29, 0.007_913_396_347_203_805),
            (Indicator::ReturnDeviation { window: 5 }, 5, 0.019_736_880_165_418_823),
            (Indicator::ReturnDeviation { window: 5 }, 29, 0.021_418_780_271_050_878),
            (Indicator::ReturnDeviation { window: 2 }, 2, 0.022_798_097_945_152_435),
            (Indicator::MaxDrawdown { window: 5 }, 4, 0.017_241_379_310_344_862),
            (Indicator::MaxDrawdown { window: 5 }, 29, 0.010_060_362_173_038_184),
            (Indicator::MaxDrawdown { window: 10 }, 20, 0.033_259_423_503_325_89),
        ];
        for (indicator, index, expected) in cases {
            let got = at(indicator, &CLOSES[..=index]);
            assert!(close_to(got, expected), "{indicator:?} at {index}: {got:?}, not {expected}");
        }
    }

    #[test]
    fn a_drawdown_is_positive_and_reads_only_its_own_window() {
        // The fall from 112.75 to 109 is the deepest in these closes. A
        // window that has left the peak behind no longer sees it.
        let deepest = 1.0 - 109.0 / 112.75;
        assert!(close_to(at(Indicator::MaxDrawdown { window: 2 }, &CLOSES[..=16]), deepest));
        assert!(close_to(at(Indicator::MaxDrawdown { window: 5 }, &CLOSES[..=19]), deepest));
        assert_eq!(at(Indicator::MaxDrawdown { window: 3 }, &CLOSES[..=18]), Some(0.0));
        // Rising closes have no fall in them.
        assert_eq!(at(Indicator::MaxDrawdown { window: 4 }, &[1.0, 2.0, 3.0, 4.0]), Some(0.0));
    }

    #[test]
    fn the_exponential_average_is_seeded_with_the_mean_of_its_first_window() {
        let values = series(Indicator::ExponentialAveragePrice { window: 5 }, &CLOSES);
        // The seed is the plain mean of the first five closes, at the fifth.
        assert!(close_to(values[4], 101.3));
        // Then a third of each new close: 2 / (5 + 1).
        assert!(close_to(values[5], 101.3 + (101.0 - 101.3) / 3.0));
        assert!(close_to(values[6], 102.3));
        assert!(close_to(values[29], 123.171_754_485_217_32));
        // A window of one is the price itself.
        assert_eq!(series(Indicator::ExponentialAveragePrice { window: 1 }, &CLOSES), CLOSES.map(Some));
    }

    #[test]
    fn relative_strength_is_seeded_with_plain_means_and_then_smoothed() {
        let values = series(Indicator::RelativeStrength { window: 5 }, &CLOSES);
        // Changes 1 to 5 are +1.5, -1.75, +2.5, +0.75, -2: rises 4.75 and falls 3.75.
        assert!(close_to(values[5], 4.75 / 8.5));
        // The sixth change is +3.5: (0.95 * 4 + 3.5) / 5 against (0.75 * 4) / 5.
        assert!(close_to(values[6], 1.46 / 2.06));
        assert!(close_to(values[29], 0.758_816_902_861_675_6));
        // A longer window, seeded at its own first full window of changes.
        let longer = series(Indicator::RelativeStrength { window: 14 }, &CLOSES);
        assert_eq!(longer.iter().position(Option::is_some), Some(14));
        assert!(close_to(longer[14], 0.698_113_207_547_169_8));
        assert!(close_to(longer[29], 0.722_192_325_237_287_8));
    }

    #[test]
    fn where_a_carried_indicator_starts_is_part_of_its_value() {
        // The same last close, reached from the first close and from the eleventh.
        for (indicator, whole, late) in [
            (Indicator::RelativeStrength { window: 5 }, 0.758_816_902_861_675_6, 0.756_055_027_685_813_5),
            (Indicator::ExponentialAveragePrice { window: 5 }, 123.171_754_485_217_32, 123.171_491_260_623_55),
        ] {
            assert!(close_to(at(indicator, &CLOSES), whole), "{indicator:?}");
            assert!(close_to(at(indicator, &CLOSES[10..]), late), "{indicator:?}");
        }
        // The seven that read a window give the same value from anywhere.
        let windowed = Indicator::ReturnDeviation { window: 5 };
        assert_eq!(at(windowed, &CLOSES), at(windowed, &CLOSES[10..]));
    }

    #[test]
    fn relative_strength_with_no_fall_is_one_and_with_no_move_is_a_half() {
        let strength = Indicator::RelativeStrength { window: 2 };
        // Flat, up, up, flat, up, then down.
        let closes = [91.0, 91.0, 91.02, 91.05, 91.05, 91.07, 91.01];
        let values = series(strength, &closes);
        assert_eq!(values[..5], [None, None, Some(1.0), Some(1.0), Some(1.0)]);
        assert_eq!(values[5], Some(1.0));
        assert!(values[6].is_some_and(|value| value < 1.0 && value > 0.0));
        // A series that has never moved.
        assert_eq!(series(strength, &[50.0; 6])[2..], [Some(0.5); 4]);
        // Only falls.
        assert_eq!(at(strength, &[3.0, 2.0, 1.0]), Some(0.0));
    }

    #[test]
    fn a_flat_window_reads_exactly_zero() {
        // Three closes of 0.1 add up to more than 0.3 in floats, so their
        // mean is not 0.1 and the plain formula would leave a residue.
        assert_ne!((0.1 + 0.1 + 0.1) / 3.0, 0.1);
        let flat = [7.25, 0.1, 0.1, 0.1, 0.1];
        for indicator in [
            Indicator::PriceDeviation { window: 3 },
            Indicator::ReturnDeviation { window: 3 },
            Indicator::CumulativeReturn { window: 3 },
            Indicator::AverageReturn { window: 3 },
            Indicator::MaxDrawdown { window: 3 },
        ] {
            assert_eq!(at(indicator, &flat), Some(0.0), "{indicator:?}");
        }
        // One close earlier the windows still hold the fall from 7.25.
        assert!(at(Indicator::PriceDeviation { window: 3 }, &flat[..3]).is_some_and(|value| value > 0.0));
        // Relative strength does not move through a flat stretch.
        let strength = Indicator::RelativeStrength { window: 2 };
        let moved = [10.0, 11.0, 10.5, 10.5, 10.5, 10.5];
        let values = series(strength, &moved);
        assert!(close_to(values[3], values[2].unwrap()) && close_to(values[5], values[2].unwrap()));
    }

    #[test]
    fn a_window_too_short_to_mean_anything_has_no_value() {
        for indicator in [
            Indicator::PriceDeviation { window: 1 },
            Indicator::ReturnDeviation { window: 1 },
            Indicator::MaxDrawdown { window: 1 },
            Indicator::RelativeStrength { window: 1 },
        ] {
            assert_eq!(at(indicator, &CLOSES), None, "{indicator:?}");
            assert!(Running::new(indicator).is_none(), "{indicator:?}");
        }
        assert!(Running::new(Indicator::AveragePrice { window: 5 }).is_none());
    }
}
