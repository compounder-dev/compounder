//! The days a run covers, and why they are not the days asked for.
//!
//! A strategy starts on the first trading day on which every reading in the
//! whole tree has a value. A branch that is never taken still counts, and a
//! child with no data is never skipped: there is no state in which part of
//! the tree is missing. It ends on the last trading day every asset in it
//! still has a price.
//!
//! Each asset that is not ready on the day asked for gives one warning with
//! the day it is ready, and each asset whose prices stop before the day
//! asked for gives one with its last day. With no day asked for, the ends of
//! the calendar stand for it.

use super::Evaluator;
use crate::rules::{Asset, Issue, IssueCode};
use compounder_prices::day::Day;
use serde::{Serialize, Serializer};

/// Something a run did that was not asked for, and the asset behind it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Warning {
    pub code: WarningCode,
    pub asset: Asset,
    /// For a late start, the first day this asset alone would let the
    /// strategy be evaluated. For an early end, its last day with a price.
    #[serde(serialize_with = "day_text")]
    pub day: Day,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WarningCode {
    LateStart,
    EarlyEnd,
}

/// A day as `2026-10-09`, the way it is written everywhere.
pub(crate) fn day_text<S: Serializer>(day: &Day, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_str(day)
}

/// An asset as it reads in a message.
pub(crate) fn name(asset: &Asset) -> &str {
    match asset {
        Asset::Listed { symbol, .. } => symbol,
        Asset::Token { id } => id,
    }
}

/// The trading days a run covers, both included, and why they are not the
/// days asked for when they are not.
#[derive(Debug, Clone, PartialEq)]
pub struct Window {
    pub first: Day,
    pub last: Day,
    pub warnings: Vec<Warning>,
}

impl Evaluator<'_> {
    /// The days a run from `start` to `end` covers. A start on a closed day
    /// moves to the next trading day and an end on one moves back. `None`
    /// asks for as early, or as late, as the prices allow.
    pub fn window(&self, start: Option<Day>, end: Option<Day>) -> Result<Window, Vec<Issue>> {
        let calendar = self.history.calendar();
        let none = |path: &str, message: String, fix: &str| vec![Issue { code: IssueCode::NoTradingDay, path: path.to_string(), message, fix: fix.to_string() }];
        if let (Some(start), Some(end)) = (start, end) {
            if start > end {
                return Err(none("run.end", format!("The run ends on {end}, before it starts on {start}."), "Set an end on or after the start."));
            }
        }
        let asked_first = match start {
            Some(day) => calendar.on_or_after(day).ok_or_else(|| none("run.start", format!("The run starts on {day}, after the last trading day there are prices for, {}.", calendar.last()), "Start earlier."))?,
            None => 0,
        };
        let asked_last = match end {
            Some(day) => calendar.on_or_before(day).ok_or_else(|| none("run.end", format!("The run ends on {day}, before the first trading day there are prices for, {}.", calendar.first()), "End later."))?,
            None => calendar.len() - 1,
        };
        let first = (asked_first as i64).max(self.ready);
        let last = asked_last.min(self.last);
        if first > last as i64 {
            let can = match self.days() {
                Some((first, last)) => format!("The strategy can be evaluated from {first} to {last}."),
                None => "There is no day on which every reading in the strategy has a value.".to_string(),
            };
            return Err(none("run", format!("No trading day in the run has a value for the whole strategy. {can}"), "Move the run inside those days, or load more price history."));
        }

        let mut warnings = Vec::new();
        let assets = || self.plan.assets.iter().zip(&self.prices).zip(&self.waits);
        for ((asset, prices), wait) in assets().filter(|(_, wait)| **wait > asked_first as i64) {
            // No wait is later than the first day, which is on the calendar.
            let (day, begins) = (calendar.day(*wait as usize), calendar.day(prices.range().start));
            let message = if day == begins {
                format!("{} has no price before {begins}, so the run cannot start before then.", name(asset))
            } else {
                format!("{} has prices from {begins}, and the readings that rest on it first have a value on {day}, so the run cannot start before then.", name(asset))
            };
            warnings.push(Warning { code: WarningCode::LateStart, asset: asset.clone(), day, message });
        }
        for ((asset, prices), _) in assets().filter(|((_, prices), _)| prices.range().end - 1 < asked_last) {
            let day = calendar.day(prices.range().end - 1);
            let message = format!("{} has no price after {day}, so the run cannot go on past then.", name(asset));
            warnings.push(Warning { code: WarningCode::EarlyEnd, asset: asset.clone(), day, message });
        }
        Ok(Window { first: calendar.day(first as usize), last: calendar.day(last), warnings })
    }
}
