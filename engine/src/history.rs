//! The prices a strategy is evaluated on: the trading calendar, and each
//! asset's adjusted closes laid on it.
//!
//! A history is handed to the engine whole. The engine never opens a file or
//! asks a feed for a price, so the same history gives the same answer on a
//! server and in a browser. Which series stands behind a token is settled by
//! whoever builds the history: here an asset simply has a series or has none.

use crate::rules::Asset;
use compounder_prices::{
    adjust::Series,
    calendar::{Aligned, Calendar},
};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub struct History {
    calendar: Calendar,
    series: BTreeMap<Asset, Aligned>,
}

impl History {
    /// A history on this calendar, with no asset in it yet.
    pub fn new(calendar: Calendar) -> History {
        History { calendar, series: BTreeMap::new() }
    }

    /// Lay an asset's adjusted series on the calendar. A day inside its life
    /// with no bar repeats the close before it. Adding the same asset again
    /// replaces what was there.
    pub fn add(&mut self, asset: Asset, series: &Series) {
        self.series.insert(asset, self.calendar.align(series));
    }

    pub fn calendar(&self) -> &Calendar {
        &self.calendar
    }

    /// The asset's prices on the calendar, when it has any.
    pub fn of(&self, asset: &Asset) -> Option<&Aligned> {
        self.series.get(asset).filter(|aligned| !aligned.range().is_empty())
    }
}
