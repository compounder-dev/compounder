//! The price histories a backtest runs on, read from the price store.
//!
//! The store is written by another program, once a day. What is read from
//! it is kept for a few minutes, so a run of backtests opens each file once
//! and a refresh is still picked up soon after it lands.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use compounder_prices::{
    adjust::{adjust, Series},
    calendar::Calendar,
    guard,
    store::{check_ticker, Store},
};

/// How long what was read from the store is kept.
pub const KEEP: Duration = Duration::from_secs(300);
/// The tickers whose bars make the trading calendar: a broad fund for the
/// years it covers, and an older share to carry the calendar back before it.
const CALENDAR_TICKERS: [&str; 2] = ["SPY", "KO"];

/// One ticker's adjusted series, and what was done to its bars on the way.
pub struct Listing {
    pub series: Series,
    /// Said when a first bar nobody could have traded at was left out.
    pub dropped_first_bar: Option<String>,
}

/// What the store has for a ticker: `None` when it has no such ticker.
pub type Found = Option<Arc<Listing>>;

pub struct Prices {
    dir: PathBuf,
    read: Mutex<Option<Read>>,
}

/// What has been read from the store, and when the reading began.
struct Read {
    at: Instant,
    store: Store,
    calendar: Arc<Calendar>,
    /// Every ticker asked for so far.
    listings: HashMap<String, Found>,
}

impl Prices {
    /// The prices in the store at `dir`. Nothing is opened until they are asked for.
    pub fn at(dir: impl Into<PathBuf>) -> Prices {
        Prices { dir: dir.into(), read: Mutex::new(None) }
    }

    /// The trading calendar, and each ticker's series in the order asked.
    pub fn of(&self, tickers: &[&str]) -> Result<(Arc<Calendar>, Vec<Found>), String> {
        // A reader that died mid-way left nothing half-read: what is kept is only ever added to whole.
        let mut kept = self.read.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if kept.as_ref().is_none_or(|read| read.at.elapsed() > KEEP) {
            *kept = Some(Read::open(&self.dir)?);
        }
        let read = kept.as_mut().expect("just read");
        let listings = tickers.iter().map(|ticker| read.listing(ticker)).collect::<Result<_, _>>()?;
        Ok((read.calendar.clone(), listings))
    }

    /// Every ticker the store has prices for, with the first day it has them, in the store's own order.
    pub fn listed(&self) -> Result<Vec<(String, String)>, String> {
        let mut kept = self.read.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if kept.as_ref().is_none_or(|read| read.at.elapsed() > KEEP) {
            *kept = Some(Read::open(&self.dir)?);
        }
        let store = &kept.as_ref().expect("just read").store;
        Ok(store.tickers().filter_map(|ticker| Some((ticker.to_string(), store.span(ticker)?.first_day.to_string()))).collect())
    }
}

impl Read {
    fn open(dir: &Path) -> Result<Read, String> {
        let store = Store::open(dir)?;
        let references = CALENDAR_TICKERS.iter().map(|ticker| store.load(ticker)).collect::<Result<Vec<_>, _>>()?;
        let calendar = Calendar::from_bars(&references.iter().map(Vec::as_slice).collect::<Vec<_>>())?;
        Ok(Read { at: Instant::now(), store, calendar: Arc::new(calendar), listings: HashMap::new() })
    }

    fn listing(&mut self, ticker: &str) -> Result<Found, String> {
        if let Some(listing) = self.listings.get(ticker) {
            return Ok(listing.clone());
        }
        // A name that could not be a ticker is one the store does not have.
        let bars = match check_ticker(ticker) {
            Ok(()) => self.store.find(ticker)?,
            Err(_) => None,
        };
        let listing = match bars {
            Some(mut bars) => {
                let dropped_first_bar = guard::first_bar_jump(&bars);
                if dropped_first_bar.is_some() {
                    bars.remove(0);
                }
                Some(Arc::new(Listing { series: adjust(&bars).map_err(|reason| format!("{ticker}: {reason}"))?, dropped_first_bar }))
            }
            None => None,
        };
        self.listings.insert(ticker.to_string(), listing.clone());
        Ok(listing)
    }
}
