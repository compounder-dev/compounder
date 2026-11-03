//! Bars, books and what a strategy sees when it decides.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub const HOUR_MS: i64 = 3_600_000;
pub const DAY_MS: i64 = 24 * HOUR_MS;
/// These markets never close, so a year is every hour of it.
pub const YEAR_MS: f64 = 365.0 * DAY_MS as f64;

/// One bar of a market, opening at `t` (milliseconds since 1970).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Bar {
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: f64,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Interval {
    #[serde(rename = "15m")]
    M15,
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "4h")]
    H4,
    #[serde(rename = "1d")]
    D1,
}

impl Interval {
    pub fn ms(self) -> i64 {
        match self {
            Interval::M15 => HOUR_MS / 4,
            Interval::H1 => HOUR_MS,
            Interval::H4 => 4 * HOUR_MS,
            Interval::D1 => DAY_MS,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Interval::M15 => "15m",
            Interval::H1 => "1h",
            Interval::H4 => "4h",
            Interval::D1 => "1d",
        }
    }

    pub fn per_year(self) -> f64 {
        YEAR_MS / self.ms() as f64
    }

    /// The start of the bar that holds `t`.
    pub fn floor(self, t: i64) -> i64 {
        t - t.rem_euclid(self.ms())
    }
}

/// One side of a book: price and size, best first.
pub type Levels = Vec<(f64, f64)>;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Book {
    pub bids: Levels,
    pub asks: Levels,
}

impl Book {
    pub fn mid(&self) -> Option<f64> {
        Some((self.bids.first()?.0 + self.asks.first()?.0) / 2.0)
    }
}

/// What a strategy sees when it decides: for each market its closed bars, oldest
/// first, ending with the bar that closed at `now`; its hourly funding rate; and
/// the share of the fund's equity each market holds now.
pub struct View<'a> {
    pub now: i64,
    pub bars: BTreeMap<String, &'a [Bar]>,
    pub funding: BTreeMap<String, f64>,
    pub weights: BTreeMap<String, f64>,
}

impl View<'_> {
    pub fn closes(&self, coin: &str) -> Vec<f64> {
        self.bars.get(coin).map(|bars| bars.iter().map(|bar| bar.c).collect()).unwrap_or_default()
    }

    pub fn last(&self, coin: &str) -> Option<f64> {
        self.bars.get(coin)?.last().map(|bar| bar.c)
    }
}

/// Log returns of a price series.
pub fn returns(closes: &[f64]) -> Vec<f64> {
    closes.windows(2).filter(|w| w[0] > 0.0 && w[1] > 0.0).map(|w| (w[1] / w[0]).ln()).collect()
}

/// Standard deviation of a sample.
pub fn stdev(xs: &[f64]) -> f64 {
    if xs.len() < 2 {
        return 0.0;
    }
    let mean = xs.iter().sum::<f64>() / xs.len() as f64;
    (xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / (xs.len() - 1) as f64).sqrt()
}

/// Exponential moving average of the whole series, seeded with its first value.
pub fn ema(xs: &[f64], span: usize) -> Option<f64> {
    let first = *xs.first()?;
    let k = 2.0 / (span as f64 + 1.0);
    Some(xs[1..].iter().fold(first, |acc, x| acc + k * (x - acc)))
}

/// Where a moment falls in New York: days since 1970 there, the hour, and the
/// weekday (0 is Monday). Daylight saving runs from 2am on the second Sunday of
/// March to 2am on the first Sunday of November.
pub fn new_york(t: i64) -> (i64, u32, u32) {
    let offset = if daylight_saving(t) { 4 } else { 5 };
    let local = t - offset * HOUR_MS;
    let day = local.div_euclid(DAY_MS);
    let hour = (local.rem_euclid(DAY_MS) / HOUR_MS) as u32;
    // 1970-01-01 was a Thursday.
    let weekday = (day + 3).rem_euclid(7) as u32;
    (day, hour, weekday)
}

fn daylight_saving(t: i64) -> bool {
    let year = year_of(t.div_euclid(DAY_MS));
    // The nth Sunday of a month, as days since 1970.
    let sunday = |month: u32, nth: i64| {
        let first = days_from_civil(year, month, 1);
        let weekday = (first + 3).rem_euclid(7); // 0 is Monday
        first + (6 - weekday).rem_euclid(7) + 7 * (nth - 1)
    };
    let starts = sunday(3, 2) * DAY_MS + 7 * HOUR_MS; // 2am EST is 7am UTC
    let ends = sunday(11, 1) * DAY_MS + 6 * HOUR_MS; // 2am EDT is 6am UTC
    t >= starts && t < ends
}

fn year_of(days: i64) -> i64 {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    yoe + era * 400 + if mp >= 10 { 1 } else { 0 }
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let m = month as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_york_time_follows_daylight_saving() {
        // 2026-07-01 14:00 UTC is 10:00 in New York, a Wednesday.
        let summer = days_from_civil(2026, 7, 1) * DAY_MS + 14 * HOUR_MS;
        assert_eq!(new_york(summer).1, 10);
        assert_eq!(new_york(summer).2, 2);
        // 2026-12-01 15:00 UTC is 10:00 in New York, a Tuesday.
        let winter = days_from_civil(2026, 12, 1) * DAY_MS + 15 * HOUR_MS;
        assert_eq!(new_york(winter).1, 10);
        assert_eq!(new_york(winter).2, 1);
        // Clocks went back at 6am UTC on 2026-11-01.
        let change = days_from_civil(2026, 11, 1) * DAY_MS + 6 * HOUR_MS;
        assert_eq!(new_york(change - 1).1, 1);
        assert_eq!(new_york(change).1, 1);
        assert_eq!(year_of(days_from_civil(2027, 1, 1)), 2027);
        assert_eq!(year_of(days_from_civil(2026, 12, 31)), 2026);
    }
}
