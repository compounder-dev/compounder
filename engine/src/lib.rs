//! Portfolio engine: strategy documents, rule trees and their evaluation,
//! backtests, exact-money rebalancing and batching.
//! Pure functions: no chain access, no file, no clock and no network.

pub mod backtest;
pub mod batch;
pub mod evaluate;
pub mod history;
pub mod indicator;
pub mod rebalance;
pub mod rules;
pub mod schedule;
pub mod stats;
pub mod strategy;
