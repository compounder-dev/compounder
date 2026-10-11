//! Where histories come from, and keeping the store in step with them.
//!
//! A source hands over a ticker's whole history each time it is asked.
//! `refresh` sets that beside what the store holds and says what happened,
//! because what happened decides what a reader may keep:
//!
//! - Nothing new, or only new days at the end with no dividend and no split
//!   among them. Every adjusted price read before still stands.
//! - A new dividend or split, or a past day the source now tells differently:
//!   a payout it had missed, a split it recorded late, a close it corrected.
//!   Earlier adjusted prices are no longer right. The series has to be
//!   rebuilt, and whatever was worked out from the old one thrown away.
//!
//! The store keeps raw bars and the adjusted series is worked out from them
//! each time, so a rebuild needs no path of its own: the next `adjust` is the
//! rebuild. After a refresh the store holds exactly what the source said.
//! The one thing a refresh will not do is lose days. An answer that stops
//! short of the stored history, or starts after it, is refused and the stored
//! history stays.

use std::fmt;

use crate::{
    adjust::{validate, Bar},
    store::{check_ticker, Store},
};

/// Somewhere daily histories come from.
pub trait Source {
    /// Every bar the source has for `ticker`, oldest day first, with closes
    /// as printed on the day.
    fn fetch(&mut self, ticker: &str) -> Result<Vec<Bar>, String>;
}

/// What a refresh did to one ticker.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The ticker was not in the store. Its whole history is now.
    New { days: usize },
    /// The source says nothing the store does not hold.
    Unchanged,
    /// Days were added at the end, and no price before them moved.
    Appended { days: usize },
    /// Earlier adjusted prices no longer stand, for the reason given.
    Rebuilt { why: String },
}

impl fmt::Display for Outcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let days = |count: &usize| if *count == 1 { "1 day".to_string() } else { format!("{count} days") };
        match self {
            Outcome::New { days: count } => write!(formatter, "new, {}", days(count)),
            Outcome::Unchanged => write!(formatter, "unchanged"),
            Outcome::Appended { days: count } => write!(formatter, "{} added", days(count)),
            Outcome::Rebuilt { why } => write!(formatter, "rebuilt: {why}"),
        }
    }
}

/// A split ratio in words: `2-for-1`, `1-for-5`, `3-for-2`.
fn split_in_words(ratio: f64) -> String {
    (1..=1_000)
        .find_map(|old| {
            let new = ratio * f64::from(old);
            ((new - new.round()).abs() < 1e-9 && new >= 1.0).then(|| format!("{}-for-{old}", new.round()))
        })
        .unwrap_or_else(|| format!("{ratio}-for-1"))
}

/// The first day on which the source's telling of the stored days differs
/// from the store's, and how. `past` is the source's bars up to the last
/// stored day.
fn first_difference(stored: &[Bar], past: &[Bar]) -> Option<String> {
    let (mut old_index, mut new_index) = (0, 0);
    loop {
        let (old, new) = match (stored.get(old_index), past.get(new_index)) {
            (None, None) => return None,
            (Some(old), Some(new)) if old.day == new.day => (old, new),
            (Some(old), Some(new)) if old.day > new.day => return Some(format!("{} is a day in its history now and was not before", new.day)),
            (None, Some(new)) => return Some(format!("{} is a day in its history now and was not before", new.day)),
            (Some(old), _) => return Some(format!("{} is no longer a day in its history", old.day)),
        };
        let day = old.day;
        if old.split != new.split {
            return Some(match (old.split == 1.0, new.split == 1.0) {
                (true, _) => format!("a {} split appeared on {day}", split_in_words(new.split)),
                (_, true) => format!("the split on {day} is gone"),
                _ => format!("the split on {day} changed from {} to {}", split_in_words(old.split), split_in_words(new.split)),
            });
        }
        if old.dividend != new.dividend {
            return Some(match (old.dividend == 0.0, new.dividend == 0.0) {
                (true, _) => format!("a dividend of {} appeared on {day}", new.dividend),
                (_, true) => format!("the dividend on {day} is gone"),
                _ => format!("the dividend on {day} changed from {} to {}", old.dividend, new.dividend),
            });
        }
        if old.close != new.close {
            return Some(format!("the close on {day} changed from {} to {}", old.close, new.close));
        }
        old_index += 1;
        new_index += 1;
    }
}

/// What bringing `stored` in step with `fetched` amounts to.
fn compare(stored: &[Bar], fetched: &[Bar]) -> Result<Outcome, String> {
    let (first, last) = (stored[0].day, stored[stored.len() - 1].day);
    let (fetched_first, fetched_last) = (fetched[0].day, fetched[fetched.len() - 1].day);
    if fetched_last < last {
        return Err(format!("the source's history ends on {fetched_last}, before the stored one does on {last}. The stored history is kept"));
    }
    if fetched_first > first {
        return Err(format!("the source's history starts on {fetched_first}, after the stored one does on {first}. The stored history is kept"));
    }
    let (past, fresh) = fetched.split_at(fetched.partition_point(|bar| bar.day <= last));

    let mut reasons = Vec::new();
    if let Some(bar) = fresh.iter().find(|bar| bar.split != 1.0) {
        reasons.push(format!("a {} split on {} moves every earlier price", split_in_words(bar.split), bar.day));
    }
    if let Some(bar) = fresh.iter().find(|bar| bar.dividend != 0.0) {
        reasons.push(format!("a dividend of {} on {} moves every earlier price", bar.dividend, bar.day));
    }
    if let Some(difference) = first_difference(stored, past) {
        reasons.push(format!("the source tells the past differently: {difference}"));
    }
    Ok(match (reasons.is_empty(), fresh.len()) {
        (false, _) => Outcome::Rebuilt { why: reasons.join(", and ") },
        (true, 0) => Outcome::Unchanged,
        (true, days) => Outcome::Appended { days },
    })
}

fn refresh_one(store: &mut Store, source: &mut dyn Source, ticker: &str) -> Result<Outcome, String> {
    check_ticker(ticker)?;
    // A stored history that cannot be read is found out before the source is troubled.
    let stored = store.find(ticker)?;
    let fetched = source.fetch(ticker)?;
    validate(&fetched).map_err(|reason| format!("the source's history makes no sense: {reason}"))?;
    let outcome = match &stored {
        None => Outcome::New { days: fetched.len() },
        Some(stored) => compare(stored, &fetched)?,
    };
    if outcome != Outcome::Unchanged {
        store.save(ticker, &fetched)?;
    }
    Ok(outcome)
}

/// Bring each ticker in the store up to date with the source, and say what
/// happened to it. A ticker that fails keeps its stored history, and does
/// not stop the ones after it.
pub fn refresh(store: &mut Store, source: &mut dyn Source, tickers: &[&str]) -> Vec<(String, Result<Outcome, String>)> {
    tickers.iter().map(|ticker| (ticker.to_string(), refresh_one(store, source, ticker))).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adjust::{adjust, fixtures::*};
    use crate::store::scratch::Scratch;
    use std::collections::BTreeMap;

    /// A source that answers from memory, and remembers what it was asked.
    #[derive(Default)]
    struct Canned {
        histories: BTreeMap<&'static str, Vec<Bar>>,
        asked: Vec<String>,
    }

    impl Canned {
        fn holding(ticker: &'static str, bars: &[Bar]) -> Canned {
            let mut source = Canned::default();
            source.histories.insert(ticker, bars.to_vec());
            source
        }
    }

    impl Source for Canned {
        fn fetch(&mut self, ticker: &str) -> Result<Vec<Bar>, String> {
            self.asked.push(ticker.to_string());
            self.histories.get(ticker).cloned().ok_or_else(|| format!("the source has no {ticker}"))
        }
    }

    /// Store `before`, then refresh against a source that says `after`.
    fn refreshed(name: &str, before: &[Bar], after: &[Bar]) -> (Result<Outcome, String>, Vec<Bar>) {
        let scratch = Scratch::new(name);
        let mut store = scratch.store();
        store.save("ABC", before).unwrap();
        let outcome = refresh(&mut store, &mut Canned::holding("ABC", after), &["ABC"]).remove(0).1;
        (outcome, store.load("ABC").unwrap())
    }

    fn why(outcome: Result<Outcome, String>) -> String {
        match outcome {
            Ok(Outcome::Rebuilt { why }) => why,
            other => panic!("expected a rebuild, got {other:?}"),
        }
    }

    fn position(bars: &[Bar], text: &str) -> usize {
        bars.iter().position(|bar| bar.day == day(text)).unwrap()
    }

    #[test]
    fn a_ticker_seen_for_the_first_time_is_stored_whole() {
        let scratch = Scratch::new("refresh-new");
        let mut store = scratch.store();
        let bars = bars(DIVIDEND_PAYER);
        let mut source = Canned::holding("KO", &bars);
        let report = refresh(&mut store, &mut source, &["KO"]);
        assert_eq!(report, [("KO".to_string(), Ok(Outcome::New { days: 3_714 }))]);
        assert_eq!(store.load("KO").unwrap(), bars);
        assert_eq!(store.series("KO").unwrap(), adjust(&bars).unwrap());
        // Asked again with nothing new, nothing happens.
        assert_eq!(refresh(&mut store, &mut source, &["KO"])[0].1, Ok(Outcome::Unchanged));
        assert_eq!(source.asked, ["KO", "KO"]);
    }

    #[test]
    fn new_days_are_added_and_earlier_prices_stand() {
        // The last five days of the index fund carry no dividend.
        let bars = bars(INDEX_FUND);
        let cut = bars.len() - 5;
        let (outcome, stored) = refreshed("refresh-appended", &bars[..cut], &bars);
        assert_eq!(outcome, Ok(Outcome::Appended { days: 5 }));
        assert_eq!(stored, bars);
        let (before, after) = (adjust(&bars[..cut]).unwrap(), adjust(&bars).unwrap());
        assert_eq!(before.prices(), &after.prices()[..cut]);
    }

    #[test]
    fn a_new_dividend_rebuilds_the_series() {
        let bars = bars(DIVIDEND_PAYER);
        let cut = position(&bars, "2026-09-15");
        let (outcome, stored) = refreshed("refresh-dividend", &bars[..cut], &bars);
        assert_eq!(why(outcome), "a dividend of 0.53 on 2026-09-15 moves every earlier price");
        assert_eq!(stored, bars);
        // The day before the ex-date was at its real close. Now it is 53 cents lower, and so on back.
        let (before, after) = (adjust(&bars[..cut]).unwrap(), adjust(&bars).unwrap());
        assert_eq!((before.prices()[cut - 1], after.prices()[cut - 1]), (89.35, 88.82));
        assert!((0..cut).all(|index| after.prices()[index] < before.prices()[index]));
    }

    #[test]
    fn a_new_split_rebuilds_the_series() {
        let bars = bars(SPLIT_STOCK);
        let cut = position(&bars, "2025-11-17");
        let (outcome, stored) = refreshed("refresh-split", &bars[..cut], &bars);
        assert_eq!(why(outcome), "a 10-for-1 split on 2025-11-17 moves every earlier price");
        assert_eq!(stored, bars);
        let (before, after) = (adjust(&bars[..cut]).unwrap(), adjust(&bars).unwrap());
        assert_eq!((before.prices()[cut - 1], after.prices()[cut - 1]), (1_112.17, 111.217));
    }

    #[test]
    fn a_past_dividend_the_source_now_tells_differently_rebuilds_the_series() {
        let bars = bars(DIVIDEND_PAYER);
        let ex_date = position(&bars, "2023-11-30");
        assert_eq!(bars[ex_date].dividend, 0.46);

        // The store missed a payout that the source now has.
        let mut missed = bars.clone();
        missed[ex_date].dividend = 0.0;
        let (outcome, stored) = refreshed("refresh-dividend-appeared", &missed, &bars);
        assert_eq!(why(outcome), "the source tells the past differently: a dividend of 0.46 appeared on 2023-11-30");
        assert_eq!(stored, bars);
        assert_ne!(adjust(&missed).unwrap().prices()[..ex_date], adjust(&stored).unwrap().prices()[..ex_date]);

        // The source corrected an amount, and withdrew one.
        let mut wrong = bars.clone();
        wrong[ex_date].dividend = 0.64;
        let (outcome, stored) = refreshed("refresh-dividend-changed", &wrong, &bars);
        assert_eq!(why(outcome), "the source tells the past differently: the dividend on 2023-11-30 changed from 0.64 to 0.46");
        assert_eq!(stored, bars);
        let (outcome, stored) = refreshed("refresh-dividend-gone", &bars, &missed);
        assert_eq!(why(outcome), "the source tells the past differently: the dividend on 2023-11-30 is gone");
        assert_eq!(stored, missed);
    }

    #[test]
    fn a_past_split_the_source_now_tells_differently_rebuilds_the_series() {
        let bars = bars(SPLIT_STOCK);
        let split_day = position(&bars, "2015-07-15");
        assert_eq!(bars[split_day].split, 7.0);

        // The source recorded the split late: the store has the fall in the close and no split to explain it.
        let mut late = bars.clone();
        late[split_day].split = 1.0;
        let (outcome, stored) = refreshed("refresh-split-appeared", &late, &bars);
        assert_eq!(why(outcome), "the source tells the past differently: a 7-for-1 split appeared on 2015-07-15");
        assert_eq!(stored, bars);

        let mut wrong = bars.clone();
        wrong[split_day].split = 0.7;
        let (outcome, _) = refreshed("refresh-split-changed", &wrong, &bars);
        assert_eq!(why(outcome), "the source tells the past differently: the split on 2015-07-15 changed from 7-for-10 to 7-for-1");
        let (outcome, _) = refreshed("refresh-split-gone", &bars, &late);
        assert_eq!(why(outcome), "the source tells the past differently: the split on 2015-07-15 is gone");
    }

    #[test]
    fn a_past_close_or_day_the_source_now_tells_differently_rebuilds_the_series() {
        let bars = bars(NEW_LISTING);
        let mut corrected = bars.clone();
        corrected[10].close = 240.29;
        let (outcome, stored) = refreshed("refresh-close", &bars, &corrected);
        assert_eq!(why(outcome), "the source tells the past differently: the close on 2025-06-20 changed from 240.28 to 240.29");
        assert_eq!(stored, corrected);

        let mut without = bars.clone();
        without.remove(10);
        let (outcome, stored) = refreshed("refresh-day-gone", &bars, &without);
        assert_eq!(why(outcome), "the source tells the past differently: 2025-06-20 is no longer a day in its history");
        assert_eq!(stored, without);
        let (outcome, _) = refreshed("refresh-day-appeared", &without, &bars);
        assert_eq!(why(outcome), "the source tells the past differently: 2025-06-20 is a day in its history now and was not before");
        // A day before the stored history began counts too.
        let (outcome, _) = refreshed("refresh-earlier-start", &bars[1..], &bars);
        assert_eq!(why(outcome), "the source tells the past differently: 2025-06-05 is a day in its history now and was not before");
    }

    #[test]
    fn a_new_event_and_a_changed_past_are_both_named() {
        let bars = bars(SPLIT_STOCK);
        let cut = position(&bars, "2025-11-17");
        let mut before = bars[..cut].to_vec();
        before[0].close = 72.25;
        let (outcome, stored) = refreshed("refresh-both", &before, &bars);
        assert_eq!(
            why(outcome),
            "a 10-for-1 split on 2025-11-17 moves every earlier price, and the source tells the past differently: the close on 2012-01-03 changed from 72.25 to 72.24"
        );
        assert_eq!(stored, bars);
    }

    #[test]
    fn an_answer_that_would_lose_days_is_refused() {
        let bars = bars(NEW_LISTING);
        // The source stops short of what is stored.
        let (outcome, stored) = refreshed("refresh-short", &bars, &bars[..300]);
        assert_eq!(outcome.unwrap_err(), "the source's history ends on 2026-08-14, before the stored one does on 2026-10-09. The stored history is kept");
        assert_eq!(stored, bars);
        // The source has lost the early days.
        let (outcome, stored) = refreshed("refresh-late", &bars, &bars[30..]);
        assert_eq!(outcome.unwrap_err(), "the source's history starts on 2025-07-21, after the stored one does on 2025-06-05. The stored history is kept");
        assert_eq!(stored, bars);
    }

    #[test]
    fn an_answer_that_makes_no_sense_is_refused() {
        let bars = bars(NEW_LISTING);
        let (outcome, stored) = refreshed("refresh-empty", &bars, &[]);
        assert_eq!(outcome.unwrap_err(), "the source's history makes no sense: there are no days");
        assert_eq!(stored, bars);
        let mut broken = bars.clone();
        broken.swap(3, 4);
        let (outcome, stored) = refreshed("refresh-broken", &bars, &broken);
        assert!(outcome.unwrap_err().contains("the days are out of order"));
        assert_eq!(stored, bars);
    }

    #[test]
    fn one_ticker_failing_does_not_stop_the_rest() {
        let scratch = Scratch::new("refresh-many");
        let mut store = scratch.store();
        let mut source = Canned::holding("SPY", &bars(INDEX_FUND));
        source.histories.insert("CRCL", bars(NEW_LISTING));
        let report = refresh(&mut store, &mut source, &["SPY", "GONE", "CRCL"]);
        assert_eq!(report[0], ("SPY".to_string(), Ok(Outcome::New { days: 3_714 })));
        assert_eq!(report[1], ("GONE".to_string(), Err("the source has no GONE".to_string())));
        assert_eq!(report[2], ("CRCL".to_string(), Ok(Outcome::New { days: 339 })));
        assert_eq!(store.tickers().collect::<Vec<_>>(), ["CRCL", "SPY"]);
        // Something that is not a ticker is never put to the source at all.
        let report = refresh(&mut store, &mut source, &["../SPY"]);
        assert_eq!(report[0].1, Err("\"../SPY\" is not a ticker".to_string()));
        // Nor is a ticker whose stored history cannot be read. It is left for a person to mend.
        std::fs::write(scratch.dir.join("CRCL.csv"), "day,close,dividend,split\n2025-06-05,eighty,,\n").unwrap();
        let report = refresh(&mut store, &mut source, &["CRCL"]);
        assert!(report[0].1.as_ref().unwrap_err().contains("line 2: the close is not a number"));
        assert_eq!(source.asked, ["SPY", "GONE", "CRCL"]);
    }

    #[test]
    fn an_outcome_reads_as_a_plain_line() {
        assert_eq!(Outcome::New { days: 3_714 }.to_string(), "new, 3714 days");
        assert_eq!(Outcome::Unchanged.to_string(), "unchanged");
        assert_eq!(Outcome::Appended { days: 1 }.to_string(), "1 day added");
        assert_eq!(Outcome::Appended { days: 3 }.to_string(), "3 days added");
        assert_eq!(Outcome::Rebuilt { why: "a 2-for-1 split on 2012-08-13".into() }.to_string(), "rebuilt: a 2-for-1 split on 2012-08-13");
        for (ratio, words) in [(2.0, "2-for-1"), (0.2, "1-for-5"), (1.5, "3-for-2"), (0.125, "1-for-8"), (1.281, "1281-for-1000"), (1.23456, "1.23456-for-1")] {
            assert_eq!(split_in_words(ratio), words);
        }
    }
}
