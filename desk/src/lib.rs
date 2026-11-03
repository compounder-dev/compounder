//! The desk: funds that trade perpetual markets on stocks, indices, commodities
//! and crypto around the clock. A fund is a strategy, an account and a ledger.
//! The same strategy code runs a backtest over past bars and a live run against
//! real order books, so a fund's test and its track record mean the same thing.
//!
//! Everything here but `venue` and the agent strategy is free of the network.

pub mod account;
pub mod backtest;
#[cfg(feature = "venue")]
pub mod exchange;
pub mod fill;
pub mod fund;
pub mod ledger;
pub mod market;
#[cfg(feature = "venue")]
pub mod real;
pub mod stats;
pub mod strategy;
#[cfg(feature = "venue")]
pub mod venue;
