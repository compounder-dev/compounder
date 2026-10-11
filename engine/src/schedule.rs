//! Whether a strategy trades today.
//!
//! A calendar strategy trades on the first trading day of each period: every
//! day, each week, month, quarter or year. One rule covers all five:
//!
//! ```text
//! due(today) = period(today) differs from period(last day traded)
//! ```
//!
//! The schedule is tied to the calendar, not to the day a strategy started:
//! one that starts on a Wednesday trades again the next Monday. A run that
//! was missed stays due until it happens, because the period it was last
//! traded in is still an old one.
//!
//! A threshold strategy looks every day and trades when one holding has
//! drifted far enough from its target. The gap is in weight points of the
//! whole portfolio, and the largest single gap decides.

use crate::rules::Frequency;
use compounder_prices::day::Day;

/// A number that is the same for two days exactly when they fall in the
/// same period.
fn period(frequency: Frequency, day: Day) -> i64 {
    let (year, month, _) = day.ymd();
    let (year, month) = (i64::from(year), i64::from(month));
    match frequency {
        Frequency::Daily => i64::from(day.number()),
        // Weeks start on Monday. 1970-01-01 was a Thursday, three days in.
        Frequency::Weekly => (i64::from(day.number()) + 3).div_euclid(7),
        Frequency::Monthly => year * 12 + month,
        Frequency::Quarterly => year * 4 + (month - 1) / 3,
        Frequency::Yearly => year,
    }
}

/// Whether a calendar strategy trades on `today`, a trading day, when it
/// last traded on `last_traded`. A strategy that has never traded is due.
pub fn due(frequency: Frequency, today: Day, last_traded: Option<Day>) -> bool {
    last_traded.is_none_or(|last| period(frequency, today) != period(frequency, last))
}

/// How far the holdings stand from the targets: the largest gap between one
/// asset's held weight and its target weight, in weight points. Both lists
/// are in the same asset order. An asset held and not wanted has a target of
/// 0, and one wanted and not held has a held weight of 0. Held weights count
/// each holding over the whole value, cash included, so they need not sum to 1.
pub fn drift(held: &[f64], targets: &[f64]) -> f64 {
    held.iter().zip(targets).map(|(held, target)| (held - target).abs()).fold(0.0, f64::max)
}

/// Whether a threshold strategy trades: the drift is at least the corridor.
pub fn breached(corridor: f64, held: &[f64], targets: &[f64]) -> bool {
    drift(held, targets) >= corridor
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> Day {
        text.parse().expect("a day")
    }

    /// The trading days of a made-up stretch: weekdays from `first` to
    /// `last`, less the closed days.
    fn trading_days(first: &str, last: &str, closed: &[&str]) -> Vec<Day> {
        (day(first).number()..=day(last).number())
            .map(Day::from_number)
            // 1970-01-01 was a Thursday, so days 2 and 3 of each seven are a weekend.
            .filter(|day| !matches!(day.number().rem_euclid(7), 2 | 3) && !closed.contains(&day.to_string().as_str()))
            .collect()
    }

    /// The days a strategy trades on when it starts on the first of `days`.
    fn traded(frequency: Frequency, days: &[Day]) -> Vec<String> {
        let mut last = None;
        let mut traded = Vec::new();
        for today in days {
            if due(frequency, *today, last) {
                last = Some(*today);
                traded.push(today.to_string());
            }
        }
        traded
    }

    #[test]
    fn a_weekly_strategy_trades_on_the_first_trading_day_of_each_week() {
        // A start on a Wednesday. The Monday of the third week is closed.
        let days = trading_days("2025-01-08", "2025-01-31", &["2025-01-20"]);
        assert_eq!(traded(Frequency::Weekly, &days), ["2025-01-08", "2025-01-13", "2025-01-21", "2025-01-27"]);
        assert_eq!(traded(Frequency::Daily, &days), days.iter().map(Day::to_string).collect::<Vec<_>>());
    }

    #[test]
    fn longer_schedules_trade_on_the_first_trading_day_of_each_period() {
        // Mid November 2024 to early February 2026. New Year's Day is closed
        // both years, and the first of some months falls on a weekend.
        let days = trading_days("2024-11-14", "2026-02-06", &["2025-01-01", "2026-01-01"]);
        assert_eq!(
            traded(Frequency::Monthly, &days),
            [
                "2024-11-14", "2024-12-02", "2025-01-02", "2025-02-03", "2025-03-03", "2025-04-01", "2025-05-01", "2025-06-02", "2025-07-01", "2025-08-01",
                "2025-09-01", "2025-10-01", "2025-11-03", "2025-12-01", "2026-01-02", "2026-02-02"
            ]
        );
        assert_eq!(traded(Frequency::Quarterly, &days), ["2024-11-14", "2025-01-02", "2025-04-01", "2025-07-01", "2025-10-01", "2026-01-02"]);
        assert_eq!(traded(Frequency::Yearly, &days), ["2024-11-14", "2025-01-02", "2026-01-02"]);
    }

    #[test]
    fn a_week_runs_from_monday_to_sunday() {
        let (friday, saturday, sunday, monday) = (day("2025-03-07"), day("2025-03-08"), day("2025-03-09"), day("2025-03-10"));
        assert!(!due(Frequency::Weekly, saturday, Some(friday)) && !due(Frequency::Weekly, sunday, Some(friday)));
        assert!(due(Frequency::Weekly, monday, Some(sunday)) && due(Frequency::Weekly, monday, Some(friday)));
        // The week that holds New Year is one week, though the year changes inside it.
        assert!(!due(Frequency::Weekly, day("2026-01-02"), Some(day("2025-12-29"))));
        assert!(due(Frequency::Yearly, day("2026-01-02"), Some(day("2025-12-29"))));
        // Long before 1970 too.
        assert!(!due(Frequency::Weekly, day("1962-07-06"), Some(day("1962-07-02"))) && due(Frequency::Weekly, day("1962-07-09"), Some(day("1962-07-06"))));
    }

    #[test]
    fn a_strategy_that_has_never_traded_is_due_and_a_missed_run_stays_due() {
        for frequency in [Frequency::Daily, Frequency::Weekly, Frequency::Monthly, Frequency::Quarterly, Frequency::Yearly] {
            assert!(due(frequency, day("2025-06-18"), None), "{frequency:?}");
            assert!(!due(frequency, day("2025-06-18"), Some(day("2025-06-18"))), "{frequency:?}");
        }
        // Last traded in May. The first days of June were missed, and the run is still owed mid month.
        let last = Some(day("2025-05-01"));
        assert!(due(Frequency::Monthly, day("2025-06-02"), last) && due(Frequency::Monthly, day("2025-06-17"), last));
        assert!(!due(Frequency::Quarterly, day("2025-06-17"), last) && due(Frequency::Quarterly, day("2025-07-09"), last));
    }

    #[test]
    fn drift_is_the_largest_single_gap_in_weight_points() {
        // Targets of 20 and 80. Held at 25.42 and 74.58: both are 5.42 points out.
        assert!((drift(&[0.2542, 0.7458], &[0.2, 0.8]) - 0.0542).abs() < 1e-15);
        // Three small gaps do not add up to a breach.
        let (held, targets) = ([0.37, 0.31, 0.32], [0.34, 0.33, 0.33]);
        assert!((drift(&held, &targets) - 0.03).abs() < 1e-15);
        assert!(!breached(0.05, &held, &targets) && breached(0.02, &held, &targets));
        // An asset held and no longer wanted is out by its whole weight, and so is one wanted and not held.
        assert_eq!(drift(&[0.5, 0.5, 0.0], &[0.5, 0.0, 0.5]), 0.5);
        // Cash left over is not an asset: holdings of 49 and 49 against 50 and 50.
        assert!((drift(&[0.49, 0.49], &[0.5, 0.5]) - 0.01).abs() < 1e-15);
        assert_eq!(drift(&[], &[]), 0.0);
    }

    #[test]
    fn a_gap_of_exactly_the_corridor_trades() {
        assert!(breached(0.25, &[0.75, 0.25], &[0.5, 0.5]));
        assert!(!breached(0.25, &[0.7499, 0.2501], &[0.5, 0.5]));
        assert!(breached(0.05, &[0.2542, 0.7458], &[0.2, 0.8]) && !breached(0.05, &[0.2481, 0.7519], &[0.2, 0.8]));
    }
}
