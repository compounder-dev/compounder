//! Rebalancing: holdings, prices and target weights in; the trades that bring
//! the portfolio back on target out.
//!
//! All money is exact. Token amounts are integers in the token's smallest
//! unit. Prices are dollars per whole token scaled by 1e18, and dollar values
//! are scaled by 1e18. Once target weights are fixed to parts per billion,
//! nothing here uses floating point.
//!
//! Every trade settles against one quote asset: a sell pays into it and a buy
//! spends it. A holding is traded only when it has drifted out of band, so a
//! portfolio near its targets is left alone.

use alloy_primitives::U256;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Target weights are whole parts per billion and sum to exactly this.
pub const WEIGHT_SCALE: u64 = 1_000_000_000;
const BASIS_POINTS: u64 = 10_000;
/// Token decimals above this are refused; no real token comes close.
const MAX_DECIMALS: u8 = 36;

/// Read and write a `U256` as a decimal string, the form JSON clients can hold.
mod decimal {
    use alloy_primitives::U256;
    use serde::{de::Error, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &U256, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<U256, D::Error> {
        let text = String::deserialize(deserializer)?;
        U256::from_str_radix(&text, 10).map_err(|_| D::Error::custom(format!("{text:?} is not a whole number")))
    }
}

/// One holding and what it is worth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Position {
    pub asset_id: String,
    /// Balance in the token's smallest unit.
    #[serde(with = "decimal")]
    pub amount: U256,
    pub decimals: u8,
    /// Dollars per whole token, scaled by 1e18.
    #[serde(with = "decimal")]
    pub price: U256,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rules {
    /// The asset trades settle in. Sells pay into it and buys spend it.
    pub quote_asset_id: String,
    /// How far a holding may sit from its target before it is traded, in
    /// basis points of the target. 1000 means 10% above or below target.
    pub trigger_bps: u32,
    /// Trades worth less than this many dollars, scaled by 1e18, are skipped.
    #[serde(with = "decimal")]
    pub min_trade_usd: U256,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Sell,
    Buy,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Trade {
    pub asset_id: String,
    pub side: Side,
    /// Tokens to sell, or the tokens a buy returns at the given price.
    #[serde(with = "decimal")]
    pub amount: U256,
    /// Dollars the sell raises or the buy spends, scaled by 1e18.
    #[serde(with = "decimal")]
    pub value_usd: U256,
    /// The same in the quote token's smallest unit: what a sell should return
    /// and exactly what a buy spends. Buys never exceed cash plus sells.
    #[serde(with = "decimal")]
    pub quote_amount: U256,
}

/// Sells first, then buys, each in asset-id order.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Plan {
    #[serde(with = "decimal")]
    pub total_value_usd: U256,
    pub trades: Vec<Trade>,
}

/// Fix fractional target weights to parts per billion that sum to exactly one.
/// The rounding remainder goes to the weights that lost the most, largest first.
pub fn fix_weights(targets: &BTreeMap<String, f64>) -> Result<BTreeMap<String, u64>, String> {
    let mut fixed = BTreeMap::new();
    let mut lost: Vec<(f64, &String)> = Vec::new();
    let mut total: u64 = 0;
    for (asset_id, weight) in targets {
        if !weight.is_finite() || *weight < 0.0 || *weight > 1.0 {
            return Err(format!("weight {weight} for {asset_id} is not between 0 and 1"));
        }
        let scaled = weight * WEIGHT_SCALE as f64;
        let floor = scaled.floor();
        fixed.insert(asset_id.to_ascii_lowercase(), floor as u64);
        lost.push((scaled - floor, asset_id));
        total += floor as u64;
    }
    let remainder = WEIGHT_SCALE.checked_sub(total).ok_or("target weights sum to more than one")?;
    // Flooring loses less than one part per asset; float error on the sum adds at most a few more.
    if remainder > targets.len() as u64 + 8 {
        return Err("target weights do not sum to one".into());
    }
    lost.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    for index in 0..remainder as usize {
        let (_, asset_id) = lost[index % lost.len()];
        *fixed.get_mut(&asset_id.to_ascii_lowercase()).expect("inserted above") += 1;
    }
    Ok(fixed)
}

fn mul_div(a: U256, b: U256, divisor: U256) -> Result<U256, String> {
    a.checked_mul(b).map(|product| product / divisor).ok_or_else(|| "an amount is too large to price".to_string())
}

fn pow10(decimals: u8) -> U256 {
    U256::from(10u8).pow(U256::from(decimals))
}

struct Held<'a> {
    position: &'a Position,
    value: U256,
    target: U256,
}

/// Is `gap` at least `trigger_bps` of `target`? A zero target is always out of band.
fn out_of_band(gap: U256, target: U256, trigger_bps: u32) -> Result<bool, String> {
    if target.is_zero() {
        return Ok(true);
    }
    let left = gap.checked_mul(U256::from(BASIS_POINTS)).ok_or("a value is too large to compare")?;
    let right = target.checked_mul(U256::from(trigger_bps)).ok_or("a value is too large to compare")?;
    Ok(left >= right)
}

/// Plan the trades that bring `positions` back to `weights`.
///
/// `positions` must cover the quote asset and every asset with a target. A
/// position with no target is treated as a target of zero and sold in full.
pub fn plan(weights: &BTreeMap<String, u64>, positions: &[Position], rules: &Rules) -> Result<Plan, String> {
    if weights.values().sum::<u64>() != WEIGHT_SCALE {
        return Err("target weights do not sum to one".into());
    }
    let quote_id = rules.quote_asset_id.to_ascii_lowercase();

    let mut held: BTreeMap<String, Held> = BTreeMap::new();
    let mut total = U256::ZERO;
    for position in positions {
        let id = position.asset_id.to_ascii_lowercase();
        if position.decimals > MAX_DECIMALS {
            return Err(format!("{id} has {} decimals; the most supported is {MAX_DECIMALS}", position.decimals));
        }
        let weight = weights.get(&id).copied().unwrap_or(0);
        if position.price.is_zero() && (!position.amount.is_zero() || weight > 0) {
            return Err(format!("no price for {id}"));
        }
        let value = mul_div(position.amount, position.price, pow10(position.decimals))?;
        total = total.checked_add(value).ok_or("the portfolio is too large to value")?;
        if held.insert(id.clone(), Held { position, value, target: U256::ZERO }).is_some() {
            return Err(format!("{id} is listed twice"));
        }
    }
    for id in weights.keys() {
        if !held.contains_key(id) {
            return Err(format!("no position or price for target {id}"));
        }
    }
    if !held.contains_key(&quote_id) {
        return Err(format!("no position for the quote asset {quote_id}"));
    }
    if total.is_zero() {
        return Ok(Plan { total_value_usd: total, trades: Vec::new() });
    }
    for (id, entry) in held.iter_mut() {
        let weight = weights.get(id).copied().unwrap_or(0);
        entry.target = mul_div(total, U256::from(weight), U256::from(WEIGHT_SCALE))?;
    }

    // Funding is counted in whole quote tokens, the unit trades settle in, so
    // a plan can never spend a fraction of a unit it does not have.
    let quote = &held[&quote_id];
    let quote_unit = pow10(quote.position.decimals);
    let quote_price = quote.position.price;
    let to_quote = |value_usd: U256| mul_div(value_usd, quote_unit, quote_price);
    let to_usd = |quote_amount: U256| mul_div(quote_amount, quote_price, quote_unit);

    let mut trades = Vec::new();
    let mut raised = U256::ZERO;
    // An asset below target, how many dollars short it is, and whether that is out of band.
    let mut short: Vec<(&String, U256, bool)> = Vec::new();
    for (id, entry) in &held {
        if *id == quote_id {
            continue;
        }
        if entry.value > entry.target {
            let over = entry.value - entry.target;
            if !out_of_band(over, entry.target, rules.trigger_bps)? {
                continue;
            }
            let position = entry.position;
            // Leaving a holding with no target means selling all of it, not all but dust.
            let amount = if entry.target.is_zero() {
                position.amount
            } else {
                mul_div(over, pow10(position.decimals), position.price)?.min(position.amount)
            };
            let quote_amount = to_quote(mul_div(amount, position.price, pow10(position.decimals))?)?;
            let value_usd = to_usd(quote_amount)?;
            if value_usd < rules.min_trade_usd || amount.is_zero() {
                continue;
            }
            raised += quote_amount;
            trades.push(Trade { asset_id: id.clone(), side: Side::Sell, amount, value_usd, quote_amount });
        } else if entry.target > entry.value {
            let gap = entry.target - entry.value;
            short.push((id, gap, out_of_band(gap, entry.target, rules.trigger_bps)?));
        }
    }

    let spare_usd = quote.value.saturating_sub(quote.target);
    let spare = quote.position.amount.saturating_sub(to_quote(quote.target)?);
    // Cash well above its own target is put to work across everything below
    // target, which is how a fresh deposit gets invested.
    let invest_cash = spare_usd >= rules.min_trade_usd && out_of_band(spare_usd, quote.target, rules.trigger_bps)?;
    let mut wanted: Vec<(&String, U256)> = Vec::new();
    let mut wanted_total = U256::ZERO;
    for (id, gap, is_out) in short {
        if is_out || invest_cash {
            let gap_quote = to_quote(gap)?;
            wanted_total += gap_quote;
            wanted.push((id, gap_quote));
        }
    }
    let funds = spare + raised;

    for (id, gap_quote) in wanted {
        // When there is not enough to fill every gap, each buy gets its fair share.
        let quote_amount = if wanted_total > funds { mul_div(gap_quote, funds, wanted_total)? } else { gap_quote };
        let value_usd = to_usd(quote_amount)?;
        if value_usd < rules.min_trade_usd {
            continue;
        }
        let position = held[id].position;
        let amount = mul_div(value_usd, pow10(position.decimals), position.price)?;
        if amount.is_zero() {
            continue;
        }
        trades.push(Trade { asset_id: id.clone(), side: Side::Buy, amount, value_usd, quote_amount });
    }

    Ok(Plan { total_value_usd: total, trades })
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUOTE: &str = "0x00000000000000000000000000000000000000c0:56";

    fn dollars(amount: f64) -> U256 {
        U256::from((amount * 1e6).round() as u128) * U256::from(10u64).pow(U256::from(12u8))
    }

    fn asset(n: u8) -> String {
        format!("0x{n:040x}:56")
    }

    /// A position holding `usd` dollars of an 18-decimal token priced at `price`.
    fn holding(id: &str, usd: f64, price: f64) -> Position {
        let amount = if price == 0.0 { U256::ZERO } else { dollars(usd) * pow10(18) / dollars(price) };
        Position { asset_id: id.into(), amount, decimals: 18, price: dollars(price) }
    }

    /// Cash in a six-decimal dollar token.
    fn cash(usd: f64) -> Position {
        Position { asset_id: QUOTE.into(), amount: U256::from((usd * 1e6).round() as u128), decimals: 6, price: dollars(1.0) }
    }

    fn rules() -> Rules {
        Rules { quote_asset_id: QUOTE.into(), trigger_bps: 1_000, min_trade_usd: dollars(1.0) }
    }

    fn weights(parts: &[(&str, f64)]) -> BTreeMap<String, u64> {
        fix_weights(&parts.iter().map(|(id, weight)| (id.to_string(), *weight)).collect()).expect("weights fix")
    }

    fn usd(value: U256) -> f64 {
        (value / U256::from(10u64).pow(U256::from(12u8))).to::<u128>() as f64 / 1e6
    }

    fn trade<'a>(plan: &'a Plan, id: &str) -> Option<&'a Trade> {
        plan.trades.iter().find(|trade| trade.asset_id == id)
    }

    /// Settle a plan at the plan's own prices.
    fn apply(positions: &[Position], plan: &Plan) -> Vec<Position> {
        let mut after: Vec<Position> = positions.to_vec();
        let mut received = U256::ZERO;
        let mut paid = U256::ZERO;
        for trade in &plan.trades {
            let position = after.iter_mut().find(|p| p.asset_id.eq_ignore_ascii_case(&trade.asset_id)).unwrap();
            match trade.side {
                Side::Sell => {
                    position.amount -= trade.amount;
                    received += trade.quote_amount;
                }
                Side::Buy => {
                    position.amount += trade.amount;
                    paid += trade.quote_amount;
                }
            }
        }
        let cash = after.iter_mut().find(|p| p.asset_id == QUOTE).unwrap();
        cash.amount = (cash.amount + received).checked_sub(paid).expect("a plan never spends more quote than it has");
        after
    }

    #[test]
    fn weights_are_fixed_to_exactly_one() {
        let sevenths: BTreeMap<String, f64> = (1..=7).map(|n| (asset(n), 1.0 / 7.0)).collect();
        let fixed = fix_weights(&sevenths).unwrap();
        assert_eq!(fixed.values().sum::<u64>(), WEIGHT_SCALE);
        let (low, high) = (fixed.values().min().unwrap(), fixed.values().max().unwrap());
        assert!(high - low <= 1, "sevenths differ by more than one part: {low}..{high}");

        assert!(fix_weights(&[(asset(1), 0.6), (asset(2), 0.3)].into_iter().collect()).is_err());
        assert!(fix_weights(&[(asset(1), 0.7), (asset(2), 0.4)].into_iter().collect()).is_err());
        assert!(fix_weights(&[(asset(1), f64::NAN)].into_iter().collect()).is_err());
    }

    #[test]
    fn a_fresh_deposit_is_invested_across_every_target() {
        let (a, b) = (asset(1), asset(2));
        let positions = [cash(1_000.0), holding(&a, 0.0, 250.0), holding(&b, 0.0, 40.0)];
        let plan = plan(&weights(&[(&a, 0.7), (&b, 0.3)]), &positions, &rules()).unwrap();
        assert_eq!(plan.trades.len(), 2);
        assert!((usd(trade(&plan, &a).unwrap().value_usd) - 700.0).abs() < 1e-6);
        assert!((usd(trade(&plan, &b).unwrap().value_usd) - 300.0).abs() < 1e-6);
        assert!(plan.trades.iter().all(|trade| trade.side == Side::Buy));
        // 700 dollars at 250 a token is 2.8 tokens.
        assert_eq!(trade(&plan, &a).unwrap().amount, U256::from(28u8) * pow10(17));
    }

    #[test]
    fn a_portfolio_inside_its_bands_is_left_alone() {
        let (a, b) = (asset(1), asset(2));
        // 54/46 against 50/50: each is 8% off target, inside the 10% band.
        let positions = [cash(0.0), holding(&a, 540.0, 100.0), holding(&b, 460.0, 100.0)];
        let plan = plan(&weights(&[(&a, 0.5), (&b, 0.5)]), &positions, &rules()).unwrap();
        assert!(plan.trades.is_empty(), "{:?}", plan.trades);
    }

    #[test]
    fn only_the_holdings_out_of_band_are_traded() {
        let (a, b, c) = (asset(1), asset(2), asset(3));
        // A is 20% over its target, B is 20% under, C is 2% off and stays put.
        let positions = [cash(0.0), holding(&a, 400.0, 100.0), holding(&b, 266.0, 50.0), holding(&c, 334.0, 20.0)];
        let third = 1.0 / 3.0;
        let plan = plan(&weights(&[(&a, third), (&b, third), (&c, third)]), &positions, &rules()).unwrap();
        assert_eq!(plan.trades.len(), 2);
        assert_eq!(plan.trades[0].side, Side::Sell);
        assert_eq!(plan.trades[0].asset_id, a);
        assert!((usd(plan.trades[0].value_usd) - 66.666666).abs() < 1e-4);
        assert_eq!(plan.trades[1].side, Side::Buy);
        assert_eq!(plan.trades[1].asset_id, b);
        assert!(trade(&plan, &c).is_none());
    }

    #[test]
    fn the_band_is_measured_against_the_target() {
        let (a, b) = (asset(1), asset(2));
        let w = weights(&[(&a, 0.5), (&b, 0.5)]);
        // 54.9 of 50 is 9.8% over: inside. 55.1 is 10.2% over: outside.
        let inside = [cash(0.0), holding(&a, 549.0, 100.0), holding(&b, 451.0, 100.0)];
        assert!(plan(&w, &inside, &rules()).unwrap().trades.is_empty());
        let outside = [cash(0.0), holding(&a, 551.0, 100.0), holding(&b, 449.0, 100.0)];
        assert_eq!(plan(&w, &outside, &rules()).unwrap().trades.len(), 2);
    }

    #[test]
    fn a_holding_with_no_target_is_sold_in_full() {
        let (a, stray) = (asset(1), asset(9));
        let positions = [cash(0.0), holding(&a, 900.0, 100.0), holding(&stray, 100.0, 7.0)];
        let plan = plan(&weights(&[(&a, 1.0)]), &positions, &rules()).unwrap();
        let sell = trade(&plan, &stray).unwrap();
        assert_eq!(sell.side, Side::Sell);
        assert_eq!(sell.amount, positions[2].amount);
    }

    #[test]
    fn buys_share_what_the_sells_raise_when_it_is_not_enough() {
        let ids: Vec<String> = (1..=5).map(asset).collect();
        // Five equal targets of 200. A is 20% over and is sold for 40. D and E
        // are 9% over, inside the band, so they keep their excess. B is 50
        // short and C is 26 short: 76 wanted with only 40 raised.
        let values = [240.0, 150.0, 174.0, 218.0, 218.0];
        let mut positions = vec![cash(0.0)];
        positions.extend(ids.iter().zip(values).map(|(id, value)| holding(id, value, 10.0)));
        let w = weights(&ids.iter().map(|id| (id.as_str(), 0.2)).collect::<Vec<_>>());
        let plan = plan(&w, &positions, &rules()).unwrap();

        assert!((usd(trade(&plan, &ids[0]).unwrap().value_usd) - 40.0).abs() < 1e-6);
        let (buy_b, buy_c) = (usd(trade(&plan, &ids[1]).unwrap().value_usd), usd(trade(&plan, &ids[2]).unwrap().value_usd));
        assert!((buy_b - 40.0 * 50.0 / 76.0).abs() < 1e-5, "{buy_b}");
        assert!((buy_c - 40.0 * 26.0 / 76.0).abs() < 1e-5, "{buy_c}");
        assert!(buy_b + buy_c <= 40.0 + 1e-9);
        assert!(trade(&plan, &ids[3]).is_none() && trade(&plan, &ids[4]).is_none());
    }

    #[test]
    fn trades_below_the_minimum_are_skipped() {
        let (a, b) = (asset(1), asset(2));
        // 20% off target, but the whole gap is 60 cents.
        let positions = [cash(0.0), holding(&a, 3.6, 100.0), holding(&b, 2.4, 100.0)];
        let plan = plan(&weights(&[(&a, 0.5), (&b, 0.5)]), &positions, &rules()).unwrap();
        assert!(plan.trades.is_empty(), "{:?}", plan.trades);
    }

    #[test]
    fn an_empty_portfolio_has_nothing_to_do() {
        let a = asset(1);
        let plan = plan(&weights(&[(&a, 1.0)]), &[cash(0.0), holding(&a, 0.0, 100.0)], &rules()).unwrap();
        assert!(plan.trades.is_empty());
        assert!(plan.total_value_usd.is_zero());
    }

    #[test]
    fn bad_input_is_refused_with_a_reason() {
        let (a, b) = (asset(1), asset(2));
        let w = weights(&[(&a, 0.5), (&b, 0.5)]);
        let err = |positions: &[Position]| plan(&w, positions, &rules()).unwrap_err();
        assert!(err(&[cash(10.0), holding(&a, 10.0, 100.0)]).contains("no position or price for target"));
        assert!(err(&[holding(&a, 10.0, 100.0), holding(&b, 10.0, 100.0)]).contains("quote asset"));
        assert!(err(&[cash(10.0), holding(&a, 10.0, 100.0), holding(&b, 0.0, 0.0)]).contains("no price"));
        assert!(err(&[cash(10.0), cash(5.0), holding(&a, 1.0, 1.0), holding(&b, 1.0, 1.0)]).contains("listed twice"));
        let mut odd = BTreeMap::new();
        odd.insert(a.clone(), WEIGHT_SCALE - 1);
        assert!(plan(&odd, &[cash(10.0), holding(&a, 10.0, 100.0)], &rules()).is_err());
    }

    #[test]
    fn amounts_cross_json_as_decimal_strings() {
        let position = holding(&asset(1), 100.0, 250.0);
        let json = serde_json::to_value(&position).unwrap();
        assert_eq!(json["amount"], "400000000000000000");
        assert_eq!(json["price"], "250000000000000000000");
        assert_eq!(serde_json::from_value::<Position>(json).unwrap(), position);
        assert!(serde_json::from_str::<Position>(r#"{"asset_id":"x","amount":"1.5","decimals":6,"price":"1"}"#).is_err());
    }

    /// Whatever the portfolio, a plan never sells more than is held, never
    /// spends more than it has, respects the minimum, loses no value beyond
    /// rounding, and settling it leaves nothing more to do for the same trades.
    #[test]
    fn plans_are_always_fundable_and_settle_cleanly() {
        let mut state: u64 = 0xC0FFEE;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for case in 0..400 {
            let count = (next() % 5) as u8 + 2;
            let ids: Vec<String> = (1..=count).map(asset).collect();
            let raw: Vec<f64> = ids.iter().map(|_| (next() % 1000) as f64 + 1.0).collect();
            let sum: f64 = raw.iter().sum();
            let cash_weight = if next() % 3 == 0 { 0.1 } else { 0.0 };
            let mut targets: BTreeMap<String, f64> = ids.iter().zip(&raw).map(|(id, r)| (id.clone(), r / sum * (1.0 - cash_weight))).collect();
            if cash_weight > 0.0 {
                targets.insert(QUOTE.into(), cash_weight);
            }
            let w = fix_weights(&targets).unwrap();
            let mut positions = vec![cash((next() % 5_000) as f64)];
            for id in &ids {
                let decimals = [6u8, 8, 18][(next() % 3) as usize];
                let price = dollars((next() % 90_000) as f64 / 7.0 + 0.5);
                let value = dollars((next() % 4_000) as f64);
                positions.push(Position { asset_id: id.clone(), amount: value * pow10(decimals) / price, decimals, price });
            }
            let r = rules();
            let plan = plan(&w, &positions, &r).unwrap_or_else(|e| panic!("case {case}: {e}"));

            let mut raised = U256::ZERO;
            let mut spent = U256::ZERO;
            for trade in &plan.trades {
                assert!(trade.value_usd >= r.min_trade_usd, "case {case}: trade under the minimum");
                let position = positions.iter().find(|p| p.asset_id == trade.asset_id).unwrap();
                match trade.side {
                    Side::Sell => {
                        assert!(trade.amount <= position.amount, "case {case}: sells more than held");
                        raised += trade.quote_amount;
                    }
                    Side::Buy => spent += trade.quote_amount,
                }
            }
            let cash_held = positions[0].amount;
            assert!(spent <= cash_held + raised, "case {case}: spends {spent} quote with {cash_held} held and {raised} raised");
            let sells_end = plan.trades.iter().position(|t| t.side == Side::Buy).unwrap_or(plan.trades.len());
            assert!(plan.trades[sells_end..].iter().all(|t| t.side == Side::Buy), "case {case}: a sell after a buy");

            let after = apply(&positions, &plan);
            let value = |list: &[Position]| list.iter().fold(U256::ZERO, |sum, p| sum + p.amount * p.price / pow10(p.decimals));
            let (before_total, after_total) = (value(&positions), value(&after));
            // Each position's value is floored, so the total can wobble by under one unit per position.
            let wobble = U256::from(positions.len());
            assert!(after_total <= before_total + wobble, "case {case}: settling created value");
            // Each trade can round away under one smallest unit of its token and of the quote.
            let mut slack = U256::ZERO;
            for trade in &plan.trades {
                let position = positions.iter().find(|p| p.asset_id == trade.asset_id).unwrap();
                slack += position.price / pow10(position.decimals) + U256::from(1u8) + pow10(12) * U256::from(2u8);
            }
            assert!(before_total.saturating_sub(after_total) <= slack, "case {case}: lost {} over {slack}", before_total.saturating_sub(after_total));

            let again = super::plan(&w, &after, &r).unwrap();
            for trade in &again.trades {
                let first = plan.trades.iter().find(|t| t.asset_id == trade.asset_id);
                assert!(first.is_none_or(|t| t.side == trade.side), "case {case}: {} flips direction after settling", trade.asset_id);
            }
        }
    }
}
