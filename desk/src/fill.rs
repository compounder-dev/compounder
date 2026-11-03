//! From targets to trades, and trades to fills: against a real book when the
//! fund runs live, against the next bar's open when it is tested.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    account::{Account, Fill},
    market::Book,
};

/// What trading costs. `taker` is the fee as a share of notional. `slippage` is
/// the share of price a test gives up on each fill, for the spread and depth a
/// bar does not show; a live fill pays what the book asks instead.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Costs {
    pub taker: f64,
    pub slippage: f64,
}

impl Default for Costs {
    fn default() -> Self {
        Self { taker: 0.0009, slippage: 0.0002 }
    }
}

/// The trades that take the account from where it is to the target shares of
/// its equity. A coin moves only when its change is at least `min_trade` of
/// equity, or when the target closes it out.
pub fn orders(account: &Account, targets: &BTreeMap<String, f64>, marks: &BTreeMap<String, f64>, min_trade: f64) -> Vec<(String, f64)> {
    let equity = account.equity(marks);
    if equity <= 0.0 {
        return account.qty.iter().map(|(coin, qty)| (coin.clone(), -qty)).collect();
    }
    let mut coins: Vec<&String> = targets.keys().chain(account.qty.keys()).collect();
    coins.sort();
    coins.dedup();
    coins
        .into_iter()
        .filter_map(|coin| {
            let mark = *marks.get(coin)?;
            let held = account.qty.get(coin).copied().unwrap_or(0.0);
            let want = targets.get(coin).copied().unwrap_or(0.0) * equity / mark;
            let delta = want - held;
            let closing = want == 0.0 && held != 0.0;
            (closing || (delta * mark).abs() >= min_trade * equity).then(|| (coin.clone(), delta))
        })
        .collect()
}

/// Fills a market order by walking the book. Buys take the asks, sells the bids.
/// What the book cannot fill is left unfilled.
pub fn walk(book: &Book, coin: &str, qty: f64, costs: &Costs) -> Option<Fill> {
    let levels = if qty > 0.0 { &book.asks } else { &book.bids };
    let mut left = qty.abs();
    let mut notional = 0.0;
    for &(px, size) in levels {
        if left <= 0.0 {
            break;
        }
        let take = left.min(size);
        notional += take * px;
        left -= take;
    }
    let filled = qty.abs() - left;
    if filled <= 0.0 {
        return None;
    }
    let signed = filled * qty.signum();
    Some(Fill { coin: coin.to_string(), qty: signed, px: notional / filled, fee: notional * costs.taker })
}

/// A test's fill at a bar's open, worse by the slippage.
pub fn at_open(coin: &str, qty: f64, open: f64, costs: &Costs) -> Fill {
    let px = open * (1.0 + costs.slippage * qty.signum());
    Fill { coin: coin.to_string(), qty, px, fee: (qty * px).abs() * costs.taker }
}

/// Rounds a quantity toward zero to the market's size step.
pub fn to_step(qty: f64, decimals: u32) -> f64 {
    let scale = 10f64.powi(decimals as i32);
    (qty * scale).trunc() / scale
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walking_the_book_averages_the_levels_taken() {
        let book = Book { bids: vec![(99.0, 1.0)], asks: vec![(100.0, 1.0), (102.0, 3.0)] };
        let fill = walk(&book, "X", 2.0, &Costs { taker: 0.001, slippage: 0.0 }).unwrap();
        assert!((fill.px - 101.0).abs() < 1e-9);
        assert!((fill.fee - 0.202).abs() < 1e-9);
        let sell = walk(&book, "X", -5.0, &Costs::default()).unwrap();
        assert_eq!(sell.qty, -1.0);
    }

    #[test]
    fn small_changes_wait_and_closing_out_never_does() {
        let mut account = Account::new(1000.0);
        account.apply_fill(&Fill { coin: "A".into(), qty: 1.0, px: 100.0, fee: 0.0 });
        let marks = BTreeMap::from([("A".to_string(), 100.0), ("B".to_string(), 10.0)]);
        let targets = BTreeMap::from([("A".to_string(), 0.105), ("B".to_string(), 0.5)]);
        let trades = orders(&account, &targets, &marks, 0.01);
        assert_eq!(trades, vec![("B".to_string(), 50.0)]);
        let trades = orders(&account, &BTreeMap::new(), &marks, 0.5);
        assert_eq!(trades, vec![("A".to_string(), -1.0)]);
    }
}
