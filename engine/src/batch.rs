//! Batching: many accounts' orders in one token become one settlement trade.
//!
//! Buyers and sellers are matched against each other first. Only the
//! difference goes to the market, as a single buy or a single sell. The
//! output is shaped for the settlement contract: fills in whole token units
//! per account, the net quantity, and the cash limit for the market leg.
//!
//! Cash and stock tokens both have 18 decimals here, as the settlement
//! contract requires. Prices are dollars per whole token scaled by 1e18.

use alloy_primitives::U256;
use serde::Serialize;
use std::collections::BTreeMap;

const BASIS_POINTS: u64 = 10_000;

fn one() -> U256 {
    U256::from(10u8).pow(U256::from(18u8))
}

fn mul_div(a: U256, b: U256, divisor: U256) -> Result<U256, String> {
    a.checked_mul(b).map(|product| product / divisor).ok_or_else(|| "an amount is too large".to_string())
}

fn mul_div_up(a: U256, b: U256, divisor: U256) -> Result<U256, String> {
    let product = a.checked_mul(b).ok_or("an amount is too large")?;
    Ok(if product.is_zero() { U256::ZERO } else { (product - U256::from(1u8)) / divisor + U256::from(1u8) })
}

/// What one account wants to do in one token.
#[derive(Debug, Clone, PartialEq)]
pub enum Want {
    /// Sell exactly this many tokens.
    Sell { tokens: U256 },
    /// Spend up to this much cash on the token.
    Buy { cash: U256 },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Order {
    pub account: String,
    pub token: String,
    pub want: Want,
}

/// Every order in one token.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Book {
    pub token: String,
    /// Account and tokens to sell.
    pub sells: Vec<(String, U256)>,
    /// Account and cash budget.
    pub buys: Vec<(String, U256)>,
}

impl Book {
    fn sold(&self) -> U256 {
        self.sells.iter().fold(U256::ZERO, |sum, (_, tokens)| sum + *tokens)
    }

    fn budget(&self) -> U256 {
        self.buys.iter().fold(U256::ZERO, |sum, (_, cash)| sum + *cash)
    }
}

/// Group orders by token. An account may place one order per token; empty
/// orders are refused rather than dropped, so a planning bug is seen.
pub fn books(orders: &[Order]) -> Result<BTreeMap<String, Book>, String> {
    let mut books: BTreeMap<String, Book> = BTreeMap::new();
    let mut seen: BTreeMap<(String, String), ()> = BTreeMap::new();
    for order in orders {
        let token = order.token.to_ascii_lowercase();
        let account = order.account.to_ascii_lowercase();
        if seen.insert((token.clone(), account.clone()), ()).is_some() {
            return Err(format!("{account} has two orders in {token}"));
        }
        let book = books.entry(token.clone()).or_insert_with(|| Book { token, ..Book::default() });
        match &order.want {
            Want::Sell { tokens } if tokens.is_zero() => return Err(format!("{account} has an empty sell")),
            Want::Buy { cash } if cash.is_zero() => return Err(format!("{account} has an empty buy")),
            Want::Sell { tokens } => book.sells.push((account, *tokens)),
            Want::Buy { cash } => book.buys.push((account, *cash)),
        }
    }
    Ok(books)
}

/// What the market must do for a book once buyers and sellers are matched.
#[derive(Debug, Clone, PartialEq)]
pub enum Leg {
    /// Buyers want more than sellers offer: spend this much cash on the market.
    Buy { cash_in: U256 },
    /// Sellers offer more than buyers want: sell this many tokens on the market.
    Sell { tokens_in: U256 },
    /// Buyers and sellers cancel out exactly. There is no market trade to
    /// set a price, so this book waits for the next round.
    Matched,
}

/// Each buyer's budget with the cushion taken off, so a price a little worse
/// than the reference still fits inside what the buyer set aside.
fn cushioned(cash: U256, cushion_bps: u32) -> Result<U256, String> {
    mul_div(cash, U256::from(BASIS_POINTS - cushion_bps as u64), U256::from(BASIS_POINTS))
}

fn check_inputs(reference_price: U256, cushion_bps: u32) -> Result<(), String> {
    if reference_price.is_zero() {
        return Err("no reference price".into());
    }
    if cushion_bps as u64 >= BASIS_POINTS {
        return Err("the cushion must be under 100%".into());
    }
    Ok(())
}

/// Tokens each buyer can afford at the reference price, cushion applied.
fn affordable(book: &Book, reference_price: U256, cushion_bps: u32) -> Result<Vec<U256>, String> {
    book.buys
        .iter()
        .map(|(_, cash)| mul_div(cushioned(*cash, cushion_bps)?, one(), reference_price))
        .collect()
}

/// Decide the market leg for a book at a reference price.
pub fn leg(book: &Book, reference_price: U256, cushion_bps: u32) -> Result<Leg, String> {
    check_inputs(reference_price, cushion_bps)?;
    let sold = book.sold();
    let wanted = affordable(book, reference_price, cushion_bps)?.into_iter().fold(U256::ZERO, |sum, q| sum + q);
    Ok(if wanted > sold {
        // Cash left over once the sellers' tokens have been paid for at the reference price.
        let spend = cushioned(book.budget(), cushion_bps)?.saturating_sub(mul_div_up(sold, reference_price, one())?);
        if spend.is_zero() { Leg::Matched } else { Leg::Buy { cash_in: spend } }
    } else if sold > wanted {
        Leg::Sell { tokens_in: sold - wanted }
    } else {
        Leg::Matched
    })
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Fill {
    pub account: String,
    #[serde(serialize_with = "decimal")]
    pub quantity: U256,
}

/// One token's trade, ready for the settlement contract.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TokenBatch {
    pub token: String,
    pub net_buy: bool,
    /// Tokens the market leg trades: buys minus sells, or sells minus buys.
    #[serde(serialize_with = "decimal")]
    pub net_quantity: U256,
    /// Net buy: the most cash the market leg may spend.
    /// Net sell: the least cash it must return.
    #[serde(serialize_with = "decimal")]
    pub cash_limit: U256,
    pub buys: Vec<Fill>,
    pub sells: Vec<Fill>,
}

fn decimal<S: serde::Serializer>(value: &U256, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

fn sell_fills(book: &Book) -> Vec<Fill> {
    book.sells.iter().map(|(account, tokens)| Fill { account: account.clone(), quantity: *tokens }).collect()
}

/// Fill a book whose market leg is a buy.
///
/// The route spends `cash_in` and guarantees at least `min_tokens_out`. The
/// buyers share those tokens plus everything the sellers offer, in proportion
/// to their budgets. Fails if the guaranteed price would cost any buyer more
/// than their budget: the caller should fetch a better route or a smaller one.
pub fn fill_net_buy(book: &Book, cash_in: U256, min_tokens_out: U256) -> Result<TokenBatch, String> {
    if cash_in.is_zero() || min_tokens_out.is_zero() {
        return Err("the market leg is empty".into());
    }
    if book.buys.is_empty() {
        return Err("a net buy needs buyers".into());
    }
    let budget = book.budget();
    let total = book.sold() + min_tokens_out;
    // The worst price the contract can charge: all the cash for the least tokens.
    let worst_price = mul_div_up(cash_in, one(), min_tokens_out)?;

    let mut buys = Vec::with_capacity(book.buys.len());
    let mut left = total;
    for (index, (account, cash)) in book.buys.iter().enumerate() {
        let last = index + 1 == book.buys.len();
        let quantity = if last { left } else { mul_div(total, *cash, budget)? };
        left -= quantity;
        if quantity.is_zero() {
            return Err(format!("{account}'s share of the batch rounds to nothing"));
        }
        let worst_cost = mul_div_up(quantity, worst_price, one())?;
        if worst_cost > *cash {
            return Err(format!("{account} would pay up to {worst_cost} against a budget of {cash}"));
        }
        buys.push(Fill { account: account.clone(), quantity });
    }
    Ok(TokenBatch {
        token: book.token.clone(),
        net_buy: true,
        net_quantity: min_tokens_out,
        cash_limit: cash_in,
        buys,
        sells: sell_fills(book),
    })
}

/// Fill a book whose market leg is a sell.
///
/// Buyers take what they can afford at the reference price, with the cushion
/// applied, straight from the sellers. The route sells the rest and must
/// return at least `min_cash_out`.
pub fn fill_net_sell(book: &Book, reference_price: U256, cushion_bps: u32, min_cash_out: U256) -> Result<TokenBatch, String> {
    check_inputs(reference_price, cushion_bps)?;
    let quantities = affordable(book, reference_price, cushion_bps)?;
    let wanted = quantities.iter().fold(U256::ZERO, |sum, q| sum + *q);
    let sold = book.sold();
    if sold <= wanted {
        return Err("a net sell needs more offered than wanted".into());
    }
    let mut buys = Vec::with_capacity(book.buys.len());
    for ((account, _), quantity) in book.buys.iter().zip(quantities) {
        if quantity.is_zero() {
            return Err(format!("{account}'s budget buys nothing at this price"));
        }
        buys.push(Fill { account: account.clone(), quantity });
    }
    Ok(TokenBatch {
        token: book.token.clone(),
        net_buy: false,
        net_quantity: sold - wanted,
        cash_limit: min_cash_out,
        buys,
        sells: sell_fills(book),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NVDA: &str = "0x00000000000000000000000000000000000000a1";

    fn units(amount: f64) -> U256 {
        U256::from((amount * 1e9).round() as u128) * U256::from(10u64).pow(U256::from(9u8))
    }

    fn sell(account: &str, tokens: f64) -> Order {
        Order { account: account.into(), token: NVDA.into(), want: Want::Sell { tokens: units(tokens) } }
    }

    fn buy(account: &str, cash: f64) -> Order {
        Order { account: account.into(), token: NVDA.into(), want: Want::Buy { cash: units(cash) } }
    }

    fn book(orders: &[Order]) -> Book {
        books(orders).expect("orders group").remove(NVDA).expect("a book for the token")
    }

    fn total(fills: &[Fill]) -> U256 {
        fills.iter().fold(U256::ZERO, |sum, fill| sum + fill.quantity)
    }

    #[test]
    fn orders_are_grouped_by_token_and_duplicates_are_refused() {
        let mut other = buy("carol", 100.0);
        other.token = "0x00000000000000000000000000000000000000B2".into();
        let grouped = books(&[sell("alice", 1.0), buy("bob", 250.0), other]).unwrap();
        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[NVDA].sells.len(), 1);
        assert_eq!(grouped[NVDA].buys.len(), 1);
        assert!(grouped.contains_key("0x00000000000000000000000000000000000000b2"), "tokens are matched without regard to case");

        assert!(books(&[sell("alice", 1.0), buy("ALICE", 10.0)]).unwrap_err().contains("two orders"));
        assert!(books(&[sell("alice", 0.0)]).unwrap_err().contains("empty sell"));
    }

    #[test]
    fn more_buying_than_selling_sends_only_the_extra_cash_to_market() {
        // Buyers bring 1,000 of cash; a seller offers 1 token worth 250.
        let b = book(&[buy("alice", 600.0), buy("bob", 400.0), sell("carol", 1.0)]);
        assert_eq!(leg(&b, units(250.0), 0).unwrap(), Leg::Buy { cash_in: units(750.0) });
        // With a 1% cushion the buyers hold back 10.
        assert_eq!(leg(&b, units(250.0), 100).unwrap(), Leg::Buy { cash_in: units(740.0) });
    }

    #[test]
    fn more_selling_than_buying_sends_only_the_extra_tokens_to_market() {
        let b = book(&[sell("alice", 5.0), buy("bob", 250.0)]);
        assert_eq!(leg(&b, units(250.0), 0).unwrap(), Leg::Sell { tokens_in: units(4.0) });
    }

    #[test]
    fn an_exact_match_has_no_market_leg() {
        let b = book(&[sell("alice", 1.0), buy("bob", 250.0)]);
        assert_eq!(leg(&b, units(250.0), 0).unwrap(), Leg::Matched);
    }

    #[test]
    fn a_net_buy_shares_every_token_among_buyers_by_budget() {
        let b = book(&[buy("alice", 600.0), buy("bob", 400.0), sell("carol", 1.0)]);
        // The route spends 740 and guarantees 2.9 tokens: a worst price of about 255.17.
        let batch = fill_net_buy(&b, units(740.0), units(2.9)).unwrap();
        assert!(batch.net_buy);
        assert_eq!(batch.net_quantity, units(2.9));
        assert_eq!(batch.cash_limit, units(740.0));
        assert_eq!(total(&batch.buys), units(3.9), "buyers take the market's tokens and the seller's");
        assert_eq!(total(&batch.buys) - total(&batch.sells), batch.net_quantity, "the contract's own check");
        assert_eq!(batch.buys[0].quantity, units(2.34));
        assert_eq!(batch.buys[1].quantity, units(1.56));
    }

    #[test]
    fn a_net_buy_refuses_a_route_too_dear_for_a_buyer() {
        let b = book(&[buy("alice", 600.0), buy("bob", 400.0), sell("carol", 1.0)]);
        // All 750 spent for only 2.9 tokens: 3.9 tokens at 258.63 costs more than the 1,000 set aside.
        let refusal = fill_net_buy(&b, units(750.0), units(2.9)).unwrap_err();
        assert!(refusal.contains("against a budget"), "{refusal}");
    }

    #[test]
    fn a_net_sell_lets_buyers_take_from_sellers_first() {
        let b = book(&[sell("alice", 5.0), buy("bob", 250.0)]);
        let batch = fill_net_sell(&b, units(250.0), 100, units(1_000.0)).unwrap();
        assert!(!batch.net_buy);
        // Bob's 250 less a 1% cushion buys 0.99 tokens at 250.
        assert_eq!(batch.buys[0].quantity, units(0.99));
        assert_eq!(batch.net_quantity, units(4.01));
        assert_eq!(total(&batch.sells) - total(&batch.buys), batch.net_quantity, "the contract's own check");
        assert_eq!(batch.cash_limit, units(1_000.0));
    }

    #[test]
    fn the_wrong_fill_for_a_book_is_refused() {
        let sellers_only = book(&[sell("alice", 1.0)]);
        assert!(fill_net_buy(&sellers_only, units(100.0), units(0.4)).is_err());
        let buyers_only = book(&[buy("bob", 250.0)]);
        assert!(fill_net_sell(&buyers_only, units(250.0), 0, units(1.0)).is_err());
        assert!(leg(&buyers_only, U256::ZERO, 0).is_err());
        assert!(leg(&buyers_only, units(250.0), 10_000).is_err());
    }

    /// For any book, the fills satisfy the settlement contract's checks and no
    /// buyer can be charged more than their budget at the worst allowed price.
    #[test]
    fn batches_always_fit_the_contract_and_the_budgets() {
        let mut state: u64 = 0xBA7C4;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut buys_seen = 0;
        let mut sells_seen = 0;
        for case in 0..600 {
            let price = units((next() % 90_000) as f64 / 37.0 + 1.0);
            let mut orders = Vec::new();
            for n in 0..(next() % 5 + 1) {
                let account = format!("account-{n}");
                if next() % 2 == 0 {
                    orders.push(buy(&account, (next() % 50_000) as f64 / 10.0 + 5.0));
                } else {
                    orders.push(sell(&account, (next() % 4_000) as f64 / 1_000.0 + 0.001));
                }
            }
            let b = book(&orders);
            let cushion = 100;
            match leg(&b, price, cushion).unwrap() {
                Leg::Matched => {}
                Leg::Buy { cash_in } => {
                    // A market that fills 0.4% worse than the reference.
                    let min_out = cash_in * one() / (price * U256::from(1_004u64) / U256::from(1_000u64));
                    if min_out.is_zero() {
                        continue;
                    }
                    let batch = match fill_net_buy(&b, cash_in, min_out) {
                        Ok(batch) => batch,
                        // A buyer whose share rounds to nothing is a valid refusal.
                        Err(reason) if reason.contains("rounds to nothing") => continue,
                        Err(reason) => panic!("case {case}: {reason}"),
                    };
                    assert_eq!(total(&batch.buys) - total(&batch.sells), batch.net_quantity, "case {case}");
                    let worst = mul_div_up(batch.cash_limit, one(), batch.net_quantity).unwrap();
                    for (fill, (_, cash)) in batch.buys.iter().zip(&b.buys) {
                        assert!(mul_div_up(fill.quantity, worst, one()).unwrap() <= *cash, "case {case}: a buyer over budget");
                    }
                    buys_seen += 1;
                }
                Leg::Sell { tokens_in } => {
                    let batch = match fill_net_sell(&b, price, cushion, U256::from(1u8)) {
                        Ok(batch) => batch,
                        Err(reason) if reason.contains("buys nothing") => continue,
                        Err(reason) => panic!("case {case}: {reason}"),
                    };
                    assert_eq!(batch.net_quantity, tokens_in, "case {case}");
                    assert_eq!(total(&batch.sells) - total(&batch.buys), batch.net_quantity, "case {case}");
                    // The contract's band lets the price rise at most 1% above the reference.
                    let ceiling = price * U256::from(101u64) / U256::from(100u64);
                    for (fill, (_, cash)) in batch.buys.iter().zip(&b.buys) {
                        assert!(mul_div_up(fill.quantity, ceiling, one()).unwrap() <= *cash, "case {case}: a buyer over budget");
                    }
                    sells_seen += 1;
                }
            }
        }
        assert!(buys_seen > 50 && sells_seen > 50, "the cases did not cover both legs: {buys_seen} buys, {sells_seen} sells");
    }
}
