//! A daily price feed on the web, as a source of histories.
//!
//! The feed answers one request with everything it has for a ticker: a list
//! of moments, one per daily bar, the close of each, and the dividends and
//! splits by date. Where it lives is for the caller to say. Nothing here
//! holds an address.
//!
//! The feed's closes are already divided by every later split, and so are
//! its dividends. Both are multiplied back here, to the close the tape
//! printed and the cash that was paid, because those never change and the
//! divided numbers change with every new split.
//!
//! Two things blur a number on its way through the feed, and both are
//! undone. The feed holds a close as a 32-bit float, about seven digits, so
//! 1284.86 comes out as 1284.8599853515625. The number that went in is the
//! shortest decimal that gives the same float, and that is what is read. And
//! the feed keeps six decimals of the divided number, so one multiplied back
//! through 140 later shares is only known to a ten-thousandth. A tape price
//! or a declared dividend has few decimals, so what is kept is the number
//! with the fewest decimals that the feed's own figure cannot tell apart from
//! it: 16.75002 is 16.75. That number stays the same when the next split
//! comes, which a plain rounding would not.
//!
//! The feed is free and it is someone else's machine. Requests go out at
//! most once a second, each has a time limit, and a refusal is reported and
//! not retried.

use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use serde::Deserialize;

use crate::{adjust::Bar, day::Day, source::Source, store::check_ticker};

/// The least time between two requests.
const PAUSE: Duration = Duration::from_secs(1);
/// How long one answer may take.
const TIME_LIMIT: Duration = Duration::from_secs(20);
const SECONDS_IN_A_DAY: i64 = 86_400;
/// The feed keeps six decimals, so a number from it can be half a millionth
/// off, with a little room for the arithmetic.
const KEPT: f64 = 0.51e-6;
/// A 32-bit float, and the shortest decimal read from it, can together be
/// off by this share of the number.
const FLOAT_GRAIN: f64 = 1.2e-7;
/// No tape price or dividend is kept to more decimals than the feed has.
const MOST_DECIMALS: i32 = 6;

/// Where the feed is and what to ask it for. All of it comes from the caller.
#[derive(Debug, Clone)]
pub struct Feed {
    /// The address one ticker's history is asked at, with `{symbol}`,
    /// `{from}` and `{to}` where the ticker and the first and last moment
    /// wanted go. The moments are filled in as seconds since 1970.
    pub url: String,
    /// How the client names itself to the feed.
    pub user_agent: String,
    /// The first and the last day wanted, both included. The last must be a
    /// day that is over, or its bar is a price from the middle of the day:
    /// `last_finished_day` gives it.
    pub from: Day,
    pub through: Day,
}

/// The last day whose close is final at `now`, a moment in seconds since
/// 1970. New York closes at 20:00 or 21:00 in Greenwich with the season, so a
/// day counts from 22:00.
pub fn last_finished_day(now: i64) -> Day {
    Day::from_unix(now - 22 * 3_600)
}

pub struct DailyFeed {
    http: reqwest::blocking::Client,
    feed: Feed,
    time_limit: Duration,
    last_request: Option<Instant>,
}

impl DailyFeed {
    pub fn open(feed: Feed) -> Result<DailyFeed, String> {
        DailyFeed::with_time_limit(feed, TIME_LIMIT)
    }

    fn with_time_limit(feed: Feed, time_limit: Duration) -> Result<DailyFeed, String> {
        if !feed.url.contains("{symbol}") {
            return Err("the feed's address has no {symbol} for the ticker to go in".into());
        }
        if feed.from > feed.through {
            return Err(format!("the first day wanted, {}, is after the last, {}", feed.from, feed.through));
        }
        let http = reqwest::blocking::Client::builder()
            .user_agent(feed.user_agent.as_str())
            .timeout(time_limit)
            .build()
            .map_err(|error| format!("cannot set up the client for the price feed: {error}"))?;
        Ok(DailyFeed { http, feed, time_limit, last_request: None })
    }

    /// Wait until a second has passed since the last request went out.
    fn pace(&mut self) {
        if let Some(last) = self.last_request {
            std::thread::sleep(PAUSE.saturating_sub(last.elapsed()));
        }
        self.last_request = Some(Instant::now());
    }
}

impl Source for DailyFeed {
    fn fetch(&mut self, ticker: &str) -> Result<Vec<Bar>, String> {
        check_ticker(ticker)?;
        // The feed writes a share class with a dash where a ticker has a slash.
        let url = self
            .feed
            .url
            .replace("{symbol}", &ticker.replace('/', "-"))
            .replace("{from}", &self.feed.from.unix().to_string())
            .replace("{to}", &(self.feed.through.unix() + SECONDS_IN_A_DAY).to_string());
        self.pace();
        let time_limit = self.time_limit;
        let failed = |error: reqwest::Error| {
            if error.is_timeout() {
                format!("the price feed did not answer for {ticker} within {time_limit:?}")
            } else {
                format!("the price feed could not be reached for {ticker}: {}", error.without_url())
            }
        };
        let answer = self.http.get(url).send().map_err(failed)?;
        let status = answer.status();
        if !status.is_success() {
            return Err(format!("the price feed answered {status} for {ticker}"));
        }
        read(&answer.text().map_err(failed)?, self.feed.through).map_err(|reason| format!("{ticker}: {reason}"))
    }
}

#[derive(Deserialize)]
struct Answer {
    chart: Chart,
}

#[derive(Deserialize)]
struct Chart {
    result: Option<Vec<History>>,
    error: Option<Problem>,
}

#[derive(Deserialize)]
struct Problem {
    description: Option<String>,
}

#[derive(Deserialize)]
struct History {
    /// The moment each bar is stamped with: the open of its day in New York.
    #[serde(default)]
    timestamp: Vec<i64>,
    indicators: Indicators,
    #[serde(default)]
    events: Events,
}

#[derive(Deserialize)]
struct Indicators {
    quote: Vec<Quote>,
}

#[derive(Deserialize)]
struct Quote {
    /// One close per moment, divided by every later split. Null on a day the feed has no price for.
    #[serde(default)]
    close: Vec<Option<f64>>,
}

/// Dividends and splits, each filed under the moment of a bar.
#[derive(Deserialize, Default)]
struct Events {
    #[serde(default)]
    dividends: BTreeMap<String, Dividend>,
    #[serde(default)]
    splits: BTreeMap<String, Split>,
}

#[derive(Deserialize)]
struct Dividend {
    /// Cash per share, divided by every later split.
    amount: f64,
    date: Option<i64>,
}

#[derive(Deserialize)]
struct Split {
    numerator: f64,
    denominator: f64,
    date: Option<i64>,
}

/// The close the feed was given, before it stored it as a 32-bit float: the
/// shortest decimal that is the same float.
fn as_given(close: f64) -> f64 {
    (close as f32).to_string().parse().unwrap_or(close)
}

fn round_to(value: f64, decimals: i32) -> f64 {
    let scale = 10f64.powi(decimals);
    (value * scale).round() / scale
}

/// The number with the fewest decimals that lies within `slack` of `value`.
fn simplest(value: f64, slack: f64) -> f64 {
    let rounded = (0..MOST_DECIMALS).map(|decimals| round_to(value, decimals)).find(|rounded| (rounded - value).abs() <= slack);
    rounded.unwrap_or(round_to(value, MOST_DECIMALS))
}

/// Put each event of one kind on the bar it belongs to: the first bar on or
/// after its own date. `events` holds, for each, the moment it is filed
/// under, its own date when the feed gives one, and its size.
///
/// The two dates agree, except when an ex-date fell on a day the exchange was
/// unexpectedly shut. The feed then files the event under the bar before the
/// closure, which is wrong: the price did not react until trading came back.
/// It may also list the same event again under the bar after the closure. So
/// an event filed away from its own date goes to the first bar on or after
/// that date, unless that bar already has one, in which case it is the
/// repeat and is dropped.
fn place(days: &[Day], events: Vec<(&String, Option<i64>, f64)>, on_bar: &mut [f64], join: fn(f64, f64) -> f64) -> Result<(), String> {
    let mut taken = vec![false; days.len()];
    let mut moved = Vec::new();
    for (filed, own, size) in events {
        let filed = Day::from_unix(filed.parse().map_err(|_| format!("the price feed files an event under {filed:?}, which is not a moment"))?);
        let own = own.map_or(filed, Day::from_unix);
        let index = days.partition_point(|day| *day < own);
        // An event after the last bar has not happened yet.
        if index == days.len() {
            continue;
        }
        if own != filed {
            moved.push((index, size));
            continue;
        }
        taken[index] = true;
        on_bar[index] = join(on_bar[index], size);
    }
    for (index, size) in moved {
        if !taken[index] {
            on_bar[index] = join(on_bar[index], size);
        }
    }
    Ok(())
}

/// The bars in one answer from the feed, up to the day `through`.
pub fn read(body: &str, through: Day) -> Result<Vec<Bar>, String> {
    let answer: Answer = serde_json::from_str(body).map_err(|error| format!("the price feed's answer is not in the form expected: {error}"))?;
    if let Some(problem) = answer.chart.error {
        return Err(format!("the price feed says: {}", problem.description.as_deref().unwrap_or("there is a problem, with no description")));
    }
    let history = answer.chart.result.and_then(|histories| histories.into_iter().next()).ok_or("the price feed's answer holds no history")?;
    let closes = history.indicators.quote.into_iter().next().map(|quote| quote.close).unwrap_or_default();
    if closes.len() != history.timestamp.len() {
        return Err(format!("the price feed's answer has {} days and {} closes", history.timestamp.len(), closes.len()));
    }

    // A day with no close has no bar. Neither has a day that is not over yet.
    let (days, closes): (Vec<Day>, Vec<f64>) = history
        .timestamp
        .iter()
        .zip(&closes)
        .filter_map(|(moment, close)| Some((Day::from_unix(*moment), as_given((*close)?))))
        .filter(|(day, close)| *day <= through && *close > 0.0)
        .unzip();
    if days.is_empty() {
        return Err("the price feed has no days for it".into());
    }

    let mut dividends = vec![0.0; days.len()];
    let mut splits = vec![1.0; days.len()];
    let paid = history.events.dividends.iter().filter(|(_, dividend)| dividend.amount > 0.0);
    place(&days, paid.map(|(filed, dividend)| (filed, dividend.date, dividend.amount)).collect(), &mut dividends, |sum, amount| sum + amount)?;
    for split in history.events.splits.values() {
        if !(split.numerator > 0.0 && split.denominator > 0.0) {
            return Err(format!("the price feed lists a split of {} for {}", split.numerator, split.denominator));
        }
    }
    let ratios = history.events.splits.iter().map(|(filed, split)| (filed, split.date, split.numerator / split.denominator));
    place(&days, ratios.collect(), &mut splits, |product, ratio| product * ratio)?;

    // Walk back from the last day. `later` is every split after the day in
    // hand: what its close and its dividend were divided by, and so how much
    // wider the feed's half a millionth is once they are multiplied back.
    let mut bars = vec![Bar::new(days[0], 0.0); days.len()];
    let mut later = 1.0;
    for index in (0..days.len()).rev() {
        bars[index] = Bar {
            day: days[index],
            close: simplest(closes[index] * later, later * (KEPT + closes[index] * FLOAT_GRAIN)),
            dividend: simplest(dividends[index] * later, later * KEPT),
            split: splits[index],
        };
        later *= splits[index];
    }
    Ok(bars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adjust::fixtures::{bars, day, DIVIDEND_PAYER};
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        sync::mpsc::{channel, Receiver},
    };

    /// A real answer, cut to fourteen weeks that hold a 2-for-1 split with a
    /// dividend before it and one after.
    const ANSWER: &str = include_str!("../fixtures/daily_answer.json");

    fn far() -> Day {
        day("2030-01-01")
    }

    #[test]
    fn an_answer_becomes_the_bars_the_tape_printed() {
        let read = read(ANSWER, far()).unwrap();
        assert_eq!(read.len(), 68);
        // The same days as the stored history of the same ticker, bar for bar.
        let stored = bars(DIVIDEND_PAYER);
        let from = stored.iter().position(|bar| bar.day == day("2012-06-11")).unwrap();
        assert_eq!(read, stored[from..from + 68]);

        let on = |text: &str| *read.iter().find(|bar| bar.day == day(text)).unwrap();
        // Before the split the feed gives half the close and half the dividend. Both come back whole.
        assert_eq!(on("2012-06-13"), Bar { day: day("2012-06-13"), close: 74.69, dividend: 0.51, split: 1.0 });
        assert_eq!(on("2012-08-10"), Bar::new(day("2012-08-10"), 78.79));
        // From the split on they are as the feed gives them.
        assert_eq!(on("2012-08-13"), Bar { day: day("2012-08-13"), close: 39.3, dividend: 0.0, split: 2.0 });
        assert_eq!(on("2012-09-12"), Bar { day: day("2012-09-12"), close: 37.55, dividend: 0.255, split: 1.0 });
    }

    /// An answer with these days, closes and events, in the feed's form.
    fn answer(days: &[&str], closes: &str, events: &str) -> String {
        let moments: Vec<String> = days.iter().map(|text| (day(text).unix() + 13 * 3_600 + 1_800).to_string()).collect();
        format!(r#"{{"chart":{{"result":[{{"timestamp":[{}],"events":{events},"indicators":{{"quote":[{{"close":[{closes}]}}]}}}}],"error":null}}}}"#, moments.join(","))
    }

    fn moment(text: &str) -> i64 {
        day(text).unix() + 13 * 3_600 + 1_800
    }

    #[test]
    fn a_close_is_read_as_the_number_the_feed_was_given() {
        // Real numbers, as the feed sends them: 1284.86, 85.82, a half cent and a sixteenth.
        let days = ["2025-07-01", "2025-07-02", "2025-07-03", "2025-07-07"];
        let body = answer(&days, "1284.8599853515625,85.81999969482422,71.1449966430664,43.9375", "{}");
        assert_eq!(read(&body, far()).unwrap().iter().map(|bar| bar.close).collect::<Vec<_>>(), [1_284.86, 85.82, 71.145, 43.9375]);
    }

    #[test]
    fn a_number_divided_by_later_splits_comes_back_as_it_was() {
        let days = ["2025-07-01", "2025-07-02", "2025-07-03", "2025-07-07"];
        let split = |ratio: &str| format!(r#"{{"date":{0},"numerator":{ratio},"denominator":1.0}}"#, moment("2025-07-07"));
        // Real numbers. Closes of 16.75 and 16.94 since split 140-for-1 in all, sent as six decimals of the 140th part.
        let events = format!(r#"{{"splits":{{"{}":{}}}}}"#, moment("2025-07-07"), split("140.0"));
        let read_back = read(&answer(&days, "0.11964300274848938,0.12099999934434891,0.12142900377511978,17.5", &events), far()).unwrap();
        assert_eq!(read_back.iter().map(|bar| bar.close).collect::<Vec<_>>(), [16.75, 16.94, 17.0, 17.5]);

        // After 480 later shares a close of 21.75 still comes back whole, where rounding to four decimals
        // would read 21.7502. One of 19.6875, a sixteenth, is finer than the feed can tell by then, and
        // comes back as the nearest ten-thousandth. A dividend of 12 cents comes back through 224.
        let events = format!(r#"{{"splits":{{"{}":{}}}}}"#, moment("2025-07-07"), split("480.0"));
        let read_back = read(&answer(&days, "0.045312998816370964,0.04101600125432014,0.05,26.5", &events), far()).unwrap();
        assert_eq!(read_back.iter().map(|bar| bar.close).collect::<Vec<_>>(), [21.75, 19.6877, 24.0, 26.5]);
        let events = format!(
            r#"{{"dividends":{{"{0}":{{"amount":0.000536,"date":{0}}}}},"splits":{{"{1}":{2}}}}}"#,
            moment("2025-07-02"),
            moment("2025-07-07"),
            split("224.0")
        );
        assert_eq!(read(&answer(&days, "0.5,0.5,0.5,112", &events), far()).unwrap()[1].dividend, 0.12);

        // A reverse split makes the feed's figures larger, not smaller, and they come back just the same.
        let split = format!(r#"{{"date":{0},"numerator":1.0,"denominator":8.0}}"#, moment("2025-07-07"));
        let events = format!(r#"{{"splits":{{"{}":{split}}}}}"#, moment("2025-07-07"));
        let read_back = read(&answer(&days, "150.16000366210938,220.0,211.75999450683594,26.0", &events), far()).unwrap();
        assert_eq!(read_back.iter().map(|bar| bar.close).collect::<Vec<_>>(), [18.77, 27.5, 26.47, 26.0]);
    }

    #[test]
    fn a_day_with_no_close_has_no_bar() {
        let body = answer(&["2026-01-05", "2026-01-06", "2026-01-07", "2026-01-08"], "10.5,null,0,11.25", "{}");
        assert_eq!(read(&body, far()).unwrap(), [Bar::new(day("2026-01-05"), 10.5), Bar::new(day("2026-01-08"), 11.25)]);
    }

    #[test]
    fn a_day_that_is_not_over_has_no_bar() {
        let body = answer(&["2026-01-05", "2026-01-06", "2026-01-07"], "10.5,10.75,11.25", "{}");
        let read = read(&body, day("2026-01-06")).unwrap();
        assert_eq!(read.last().unwrap(), &Bar::new(day("2026-01-06"), 10.75));
        // 22:00 in Greenwich is when a day's close counts as final.
        assert_eq!(last_finished_day(day("2026-01-07").unix() + 21 * 3_600 + 3_599), day("2026-01-06"));
        assert_eq!(last_finished_day(day("2026-01-07").unix() + 22 * 3_600), day("2026-01-07"));
    }

    #[test]
    fn an_event_goes_on_the_first_bar_on_or_after_its_date() {
        let days = ["2026-01-05", "2026-01-06", "2026-01-09", "2026-01-12"];
        // A dividend dated on a day with no bar, one dated after the last bar, and a 1-for-4 split.
        let events = format!(
            r#"{{"dividends":{{"{0}":{{"amount":0.8,"date":{0}}},"{1}":{{"amount":0.3,"date":{1}}}}},"splits":{{"{2}":{{"date":{2},"numerator":1.0,"denominator":4.0,"splitRatio":"1:4"}}}}}}"#,
            moment("2026-01-07"),
            moment("2026-02-02"),
            moment("2026-01-12"),
        );
        let read = read(&answer(&days, "40,42,41,44", &events), far()).unwrap();
        // The closes and the dividend before the split were given four times too large.
        assert_eq!(read[0], Bar::new(day("2026-01-05"), 10.0));
        assert_eq!(read[1], Bar::new(day("2026-01-06"), 10.5));
        assert_eq!(read[2], Bar { day: day("2026-01-09"), close: 10.25, dividend: 0.2, split: 1.0 });
        assert_eq!(read[3], Bar { day: day("2026-01-12"), close: 44.0, dividend: 0.0, split: 0.25 });
    }

    #[test]
    fn an_event_filed_before_a_closure_moves_to_the_day_trading_came_back() {
        // Real numbers: the exchange was shut from the 11th to the 14th, and
        // a split dated the 12th is filed under the 10th.
        let days = ["2001-09-07", "2001-09-10", "2001-09-17", "2001-09-18"];
        let events = format!(r#"{{"splits":{{"{}":{{"date":{},"numerator":2.0,"denominator":1.0}}}}}}"#, moment("2001-09-10"), moment("2001-09-12"));
        let read = read(&answer(&days, "39.01,37.88,33.99,31.94", &events), far()).unwrap();
        assert_eq!(read[1], Bar::new(day("2001-09-10"), 75.76));
        assert_eq!(read[2], Bar { day: day("2001-09-17"), close: 33.99, dividend: 0.0, split: 2.0 });
    }

    #[test]
    fn an_event_listed_again_after_a_closure_is_counted_once() {
        // Real numbers again: one dividend, dated the 12th, filed under the
        // 10th and listed a second time under the 17th.
        let days = ["2001-09-07", "2001-09-10", "2001-09-17", "2001-09-18"];
        let events = format!(
            r#"{{"dividends":{{"{}":{{"amount":0.18,"date":{}}},"{2}":{{"amount":0.09,"date":{2}}}}}}}"#,
            moment("2001-09-10"),
            moment("2001-09-12"),
            moment("2001-09-17"),
        );
        let read = read(&answer(&days, "24.865,24.975,25.1,24.735", &events), far()).unwrap();
        assert_eq!(read.iter().map(|bar| bar.dividend).collect::<Vec<_>>(), [0.0, 0.0, 0.09, 0.0]);
    }

    #[test]
    fn an_answer_that_holds_no_history_says_why() {
        let refusal = r#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found, symbol may be delisted"}}}"#;
        assert_eq!(read(refusal, far()).unwrap_err(), "the price feed says: No data found, symbol may be delisted");
        assert_eq!(read(r#"{"chart":{"result":[],"error":null}}"#, far()).unwrap_err(), "the price feed's answer holds no history");
        assert!(read("<html>Too many requests</html>", far()).unwrap_err().starts_with("the price feed's answer is not in the form expected"));
        let empty = answer(&[], "", "{}");
        assert_eq!(read(&empty, far()).unwrap_err(), "the price feed has no days for it");
        let uneven = answer(&["2026-01-05", "2026-01-06"], "10.5", "{}");
        assert_eq!(read(&uneven, far()).unwrap_err(), "the price feed's answer has 2 days and 1 closes");
        let events = format!(r#"{{"splits":{{"{0}":{{"date":{0},"numerator":0.0,"denominator":1.0}}}}}}"#, moment("2026-01-06"));
        assert_eq!(read(&answer(&["2026-01-05", "2026-01-06"], "10.5,11", &events), far()).unwrap_err(), "the price feed lists a split of 0 for 1");
    }

    /// A feed on this machine. It gives each caller the next canned answer,
    /// or holds the line open without a word when the answer is `None`, and
    /// passes back the first line of every request it gets.
    fn serve(answers: Vec<Option<(&'static str, String)>>) -> (String, Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let (asked, requests) = channel();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for answer in answers {
                let (mut stream, _) = listener.accept().unwrap();
                let mut lines = BufReader::new(stream.try_clone().unwrap()).lines();
                asked.send(lines.next().unwrap().unwrap()).unwrap();
                while lines.next().is_some_and(|line| line.is_ok_and(|line| !line.is_empty())) {}
                match answer {
                    Some((status, body)) => {
                        let head = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close", body.len());
                        write!(stream, "{head}\r\n\r\n{body}").unwrap();
                    }
                    None => held.push(stream),
                }
            }
            // Keep a silent line open until the test is done with it.
            if !held.is_empty() {
                std::thread::sleep(Duration::from_secs(5));
            }
        });
        (format!("http://{address}/history/{{symbol}}?from={{from}}&to={{to}}&every=day"), requests)
    }

    fn feed(url: String) -> Feed {
        Feed { url, user_agent: "compounder-prices-test".into(), from: day("2012-06-11"), through: day("2012-09-14") }
    }

    #[test]
    fn a_history_is_fetched_from_the_address_the_caller_gives() {
        let (url, requests) = serve(vec![Some(("200 OK", ANSWER.to_string())), Some(("200 OK", ANSWER.to_string()))]);
        let mut source = DailyFeed::open(feed(url)).unwrap();
        let started = Instant::now();
        let first = source.fetch("KO").unwrap();
        assert_eq!(first.len(), 68);
        assert_eq!((first[0].day, first[67].day), (day("2012-06-11"), day("2012-09-14")));
        // The ticker and the span wanted are filled in: from the first midnight to the midnight after the last day.
        assert_eq!(requests.recv().unwrap(), "GET /history/KO?from=1339372800&to=1347667200&every=day HTTP/1.1");

        // A second request waits its turn: no more than one a second. A share class is asked for with a dash.
        assert_eq!(source.fetch("BRK/B").unwrap(), first);
        assert!(started.elapsed() >= PAUSE, "two requests went out {:?} apart", started.elapsed());
        assert_eq!(requests.recv().unwrap(), "GET /history/BRK-B?from=1339372800&to=1347667200&every=day HTTP/1.1");
    }

    #[test]
    fn a_refusal_is_a_clear_error() {
        let missing = r#"{"chart":{"result":null,"error":{"code":"Not Found","description":"No data found, symbol may be delisted"}}}"#;
        let (url, _requests) = serve(vec![Some(("429 Too Many Requests", "slow down".to_string())), Some(("200 OK", missing.to_string()))]);
        let mut source = DailyFeed::open(feed(url)).unwrap();
        assert_eq!(source.fetch("KO").unwrap_err(), "the price feed answered 429 Too Many Requests for KO");
        assert_eq!(source.fetch("GONE").unwrap_err(), "GONE: the price feed says: No data found, symbol may be delisted");
        // Something that is not a ticker is never sent.
        assert_eq!(source.fetch("KO?x=1").unwrap_err(), "\"KO?x=1\" is not a ticker");
    }

    #[test]
    fn a_feed_that_does_not_answer_is_given_up_on() {
        let (url, _requests) = serve(vec![None]);
        let mut source = DailyFeed::with_time_limit(feed(url), Duration::from_millis(300)).unwrap();
        let started = Instant::now();
        assert_eq!(source.fetch("KO").unwrap_err(), "the price feed did not answer for KO within 300ms");
        assert!(started.elapsed() < Duration::from_secs(3));
        // Nothing is listening here at all.
        let mut nowhere = DailyFeed::open(feed("http://127.0.0.1:9/{symbol}".into())).unwrap();
        assert!(nowhere.fetch("KO").unwrap_err().starts_with("the price feed could not be reached for KO"));
    }

    #[test]
    fn settings_that_cannot_work_are_refused() {
        let no_place = DailyFeed::open(feed("http://127.0.0.1:9/history".into()));
        assert_eq!(no_place.err().unwrap(), "the feed's address has no {symbol} for the ticker to go in");
        let backwards = DailyFeed::open(Feed { from: day("2026-01-02"), through: day("2026-01-01"), ..feed("http://127.0.0.1:9/{symbol}".into()) });
        assert_eq!(backwards.err().unwrap(), "the first day wanted, 2026-01-02, is after the last, 2026-01-01");
    }
}
