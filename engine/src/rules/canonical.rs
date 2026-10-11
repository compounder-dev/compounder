//! The canonical form: one text for one meaning, and its hash.
//!
//! The form is JSON with sorted keys, no white space and numbers in their
//! shortest spelling, as RFC 8785 lays down. It holds what decides a trade
//! and nothing else. Names are left out. Child order is kept, because shares
//! go by position and a filter may break a tie by it.

use super::{Asset, Comparator, Condition, Indicator, Market, Node, Operand, Reading, Rebalance, RuleStrategy, Take, Weighting};
use serde::{Serialize, Serializer};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

impl RuleStrategy {
    /// The strategy's meaning as text. Two strategies that mean the same
    /// give the same text, whatever they are called and however they were
    /// written down. Validate first: a number that is not finite has no
    /// spelling here and is written as null.
    pub fn canonical_json(&self) -> String {
        text(&json!({ "rebalance": rebalance(&self.rebalance), "root": node(&self.root) }))
    }

    /// SHA-256 of the canonical text, in hex.
    pub fn hash(&self) -> String {
        Sha256::digest(self.canonical_json().as_bytes()).iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

pub(crate) fn rebalance(rebalance: &Rebalance) -> Value {
    match *rebalance {
        Rebalance::Calendar(frequency) => json!({ "type": "calendar", "frequency": frequency.word() }),
        Rebalance::Threshold { corridor } => json!({ "type": "threshold", "corridor": corridor }),
    }
}

fn node(node: &Node) -> Value {
    block(node, &mut self::node)
}

/// One block in its canonical form, with each block inside it written by
/// `inner`. The whole text comes from writing the blocks inside the same
/// way. The evaluator writes the number it gave each one instead, and so
/// finds equal sub-trees in one pass over the tree. `inner` is called for
/// the children in order, and for an if the `then` blocks come first.
pub(crate) fn block(node: &Node, inner: &mut dyn FnMut(&Node) -> Value) -> Value {
    let mut nodes = |nodes: &[Node]| nodes.iter().map(&mut *inner).collect::<Vec<Value>>();
    match node {
        Node::Asset(held) => json!({ "block": "asset", "asset": asset(held) }),
        Node::Weight { weighting, children, .. } => {
            let weighting = match weighting {
                Weighting::Equal => json!({ "type": "equal" }),
                Weighting::Specified { shares } => json!({ "type": "specified", "shares": shares }),
                Weighting::InverseVolatility { window } => json!({ "type": "inverse_volatility", "window": window }),
            };
            json!({ "block": "weight", "weighting": weighting, "children": nodes(children) })
        }
        Node::If { condition: test, then, otherwise } => {
            let (then, otherwise) = (nodes(then), nodes(otherwise));
            json!({ "block": "if", "condition": condition(test), "then": then, "else": otherwise })
        }
        Node::Filter { sort_by, take, count, children } => {
            let take = match take {
                Take::Top => "top",
                Take::Bottom => "bottom",
            };
            json!({ "block": "filter", "sort_by": indicator(sort_by), "take": take, "count": count, "children": nodes(children) })
        }
    }
}

/// A canonical form as its one text.
pub(crate) fn text(value: &Value) -> String {
    let mut text = String::new();
    write(value, &mut text);
    text
}

/// An asset is written the one way wherever it is written: as it stands in
/// the canonical form.
impl Serialize for Asset {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        asset(self).serialize(serializer)
    }
}

fn asset(asset: &Asset) -> Value {
    match asset {
        Asset::Listed { market, symbol } => {
            let market = match market {
                Market::Equities => "equities",
                Market::Crypto => "crypto",
            };
            json!({ "market": market, "symbol": symbol })
        }
        // Token ids are matched without regard to case.
        Asset::Token { id } => json!({ "token": id.to_ascii_lowercase() }),
    }
}

fn indicator(indicator: &Indicator) -> Value {
    let name = match indicator {
        Indicator::Price => "price",
        Indicator::AveragePrice { .. } => "average_price",
        Indicator::ExponentialAveragePrice { .. } => "exponential_average_price",
        Indicator::PriceDeviation { .. } => "price_deviation",
        Indicator::CumulativeReturn { .. } => "cumulative_return",
        Indicator::AverageReturn { .. } => "average_return",
        Indicator::ReturnDeviation { .. } => "return_deviation",
        Indicator::MaxDrawdown { .. } => "max_drawdown",
        Indicator::RelativeStrength { .. } => "relative_strength",
    };
    match indicator.window() {
        Some(window) => json!({ "indicator": name, "window": window }),
        None => json!({ "indicator": name }),
    }
}

fn reading(reading: &Reading) -> Value {
    let mut value = indicator(&reading.indicator);
    value["asset"] = asset(&reading.asset);
    value
}

fn condition(condition: &Condition) -> Value {
    match condition {
        Condition::Compare(comparison) => {
            let compare = match comparison.comparator {
                Comparator::Above => "above",
                Comparator::AtLeast => "at_least",
                Comparator::Equal => "equal",
                Comparator::Below => "below",
                Comparator::AtMost => "at_most",
            };
            let right = match &comparison.right {
                Operand::Fixed(value) => json!({ "fixed": value }),
                Operand::Reading(right) => reading(right),
            };
            json!({ "compare": compare, "left": reading(&comparison.left), "right": right })
        }
        Condition::Any(parts) => group("any", parts),
        Condition::All(parts) => group("all", parts),
    }
}

/// A group in its plainest shape: a group of the same word inside it is
/// folded into it, and a group of one is just the one. `any [any [a, b], c]`
/// and `any [a, b, c]` mean the same and are written the same.
fn group(word: &str, parts: &[Condition]) -> Value {
    let mut flat = Vec::with_capacity(parts.len());
    for part in parts {
        match condition(part) {
            Value::Object(mut inner) if inner.len() == 1 && inner.contains_key(word) => {
                if let Some(Value::Array(items)) = inner.remove(word) {
                    flat.extend(items);
                }
            }
            other => flat.push(other),
        }
    }
    if flat.len() == 1 {
        return flat.remove(0);
    }
    json!({ word: flat })
}

fn write(value: &Value, out: &mut String) {
    match value {
        Value::Number(number) => match (number.as_u64(), number.as_i64(), number.as_f64()) {
            (Some(whole), _, _) => out.push_str(&whole.to_string()),
            (_, Some(whole), _) => out.push_str(&whole.to_string()),
            (_, _, Some(float)) => write_float(float, out),
            _ => out.push_str("null"),
        },
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write(item, out);
            }
            out.push(']');
        }
        Value::Object(fields) => {
            let mut keys: Vec<&String> = fields.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&Value::from(key.as_str()).to_string());
                out.push(':');
                write(&fields[key], out);
            }
            out.push('}');
        }
        // Null, true, false and text are already written the one way.
        other => out.push_str(&other.to_string()),
    }
}

/// A float as RFC 8785 writes it: the fewest digits that read back as the
/// same number, no decimal point on a whole number, and an exponent only
/// below 1e-6 and from 1e21 up.
fn write_float(float: f64, out: &mut String) {
    if float == 0.0 {
        return out.push('0');
    }
    if float < 0.0 {
        out.push('-');
    }
    // `{:e}` gives those fewest digits as `d.ddde±x`.
    let exponent_form = format!("{:e}", float.abs());
    let (mantissa, exponent) = exponent_form.split_once('e').unwrap_or((&exponent_form, "0"));
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let count = digits.len() as i32;
    // Where the decimal point sits, counted in digits from the left.
    let point = exponent.parse::<i32>().unwrap_or(0) + 1;
    if (count..=21).contains(&point) {
        out.push_str(&digits);
        out.push_str(&"0".repeat((point - count) as usize));
    } else if (1..=21).contains(&point) {
        let (whole, fraction) = digits.split_at(point as usize);
        out.push_str(&format!("{whole}.{fraction}"));
    } else if (-5..=0).contains(&point) {
        out.push_str(&format!("0.{}{digits}", "0".repeat(-point as usize)));
    } else {
        let (first, rest) = digits.split_at(1);
        let sign = if point > 0 { '+' } else { '-' };
        let dot = if rest.is_empty() { "" } else { "." };
        out.push_str(&format!("{first}{dot}{rest}e{sign}{}", (point - 1).abs()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{import, import_json, Comparison, Frequency};

    fn written(float: f64) -> String {
        let mut text = String::new();
        write_float(float, &mut text);
        text
    }

    #[test]
    fn a_float_is_written_with_the_fewest_digits_that_read_back() {
        let cases = [
            (0.0, "0"),
            (-0.0, "0"),
            (1.0, "1"),
            (-1.0, "-1"),
            (0.5, "0.5"),
            (4.5, "4.5"),
            (0.002, "0.002"),
            (-0.13, "-0.13"),
            (0.7, "0.7"),
            (99.9 / 100.0, "0.9990000000000001"),
            (2.9 / 100.0, "0.028999999999999998"),
            (1.0 / 3.0, "0.3333333333333333"),
            (333333333.3333333, "333333333.3333333"),
            (123456789.0, "123456789"),
            (1e-6, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-10, "1.5e-10"),
            (1e-27, "1e-27"),
            (1e20, "100000000000000000000"),
            (1e21, "1e+21"),
            (1e30, "1e+30"),
            (1.2345e25, "1.2345e+25"),
            (f64::MAX, "1.7976931348623157e+308"),
            (f64::MIN_POSITIVE, "2.2250738585072014e-308"),
            (5e-324, "5e-324"),
        ];
        for (float, text) in cases {
            assert_eq!(written(float), text);
        }
    }

    #[test]
    fn every_float_reads_back_as_the_same_number() {
        let mut state: u64 = 0x5EED;
        for _ in 0..20_000 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            let float = f64::from_bits(state);
            if float.is_finite() {
                let text = written(float);
                assert_eq!(text.parse::<f64>().ok(), Some(float), "{text}");
                assert_eq!(serde_json::from_str::<Value>(&text).ok().map(|value| value.is_number()), Some(true), "{text}");
            }
        }
    }

    fn listed(symbol: &str) -> Asset {
        Asset::Listed { market: Market::Equities, symbol: symbol.to_string() }
    }

    fn strategy(root: Node) -> RuleStrategy {
        RuleStrategy { name: "One".to_string(), rebalance: Rebalance::Calendar(Frequency::Monthly), root }
    }

    fn below(symbol: &str, fixed: f64) -> Condition {
        let left = Reading { indicator: Indicator::RelativeStrength { window: 10 }, asset: listed(symbol) };
        Condition::Compare(Comparison { left, comparator: Comparator::Below, right: Operand::Fixed(fixed) })
    }

    fn when(condition: Condition) -> RuleStrategy {
        strategy(Node::If { condition, then: vec![Node::Asset(listed("SPY"))], otherwise: vec![Node::Asset(listed("BIL"))] })
    }

    #[test]
    fn the_text_has_sorted_keys_no_white_space_and_no_names() {
        let filter = Node::Filter { sort_by: Indicator::Price, take: Take::Bottom, count: 1, children: vec![Node::Asset(listed("BRK/B")), Node::Asset(listed("SPY"))] };
        let tree = strategy(Node::Weight { name: Some("Core \"holdings\"".to_string()), weighting: Weighting::Specified { shares: vec![0.25, 0.75] }, children: vec![Node::Asset(Asset::Listed { market: Market::Crypto, symbol: "BTC".to_string() }), filter] });
        let expected = concat!(
            r#"{"rebalance":{"frequency":"monthly","type":"calendar"},"root":{"block":"weight","children":["#,
            r#"{"asset":{"market":"crypto","symbol":"BTC"},"block":"asset"},"#,
            r#"{"block":"filter","children":[{"asset":{"market":"equities","symbol":"BRK/B"},"block":"asset"},{"asset":{"market":"equities","symbol":"SPY"},"block":"asset"}],"#,
            r#""count":1,"sort_by":{"indicator":"price"},"take":"bottom"}],"#,
            r#""weighting":{"shares":[0.25,0.75],"type":"specified"}}}"#,
        );
        assert_eq!(tree.canonical_json(), expected);

        let threshold = RuleStrategy { rebalance: Rebalance::Threshold { corridor: 0.05 }, ..when(below("QQQ", 0.3)) };
        let expected = concat!(
            r#"{"rebalance":{"corridor":0.05,"type":"threshold"},"root":{"block":"if","condition":"#,
            r#"{"compare":"below","left":{"asset":{"market":"equities","symbol":"QQQ"},"indicator":"relative_strength","window":10},"right":{"fixed":0.3}},"#,
            r#""else":[{"asset":{"market":"equities","symbol":"BIL"},"block":"asset"}],"then":[{"asset":{"market":"equities","symbol":"SPY"},"block":"asset"}]}}"#,
        );
        assert_eq!(threshold.canonical_json(), expected);
    }

    #[test]
    fn a_name_changes_nothing_and_a_group_is_an_equal_weight() {
        let block = |name: Option<&str>| Node::Weight { name: name.map(str::to_string), weighting: Weighting::Equal, children: vec![Node::Asset(listed("TLT"))] };
        let hash = strategy(block(None)).hash();
        assert_eq!(strategy(block(Some("Bonds"))).hash(), hash);
        assert_eq!(RuleStrategy { name: "Another".to_string(), ..strategy(block(Some("Rates"))) }.hash(), hash);
        assert_eq!(hash.len(), 64);
        assert!(hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
    }

    #[test]
    fn any_and_all_groups_hash_in_their_plainest_shape() {
        let (a, b, c) = (below("SPY", 0.3), below("QQQ", 0.3), below("IWM", 0.3));
        let flat = when(Condition::Any(vec![a.clone(), b.clone(), c.clone()])).hash();
        assert_eq!(when(Condition::Any(vec![Condition::Any(vec![a.clone(), b.clone()]), c.clone()])).hash(), flat);
        assert_eq!(when(Condition::Any(vec![a.clone(), Condition::Any(vec![b.clone(), c.clone()])])).hash(), flat);
        assert_eq!(when(Condition::Any(vec![Condition::All(vec![Condition::Any(vec![a.clone(), b.clone()])]), c.clone()])).hash(), flat);

        // A group of one is the one, whatever word it has.
        let alone = when(a.clone()).hash();
        assert_eq!(when(Condition::Any(vec![a.clone()])).hash(), alone);
        assert_eq!(when(Condition::All(vec![Condition::Any(vec![a.clone()])])).hash(), alone);

        // The word, the order and the grouping across words all still count.
        assert_ne!(when(Condition::All(vec![a.clone(), b.clone(), c.clone()])).hash(), flat);
        assert_ne!(when(Condition::Any(vec![b.clone(), a.clone(), c.clone()])).hash(), flat);
        assert_ne!(when(Condition::Any(vec![Condition::All(vec![a.clone(), b.clone()]), c.clone()])).hash(), flat);
        let mixed = Condition::All(vec![a.clone(), Condition::Any(vec![b.clone(), c.clone()])]);
        assert_ne!(when(mixed).hash(), when(Condition::All(vec![a, b, c])).hash());
    }

    #[test]
    fn a_token_is_matched_without_regard_to_case() {
        let token = |id: &str| strategy(Node::Asset(Asset::Token { id: id.to_string() }));
        let lower = token("0xabcdef0000000000000000000000000000000001:56");
        assert_eq!(token("0xABCDEF0000000000000000000000000000000001:56").hash(), lower.hash());
        assert_ne!(token("0xabcdef0000000000000000000000000000000001:8453").hash(), lower.hash());
        assert_eq!(lower.canonical_json(), r#"{"rebalance":{"frequency":"monthly","type":"calendar"},"root":{"asset":{"token":"0xabcdef0000000000000000000000000000000001:56"},"block":"asset"}}"#);
    }

    /// 60% SPY, and 40% in BIL while the 10 day RSI of QQQ is above 79, else QQQ.
    const WRITTEN_ONE_WAY: &str = r#"{
        "id": "doc-1", "step": "root", "name": "One", "description": "", "rebalance": "none", "rebalance-corridor-width": 0.05,
        "children": [{ "id": "a", "step": "wt-cash-specified", "children": [
            { "id": "b", "step": "asset", "ticker": "SPY", "weight": { "num": "60", "den": 100 } },
            { "id": "c", "step": "if", "weight": { "num": 40, "den": 100 }, "children": [
                { "id": "d", "step": "if-child", "is-else-condition?": false,
                  "lhs-fn": "relative-strength-index", "lhs-val": "QQQ", "lhs-fn-params": { "window": 10 },
                  "comparator": "gt", "rhs-fixed-value?": true, "rhs-val": "79",
                  "children": [{ "id": "e", "step": "asset", "ticker": "BIL" }] },
                { "id": "f", "step": "if-child", "is-else-condition?": true,
                  "children": [{ "id": "g", "step": "asset", "ticker": "QQQ" }] }
            ] }
        ] }]
    }"#;

    /// The same strategy as another editor might leave it: other ids, keys
    /// in another order, numbers as text and text as numbers, long tickers,
    /// the window in its older field, the condition as an object, display
    /// state on every block.
    const WRITTEN_ANOTHER_WAY: &str = r##"{
        "children": [{ "collapsed?": true, "children": [
            { "weight": { "den": "100", "num": 60.0 }, "exchange": "ARCX", "name": "A fund", "ticker": "EQUITIES::SPY//USD", "step": "asset", "id": "9d2f" },
            { "children": [
                { "children": [{ "price": 91.5, "ticker": "BIL", "step": "asset", "id": "77aa" }],
                  "condition": { "rhs": { "constant": 79 }, "comparator": "gt", "condition-type": "binary",
                                 "lhs": { "ticker": "EQUITIES::QQQ//USD", "params": { "window": "10" }, "fn": "relative-strength-index" } },
                  "rhs-val": "12", "rhs-fixed-value?": true, "comparator": "lt", "lhs-window-days": "200", "lhs-val": "SPY",
                  "lhs-fn": "cumulative-return", "is-else-condition?": false, "step": "if-child", "id": "0b1c" },
                { "children": [{ "ticker": "QQQ", "step": "asset", "id": "31d0", "has_marketcap": false }],
                  "is-else-condition?": true, "step": "if-child", "id": "c2e4", "collapsed?": false }
              ], "weight": { "den": 5, "num": "2" }, "step": "if", "name": "Hedge", "id": "5f6a" }
        ], "step": "wt-cash-specified", "id": "e8b7" }],
        "rebalance-corridor-width": 5e-2, "rebalance": "none", "color": "#AEC3C6", "asset_class": "EQUITIES",
        "description": "Told differently", "name": "Two", "step": "root", "id": "doc-2"
    }"##;

    #[test]
    fn the_same_meaning_hashes_the_same_however_it_was_written_down() {
        let one = import_json(WRITTEN_ONE_WAY).expect("imports");
        let another = import_json(WRITTEN_ANOTHER_WAY).expect("imports");
        assert_ne!(one.name, another.name);
        assert_eq!(one.canonical_json(), another.canonical_json());
        assert_eq!(one.hash(), another.hash());

        // The window may sit in the older field and the tickers in a group.
        let older = WRITTEN_ONE_WAY.replace(r#""lhs-fn-params": { "window": 10 }"#, r#""lhs-window-days": "10""#);
        assert_ne!(older, WRITTEN_ONE_WAY);
        assert_eq!(import_json(&older).expect("imports").hash(), one.hash());
    }

    #[test]
    fn a_change_of_meaning_changes_the_hash() {
        let one = import_json(WRITTEN_ONE_WAY).expect("imports").hash();
        let changes = [
            (r#""rhs-val": "79""#, r#""rhs-val": "80""#),
            (r#""comparator": "gt""#, r#""comparator": "gte""#),
            (r#""window": 10"#, r#""window": 11"#),
            (r#""lhs-val": "QQQ""#, r#""lhs-val": "SPY""#),
            (r#""lhs-fn": "relative-strength-index""#, r#""lhs-fn": "cumulative-return""#),
            (r#""ticker": "BIL""#, r#""ticker": "SHY""#),
            (r#""ticker": "SPY""#, r#""ticker": "CRYPTO::SPY//USD""#),
            (r#""num": "60", "den": 100"#, r#""num": "50", "den": 100"#),
            (r#""rebalance-corridor-width": 0.05"#, r#""rebalance-corridor-width": 0.06"#),
            (r#""rebalance": "none""#, r#""rebalance": "daily""#),
            (r#""step": "wt-cash-specified""#, r#""step": "wt-cash-equal""#),
        ];
        let mut seen = vec![one.clone()];
        for (from, to) in changes {
            let changed = WRITTEN_ONE_WAY.replace(from, to);
            assert_ne!(changed, WRITTEN_ONE_WAY, "{from}");
            let hash = import_json(&changed).expect("imports").hash();
            assert!(!seen.contains(&hash), "{from} -> {to}");
            seen.push(hash);
        }

        // Child order is meaning: shares go by position.
        let mut swapped: Value = serde_json::from_str(WRITTEN_ONE_WAY).expect("json");
        swapped["children"][0]["children"].as_array_mut().expect("children").reverse();
        assert!(!seen.contains(&import(&swapped).expect("imports").hash()));
        // So is which branch is which.
        let mut branches: Value = serde_json::from_str(WRITTEN_ONE_WAY).expect("json");
        let (then, otherwise) = (branches["children"][0]["children"][1]["children"][0]["children"].take(), branches["children"][0]["children"][1]["children"][1]["children"].take());
        branches["children"][0]["children"][1]["children"][0]["children"] = otherwise;
        branches["children"][0]["children"][1]["children"][1]["children"] = then;
        assert!(!seen.contains(&import(&branches).expect("imports").hash()));
    }
}
