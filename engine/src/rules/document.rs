//! A strategy as a document: the form it is stored in and sent in.
//!
//! The document is the canonical form with the names put back: the
//! strategy's own, and each weight block's. So what is stored reads like what
//! is hashed, and a name is the one thing a document says that the hash does
//! not see.
//!
//! ```text
//! { "name": "Trend", "rebalance": { "type": "calendar", "frequency": "daily" }, "root": <block> }
//!
//! <block>     { "block": "asset", "asset": <asset> }
//!             { "block": "weight", "name": "Bonds", "weighting": <weighting>, "children": [<block>] }
//!             { "block": "if", "condition": <condition>, "then": [<block>], "else": [<block>] }
//!             { "block": "filter", "sort_by": <indicator>, "take": "top", "count": 2, "children": [<block>] }
//! <asset>     { "market": "equities", "symbol": "SPY" }   or   { "token": "0x...:56" }
//! <weighting> { "type": "equal" }   { "type": "specified", "shares": [0.6, 0.4] }
//!             { "type": "inverse_volatility", "window": 20 }
//! <indicator> { "indicator": "relative_strength", "window": 14 }   { "indicator": "price" }
//! <condition> { "compare": "above", "left": <reading>, "right": <reading> or { "fixed": 0.7 } }
//!             { "any": [<condition>] }   { "all": [<condition>] }
//! <reading>   an indicator with an "asset" beside it
//! ```
//!
//! Reading is closed: a key that is not known is a problem, so a slip in a
//! document written by hand or by a machine is said and not skipped. Reading
//! only translates. Call `validate` on the tree to learn whether it can be
//! evaluated.

use super::{
    canonical,
    import::{nesting, parse, too_deep, MAX_NESTING},
    Asset, Comparator, Comparison, Condition, Frequency, Indicator, Issue, IssueCode, Market, Node, Operand, Reading, Rebalance, RuleStrategy, Take,
    Trail, Weighting,
};
use serde_json::{json, Map, Value};

type Fields = Map<String, Value>;
type Group = fn(Vec<Condition>) -> Condition;

const MARKETS: [(&str, Market); 2] = [("equities", Market::Equities), ("crypto", Market::Crypto)];
const TAKES: [(&str, Take); 2] = [("top", Take::Top), ("bottom", Take::Bottom)];
const GROUPS: [(&str, Group); 2] = [("any", Condition::Any), ("all", Condition::All)];
const COMPARATORS: [(&str, Comparator); 5] = [
    ("above", Comparator::Above),
    ("at_least", Comparator::AtLeast),
    ("equal", Comparator::Equal),
    ("below", Comparator::Below),
    ("at_most", Comparator::AtMost),
];
/// Every indicator that looks back over a window, by its name.
const WINDOWED: [(&str, fn(u32) -> Indicator); 8] = [
    ("average_price", |window| Indicator::AveragePrice { window }),
    ("exponential_average_price", |window| Indicator::ExponentialAveragePrice { window }),
    ("price_deviation", |window| Indicator::PriceDeviation { window }),
    ("cumulative_return", |window| Indicator::CumulativeReturn { window }),
    ("average_return", |window| Indicator::AverageReturn { window }),
    ("return_deviation", |window| Indicator::ReturnDeviation { window }),
    ("max_drawdown", |window| Indicator::MaxDrawdown { window }),
    ("relative_strength", |window| Indicator::RelativeStrength { window }),
];

impl RuleStrategy {
    /// The strategy as a document. Reading it back gives a strategy with the
    /// same hash and the same names.
    pub fn document(&self) -> Value {
        json!({ "name": self.name, "rebalance": canonical::rebalance(&self.rebalance), "root": named(&self.root) })
    }

    /// Read a document that is already parsed. Every problem in it comes
    /// back instead when it has any.
    pub fn from_document(document: &Value) -> Result<RuleStrategy, Vec<Issue>> {
        if nesting(document) > MAX_NESTING {
            return Err(too_deep());
        }
        let mut reader = Reader::default();
        match reader.strategy(document) {
            Some(strategy) if reader.trail.issues.is_empty() => Ok(strategy),
            _ => Err(reader.trail.issues),
        }
    }

    /// Read a document from JSON text.
    pub fn from_document_json(text: &str) -> Result<RuleStrategy, Vec<Issue>> {
        RuleStrategy::from_document(&parse(text)?)
    }
}

/// A block in its canonical form, with a weight block's name beside it.
fn named(node: &Node) -> Value {
    let mut value = canonical::block(node, &mut named);
    if let Node::Weight { name: Some(name), .. } = node {
        value["name"] = json!(name);
    }
    value
}

/// Every method that returns `None` has reported why first.
#[derive(Default)]
struct Reader {
    trail: Trail,
    /// Where a problem is, while reading something outside the tree of blocks.
    outside: Option<&'static str>,
}

impl Reader {
    fn refuse<T>(&mut self, code: IssueCode, message: impl Into<String>, fix: impl Into<String>) -> Option<T> {
        match self.outside {
            Some(place) => self.trail.report_at(place, code, message, fix),
            None => self.trail.report(code, message, fix),
        }
        None
    }

    /// The fields of an object that may hold `known` and nothing else.
    fn fields<'a>(&mut self, value: &'a Value, what: &str, known: &[&str]) -> Option<&'a Fields> {
        let Some(fields) = value.as_object() else {
            return self.refuse(IssueCode::InvalidStructure, format!("{what} is {value}, which is not an object."), "Write it as an object in braces.");
        };
        let unknown: Vec<&str> = fields.keys().map(String::as_str).filter(|key| !known.contains(key)).collect();
        if !unknown.is_empty() {
            return self.refuse(
                IssueCode::UnknownField,
                format!("{what} has {} that it cannot have: {}.", if unknown.len() == 1 { "a field" } else { "fields" }, unknown.join(", ")),
                format!("Use only: {}.", known.join(", ")),
            );
        }
        Some(fields)
    }

    fn need<'a>(&mut self, fields: &'a Fields, key: &str) -> Option<&'a Value> {
        match fields.get(key) {
            Some(value) => Some(value),
            None => self.refuse(IssueCode::MissingField, format!("\"{key}\" is missing."), format!("Add \"{key}\".")),
        }
    }

    fn text<'a>(&mut self, fields: &'a Fields, key: &str) -> Option<&'a str> {
        let value = self.need(fields, key)?;
        match value.as_str() {
            Some(text) => Some(text),
            None => self.refuse(IssueCode::InvalidField, format!("\"{key}\" is {value}, which is not text."), "Write it in quotes."),
        }
    }

    /// The value of a field that takes one of a few words.
    fn choice<T: Copy>(&mut self, fields: &Fields, key: &str, words: &[(&str, T)]) -> Option<T> {
        let word = self.text(fields, key)?;
        match words.iter().find(|(known, _)| *known == word) {
            Some((_, value)) => Some(*value),
            None => {
                let known: Vec<&str> = words.iter().map(|(known, _)| *known).collect();
                self.refuse(IssueCode::InvalidField, format!("\"{key}\" is {word:?}, which is not a word it takes."), format!("Use one of: {}.", known.join(", ")))
            }
        }
    }

    fn number(&mut self, value: &Value, what: &str) -> Option<f64> {
        match value.as_f64() {
            Some(number) => Some(number),
            None => self.refuse(IssueCode::InvalidField, format!("{what} is {value}, which is not a number."), "Write a plain number, with no quotes."),
        }
    }

    /// A count of days or of blocks: a whole number.
    fn whole(&mut self, fields: &Fields, key: &str) -> Option<u32> {
        let value = self.need(fields, key)?;
        match value.as_u64().and_then(|whole| u32::try_from(whole).ok()) {
            Some(whole) => Some(whole),
            None => self.refuse(IssueCode::InvalidField, format!("\"{key}\" is {value}, which is not a whole number."), "Write a whole number, such as 20."),
        }
    }

    /// What is under `key`, read with the trail pointing at it.
    fn child<T>(&mut self, fields: &Fields, key: &'static str, read: impl FnOnce(&mut Self, &Value) -> Option<T>) -> Option<T> {
        let value = self.need(fields, key)?;
        self.trail.enter(key);
        let read = read(self, value);
        self.trail.leave();
        read
    }

    /// The list under `key`, each item read with the trail pointing at it.
    /// Every item is read, so every problem in the list is found.
    fn each<T>(&mut self, fields: &Fields, key: &'static str, mut read: impl FnMut(&mut Self, &Value) -> Option<T>) -> Option<Vec<T>> {
        let value = self.need(fields, key)?;
        let Some(items) = value.as_array() else {
            return self.refuse(IssueCode::InvalidField, format!("\"{key}\" is not a list."), "Write it as a list in square brackets.");
        };
        let mut all = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            self.trail.enter_item(key, index);
            all.push(read(self, item));
            self.trail.leave();
        }
        all.into_iter().collect()
    }

    fn strategy(&mut self, document: &Value) -> Option<RuleStrategy> {
        self.outside = Some("strategy");
        let fields = self.fields(document, "A strategy", &["name", "rebalance", "root"])?;
        let name = match fields.get("name") {
            Some(_) => self.text(fields, "name").map(str::to_string),
            None => Some(String::new()),
        };
        self.outside = Some("rebalance");
        let rebalance = self.need(fields, "rebalance").and_then(|value| self.rebalance(value));
        self.outside = Some("strategy");
        let root = self.need(fields, "root");
        self.outside = None;
        let root = root.and_then(|value| self.block(value));
        Some(RuleStrategy { name: name?, rebalance: rebalance?, root: root? })
    }

    fn rebalance(&mut self, value: &Value) -> Option<Rebalance> {
        let kind = value.get("type").and_then(Value::as_str);
        match kind {
            Some("calendar") => {
                let fields = self.fields(value, "A calendar rebalance", &["type", "frequency"])?;
                let word = self.text(fields, "frequency")?;
                match Frequency::named(word) {
                    Some(frequency) => Some(Rebalance::Calendar(frequency)),
                    None => self.refuse(
                        IssueCode::InvalidRebalance,
                        format!("{word:?} is not a rebalance frequency."),
                        "Use one of: daily, weekly, monthly, quarterly, yearly.",
                    ),
                }
            }
            Some("threshold") => {
                let fields = self.fields(value, "A threshold rebalance", &["type", "corridor"])?;
                let corridor = self.need(fields, "corridor")?;
                Some(Rebalance::Threshold { corridor: self.number(corridor, "The corridor")? })
            }
            _ => self.refuse(
                IssueCode::InvalidRebalance,
                "The rebalance rule does not say which kind it is.",
                "Give it a \"type\" of calendar, with a frequency, or of threshold, with a corridor.",
            ),
        }
    }

    fn block(&mut self, value: &Value) -> Option<Node> {
        match value.get("block").and_then(Value::as_str) {
            Some("asset") => {
                let fields = self.fields(value, "An asset block", &["block", "asset"])?;
                Some(Node::Asset(self.child(fields, "asset", Self::asset)?))
            }
            Some("weight") => {
                let fields = self.fields(value, "A weight block", &["block", "name", "weighting", "children"])?;
                let name = match fields.get("name") {
                    Some(_) => self.text(fields, "name").map(|name| Some(name.to_string())),
                    None => Some(None),
                };
                let weighting = self.child(fields, "weighting", Self::weighting);
                let children = self.each(fields, "children", Self::block);
                Some(Node::Weight { name: name?, weighting: weighting?, children: children? })
            }
            Some("if") => {
                let fields = self.fields(value, "An if block", &["block", "condition", "then", "else"])?;
                let condition = self.child(fields, "condition", Self::condition);
                let then = self.each(fields, "then", Self::block);
                let otherwise = self.each(fields, "else", Self::block);
                Some(Node::If { condition: condition?, then: then?, otherwise: otherwise? })
            }
            Some("filter") => {
                let fields = self.fields(value, "A filter block", &["block", "sort_by", "take", "count", "children"])?;
                let sort_by = self.child(fields, "sort_by", |reader, value| {
                    let fields = reader.fields(value, "An indicator", &["indicator", "window"])?;
                    reader.indicator(fields)
                });
                let take = self.choice(fields, "take", &TAKES);
                let count = self.whole(fields, "count");
                let children = self.each(fields, "children", Self::block);
                Some(Node::Filter { sort_by: sort_by?, take: take?, count: count?, children: children? })
            }
            _ => self.refuse(IssueCode::InvalidBlockType, "This does not say which block it is.", "Give it a \"block\" of asset, weight, if or filter."),
        }
    }

    fn asset(&mut self, value: &Value) -> Option<Asset> {
        if value.get("token").is_some() {
            let fields = self.fields(value, "A token", &["token"])?;
            // One token is one value, whatever case its address was typed in.
            return Some(Asset::Token { id: self.text(fields, "token")?.to_ascii_lowercase() });
        }
        let fields = self.fields(value, "An asset", &["market", "symbol"])?;
        let market = self.choice(fields, "market", &MARKETS);
        let symbol = self.text(fields, "symbol");
        Some(Asset::Listed { market: market?, symbol: symbol?.to_string() })
    }

    fn weighting(&mut self, value: &Value) -> Option<Weighting> {
        match value.get("type").and_then(Value::as_str) {
            Some("equal") => self.fields(value, "An equal weighting", &["type"]).map(|_| Weighting::Equal),
            Some("specified") => {
                let fields = self.fields(value, "A specified weighting", &["type", "shares"])?;
                Some(Weighting::Specified { shares: self.each(fields, "shares", |reader, share| reader.number(share, "A share"))? })
            }
            Some("inverse_volatility") => {
                let fields = self.fields(value, "An inverse volatility weighting", &["type", "window"])?;
                Some(Weighting::InverseVolatility { window: self.whole(fields, "window")? })
            }
            _ => self.refuse(
                IssueCode::InvalidField,
                "The weighting does not say which kind it is.",
                "Give it a \"type\" of equal, specified or inverse_volatility.",
            ),
        }
    }

    /// The indicator named in `fields`, with its window when it takes one.
    fn indicator(&mut self, fields: &Fields) -> Option<Indicator> {
        let name = self.text(fields, "indicator")?;
        if name == "price" {
            return match fields.contains_key("window") {
                true => self.refuse(IssueCode::InvalidWindow, "The price has no window.", "Take the window out, or use average_price."),
                false => Some(Indicator::Price),
            };
        }
        let Some((_, windowed)) = WINDOWED.iter().find(|(known, _)| *known == name) else {
            let known: Vec<&str> = WINDOWED.iter().map(|(known, _)| *known).collect();
            return self.refuse(IssueCode::InvalidIndicator, format!("{name:?} is not an indicator."), format!("Use one of: price, {}.", known.join(", ")));
        };
        Some(windowed(self.whole(fields, "window")?))
    }

    fn reading(&mut self, value: &Value) -> Option<Reading> {
        let fields = self.fields(value, "A reading", &["indicator", "window", "asset"])?;
        let indicator = self.indicator(fields);
        let asset = self.child(fields, "asset", Self::asset);
        Some(Reading { indicator: indicator?, asset: asset? })
    }

    fn condition(&mut self, value: &Value) -> Option<Condition> {
        for (word, group) in GROUPS {
            if value.get(word).is_some() {
                let fields = self.fields(value, "A group of conditions", &[word])?;
                return self.each(fields, word, Self::condition).map(group);
            }
        }
        let fields = self.fields(value, "A condition", &["compare", "left", "right"])?;
        let comparator = self.choice(fields, "compare", &COMPARATORS);
        let left = self.child(fields, "left", Self::reading);
        let right = self.child(fields, "right", |reader, value| match value.get("fixed") {
            Some(fixed) => {
                reader.fields(value, "A fixed value", &["fixed"])?;
                Some(Operand::Fixed(reader.number(fixed, "The fixed value")?))
            }
            None => reader.reading(value).map(Operand::Reading),
        });
        Some(Condition::Compare(Comparison { left: left?, comparator: comparator?, right: right? }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::MAX_NODES;

    fn listed(symbol: &str) -> Asset {
        Asset::Listed { market: Market::Equities, symbol: symbol.to_string() }
    }

    fn asset(symbol: &str) -> Node {
        Node::Asset(listed(symbol))
    }

    fn reading(indicator: Indicator, symbol: &str) -> Reading {
        Reading { indicator, asset: listed(symbol) }
    }

    /// A strategy that uses every block, every weighting, every indicator
    /// and both kinds of right-hand side.
    fn everything() -> RuleStrategy {
        let indicators = [Indicator::Price].into_iter().chain(WINDOWED.iter().map(|(_, windowed)| windowed(10)));
        let conditions: Vec<Condition> = indicators
            .zip(COMPARATORS.iter().cycle())
            .map(|(indicator, (_, comparator))| {
                Condition::Compare(Comparison { left: reading(indicator, "SPY"), comparator: *comparator, right: Operand::Fixed(0.5) })
            })
            .collect();
        let trend = Condition::Compare(Comparison {
            left: reading(Indicator::Price, "SPY"),
            comparator: Comparator::Above,
            right: Operand::Reading(reading(Indicator::AveragePrice { window: 200 }, "SPY")),
        });
        let bonds = Node::Weight {
            name: Some("Bonds".to_string()),
            weighting: Weighting::InverseVolatility { window: 20 },
            children: vec![asset("TLT"), asset("IEF"), Node::Asset(Asset::Token { id: "0x00000000000000000000000000000000000000aa:56".to_string() })],
        };
        let stocks = Node::Filter { sort_by: Indicator::CumulativeReturn { window: 60 }, take: Take::Top, count: 2, children: vec![asset("QQQ"), asset("SPY"), asset("BRK/B")] };
        let core = Node::Weight { name: None, weighting: Weighting::Specified { shares: vec![0.6, 0.4] }, children: vec![stocks, bonds.clone()] };
        let root = Node::If { condition: Condition::All(vec![trend, Condition::Any(conditions)]), then: vec![core], otherwise: vec![bonds, asset("BIL")] };
        RuleStrategy { name: "Everything".to_string(), rebalance: Rebalance::Threshold { corridor: 0.05 }, root }
    }

    fn issues(document: Value) -> Vec<(IssueCode, String)> {
        RuleStrategy::from_document(&document).expect_err("a problem").into_iter().map(|issue| (issue.code, issue.path)).collect()
    }

    #[test]
    fn a_document_reads_back_as_the_same_strategy() {
        let strategy = everything();
        assert!(strategy.validate(MAX_NODES).is_empty());
        let read = RuleStrategy::from_document(&strategy.document()).expect("a sound document");
        assert_eq!(read, strategy);
        assert_eq!(read.hash(), strategy.hash());
        // Through text too, and for each schedule.
        for frequency in Frequency::ALL {
            let scheduled = RuleStrategy { rebalance: Rebalance::Calendar(frequency), ..strategy.clone() };
            assert_eq!(RuleStrategy::from_document_json(&scheduled.document().to_string()), Ok(scheduled));
        }
    }

    #[test]
    fn a_strategy_lists_every_asset_it_holds_or_reads_once() {
        let token = Asset::Token { id: "0x00000000000000000000000000000000000000aa:56".to_string() };
        let expected: Vec<Asset> = ["BIL", "BRK/B", "IEF", "QQQ", "SPY", "TLT"].into_iter().map(listed).chain([token]).collect();
        assert_eq!(everything().assets(), expected);
    }

    #[test]
    fn a_document_is_the_canonical_form_with_the_names_beside_it() {
        let strategy = everything();
        let mut document = strategy.document();
        assert_eq!(document["name"], "Everything");
        assert_eq!(document["root"]["else"][0]["name"], "Bonds");
        // Without its names it is the canonical text itself.
        document.as_object_mut().unwrap().remove("name");
        document["root"]["else"][0].as_object_mut().unwrap().remove("name");
        document["root"]["then"][0]["children"][1].as_object_mut().unwrap().remove("name");
        assert_eq!(canonical::text(&document), strategy.canonical_json());
    }

    #[test]
    fn a_name_is_optional_and_a_token_is_one_value_in_any_case() {
        let document = json!({
            "rebalance": { "type": "calendar", "frequency": "monthly" },
            "root": { "block": "asset", "asset": { "token": "0x00000000000000000000000000000000000000AA:56" } }
        });
        let strategy = RuleStrategy::from_document(&document).expect("a sound document");
        assert_eq!(strategy.name, "");
        assert_eq!(strategy.root, Node::Asset(Asset::Token { id: "0x00000000000000000000000000000000000000aa:56".to_string() }));
    }

    #[test]
    fn every_problem_is_reported_with_its_place() {
        let document = json!({
            "name": 7,
            "rebalance": { "type": "calendar", "frequency": "hourly" },
            "root": { "block": "weight", "weighting": { "type": "equal" }, "children": [
                { "block": "asset", "asset": { "market": "bonds", "symbol": "TLT" } },
                { "block": "asset", "asset": { "market": "equities", "symbol": "SPY" }, "colour": "red" },
                { "block": "if",
                  "condition": { "compare": "over", "left": { "indicator": "price", "window": 5, "asset": { "market": "equities", "symbol": "SPY" } }, "right": { "fixed": "high" } },
                  "then": [{ "block": "group" }] },
                { "block": "filter", "sort_by": { "indicator": "momentum", "window": 5 }, "take": "top", "count": 1.5, "children": {} }
            ] }
        });
        assert_eq!(
            issues(document),
            [
                (IssueCode::InvalidField, "strategy".to_string()),
                (IssueCode::InvalidRebalance, "rebalance".to_string()),
                (IssueCode::InvalidField, "root.children[0].asset".to_string()),
                (IssueCode::UnknownField, "root.children[1]".to_string()),
                (IssueCode::InvalidField, "root.children[2].condition".to_string()),
                (IssueCode::InvalidWindow, "root.children[2].condition.left".to_string()),
                (IssueCode::InvalidField, "root.children[2].condition.right".to_string()),
                (IssueCode::InvalidBlockType, "root.children[2].then[0]".to_string()),
                (IssueCode::MissingField, "root.children[2]".to_string()),
                (IssueCode::InvalidIndicator, "root.children[3].sort_by".to_string()),
                (IssueCode::InvalidField, "root.children[3]".to_string()),
                (IssueCode::InvalidField, "root.children[3]".to_string()),
            ]
        );
    }

    #[test]
    fn what_is_not_a_strategy_is_refused() {
        assert_eq!(issues(json!([1, 2])), [(IssueCode::InvalidStructure, "strategy".to_string())]);
        assert_eq!(issues(json!({ "root": { "block": "asset", "asset": { "market": "equities", "symbol": "SPY" } } })), [(IssueCode::MissingField, "rebalance".to_string())]);
        assert_eq!(issues(json!({ "rebalance": { "type": "threshold", "corridor": 0.05 } })), [(IssueCode::MissingField, "strategy".to_string())]);
        assert_eq!(RuleStrategy::from_document_json("{").expect_err("not JSON")[0].code, IssueCode::InvalidJson);
        let deep = format!("{}1{}", "[".repeat(MAX_NESTING + 1), "]".repeat(MAX_NESTING + 1));
        assert_eq!(RuleStrategy::from_document_json(&deep).expect_err("too deep")[0].code, IssueCode::ExcessiveDepth);
    }
}
