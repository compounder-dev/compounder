//! Keep a price store up to date, and read from it.
//!
//!   compounder-prices refresh <store> <ticker>...   bring each ticker up to date from the feed
//!   compounder-prices show <store> <ticker>         print its adjusted series, one day and price a line
//!
//! `refresh` reads the feed's address from PRICE_FEED_URL, with `{symbol}`,
//! `{from}` and `{to}` in it where the ticker and the first and last moment
//! wanted go. PRICE_FEED_AGENT, when set, is how the client names itself.
//! History is asked for from 1990-01-02 to the last day that is over.
//! Warnings are read against the days of the first ticker given, so name a
//! broad, old one first.

use std::{
    io::Write,
    time::{SystemTime, UNIX_EPOCH},
};

use compounder_prices::{
    calendar::Calendar,
    day::HISTORY_START,
    feed::{last_finished_day, DailyFeed, Feed},
    guard,
    source::refresh,
    store::Store,
};

const USAGE: &str = "usage: compounder-prices refresh <store> <ticker>... | show <store> <ticker>";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let done = match (args.first().map(String::as_str), args.get(1), args.get(2..)) {
        (Some("refresh"), Some(dir), Some(tickers)) if !tickers.is_empty() => refresh_all(dir, tickers),
        (Some("show"), Some(dir), Some([ticker])) => show(dir, ticker),
        _ => Err(USAGE.into()),
    };
    if let Err(message) = done {
        eprintln!("{message}");
        std::process::exit(1);
    }
}

fn refresh_all(dir: &str, tickers: &[String]) -> Result<(), String> {
    let url = std::env::var("PRICE_FEED_URL").map_err(|_| "set PRICE_FEED_URL to the feed's address, with {symbol}, {from} and {to} in it")?;
    let user_agent = std::env::var("PRICE_FEED_AGENT").unwrap_or_else(|_| "compounder-prices".into());
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| "this machine's clock is set before 1970")?.as_secs() as i64;
    let mut source = DailyFeed::open(Feed { url, user_agent, from: HISTORY_START, through: last_finished_day(now) })?;
    let mut store = Store::open(dir)?;

    let mut calendar = None;
    let mut failed = 0;
    for ticker in tickers.iter().map(|ticker| ticker.to_ascii_uppercase()) {
        match &refresh(&mut store, &mut source, &[&ticker])[0].1 {
            Ok(outcome) => println!("{ticker}: {outcome}"),
            Err(reason) => {
                failed += 1;
                println!("{ticker}: failed: {reason}");
            }
        }
        let Ok(bars) = store.load(&ticker) else { continue };
        if calendar.is_none() {
            let days = Calendar::from_bars(&[&bars])?;
            if let Some(warning) = guard::calendar_gap(&days) {
                println!("{ticker}: warning: {warning}");
            }
            calendar = Some(days);
        }
        for warning in calendar.iter().flat_map(|days| guard::check(&bars, days)) {
            println!("{ticker}: warning: {warning}");
        }
    }
    match failed {
        0 => Ok(()),
        _ => Err(format!("{failed} of {} tickers failed", tickers.len())),
    }
}

fn show(dir: &str, ticker: &str) -> Result<(), String> {
    let series = Store::open(dir)?.series(&ticker.to_ascii_uppercase())?;
    let mut out = std::io::stdout().lock();
    for (day, price) in series.days().iter().zip(series.prices()) {
        // A reader that has seen enough, such as `head`, ends the listing.
        if writeln!(out, "{day},{price}").is_err() {
            break;
        }
    }
    Ok(())
}
