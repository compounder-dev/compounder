//! A fund's account: cash and signed positions. Buying a perpetual moves its
//! price out of cash, so cash runs below zero on a long and equity is always
//! cash plus each position at its mark.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A trade that happened: signed quantity (positive buys), average price, fee paid.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Fill {
    pub coin: String,
    pub qty: f64,
    pub px: f64,
    pub fee: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Account {
    pub cash: f64,
    pub qty: BTreeMap<String, f64>,
    /// Fees paid so far.
    pub fees: f64,
    /// Funding paid so far; below zero when the fund was paid.
    pub funding: f64,
    /// Notional traded so far.
    pub traded: f64,
}

impl Account {
    pub fn new(cash: f64) -> Self {
        Self { cash, ..Self::default() }
    }

    /// Equity at the given marks. A position with no mark counts at nothing,
    /// so callers pass a mark for every coin held.
    pub fn equity(&self, marks: &BTreeMap<String, f64>) -> f64 {
        self.cash + self.qty.iter().map(|(coin, qty)| qty * marks.get(coin).copied().unwrap_or(0.0)).sum::<f64>()
    }

    /// Each position as a signed share of equity.
    pub fn weights(&self, marks: &BTreeMap<String, f64>) -> BTreeMap<String, f64> {
        let equity = self.equity(marks);
        if equity <= 0.0 {
            return BTreeMap::new();
        }
        self.qty.iter().filter_map(|(coin, qty)| Some((coin.clone(), qty * marks.get(coin)? / equity))).collect()
    }

    /// Gross notional over equity.
    pub fn leverage(&self, marks: &BTreeMap<String, f64>) -> f64 {
        self.weights(marks).values().map(|w| w.abs()).sum()
    }

    pub fn apply_fill(&mut self, fill: &Fill) {
        self.cash -= fill.qty * fill.px + fill.fee;
        self.fees += fill.fee;
        self.traded += (fill.qty * fill.px).abs();
        let held = self.qty.entry(fill.coin.clone()).or_insert(0.0);
        *held += fill.qty;
        if held.abs() < 1e-12 {
            self.qty.remove(&fill.coin);
        }
    }

    /// Pays one funding period on a coin: longs pay a positive rate, shorts receive it.
    /// Returns what the fund paid.
    pub fn apply_funding(&mut self, coin: &str, rate: f64, px: f64) -> f64 {
        let paid = self.qty.get(coin).copied().unwrap_or(0.0) * px * rate;
        self.cash -= paid;
        self.funding += paid;
        paid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_round_trip_costs_its_fees_and_moves_with_price() {
        let mut account = Account::new(1000.0);
        account.apply_fill(&Fill { coin: "GOLD".into(), qty: 0.5, px: 2000.0, fee: 0.9 });
        let marks = BTreeMap::from([("GOLD".to_string(), 2100.0)]);
        assert!((account.equity(&marks) - (1000.0 + 50.0 - 0.9)).abs() < 1e-9);
        assert!((account.leverage(&marks) - 1050.0 / 1049.1).abs() < 1e-9);
        account.apply_fill(&Fill { coin: "GOLD".into(), qty: -0.5, px: 2100.0, fee: 0.9 });
        assert!(account.qty.is_empty());
        assert!((account.cash - (1000.0 + 50.0 - 1.8)).abs() < 1e-9);
    }

    #[test]
    fn longs_pay_positive_funding_and_shorts_receive_it() {
        let mut account = Account::new(1000.0);
        account.apply_fill(&Fill { coin: "A".into(), qty: 1.0, px: 100.0, fee: 0.0 });
        account.apply_fill(&Fill { coin: "B".into(), qty: -2.0, px: 50.0, fee: 0.0 });
        assert!((account.apply_funding("A", 0.001, 100.0) - 0.1).abs() < 1e-12);
        assert!((account.apply_funding("B", 0.001, 50.0) + 0.1).abs() < 1e-12);
        assert!(account.funding.abs() < 1e-12);
    }
}
