//! The trading calendar, and prices laid out on it.
//!
//! The calendar is read from the data. A trading day is a day on which a
//! broad, old reference ticker has a bar. No list of holidays is kept by
//! hand, so an unplanned closure and a new holiday come out right by
//! themselves, and a half day is an ordinary day whose close came early.
//!
//! `align` lays one ticker's adjusted series on the calendar, one price for
//! every trading day from its first bar to its last. That is what a backtest
//! reads, by position. Three rules, each as seen in the series we check
//! against:
//!
//! - Before its first bar and after its last, a ticker has no price. A run
//!   over several tickers covers the days on which all of them have one, so
//!   it starts when the youngest starts. `common` gives those days.
//! - Inside its own life a ticker has a price on every trading day. A day
//!   with no bar repeats the close before it, and is marked as filled. A thin
//!   fund is the usual case, on a day nobody traded it.
//! - A date that is not a trading day belongs to the next one that is, so a
//!   start date on a holiday moves forward.

use std::ops::Range;

use crate::{
    adjust::{Bar, Series},
    day::Day,
};

/// Every trading day, oldest first.
#[derive(Debug, Clone, PartialEq)]
pub struct Calendar {
    days: Vec<Day>,
    /// For each date from the first trading day to the last, the position of
    /// the first trading day on or after it. Finding a day is one read.
    slots: Vec<u32>,
}

impl Calendar {
    /// The calendar holding exactly these days, given in any order.
    pub fn new(days: impl IntoIterator<Item = Day>) -> Result<Calendar, String> {
        let mut days: Vec<Day> = days.into_iter().collect();
        days.sort_unstable();
        days.dedup();
        let (Some(first), Some(last)) = (days.first().copied(), days.last().copied()) else {
            return Err("a calendar needs at least one trading day".into());
        };
        let mut slots = Vec::with_capacity((last.number() - first.number() + 1) as usize);
        for (position, day) in days.iter().enumerate() {
            // Every date since the trading day before, up to this one, leads here.
            slots.resize((day.number() - first.number() + 1) as usize, position as u32);
        }
        Ok(Calendar { days, slots })
    }

    /// The calendar of the days on which any of these reference tickers has
    /// a bar. One broad ticker is enough for the years it covers. A second,
    /// older one carries the calendar back before the first was listed.
    pub fn from_bars(references: &[&[Bar]]) -> Result<Calendar, String> {
        Calendar::new(references.iter().flat_map(|bars| bars.iter().map(|bar| bar.day)))
    }

    /// How many trading days there are. Never zero.
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.days.len()
    }

    pub fn days(&self) -> &[Day] {
        &self.days
    }

    pub fn first(&self) -> Day {
        self.days[0]
    }

    pub fn last(&self) -> Day {
        self.days[self.days.len() - 1]
    }

    /// The trading day at `position`. Positions run from zero, oldest first.
    pub fn day(&self, position: usize) -> Day {
        self.days[position]
    }

    /// Where `day` sits, when it is a trading day.
    pub fn position(&self, day: Day) -> Option<usize> {
        self.on_or_after(day).filter(|&position| self.days[position] == day)
    }

    /// The first trading day on or after `day`: where a start date lands.
    /// Nothing when `day` is past the end of the calendar.
    pub fn on_or_after(&self, day: Day) -> Option<usize> {
        let offset = i64::from(day.number()) - i64::from(self.first().number());
        if offset < 0 {
            return Some(0);
        }
        self.slots.get(offset as usize).map(|&position| position as usize)
    }

    /// The last trading day on or before `day`: where an end date lands.
    /// Nothing when `day` is before the calendar starts.
    pub fn on_or_before(&self, day: Day) -> Option<usize> {
        match self.on_or_after(day) {
            Some(position) if self.days[position] == day => Some(position),
            Some(position) => position.checked_sub(1),
            None => Some(self.days.len() - 1),
        }
    }

    /// Lay a ticker's series on this calendar.
    pub fn align(&self, series: &Series) -> Aligned {
        let mut aligned = Aligned { start: 0, prices: Vec::new(), filled: Vec::new() };
        for (index, day) in series.days().iter().enumerate() {
            // A bar on a day the calendar does not have is left out.
            let Some(position) = self.position(*day) else { continue };
            let price = series.prices()[index];
            match aligned.prices.last().copied() {
                None => aligned.start = position,
                Some(before) => {
                    for missing in aligned.start + aligned.prices.len()..position {
                        aligned.filled.push(missing);
                        aligned.prices.push(before);
                    }
                }
            }
            aligned.prices.push(price);
        }
        aligned
    }
}

/// One ticker's adjusted prices on a calendar: a price for every trading day
/// from its first bar to its last.
#[derive(Debug, Clone, PartialEq)]
pub struct Aligned {
    start: usize,
    prices: Vec<f64>,
    filled: Vec<usize>,
}

impl Aligned {
    /// The calendar positions this ticker has a price on. Empty when none of
    /// its bars falls on a trading day.
    pub fn range(&self) -> Range<usize> {
        self.start..self.start + self.prices.len()
    }

    /// One price per trading day in `range`, oldest first.
    pub fn prices(&self) -> &[f64] {
        &self.prices
    }

    /// The price at a calendar position, when it is inside the ticker's life.
    pub fn at(&self, position: usize) -> Option<f64> {
        self.prices.get(position.checked_sub(self.start)?).copied()
    }

    /// The prices over a stretch of calendar positions, when the ticker has
    /// one on every day of it.
    pub fn over(&self, positions: Range<usize>) -> Option<&[f64]> {
        self.prices.get(positions.start.checked_sub(self.start)?..positions.end.checked_sub(self.start)?)
    }

    /// The positions that had no bar and repeat the close before them, oldest first.
    pub fn filled(&self) -> &[usize] {
        &self.filled
    }

    /// Whether the price at `position` is a repeat of the close before it.
    pub fn is_filled(&self, position: usize) -> bool {
        self.filled.binary_search(&position).is_ok()
    }
}

/// The calendar positions on which every one of these tickers has a price:
/// from the first day of the youngest to the last day of the one that ends
/// first. Empty when their lives do not overlap, or when no ticker is given.
pub fn common(tickers: &[&Aligned]) -> Range<usize> {
    let start = tickers.iter().map(|ticker| ticker.range().start).max().unwrap_or(0);
    let end = tickers.iter().map(|ticker| ticker.range().end).min().unwrap_or(0);
    start..end.max(start)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adjust::{adjust, fixtures::*};

    fn calendar() -> Calendar {
        Calendar::from_bars(&[&bars(INDEX_FUND)]).unwrap()
    }

    fn has(calendar: &Calendar, text: &str) -> bool {
        calendar.position(day(text)).is_some()
    }

    #[test]
    fn the_calendar_is_the_days_the_reference_traded() {
        let calendar = calendar();
        assert_eq!(calendar.len(), 3_714);
        assert_eq!((calendar.first(), calendar.last()), (day("2012-01-03"), day("2026-10-09")));
        // 1970-01-01 was a Thursday, so day numbers 2 and 3 past a multiple of seven are a weekend.
        assert!(calendar.days().iter().all(|day| !matches!(day.number().rem_euclid(7), 2 | 3)), "a weekend day is in the calendar");
        for (position, day) in calendar.days().iter().enumerate() {
            assert_eq!(calendar.position(*day), Some(position));
            assert_eq!(calendar.day(position), *day);
        }
    }

    #[test]
    fn closures_nobody_planned_are_not_trading_days() {
        let calendar = calendar();
        // A storm shut the exchange for two days, and it opened again on the third.
        assert!(!has(&calendar, "2012-10-29") && !has(&calendar, "2012-10-30") && has(&calendar, "2012-10-31"));
        // Two days of mourning.
        assert!(!has(&calendar, "2018-12-05") && !has(&calendar, "2025-01-09"));
    }

    #[test]
    fn a_holiday_added_later_is_a_closure_only_from_then_on() {
        let calendar = calendar();
        // The Friday before the first one was an ordinary day.
        assert!(has(&calendar, "2021-06-18"));
        assert!(!has(&calendar, "2022-06-20") && !has(&calendar, "2025-06-19"));
    }

    #[test]
    fn half_days_are_ordinary_trading_days() {
        let calendar = calendar();
        for half_day in ["2025-07-03", "2025-11-28", "2025-12-24"] {
            assert!(has(&calendar, half_day), "{half_day}");
        }
        for closed in ["2025-07-04", "2025-11-27", "2025-12-25", "2026-04-03"] {
            assert!(!has(&calendar, closed), "{closed}");
        }
    }

    #[test]
    fn a_start_on_a_closed_day_moves_to_the_next_trading_day() {
        let calendar = calendar();
        let lands = |text: &str| calendar.on_or_after(day(text)).map(|position| calendar.day(position).to_string());
        // A holiday on a Friday, then a Sunday.
        assert_eq!(lands("2025-07-04").as_deref(), Some("2025-07-07"));
        assert_eq!(lands("2025-01-12").as_deref(), Some("2025-01-13"));
        assert_eq!(lands("2025-07-03").as_deref(), Some("2025-07-03"));
        // Before the calendar starts there is still a first day. Past its end there is nothing.
        assert_eq!(lands("1999-12-31").as_deref(), Some("2012-01-03"));
        assert_eq!(lands("2026-10-10"), None);
    }

    #[test]
    fn an_end_on_a_closed_day_moves_back_to_the_last_trading_day() {
        let calendar = calendar();
        let lands = |text: &str| calendar.on_or_before(day(text)).map(|position| calendar.day(position).to_string());
        assert_eq!(lands("2025-07-06").as_deref(), Some("2025-07-03"));
        assert_eq!(lands("2025-07-07").as_deref(), Some("2025-07-07"));
        assert_eq!(lands("2030-01-01").as_deref(), Some("2026-10-09"));
        assert_eq!(lands("2012-01-03").as_deref(), Some("2012-01-03"));
        assert_eq!(lands("2012-01-02"), None);
    }

    #[test]
    fn several_references_make_one_calendar() {
        let old = [Bar::new(day("2011-12-29"), 1.0), Bar::new(day("2011-12-30"), 1.0), Bar::new(day("2012-01-03"), 1.0)];
        let joined = Calendar::from_bars(&[&bars(INDEX_FUND), &old]).unwrap();
        assert_eq!(joined.len(), 3_716);
        assert_eq!(joined.first(), day("2011-12-29"));
        assert_eq!(joined.position(day("2012-01-03")), Some(2));
        // New Year's Day fell on a Sunday and was kept on the Monday.
        assert_eq!(joined.on_or_after(day("2011-12-31")), Some(2));
        assert!(Calendar::from_bars(&[]).unwrap_err().contains("at least one trading day"));
    }

    #[test]
    fn a_ticker_has_a_price_on_every_trading_day_of_its_own_life() {
        let calendar = calendar();
        for text in [DIVIDEND_PAYER, SPLIT_STOCK, BOND_FUND, NEW_LISTING, THIN_FUND] {
            let series = adjust(&bars(text)).unwrap();
            let aligned = calendar.align(&series);
            // None of them lacks a day the reference has, or has a day it lacks.
            assert_eq!(aligned.prices(), series.prices());
            assert!(aligned.filled().is_empty());
            assert_eq!(calendar.day(aligned.range().start), series.first_day());
            assert_eq!(calendar.day(aligned.range().end - 1), series.last_day());
        }
    }

    #[test]
    fn a_run_over_several_tickers_starts_when_the_youngest_does() {
        let calendar = calendar();
        let align = |text: &str| calendar.align(&adjust(&bars(text)).unwrap());
        let (old, young, gone) = (align(DIVIDEND_PAYER), align(NEW_LISTING), align(THIN_FUND));

        let days = common(&[&old, &young]);
        assert_eq!(calendar.day(days.start), day("2025-06-05"));
        assert_eq!(calendar.day(days.end - 1), day("2026-10-09"));
        assert_eq!(days.len(), 339);
        // Both have a price on each of those days, and the old one on earlier days too.
        assert_eq!(young.over(days.clone()).unwrap(), young.prices());
        assert_eq!(old.over(days.clone()).unwrap().len(), 339);
        assert_eq!(old.at(days.start), old.over(days.clone()).map(|prices| prices[0]));
        assert!(old.at(days.start - 1).is_some() && young.at(days.start - 1).is_none());
        assert!(young.over(days.start - 1..days.end).is_none());

        // The thin fund's bars here stop in 2015, long before the listing: no day in common.
        assert!(common(&[&old, &young, &gone]).is_empty());
        assert_eq!(common(&[&old, &gone]).len(), 252);
        assert!(common(&[]).is_empty());
    }

    fn series(days: &[&str]) -> Series {
        let bars: Vec<Bar> = days.iter().enumerate().map(|(index, text)| Bar::new(day(text), 10.0 + index as f64)).collect();
        adjust(&bars).unwrap()
    }

    #[test]
    fn a_trading_day_with_no_bar_repeats_the_close_before_and_says_so() {
        let calendar = calendar();
        // Bars on a Thursday, the Monday after and the Thursday after that:
        // the Friday, the Tuesday and the Wednesday have none.
        let aligned = calendar.align(&series(&["2026-10-01", "2026-10-05", "2026-10-08"]));
        assert_eq!(aligned.prices(), [10.0, 10.0, 11.0, 11.0, 11.0, 12.0]);
        let start = calendar.position(day("2026-10-01")).unwrap();
        assert_eq!(aligned.range(), start..start + 6);
        assert_eq!(aligned.filled(), [start + 1, start + 3, start + 4]);
        assert!(aligned.is_filled(start + 1) && !aligned.is_filled(start + 2));
        // The weekend is no trading day, so nothing is filled for it.
        assert_eq!(aligned.at(start + 2), Some(11.0));
        // Outside its life there is no price, filled or not.
        assert_eq!(aligned.at(start - 1), None);
        assert_eq!(aligned.at(start + 6), None);
    }

    #[test]
    fn a_bar_on_a_day_the_calendar_lacks_is_left_out() {
        let calendar = calendar();
        // A Saturday bar, then two bars before the calendar starts.
        let weekend = calendar.align(&series(&["2026-10-02", "2026-10-03", "2026-10-05"]));
        assert_eq!(weekend.prices(), [10.0, 12.0]);
        assert!(weekend.filled().is_empty());
        let early = calendar.align(&series(&["2011-12-29", "2011-12-30", "2012-01-03", "2012-01-04"]));
        assert_eq!((early.range(), early.prices()), (0..2, &[12.0, 13.0][..]));
        // A ticker with no bar on any trading day has no price at all.
        let never = calendar.align(&series(&["2011-12-29", "2011-12-30"]));
        assert!(never.range().is_empty() && never.at(0).is_none());
        assert!(common(&[&never, &weekend]).is_empty());
    }
}
