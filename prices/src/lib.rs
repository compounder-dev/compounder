//! Daily price history for the strategy engine.
//!
//! `adjust` turns a ticker's raw daily bars into the adjusted series a
//! backtest runs on. `calendar` knows the trading days and lays several
//! tickers on them. Nothing here works out an indicator or runs a strategy.

pub mod adjust;
pub mod calendar;
pub mod day;
