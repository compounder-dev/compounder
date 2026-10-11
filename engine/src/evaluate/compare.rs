//! How two values are compared in a condition.
//!
//! Not exactly. A reading and the value it is held against are often the
//! same number reached two ways, and their last digits differ. So one is
//! above the other only when it is above by more than a small fixed margin,
//! and two values inside that margin are equal. The margin is absolute: it
//! is the same for a price of 400 and a return of 0.002.

use crate::rules::Comparator;

/// How far apart two values must be before one counts as above the other.
pub const TOLERANCE: f64 = 1e-12;

/// Whether `left` stands to `right` as the comparator says. Both are in the
/// unit of the left reading. A gap of exactly the tolerance is not enough
/// for above or below, and is still equal.
pub fn holds(comparator: Comparator, left: f64, right: f64) -> bool {
    match comparator {
        Comparator::Above => left - right > TOLERANCE,
        Comparator::Below => right - left > TOLERANCE,
        Comparator::AtLeast => left - right >= -TOLERANCE,
        Comparator::AtMost => right - left >= -TOLERANCE,
        Comparator::Equal => (left - right).abs() <= TOLERANCE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Comparator::{Above, AtLeast, AtMost, Below, Equal};

    /// Which of above, at least, equal, at most and below hold.
    fn all(left: f64, right: f64) -> [bool; 5] {
        [Above, AtLeast, Equal, AtMost, Below].map(|comparator| holds(comparator, left, right))
    }

    #[test]
    fn values_inside_the_margin_are_equal_and_neither_is_above() {
        assert_eq!(all(0.5, 0.5), [false, true, true, true, false]);
        assert_eq!(all(1e-13, 0.0), [false, true, true, true, false]);
        assert_eq!(all(0.0, 1e-13), [false, true, true, true, false]);
        // Plain doubles call the sum of a tenth and a fifth more than three tenths.
        let (tenth, fifth) = (0.1_f64, 0.2_f64);
        assert!(tenth + fifth > 0.3);
        assert_eq!(all(tenth + fifth, 0.3), [false, true, true, true, false]);
    }

    #[test]
    fn values_outside_the_margin_are_ordered_and_not_equal() {
        assert_eq!(all(1e-11, 0.0), [true, true, false, false, false]);
        assert_eq!(all(0.0, 1e-11), [false, false, false, true, true]);
        assert_eq!(all(2.0, 1.0), [true, true, false, false, false]);
        assert_eq!(all(-0.02, 0.01), [false, false, false, true, true]);
    }

    #[test]
    fn a_gap_of_exactly_the_margin_is_equal_and_not_above() {
        // Both gaps are the margin to the last bit.
        assert_eq!(1e-12_f64 - 0.0, TOLERANCE);
        assert_eq!(2e-12_f64 - 1e-12, TOLERANCE);
        assert_eq!(all(1e-12, 0.0), [false, true, true, true, false]);
        assert_eq!(all(0.0, 1e-12), [false, true, true, true, false]);
        assert_eq!(all(2e-12, 1e-12), [false, true, true, true, false]);
        // The next double past the margin is above.
        let past = f64::from_bits(TOLERANCE.to_bits() + 1);
        assert_eq!(all(past, 0.0), [true, true, false, false, false]);
        assert_eq!(all(0.0, past), [false, false, false, true, true]);
    }

    #[test]
    fn the_margin_is_the_same_at_every_size_of_value() {
        // Near 387 neighbouring doubles are 5.7e-14 apart, so 18 steps is just past the margin and 17 is inside it.
        let step = |value: f64, steps: u64| f64::from_bits(value.to_bits() + steps);
        assert!(step(387.27, 18) - 387.27 > TOLERANCE && step(387.27, 17) - 387.27 < TOLERANCE);
        assert_eq!(all(step(387.27, 18), 387.27), [true, true, false, false, false]);
        assert_eq!(all(step(387.27, 17), 387.27), [false, true, true, true, false]);
        // A relative margin would call these two returns equal. They are 4e-12 apart.
        assert_eq!(all(0.000_000_86, 0.000_000_860_004), [false, false, false, true, true]);
    }
}
