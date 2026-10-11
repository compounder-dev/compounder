//! Trading a book to its targets at one close.
//!
//! Money in a backtest is `f64` at full precision. Nothing is rounded here:
//! a value is rounded once, where it is shown.
//!
//! 1. Anything the targets no longer hold is sold in full.
//! 2. For each wanted asset the target in dollars is its weight times the
//!    value before trading. The order is the target less the holding.
//! 3. Sells go first.
//! 4. The money for buys is the cash plus what the sells brought in.
//! 5. With slippage `s`, a buy that spends `M` dollars puts `M / (1 + s)`
//!    into the asset. When the buys wanted cost more than the money, every
//!    buy is scaled by the same factor.
//! 6. An order under the minimum is dropped, and its money stays where it
//!    was. For a buy the test comes after the scaling.
//! 7. Cash is what is left.
//!
//! Slippage is a fraction of every dollar traded, on both sides. Selling
//! `D` dollars of an asset brings in `D * (1 - s)`.

/// What is held: shares of each asset, in asset order, and cash.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Book {
    pub shares: Vec<f64>,
    pub cash: f64,
}

/// What one day's trading came to, in dollars.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct Traded {
    /// The value of what was bought, before slippage on it.
    pub bought: f64,
    /// The value of what was sold, before slippage on it.
    pub sold: f64,
    /// Slippage on both.
    pub cost: f64,
}

impl Book {
    /// The value of everything held at these prices, cash included.
    pub fn value(&self, prices: &[f64]) -> f64 {
        self.shares.iter().zip(prices).fold(self.cash, |value, (shares, price)| value + shares * price)
    }

    /// Bring the book to `targets`, the weight wanted of each asset, at
    /// `prices`. Targets are set on the whole value before costs.
    pub fn trade(&mut self, prices: &[f64], targets: &[f64], slippage: f64, min_order: f64) -> Traded {
        let value = self.value(prices);
        let mut traded = Traded::default();
        let mut wanted = vec![0.0; prices.len()];
        for (asset, price) in prices.iter().enumerate() {
            let held = self.shares[asset] * price;
            let gap = targets[asset] * value - held;
            if targets[asset] <= 0.0 {
                // A full exit sells every share, whatever it comes to.
                traded.sold += held;
                self.shares[asset] = 0.0;
            } else if gap < 0.0 && -gap >= min_order {
                traded.sold += -gap;
                self.shares[asset] += gap / price;
            } else if gap > 0.0 {
                wanted[asset] = gap;
            }
        }
        let money = self.cash + traded.sold * (1.0 - slippage);

        // What the buys wanted would cost. When that is more than the money, each is cut to the same share of itself.
        let cost = wanted.iter().sum::<f64>() * (1.0 + slippage);
        let scale = if cost > money { (money / cost).max(0.0) } else { 1.0 };
        for (asset, wanted) in wanted.iter().enumerate() {
            let order = wanted * scale;
            if order > 0.0 && order >= min_order {
                traded.bought += order;
                self.shares[asset] += order / prices[asset];
            }
        }
        // A last digit lost in the sums is not a debt.
        self.cash = (money - traded.bought * (1.0 + slippage)).max(0.0);
        traded.cost = (traded.bought + traded.sold) * slippage;
        traded
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(got: f64, expected: f64) -> bool {
        (got - expected).abs() <= 1e-9 * expected.abs().max(1.0)
    }

    fn all_close(got: &[f64], expected: &[f64]) -> bool {
        got.len() == expected.len() && got.iter().zip(expected).all(|(got, expected)| close(*got, *expected))
    }

    #[test]
    fn the_first_purchase_at_no_cost_spends_everything() {
        let mut book = Book { shares: vec![0.0, 0.0], cash: 10_000.0 };
        let traded = book.trade(&[50.0, 200.0], &[0.6, 0.4], 0.0, 1.0);
        assert_eq!(book.shares, [120.0, 20.0]);
        assert_eq!(book.cash, 0.0);
        assert_eq!(traded, Traded { bought: 10_000.0, sold: 0.0, cost: 0.0 });
    }

    #[test]
    fn a_buy_of_m_dollars_puts_m_over_one_plus_slippage_into_the_asset() {
        // 10,100 dollars at 1%: 10,000 goes into the assets and 100 is the cost. No cash is stranded.
        let mut book = Book { shares: vec![0.0, 0.0], cash: 10_100.0 };
        let traded = book.trade(&[50.0, 200.0], &[0.6, 0.4], 0.01, 1.0);
        assert!(all_close(&book.shares, &[120.0, 20.0]));
        assert!(close(book.cash, 0.0));
        assert!(close(traded.bought, 10_000.0) && close(traded.cost, 100.0) && traded.sold == 0.0);
        assert!(close(book.value(&[50.0, 200.0]), 10_000.0));
    }

    #[test]
    fn sells_go_first_and_buys_are_scaled_together_when_the_money_is_short() {
        // 100 of A at 60, 50 of B at 40 and 30 of C at 20: 6,000, 2,000 and 600, with 1,400 cash. 10,000 in all.
        // Targets of 20%, 50% and 30% want 2,000, 5,000 and 3,000.
        let mut book = Book { shares: vec![100.0, 50.0, 30.0], cash: 1_400.0 };
        let prices = [60.0, 40.0, 20.0];
        let traded = book.trade(&prices, &[0.2, 0.5, 0.3], 0.01, 1.0);
        // A sells 4,000 and brings in 3,960. With the cash that is 5,360 for buys that want 3,000 and 2,400, which cost 5,454.
        let scale = 5_360.0 / 5_454.0;
        assert!(close(traded.sold, 4_000.0) && close(traded.bought, 5_400.0 * scale));
        assert!(all_close(&book.shares, &[100.0 - 4_000.0 / 60.0, 50.0 + 3_000.0 * scale / 40.0, 30.0 + 2_400.0 * scale / 20.0]));
        assert!(close(book.cash, 0.0));
        // The whole value fell by exactly the cost.
        assert!(close(traded.cost, (4_000.0 + 5_400.0 * scale) * 0.01));
        assert!(close(book.value(&prices), 10_000.0 - traded.cost));
    }

    #[test]
    fn an_asset_no_longer_wanted_is_sold_in_full() {
        let mut book = Book { shares: vec![10.0, 0.004], cash: 0.0 };
        let prices = [100.0, 50.0];
        // B is worth 20 cents, under the minimum, and still goes.
        let traded = book.trade(&prices, &[1.0, 0.0], 0.0, 1.0);
        assert_eq!(book.shares[1], 0.0);
        assert!(close(traded.sold, 0.2));
        // Its 20 cents is too small an order to buy A with, so it stays as cash.
        assert_eq!(traded.bought, 0.0);
        assert!(close(book.cash, 0.2) && book.shares[0] == 10.0);
    }

    #[test]
    fn an_order_under_the_minimum_is_dropped_and_its_money_stays_put() {
        // 5,000.40 of A and 4,999.60 of B, to be held half and half: 40 cents each way.
        let mut book = Book { shares: vec![50.004, 49.996], cash: 0.0 };
        let prices = [100.0, 100.0];
        assert_eq!(book.clone().trade(&prices, &[0.5, 0.5], 0.0, 1.0), Traded::default());
        // With a smaller minimum both orders go through.
        let traded = book.trade(&prices, &[0.5, 0.5], 0.0, 0.25);
        assert!(close(traded.sold, 0.4) && close(traded.bought, 0.4));
        assert!(all_close(&book.shares, &[50.0, 50.0]));
    }

    #[test]
    fn a_buy_is_held_to_the_minimum_after_it_is_scaled() {
        // 1.005 dollars to put into one asset at 1% slippage: the order comes to 0.995 and is dropped.
        let mut book = Book { shares: vec![0.0], cash: 1.005 };
        assert_eq!(book.trade(&[10.0], &[1.0], 0.01, 1.0), Traded::default());
        assert_eq!((book.shares[0], book.cash), (0.0, 1.005));
        // At no slippage the same order is 1.005 and goes through.
        let traded = book.trade(&[10.0], &[1.0], 0.0, 1.0);
        assert!(close(traded.bought, 1.005) && close(book.shares[0], 0.1005) && book.cash == 0.0);
    }

    #[test]
    fn a_dropped_buy_leaves_its_money_as_cash_and_the_others_as_they_were() {
        // 1,000 in cash for targets of 99.95% and 0.05%: the second order is 50 cents.
        let mut book = Book { shares: vec![0.0, 0.0], cash: 1_000.0 };
        let traded = book.trade(&[10.0, 10.0], &[0.9995, 0.0005], 0.0, 1.0);
        assert!(close(traded.bought, 999.5) && close(book.shares[0], 99.95) && book.shares[1] == 0.0);
        assert!(close(book.cash, 0.5));
    }
}
