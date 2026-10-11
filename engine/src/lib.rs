//! Portfolio engine: strategy documents, rule trees, exact-money rebalancing
//! and batching.
//! Pure functions, no chain access.

pub mod batch;
pub mod rebalance;
pub mod rules;
pub mod stats;
pub mod strategy;
