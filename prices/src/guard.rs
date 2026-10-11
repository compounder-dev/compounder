//! Checks for the ways daily price data goes wrong.
//!
//! Each check looks for one fault that real data has shown, and answers with
//! a warning in plain words or with nothing. None of them stops anything or
//! changes a bar. What a warning is worth is for the caller to decide: a
//! backtest might start a day later, or tell its user, or leave a ticker out.
//! `check` runs every check that looks at one ticker.

use crate::{adjust::Bar, calendar::Calendar, day::Day};

/// A first bar at a whole number of dollars is doubted when the next close is
/// this far from it, as a share of it.
const JUMP_FROM_ROUND_PRICE: f64 = 0.10;
/// Any first bar is doubted when the next close is this far from it.
const JUMP_FROM_ANY_PRICE: f64 = 0.50;
/// This many trading days in a row with no new close is a stretch worth a warning.
const STALE_DAYS: usize = 5;
/// A move this small, as a share of the price, is how a fund that holds cash moves.
const QUIET_MOVE: f64 = 0.001;
/// A history that stops more than this many trading days before the calendar does has ended.
const DAYS_BEHIND: usize = 5;
/// The longest the exchange has been shut since 1990, from one trading day to
/// the next: a Monday to the Monday after, in September 2001.
const LONGEST_CLOSURE: i32 = 7;

fn count(number: usize, one: &str, many: &str) -> String {
    format!("{number} {}", if number == 1 { one } else { many })
}

/// A first bar followed by a jump nobody could have had.
///
/// Some new listings come with one bar too many: dated the day before
/// trading began, at the price the shares were offered at. The first real
/// close then reads as a one-day gain, 168% in the worst case seen, and a
/// backtest that starts there books it. An offer price is a whole number of
/// dollars and a real close almost never is, so a first bar at a whole number
/// with the next close a tenth or more away is called out. The size of the
/// move alone is no test below a half: real listings have moved 22% and 29%
/// on their second day.
pub fn first_bar_jump(bars: &[Bar]) -> Option<String> {
    let (first, second) = (bars.first()?, bars.get(1)?);
    // The second close, counted in the first day's shares.
    let change = second.close * second.split / first.close - 1.0;
    let round = first.close.fract() == 0.0;
    let doubted = change.abs() >= JUMP_FROM_ANY_PRICE || (round && change.abs() >= JUMP_FROM_ROUND_PRICE);
    doubted.then(|| {
        let price = if round { format!("a round {} dollars", first.close) } else { first.close.to_string() };
        format!("the first bar, {} at {price}, may not be a real trading day: the next close is {:+.1}% away", first.day, change * 100.0)
    })
}

/// Long stretches with no new close.
///
/// A fund that few people trade goes days without a trade. Its close then
/// repeats, or the day has no bar at all, and when a trade does come the
/// price jumps by everything that happened in between. A rule that buys dips
/// looks brilliant on such a series. Five trading days in a row with no new
/// close is called out. Of 41 tickers looked at, from 1990 on, the only ones
/// with a stretch that long are three thin funds and two funds that hold cash.
///
/// A fund that holds cash is no fault. When rates are near nothing its close
/// repeats for weeks, and the move that ends the wait is a cent. So a stretch
/// counts only when the price moved by more than a tenth of a percent going
/// into it or coming out of it.
pub fn stale_run(bars: &[Bar], calendar: &Calendar) -> Option<String> {
    let mut stretches = 0;
    let (mut longest, mut longest_after) = (0, bars.first()?.day);
    // The stretch in hand: how many days it has run, the day of the last new
    // close before it, and how far that close moved from the one before.
    let (mut running, mut after, mut move_in) = (0, longest_after, 0.0);
    let mut end_stretch = |running: usize, after: Day, largest_move: f64| {
        if running >= STALE_DAYS && largest_move > QUIET_MOVE {
            stretches += 1;
            if running > longest {
                (longest, longest_after) = (running, after);
            }
        }
    };
    for pair in bars.windows(2) {
        let (before, bar) = (&pair[0], &pair[1]);
        // Trading days between the two bars had no bar at all.
        if let (Some(from), Some(to)) = (calendar.position(before.day), calendar.position(bar.day)) {
            running += to - from - 1;
        }
        if bar.close == before.close && bar.split == 1.0 {
            running += 1;
            continue;
        }
        let move_out = (bar.close * bar.split / before.close - 1.0).abs();
        end_stretch(running, after, f64::max(move_in, move_out));
        (running, after, move_in) = (0, bar.day, move_out);
    }
    end_stretch(running, after, move_in);
    (stretches > 0).then(|| {
        let found = count(stretches, "stretch", "stretches");
        format!("{found} of {STALE_DAYS} or more trading days with no new close, the longest {longest} days after {longest_after}: it may not trade every day")
    })
}

/// Bars on days the calendar does not have.
///
/// Inside the calendar's span, such a bar means one of two things: the
/// reference ticker is missing a day, so the calendar has a gap, or the bar
/// is not a real trading day. Outside the span it means the reference is
/// younger than this ticker, or has not been refreshed. Either way `align`
/// leaves those bars out.
pub fn off_calendar(bars: &[Bar], calendar: &Calendar) -> Option<String> {
    let outside = bars.iter().filter(|bar| bar.day < calendar.first() || bar.day > calendar.last()).count();
    let mut inside = bars.iter().filter(|bar| bar.day >= calendar.first() && bar.day <= calendar.last() && calendar.position(bar.day).is_none());
    let mut warnings = Vec::new();
    if let Some(first) = inside.next() {
        warnings.push(format!(
            "{} on a day the calendar lacks, the first on {}: the calendar may have a gap, or the day is not a real trading day",
            count(1 + inside.count(), "bar is", "bars are"),
            first.day
        ));
    }
    if outside > 0 {
        warnings.push(format!("{} outside the calendar, which runs from {} to {}", count(outside, "bar is", "bars are"), calendar.first(), calendar.last()));
    }
    (!warnings.is_empty()).then(|| warnings.join(". "))
}

/// A history that stopped long ago.
///
/// A ticker that was delisted, bought or renamed stops getting bars. Tests
/// run over only the tickers still alive flatter every strategy, so a dead
/// one is worth keeping, but a backtest has to know its prices end.
pub fn ends_early(bars: &[Bar], calendar: &Calendar) -> Option<String> {
    let last = bars.last()?.day;
    let behind = calendar.len() - calendar.on_or_before(last).map_or(0, |position| position + 1);
    (behind > DAYS_BEHIND).then(|| format!("the last bar is {last}, {behind} trading days before the calendar ends on {}: the ticker may no longer trade", calendar.last()))
}

/// A hole in the calendar itself.
///
/// The calendar is the days its reference ticker has bars, so days missing
/// from the reference are missing for everything. No closure since 1990 has
/// been longer than a week, and a longer one is called out. A hole of a day
/// or two looks like a holiday here. `off_calendar` finds those, from the
/// side of a ticker that does have the day.
pub fn calendar_gap(calendar: &Calendar) -> Option<String> {
    let mut gaps = calendar.days().windows(2).filter(|pair| pair[1].number() - pair[0].number() > LONGEST_CLOSURE);
    let first = gaps.next()?;
    Some(format!(
        "the calendar has {} of more than {LONGEST_CLOSURE} days between one trading day and the next, the first from {} to {}: its reference ticker may be missing days",
        count(1 + gaps.count(), "gap", "gaps"),
        first[0],
        first[1]
    ))
}

/// Every warning about one ticker's bars, read against the calendar.
pub fn check(bars: &[Bar], calendar: &Calendar) -> Vec<String> {
    [first_bar_jump(bars), stale_run(bars, calendar), off_calendar(bars, calendar), ends_early(bars, calendar)].into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adjust::fixtures::*;

    fn calendar() -> Calendar {
        Calendar::from_bars(&[&bars(INDEX_FUND)]).unwrap()
    }

    /// Bars on the calendar's days from `start` on, one per close.
    fn on_calendar(calendar: &Calendar, start: &str, closes: &[f64]) -> Vec<Bar> {
        let from = calendar.position(day(start)).unwrap();
        closes.iter().enumerate().map(|(index, close)| Bar::new(calendar.day(from + index), *close)).collect()
    }

    #[test]
    fn sound_histories_raise_no_warning() {
        let calendar = calendar();
        for text in [DIVIDEND_PAYER, SPLIT_STOCK, BOND_FUND, INDEX_FUND, NEW_LISTING] {
            assert_eq!(check(&bars(text), &calendar), Vec::<String>::new());
        }
        assert_eq!(calendar_gap(&calendar), None);
    }

    #[test]
    fn a_bar_at_the_offer_price_before_the_first_real_day_is_called_out() {
        // The listing's real first close was 83.23. The series we check
        // against has one more bar the day before, at the offer price of 31.
        let mut with_extra_bar = vec![Bar::new(day("2025-06-04"), 31.0)];
        with_extra_bar.extend(bars(NEW_LISTING));
        assert_eq!(
            first_bar_jump(&with_extra_bar).unwrap(),
            "the first bar, 2025-06-04 at a round 31 dollars, may not be a real trading day: the next close is +168.5% away"
        );
        // Three more seen the same way: offer price, then the first real close.
        for (offer, real) in [(250.0, 328.28), (51.0, 63.59), (34.0, 50.44)] {
            assert!(first_bar_jump(&[Bar::new(day("2025-01-02"), offer), Bar::new(day("2025-01-03"), real)]).is_some(), "{offer}");
        }
    }

    #[test]
    fn a_real_first_day_is_not_doubted_for_a_large_second_day() {
        // The same listing from its real first day: up 29% on day two, and real.
        assert_eq!(first_bar_jump(&bars(NEW_LISTING)), None);
        // A whole-dollar close followed by an ordinary day.
        let ordinary = [Bar::new(day("2025-01-02"), 40.0), Bar::new(day("2025-01-03"), 37.08)];
        assert_eq!(first_bar_jump(&ordinary), None);
        // A split on day two is no jump.
        let split = [Bar::new(day("2025-01-02"), 100.0), Bar { day: day("2025-01-03"), close: 10.1, dividend: 0.0, split: 10.0 }];
        assert_eq!(first_bar_jump(&split), None);
        assert_eq!(first_bar_jump(&ordinary[..1]), None);
        assert_eq!(first_bar_jump(&[]), None);
    }

    #[test]
    fn a_first_close_more_than_half_away_from_the_next_is_called_out_at_any_price() {
        let jump = [Bar::new(day("2025-01-02"), 10.37), Bar::new(day("2025-01-03"), 16.2)];
        assert_eq!(first_bar_jump(&jump).unwrap(), "the first bar, 2025-01-02 at 10.37, may not be a real trading day: the next close is +56.2% away");
    }

    #[test]
    fn a_thin_fund_is_called_out_for_its_days_without_a_trade() {
        let calendar = calendar();
        let thin = bars(THIN_FUND);
        assert_eq!(
            stale_run(&thin, &calendar).unwrap(),
            "3 stretches of 5 or more trading days with no new close, the longest 8 days after 2015-06-22: it may not trade every day"
        );
        assert_eq!(check(&thin, &calendar).len(), 2, "its bars also stop in 2015");
    }

    #[test]
    fn a_stretch_shorter_than_five_days_is_not_called_out() {
        let calendar = calendar();
        let mut closes = vec![20.0, 20.5, 20.5, 20.5, 20.5, 20.5, 21.0, 21.5, 22.0, 22.5, 23.0, 23.5];
        assert_eq!(stale_run(&on_calendar(&calendar, "2026-09-01", &closes), &calendar), None);
        closes.insert(2, 20.5);
        let warning = stale_run(&on_calendar(&calendar, "2026-09-01", &closes), &calendar).unwrap();
        assert!(warning.starts_with("1 stretch of 5 or more trading days with no new close, the longest 5 days after 2026-09-02"), "{warning}");
    }

    #[test]
    fn trading_days_with_no_bar_count_as_days_with_no_new_close() {
        let calendar = calendar();
        let mut halted = on_calendar(&calendar, "2026-09-01", &[20.0, 20.5, 21.0, 21.5, 22.0, 22.5, 23.0, 23.5, 24.0, 24.5, 25.0, 25.5]);
        // Four trading days in a row with no bar.
        halted.drain(2..6);
        assert_eq!(stale_run(&halted, &calendar), None);
        // And the close repeats once on the day the bars come back: five.
        halted[2].close = halted[1].close;
        let warning = stale_run(&halted, &calendar).unwrap();
        assert!(warning.starts_with("1 stretch of 5 or more trading days with no new close, the longest 5 days after 2026-09-02"), "{warning}");
    }

    #[test]
    fn a_stretch_that_has_not_ended_counts_too() {
        let calendar = calendar();
        let closes = [20.0, 20.5, 20.5, 20.5, 20.5, 20.5, 20.5];
        let warning = stale_run(&on_calendar(&calendar, "2026-09-01", &closes), &calendar).unwrap();
        assert!(warning.starts_with("1 stretch of 5 or more trading days with no new close, the longest 5 days after 2026-09-02"), "{warning}");
    }

    #[test]
    fn a_cash_fund_is_left_alone_however_long_its_close_repeats() {
        let calendar = calendar();
        // A cent up every eighth day, and flat in between: a hundredth of a percent at this price.
        let closes: Vec<f64> = (0..400).map(|index| 91.0 + (index / 8) as f64 * 0.01).collect();
        assert_eq!(stale_run(&on_calendar(&calendar, "2014-01-02", &closes), &calendar), None);
        // The same flat stretches in a fund that moves a dollar when it moves are called out, every one.
        let thin: Vec<f64> = (0..400).map(|index| 91.0 + (index / 8) as f64).collect();
        let warning = stale_run(&on_calendar(&calendar, "2014-01-02", &thin), &calendar).unwrap();
        assert!(warning.starts_with("50 stretches of 5 or more trading days with no new close, the longest 7 days after 2014-01-02"), "{warning}");
    }

    #[test]
    fn a_history_that_stopped_long_ago_is_called_out() {
        let calendar = calendar();
        let all = bars(DIVIDEND_PAYER);
        assert_eq!(ends_early(&all, &calendar), None);
        // A week behind is a refresh that has not run. More than that is a ticker that stopped.
        assert_eq!(ends_early(&all[..all.len() - 5], &calendar), None);
        assert_eq!(
            ends_early(&all[..all.len() - 6], &calendar).unwrap(),
            "the last bar is 2026-10-01, 6 trading days before the calendar ends on 2026-10-09: the ticker may no longer trade"
        );
        assert!(ends_early(&bars(THIN_FUND), &calendar).unwrap().starts_with("the last bar is 2015-12-31, 2708 trading days before"));
        // Bars that end before the calendar starts are behind by all of it.
        let ancient = [Bar::new(day("2001-09-10"), 10.0)];
        assert!(ends_early(&ancient, &calendar).unwrap().contains("3714 trading days before"));
    }

    #[test]
    fn bars_on_days_the_calendar_lacks_are_called_out() {
        let calendar = calendar();
        let mut odd = on_calendar(&calendar, "2026-09-28", &[10.0, 10.5, 11.0, 11.5, 12.0, 12.5, 13.0, 13.5, 14.0, 14.5]);
        assert_eq!(off_calendar(&odd, &calendar), None);
        // A Saturday.
        odd.insert(5, Bar::new(day("2026-10-03"), 12.2));
        assert_eq!(
            off_calendar(&odd, &calendar).unwrap(),
            "1 bar is on a day the calendar lacks, the first on 2026-10-03: the calendar may have a gap, or the day is not a real trading day"
        );
        // Two days from before the reference was listed, and one after its last bar.
        odd.insert(0, Bar::new(day("2011-12-30"), 9.5));
        odd.insert(0, Bar::new(day("2011-12-29"), 9.0));
        odd.push(Bar::new(day("2026-10-12"), 15.0));
        let warning = off_calendar(&odd, &calendar).unwrap();
        assert!(warning.starts_with("1 bar is on a day the calendar lacks, the first on 2026-10-03"), "{warning}");
        assert!(warning.ends_with(". 3 bars are outside the calendar, which runs from 2012-01-03 to 2026-10-09"), "{warning}");
    }

    #[test]
    fn a_calendar_with_a_hole_longer_than_any_closure_is_called_out() {
        let reference = bars(INDEX_FUND);
        // The longest real closure in these years is the storm of 2012: a Friday to the Wednesday after.
        assert_eq!(calendar_gap(&Calendar::from_bars(&[&reference]).unwrap()), None);
        let without = |from: &str, to: &str| reference.iter().filter(|bar| bar.day < day(from) || bar.day > day(to)).copied().collect::<Vec<Bar>>();
        // A reference that lost a week of bars.
        let holed = without("2020-03-09", "2020-03-13");
        assert_eq!(
            calendar_gap(&Calendar::from_bars(&[&holed]).unwrap()).unwrap(),
            "the calendar has 1 gap of more than 7 days between one trading day and the next, the first from 2020-03-06 to 2020-03-16: its reference ticker may be missing days"
        );
        // A hole of two days reads as a long weekend. It shows from the side of a ticker that has the days.
        let small = without("2020-03-10", "2020-03-11");
        let small_calendar = Calendar::from_bars(&[&small]).unwrap();
        assert_eq!(calendar_gap(&small_calendar), None);
        assert!(off_calendar(&bars(DIVIDEND_PAYER), &small_calendar).unwrap().starts_with("2 bars are on a day the calendar lacks, the first on 2020-03-10"));
    }
}
