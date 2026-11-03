//! A fund: a name, a thesis, a strategy and the capital it starts with.

use serde::{Deserialize, Serialize};

use crate::{market::Interval, strategy::Spec};

/// How fast a fund turns over: velocity funds trade within the day, compounding
/// funds hold for weeks and grow by staying in.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Speed {
    Velocity,
    Compounding,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct FundSpec {
    pub id: String,
    pub name: String,
    pub speed: Speed,
    /// Why it should work, in a sentence.
    pub thesis: String,
    /// How it trades, in a sentence.
    pub rules: String,
    pub interval: Interval,
    pub capital: f64,
    /// The smallest change worth trading, as a share of equity.
    pub min_trade: f64,
    pub strategy: Spec,
}

impl FundSpec {
    pub fn coins(&self) -> Vec<String> {
        self.strategy.coins()
    }
}

#[derive(Deserialize)]
struct Catalogue {
    funds: Vec<FundSpec>,
}

/// The funds the desk runs.
pub fn catalogue() -> Vec<FundSpec> {
    let file: Catalogue = serde_json::from_str(include_str!("../funds.json")).expect("funds.json is valid");
    file.funds
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_catalogue_reads_and_ids_are_unique() {
        let funds = super::catalogue();
        assert!(!funds.is_empty());
        let mut ids: Vec<&str> = funds.iter().map(|f| f.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), funds.len());
    }
}
