//! Daily price history for the strategy engine.
//!
//! `store` keeps each ticker's raw bars on disk. `adjust` turns bars into the
//! adjusted series a backtest runs on. `calendar` knows the trading days and
//! lays several tickers on them. `source` brings the store up to date and
//! says whether history moved. `guard` warns about the ways daily data goes
//! wrong. Nothing here works out an indicator or runs a strategy.

pub mod adjust;
pub mod calendar;
pub mod day;
pub mod guard;
pub mod source;
pub mod store;
