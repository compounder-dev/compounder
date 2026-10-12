//! A strategy's rules in plain words, line by line.
//!
//! The words are chosen here once. The page writer and the web app both print
//! the lines as they are given, so the two can never say different things.

use compounder_engine::rules::{Asset, Comparator, Comparison, Condition, Frequency, Indicator, Market, Node, Operand, Reading, Rebalance, RuleStrategy, Take, Unit, Weighting};
use serde::{Deserialize, Serialize};

use crate::{assets, catalogue, page};

/// One line of the rules.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    /// How far in the line sits: a block's parts are one deeper than the block.
    pub depth: u8,
    /// What the line is: "split", "pick", "if", "condition", "then", "else" or "asset".
    pub kind: String,
    /// The words. For an asset, its ticker.
    pub text: String,
    /// For an asset, what a person calls it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// For an asset in a split of set shares, its share of the split.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share: Option<f64>,
}

/// The ticker a person recognises: a listed symbol as it is, a token's without the issuer's ending.
pub fn ticker(asset: &Asset) -> String {
    match asset {
        Asset::Listed { symbol, .. } => symbol.clone(),
        Asset::Token { id } => match assets().get(id) {
            Some(token) => token.symbol.strip_suffix("on").unwrap_or(&token.symbol).to_string(),
            None => id.chars().take(6).collect(),
        },
    }
}

fn name(asset: &Asset) -> Option<String> {
    match asset {
        Asset::Listed { market: Market::Equities, symbol } => catalogue::listed_name(symbol).map(str::to_string),
        Asset::Listed { .. } => None,
        Asset::Token { id } => assets().get(id).map(|token| token.name.clone()),
    }
}

/// When a fund that follows the strategy trades, in one line.
pub fn trades(rebalance: &Rebalance) -> String {
    match rebalance {
        // A fund on a daily schedule is looked at each day. It trades only when its rules say something has changed.
        Rebalance::Calendar(Frequency::Daily) => "Checked every trading day".to_string(),
        Rebalance::Calendar(Frequency::Weekly) => "Every week".to_string(),
        Rebalance::Calendar(Frequency::Monthly) => "Every month".to_string(),
        Rebalance::Calendar(Frequency::Quarterly) => "Every quarter".to_string(),
        Rebalance::Calendar(Frequency::Yearly) => "Every year".to_string(),
        Rebalance::Threshold { corridor } => format!("When a holding drifts {} from its target", page::percent(*corridor, false)),
    }
}

/// What an indicator measures, as it reads after "its" or after a ticker.
fn measure(indicator: &Indicator) -> String {
    match indicator {
        Indicator::Price => "price".to_string(),
        Indicator::AveragePrice { window } => format!("{window}-day average price"),
        Indicator::ExponentialAveragePrice { window } => format!("{window}-day weighted average price"),
        Indicator::PriceDeviation { window } => format!("{window}-day spread of prices"),
        Indicator::CumulativeReturn { window } => format!("{window}-day return"),
        Indicator::AverageReturn { window } => format!("{window}-day average daily return"),
        Indicator::ReturnDeviation { window } => format!("{window}-day volatility"),
        Indicator::MaxDrawdown { window } => format!("{window}-day worst drop"),
        Indicator::RelativeStrength { window } => format!("{window}-day relative strength"),
    }
}

fn reading(reading: &Reading) -> String {
    format!("{}'s {}", ticker(&reading.asset), measure(&reading.indicator))
}

fn comparison(comparison: &Comparison) -> String {
    let is = match comparison.comparator {
        Comparator::Above => "is above",
        Comparator::AtLeast => "is at least",
        Comparator::Equal => "equals",
        Comparator::Below => "is below",
        Comparator::AtMost => "is at most",
    };
    let right = match &comparison.right {
        Operand::Fixed(number) => match comparison.left.indicator.unit() {
            Unit::Percent => page::percent(*number, false),
            Unit::Dollars => format!("${}", page::quantity(*number)),
        },
        // The same asset on both sides is "its", so the line reads as one thought.
        Operand::Reading(right) if right.asset == comparison.left.asset => format!("its {}", measure(&right.indicator)),
        Operand::Reading(right) => reading(right),
    };
    format!("{} {is} {right}", reading(&comparison.left))
}

fn line(depth: u8, kind: &str, text: impl Into<String>) -> Line {
    Line { depth, kind: kind.to_string(), text: text.into(), name: None, share: None }
}

/// A condition as lines. One comparison is one line. A group is a line that says how many of
/// its parts must hold, with each part under it.
fn condition(condition: &Condition, depth: u8, lead: &str, out: &mut Vec<Line>) {
    let kind = if lead.is_empty() { "condition" } else { "if" };
    let led = |words: &str| if lead.is_empty() { words.to_string() } else { format!("{lead} {words}") };
    match condition {
        Condition::Compare(compared) => out.push(line(depth, kind, led(&comparison(compared)))),
        Condition::Any(parts) | Condition::All(parts) => {
            let words = match (condition, lead.is_empty()) {
                (Condition::Any(_), true) => "Any of these is true",
                (Condition::Any(_), false) => "any of these is true",
                (_, true) => "All of these are true",
                (_, false) => "all of these are true",
            };
            out.push(line(depth, kind, led(words)));
            for part in parts {
                self::condition(part, depth.saturating_add(1), "", out);
            }
        }
    }
}

fn node(node: &Node, depth: u8, share: Option<f64>, out: &mut Vec<Line>) {
    let deeper = depth.saturating_add(1);
    match node {
        Node::Asset(asset) => out.push(Line { depth, kind: "asset".to_string(), text: ticker(asset), name: name(asset), share }),
        Node::Weight { name, weighting, children } => {
            let how = match weighting {
                Weighting::Equal => "Split equally between".to_string(),
                Weighting::Specified { .. } => "Split in set shares between".to_string(),
                Weighting::InverseVolatility { window } => format!("Split between these, with more in whichever has been calmer over {window} days"),
            };
            out.push(line(depth, "split", match name {
                Some(name) => format!("{name}: {}{}", how[..1].to_lowercase(), &how[1..]),
                None => how,
            }));
            for (place, child) in children.iter().enumerate() {
                let share = match weighting {
                    Weighting::Specified { shares } => shares.get(place).copied(),
                    _ => None,
                };
                self::node(child, deeper, share, out);
            }
        }
        Node::If { condition, then, otherwise } => {
            self::condition(condition, depth, "If", out);
            // After a group of conditions, a line says where they end and the holdings begin.
            if !matches!(condition, Condition::Compare(_)) {
                out.push(line(depth, "then", "Then"));
            }
            for child in then {
                self::node(child, deeper, None, out);
            }
            out.push(line(depth, "else", "Otherwise"));
            for child in otherwise {
                self::node(child, deeper, None, out);
            }
        }
        Node::Filter { sort_by, take, count, children } => {
            let which = match take {
                Take::Top => "highest",
                Take::Bottom => "lowest",
            };
            let how_many = if *count == 1 { "the one".to_string() } else { format!("the {count}") };
            out.push(line(depth, "pick", format!("Of these, hold {how_many} with the {which} {}", measure(sort_by))));
            for child in children {
                self::node(child, deeper, None, out);
            }
        }
    }
}

/// The whole strategy, top to bottom.
pub fn lines(strategy: &RuleStrategy) -> Vec<Line> {
    let mut out = Vec::new();
    node(&strategy.root, 0, None, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn listed(symbol: &str) -> serde_json::Value {
        json!({ "block": "asset", "asset": { "market": "equities", "symbol": symbol } })
    }

    fn read(indicator: &str, symbol: &str, window: Option<u32>) -> serde_json::Value {
        let mut reading = json!({ "indicator": indicator, "asset": { "market": "equities", "symbol": symbol } });
        if let Some(window) = window {
            reading["window"] = json!(window);
        }
        reading
    }

    fn said(root: serde_json::Value) -> Vec<String> {
        let document = json!({ "name": "Test", "rebalance": { "type": "calendar", "frequency": "monthly" }, "root": root });
        let strategy = RuleStrategy::from_document(&document).expect("the test's document is valid");
        lines(&strategy)
            .iter()
            .map(|line| {
                let share = line.share.map(|share| format!(" {}", page::percent(share, false))).unwrap_or_default();
                let name = line.name.as_deref().map(|name| format!(" ({name})")).unwrap_or_default();
                format!("{}{}: {}{name}{share}", "  ".repeat(line.depth as usize), line.kind, line.text)
            })
            .collect()
    }

    #[test]
    fn a_split_of_set_shares_names_each_holding_and_its_share() {
        let root = json!({ "block": "weight", "weighting": { "type": "specified", "shares": [0.6, 0.4] }, "children": [listed("VTI"), listed("AGG")] });
        assert_eq!(said(root), ["split: Split in set shares between", "  asset: VTI (US stock market) 60%", "  asset: AGG (US bonds) 40%"]);
    }

    #[test]
    fn a_rule_on_one_asset_reads_as_one_thought() {
        let root = json!({ "block": "if",
            "condition": { "compare": "above", "left": read("price", "SPY", None), "right": read("average_price", "SPY", Some(200)) },
            "then": [listed("SPY")], "else": [listed("BIL")] });
        assert_eq!(
            said(root),
            ["if: If SPY's price is above its 200-day average price", "  asset: SPY (S&P 500)", "else: Otherwise", "  asset: BIL (Short-term US government bills)"]
        );
    }

    #[test]
    fn a_group_of_conditions_lists_each_and_says_where_the_holdings_begin() {
        let root = json!({ "block": "if",
            "condition": { "all": [
                { "compare": "above", "left": read("price", "QQQ", None), "right": read("average_price", "QQQ", Some(200)) },
                { "any": [
                    { "compare": "below", "left": read("relative_strength", "QQQ", Some(10)), "right": { "fixed": 0.8 } },
                    { "compare": "at_least", "left": read("cumulative_return", "SPY", Some(252)), "right": read("cumulative_return", "BIL", Some(252)) } ] } ] },
            "then": [listed("TQQQ")], "else": [listed("BIL")] });
        assert_eq!(
            said(root),
            [
                "if: If all of these are true",
                "  condition: QQQ's price is above its 200-day average price",
                "  condition: Any of these is true",
                "    condition: QQQ's 10-day relative strength is below 80%",
                "    condition: SPY's 252-day return is at least BIL's 252-day return",
                "then: Then",
                "  asset: TQQQ (Nasdaq 100, three times each day's move)",
                "else: Otherwise",
                "  asset: BIL (Short-term US government bills)",
            ]
        );
    }

    #[test]
    fn a_pick_says_how_many_and_by_what() {
        let pick = |count: u32, take: &str| json!({ "block": "filter", "sort_by": { "indicator": "cumulative_return", "window": 126 }, "take": take, "count": count, "children": [listed("XLK"), listed("XLE"), listed("ZZZZ")] });
        assert_eq!(said(pick(2, "top")), ["pick: Of these, hold the 2 with the highest 126-day return", "  asset: XLK (Technology)", "  asset: XLE (Energy)", "  asset: ZZZZ"]);
        assert_eq!(said(pick(1, "bottom"))[0], "pick: Of these, hold the one with the lowest 126-day return");
    }

    #[test]
    fn a_split_by_calm_and_a_named_group_say_so() {
        let root = json!({ "block": "weight", "name": "Safe side", "weighting": { "type": "equal" }, "children": [
            { "block": "weight", "weighting": { "type": "inverse_volatility", "window": 63 }, "children": [listed("TLT"), listed("GLD")] },
            listed("BIL") ] });
        assert_eq!(
            said(root),
            [
                "split: Safe side: split equally between",
                "  split: Split between these, with more in whichever has been calmer over 63 days",
                "    asset: TLT (Long-term US government bonds)",
                "    asset: GLD (Gold)",
                "  asset: BIL (Short-term US government bills)",
            ]
        );
    }

    #[test]
    fn when_it_trades_is_said_in_a_few_words() {
        assert_eq!(trades(&Rebalance::Calendar(Frequency::Daily)), "Checked every trading day");
        assert_eq!(trades(&Rebalance::Calendar(Frequency::Quarterly)), "Every quarter");
        assert_eq!(trades(&Rebalance::Threshold { corridor: 0.075 }), "When a holding drifts 7.5% from its target");
    }

    #[test]
    fn every_fund_in_the_catalogue_can_be_put_in_words() {
        for entry in catalogue::entries() {
            let said = lines(&entry.rules);
            assert!(!said.is_empty() && said.iter().all(|line| !line.text.is_empty()), "{}", entry.id);
        }
    }
}
