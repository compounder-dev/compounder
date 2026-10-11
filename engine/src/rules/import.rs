//! Import from the block-tree JSON that strategy editors write.
//!
//! That format is an editor's working file, and one fact has several
//! spellings in it. A window is a number or a string, in either of two
//! fields. A ticker is short or long. A condition is a row of flat fields or
//! a nested object. Import settles each on the one spelling the tree has, and
//! leaves out what changes no trade: ids, display flags, weights left over
//! from an earlier parent.
//!
//! Blocks below the root are open: a key that is not known is skipped, so
//! old documents keep loading. The root and the condition objects are closed.

use super::{
    is_symbol, Asset, Comparator, Comparison, Condition, Frequency, Indicator, Issue, IssueCode, Market, Node, Operand, Reading, Rebalance,
    RuleStrategy, Take, Trail, Unit, Weighting,
};
use serde::Deserialize;
use serde_json::{Map, Value};

/// Deepest JSON nesting a document may have. A block costs two levels and an
/// if costs four, so every tree that `validate` accepts fits.
pub(super) const MAX_NESTING: usize = 512;
/// Most tickers one condition may list.
const MAX_TICKERS: usize = 50;

const ROOT_FIELDS: [&str; 15] = [
    "id",
    "step",
    "name",
    "description",
    "rebalance",
    "rebalance-corridor-width",
    "children",
    "asset_class",
    "asset_classes",
    "color",
    "sparkgraph_url",
    "weight",
    "collapsed?",
    "collapsed-specified-weight?",
    "suppress-incomplete-warnings?",
];
const INDICATORS: &str = "current-price, cumulative-return, exponential-moving-average-price, max-drawdown, moving-average-price, \
                          moving-average-return, relative-strength-index, standard-deviation-price, standard-deviation-return";
const COMPARATORS: [(&str, Comparator); 5] =
    [("gt", Comparator::Above), ("gte", Comparator::AtLeast), ("eq", Comparator::Equal), ("lt", Comparator::Below), ("lte", Comparator::AtMost)];
const GROUPS: [(&str, Group); 2] = [("any", Condition::Any), ("all", Condition::All)];
const TAKES: [(&str, Take); 2] = [("top", Take::Top), ("bottom", Take::Bottom)];

type Fields = Map<String, Value>;
type Group = fn(Vec<Condition>) -> Condition;

/// Where the flat form keeps one side of a comparison.
struct Side {
    function: &'static str,
    value: &'static str,
    params: &'static str,
    /// The older place for the window. Still read when `params` has none.
    window: &'static str,
}

const LEFT: Side = Side { function: "lhs-fn", value: "lhs-val", params: "lhs-fn-params", window: "lhs-window-days" };
const RIGHT: Side = Side { function: "rhs-fn", value: "rhs-val", params: "rhs-fn-params", window: "rhs-window-days" };

/// One side of a comparison in a condition object, before `%` is filled in.
enum Template {
    Fixed(f64),
    /// An indicator on a named asset, or on each listed ticker when `None`.
    Read(Indicator, Option<Asset>),
}

/// A comparison in a condition object, to be made once for each listed ticker.
struct Pattern {
    indicator: Indicator,
    asset: Option<Asset>,
    comparator: Comparator,
    right: Template,
}

impl Pattern {
    /// The comparison with `each` standing wherever the pattern says `%`.
    /// `None` when it says `%` and there is no ticker to stand there.
    fn on(&self, each: Option<&Asset>) -> Option<Comparison> {
        let fill = |asset: &Option<Asset>| asset.as_ref().or(each).cloned();
        let right = match &self.right {
            Template::Fixed(value) => Operand::Fixed(scaled(*value, self.indicator.unit())),
            Template::Read(indicator, asset) => Operand::Reading(Reading { indicator: *indicator, asset: fill(asset)? }),
        };
        Some(Comparison { left: Reading { indicator: self.indicator, asset: fill(&self.asset)? }, comparator: self.comparator, right })
    }
}

/// A fixed value is typed in percent against a percent indicator and held as
/// a fraction. The division is a plain one, so 99.9 becomes
/// 0.9990000000000001, the same digits these documents are evaluated with.
/// Dollar values are held as typed.
fn scaled(value: f64, unit: Unit) -> f64 {
    match unit {
        Unit::Percent => value / 100.0,
        Unit::Dollars => value,
    }
}

/// A field that is there and not null.
fn present<'a>(fields: &'a Fields, key: &str) -> Option<&'a Value> {
    fields.get(key).filter(|value| !value.is_null())
}

/// A field that is there, not null and not empty text.
fn given<'a>(fields: &'a Fields, key: &str) -> Option<&'a Value> {
    present(fields, key).filter(|value| value.as_str() != Some(""))
}

/// A number, written as one or as text. Stored documents hold both.
fn number(value: &Value) -> Option<f64> {
    let number = match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    };
    number.filter(|number: &f64| number.is_finite())
}

fn whole(value: &Value) -> Option<u32> {
    number(value).filter(|number| *number >= 0.0 && *number <= f64::from(u32::MAX) && number.fract() == 0.0).map(|number| number as u32)
}

/// A value as it reads in a message.
fn shown(value: &Value) -> String {
    match value {
        Value::Array(_) => "a list".to_string(),
        Value::Object(_) => "an object".to_string(),
        Value::String(text) if text.chars().count() > 40 => format!("{:?}", text.chars().take(40).collect::<String>() + "..."),
        other => other.to_string(),
    }
}

/// One branch of an if block.
fn branch(item: &Value) -> Option<&Fields> {
    item.as_object().filter(|branch| branch.get("step").and_then(Value::as_str) == Some("if-child"))
}

/// A block's name, when it has one.
fn name(fields: &Fields) -> Option<String> {
    fields.get("name").and_then(Value::as_str).filter(|name| !name.is_empty()).map(str::to_string)
}

/// A ticker in its short form, `SPY`, or its long one, `EQUITIES::SPY//USD`,
/// either with an `@` and an exchange code at the end. The quote currency
/// and the exchange are dropped: they change nothing that is held.
fn listed(ticker: &str) -> Option<Asset> {
    let (class, rest) = ticker.split_once("::").unwrap_or(("", ticker));
    let rest = match rest.split_once('@') {
        Some((rest, exchange)) if exchange.len() >= 4 && exchange.bytes().all(|byte| byte.is_ascii_uppercase()) => rest,
        Some(_) => return None,
        None => rest,
    };
    let symbol = rest.strip_suffix("//USD").unwrap_or(rest);
    let market = match class {
        "" | "EQUITIES" => Market::Equities,
        "CRYPTO" => Market::Crypto,
        _ => return None,
    };
    is_symbol(symbol).then(|| Asset::Listed { market, symbol: symbol.to_string() })
}

/// How deep the brackets in `text` nest. Counted without following the
/// nesting, so the answer comes back however deep it goes.
fn text_nesting(text: &str) -> usize {
    let (mut depth, mut deepest, mut in_text, mut escaped) = (0usize, 0usize, false, false);
    for byte in text.bytes() {
        if in_text {
            match byte {
                _ if escaped => escaped = false,
                b'\\' => escaped = true,
                b'"' => in_text = false,
                _ => {}
            }
        } else {
            match byte {
                b'"' => in_text = true,
                b'{' | b'[' => {
                    depth += 1;
                    deepest = deepest.max(depth);
                }
                b'}' | b']' => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
    }
    deepest
}

/// The same for a parsed value, walked with a list of its own in place of
/// the call stack.
pub(super) fn nesting(value: &Value) -> usize {
    let mut deepest = 0;
    let mut waiting = vec![(value, 1)];
    while let Some((value, depth)) = waiting.pop() {
        match value {
            Value::Array(items) => {
                deepest = deepest.max(depth);
                waiting.extend(items.iter().map(|item| (item, depth + 1)));
            }
            Value::Object(fields) => {
                deepest = deepest.max(depth);
                waiting.extend(fields.values().map(|item| (item, depth + 1)));
            }
            _ => {}
        }
    }
    deepest
}

pub(super) fn too_deep() -> Vec<Issue> {
    vec![Issue {
        code: IssueCode::ExcessiveDepth,
        path: "root".to_string(),
        message: format!("The document nests more than {MAX_NESTING} levels deep."),
        fix: "Reduce the nesting.".to_string(),
    }]
}

/// Read a block-tree document from JSON text. Returns every problem in the
/// document instead when it has any. Import only translates: call `validate`
/// on the tree to learn whether it can be evaluated.
pub fn import_json(text: &str) -> Result<RuleStrategy, Vec<Issue>> {
    import(&parse(text)?)
}

/// Parse JSON text that may nest as deep as a strategy does. A stock parser
/// stops far short of that, and one with no limit can be made to run out of
/// stack, so anything that holds a strategy is parsed here.
pub fn parse(text: &str) -> Result<Value, Vec<Issue>> {
    // A parser needs stack for every level of nesting it follows, so the
    // depth is checked on the text before anything follows it.
    if text_nesting(text) > MAX_NESTING {
        return Err(too_deep());
    }
    let mut parser = serde_json::Deserializer::from_str(text);
    parser.disable_recursion_limit();
    Value::deserialize(&mut parser).and_then(|tree| parser.end().map(|()| tree)).map_err(|error| {
        vec![Issue {
            code: IssueCode::InvalidJson,
            path: "root".to_string(),
            message: format!("The document is not valid JSON: {error}."),
            fix: "Send the strategy as one JSON object.".to_string(),
        }]
    })
}

/// Read a block-tree document that is already parsed.
pub fn import(tree: &Value) -> Result<RuleStrategy, Vec<Issue>> {
    if nesting(tree) > MAX_NESTING {
        return Err(too_deep());
    }
    let mut reader = Reader::default();
    match reader.root(tree) {
        Some(strategy) if reader.trail.issues.is_empty() => Ok(strategy),
        _ => Err(reader.trail.issues),
    }
}

/// Every method that returns `None` has reported why first.
#[derive(Default)]
struct Reader {
    trail: Trail,
}

impl Reader {
    fn refuse<T>(&mut self, code: IssueCode, message: impl Into<String>, fix: impl Into<String>) -> Option<T> {
        self.trail.report(code, message, fix);
        None
    }

    fn text<'a>(&mut self, fields: &'a Fields, key: &'static str) -> Option<&'a str> {
        match present(fields, key) {
            Some(Value::String(text)) => Some(text),
            Some(other) => self.refuse(IssueCode::InvalidField, format!("{key} is {}, not text.", shown(other)), "Write it as text."),
            None => self.refuse(IssueCode::MissingField, format!("This block has no {key}."), format!("Add a {key} field.")),
        }
    }

    /// A field that must hold one of a few words.
    fn word<T: Copy>(&mut self, fields: &Fields, key: &'static str, words: &[(&'static str, T)]) -> Option<T> {
        let text = self.text(fields, key)?;
        match words.iter().find(|(word, _)| *word == text) {
            Some((_, value)) => Some(*value),
            None => {
                let allowed: Vec<&str> = words.iter().map(|(word, _)| *word).collect();
                self.refuse(IssueCode::InvalidField, format!("{key} is {text:?}."), format!("Use one of: {}.", allowed.join(", ")))
            }
        }
    }

    /// A true-or-false field. Absent reads as false.
    fn flag(&mut self, fields: &Fields, key: &'static str) -> Option<bool> {
        match present(fields, key) {
            None => Some(false),
            Some(Value::Bool(flag)) => Some(*flag),
            Some(other) => self.refuse(IssueCode::InvalidField, format!("{key} is {}, not true or false.", shown(other)), "Write true or false."),
        }
    }

    fn list<'a>(&mut self, fields: &'a Fields, key: &'static str) -> Option<&'a [Value]> {
        match present(fields, key) {
            None => Some(&[]),
            Some(Value::Array(items)) => Some(items),
            Some(other) => self.refuse(IssueCode::InvalidField, format!("{key} is {}, not a list.", shown(other)), "Write it as a list."),
        }
    }

    fn days(&mut self, value: Option<&Value>) -> Option<u32> {
        match value.map(|value| (value, whole(value))) {
            Some((_, Some(days))) => Some(days),
            Some((value, None)) => self.refuse(
                IssueCode::InvalidWindow,
                format!("The window is {}, which is not a whole number of days.", shown(value)),
                "Use a whole number, such as 10.",
            ),
            None => self.refuse(IssueCode::MissingField, "This block has no window.", "Set the number of days it looks back over."),
        }
    }

    fn fixed(&mut self, value: Option<&Value>, key: &'static str) -> Option<f64> {
        match value.map(|value| (value, number(value))) {
            Some((_, Some(number))) => Some(number),
            Some((value, None)) => {
                self.refuse(IssueCode::InvalidFixedValue, format!("{key} is {}, which is not a number.", shown(value)), "Use a number, such as 30.")
            }
            None => self.refuse(IssueCode::MissingField, format!("This condition has no {key}."), "Set the number to compare with."),
        }
    }

    fn asset(&mut self, ticker: &str) -> Option<Asset> {
        match listed(ticker) {
            Some(asset) => Some(asset),
            None => self.refuse(
                IssueCode::InvalidAsset,
                format!("{ticker:?} is not a ticker this engine can read."),
                "Use a market symbol in capitals, such as SPY or BRK/B.",
            ),
        }
    }

    /// The indicator named in `function`. Its window is in `params`, or
    /// failing that in the `older` field some documents still use.
    fn indicator(&mut self, fields: &Fields, function: &'static str, params: &'static str, older: Option<&'static str>) -> Option<Indicator> {
        let build: fn(u32) -> Indicator = match self.text(fields, function)? {
            // The latest price has no window. A leftover one is not read.
            "current-price" => return Some(Indicator::Price),
            "moving-average-price" => |window| Indicator::AveragePrice { window },
            "exponential-moving-average-price" => |window| Indicator::ExponentialAveragePrice { window },
            "standard-deviation-price" => |window| Indicator::PriceDeviation { window },
            "cumulative-return" => |window| Indicator::CumulativeReturn { window },
            "moving-average-return" => |window| Indicator::AverageReturn { window },
            "standard-deviation-return" => |window| Indicator::ReturnDeviation { window },
            "max-drawdown" => |window| Indicator::MaxDrawdown { window },
            "relative-strength-index" => |window| Indicator::RelativeStrength { window },
            other => {
                return self.refuse(
                    IssueCode::InvalidIndicator,
                    format!("{other:?} is not an indicator this engine runs."),
                    format!("Use one of: {INDICATORS}."),
                )
            }
        };
        let newer = match present(fields, params) {
            None => None,
            Some(Value::Object(params)) => given(params, "window"),
            Some(other) => {
                return self.refuse(IssueCode::InvalidField, format!("{params} is {}, not an object.", shown(other)), "Write it as {\"window\": 10}.")
            }
        };
        self.days(newer.or_else(|| older.and_then(|key| given(fields, key)))).map(build)
    }

    fn root(&mut self, tree: &Value) -> Option<RuleStrategy> {
        let fields = match tree {
            Value::Object(fields) if fields.get("step").and_then(Value::as_str) == Some("root") => fields,
            _ => {
                return self.refuse(
                    IssueCode::InvalidStructure,
                    "The document does not start with a root block.",
                    "Send the whole strategy, from its root block down.",
                )
            }
        };
        self.closed(fields, &ROOT_FIELDS);
        let name = self.text(fields, "name");
        let rebalance = self.rebalance(fields);
        if let Some(class) = present(fields, "asset_class").filter(|class| !matches!(class.as_str(), Some("EQUITIES" | "CRYPTO"))) {
            self.trail.report(IssueCode::InvalidField, format!("asset_class is {}.", shown(class)), "Use one of: EQUITIES, CRYPTO.");
        }
        let root = match self.list(fields, "children")? {
            [] => self.refuse(IssueCode::IncompleteBlock, "The strategy is empty.", "Add a weight block or an asset."),
            [only] => self.blocks(std::slice::from_ref(only)).and_then(|mut blocks| blocks.pop()),
            several => {
                self.blocks(several);
                self.refuse(
                    IssueCode::InvalidStructure,
                    format!("The root holds {} blocks. It must hold exactly one.", several.len()),
                    "Put them inside one weight block.",
                )
            }
        };
        Some(RuleStrategy { name: name?.to_string(), rebalance: rebalance?, root: root? })
    }

    /// The root and the condition objects are closed: a key outside the list
    /// is a mistake to report, not a field to skip.
    fn closed(&mut self, fields: &Fields, allowed: &[&str]) {
        for key in fields.keys().filter(|key| !allowed.contains(&key.as_str())) {
            self.trail.report(
                IssueCode::UnknownField,
                format!("{key:?} is not a field that belongs here."),
                format!("Remove it. The fields are: {}.", allowed.join(", ")),
            );
        }
    }

    fn rebalance(&mut self, fields: &Fields) -> Option<Rebalance> {
        let word = self.text(fields, "rebalance")?;
        if let Some(frequency) = Frequency::named(word) {
            // A corridor width beside a calendar word is not read.
            return Some(Rebalance::Calendar(frequency));
        }
        if word != "none" {
            return self.refuse(
                IssueCode::InvalidField,
                format!("rebalance is {word:?}."),
                "Use one of: none, daily, weekly, monthly, quarterly, yearly.",
            );
        }
        // `none` is not "never". It is the threshold mode, and the corridor
        // width beside it is the threshold.
        match present(fields, "rebalance-corridor-width") {
            Some(Value::Number(width)) => width.as_f64().map(|corridor| Rebalance::Threshold { corridor }),
            Some(other) => self.refuse(
                IssueCode::InvalidRebalance,
                format!("rebalance-corridor-width is {}, not a number.", shown(other)),
                "Write it as a fraction, such as 0.05 for 5%.",
            ),
            None => self.refuse(
                IssueCode::InvalidRebalance,
                "The strategy rebalances on a threshold but has no corridor width.",
                "Set rebalance-corridor-width, such as 0.05 for 5%.",
            ),
        }
    }

    fn children(&mut self, fields: &Fields) -> Option<Vec<Node>> {
        let items = self.list(fields, "children")?;
        self.blocks(items)
    }

    fn blocks(&mut self, items: &[Value]) -> Option<Vec<Node>> {
        let mut blocks = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            self.trail.enter_item("children", index);
            blocks.push(self.block(item));
            self.trail.leave();
        }
        blocks.into_iter().collect()
    }

    fn block(&mut self, value: &Value) -> Option<Node> {
        let Value::Object(fields) = value else {
            return self.refuse(
                IssueCode::InvalidBlockType,
                format!("This block is {}, not an object.", shown(value)),
                "Write each block as an object with a step.",
            );
        };
        match self.text(fields, "step")? {
            "asset" => self.text(fields, "ticker").and_then(|ticker| self.asset(ticker)).map(Node::Asset),
            // A group is an equal weight with a name.
            "wt-cash-equal" | "group" => {
                let children = self.children(fields)?;
                Some(Node::Weight { name: name(fields), weighting: Weighting::Equal, children })
            }
            "wt-cash-specified" => self.specified(fields),
            "wt-inverse-vol" => {
                let window = self.days(given(fields, "window-days"));
                let children = self.children(fields);
                Some(Node::Weight { name: name(fields), weighting: Weighting::InverseVolatility { window: window? }, children: children? })
            }
            "wt-marketcap" => {
                self.children(fields);
                self.refuse(
                    IssueCode::UnsupportedBlock,
                    "Market cap weighting is no longer supported.",
                    "Use equal, specified or inverse volatility weighting.",
                )
            }
            "filter" => self.filter(fields),
            "if" => self.conditional(fields),
            "empty" => self.refuse(IssueCode::IncompleteStrategy, "The strategy contains an empty block.", "Replace it with a real block or remove it."),
            "if-child" => self.refuse(IssueCode::InvalidStructure, "A branch sits outside an if block.", "Put it inside an if block."),
            "root" => self.refuse(IssueCode::InvalidStructure, "A root block is nested inside another block.", "Keep one root, at the top."),
            other => self.refuse(
                IssueCode::InvalidBlockType,
                format!("{other:?} is not a kind of block."),
                "Use one of: asset, wt-cash-equal, wt-cash-specified, wt-inverse-vol, group, if, filter.",
            ),
        }
    }

    /// A specified weight keeps each child's share on the child, as one
    /// number over another. A weight on a child of any other block is stale
    /// and is never read.
    fn specified(&mut self, fields: &Fields) -> Option<Node> {
        let items = self.list(fields, "children")?;
        let mut shares = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            self.trail.enter_item("children", index);
            let part = |key| item.get("weight").and_then(|weight| weight.get(key)).and_then(number);
            shares.push(match (part("num"), part("den")) {
                (Some(num), Some(den)) if den != 0.0 => Some(num / den),
                _ => self.refuse(
                    IssueCode::InvalidWeightValue,
                    "This block sits in a specified weight, and its own weight is missing or is not one number over another.",
                    "Give it a weight, such as {\"num\": 50, \"den\": 100}.",
                ),
            });
            self.trail.leave();
        }
        let children = self.blocks(items);
        // The display name is dropped here as it is where these documents are made.
        Some(Node::Weight { name: None, weighting: Weighting::Specified { shares: shares.into_iter().collect::<Option<_>>()? }, children: children? })
    }

    fn filter(&mut self, fields: &Fields) -> Option<Node> {
        let sort_by = self.indicator(fields, "sort-by-fn", "sort-by-fn-params", Some("sort-by-window-days"));
        let take = self.word(fields, "select-fn", &TAKES);
        let count = match present(fields, "select-n").map(|value| (value, whole(value))) {
            Some((_, Some(count))) => Some(count),
            Some((value, None)) => {
                self.refuse(IssueCode::InvalidFilter, format!("select-n is {}, which is not a whole number.", shown(value)), "Say how many to keep, such as 2.")
            }
            None => self.refuse(IssueCode::MissingField, "This filter has no select-n.", "Say how many to keep, such as 2."),
        };
        let children = self.children(fields);
        Some(Node::Filter { sort_by: sort_by?, take: take?, count: count?, children: children? })
    }

    /// An if holds exactly two branches: the one with the condition, then
    /// the else. The tree keeps them as one block.
    fn conditional(&mut self, fields: &Fields) -> Option<Node> {
        let items = self.list(fields, "children")?;
        let (Some(first), Some(second), 2) = (items.first().and_then(branch), items.get(1).and_then(branch), items.len()) else {
            return self.refuse(
                IssueCode::InvalidStructure,
                "This if block does not hold exactly two branches.",
                "Give it the branch with the condition, then the else branch.",
            );
        };
        let mut is_else = [None, None];
        for (index, branch) in [first, second].into_iter().enumerate() {
            self.trail.enter_item("children", index);
            is_else[index] = self.flag(branch, "is-else-condition?");
            self.trail.leave();
        }
        match is_else {
            [Some(false), Some(true)] => {}
            [Some(_), Some(_)] => {
                return self.refuse(
                    IssueCode::InvalidStructure,
                    "The branches of this if block are not the condition first and the else second.",
                    "Put the branch with the condition first and mark only the second as the else.",
                )
            }
            _ => return None,
        }
        self.trail.enter_item("children", 0);
        let condition = self.condition_of(first);
        let then = self.children(first);
        self.trail.leave();
        self.trail.enter_item("children", 1);
        let otherwise = self.children(second);
        self.trail.leave();
        Some(Node::If { condition: condition?, then: then?, otherwise: otherwise? })
    }

    /// A branch may carry its condition twice: as flat fields and as a
    /// condition object. The object is the one that is evaluated. The flat
    /// fields beside it are kept for display, hold one comparison at most,
    /// and can disagree with it.
    fn condition_of(&mut self, branch: &Fields) -> Option<Condition> {
        match present(branch, "condition") {
            Some(Value::Object(object)) if !object.is_empty() => {
                self.trail.enter("condition");
                let condition = self.condition(object);
                self.trail.leave();
                condition
            }
            Some(Value::Object(_)) | None => self.flat(branch).map(Condition::Compare),
            Some(other) => {
                self.refuse(IssueCode::InvalidField, format!("condition is {}, not an object.", shown(other)), "Write the condition as an object.")
            }
        }
    }

    fn flat(&mut self, branch: &Fields) -> Option<Comparison> {
        let left = self.flat_side(branch, &LEFT);
        let comparator = self.word(branch, "comparator", &COMPARATORS);
        // Without the flag the right side is an indicator on a ticker.
        let right = if self.flag(branch, "rhs-fixed-value?")? {
            let value = self.fixed(present(branch, RIGHT.value), RIGHT.value);
            left.as_ref().zip(value).map(|(left, value)| Operand::Fixed(scaled(value, left.indicator.unit())))
        } else {
            self.flat_side(branch, &RIGHT).map(Operand::Reading)
        };
        Some(Comparison { left: left?, comparator: comparator?, right: right? })
    }

    fn flat_side(&mut self, branch: &Fields, side: &Side) -> Option<Reading> {
        let indicator = self.indicator(branch, side.function, side.params, Some(side.window));
        let asset = self.text(branch, side.value).and_then(|ticker| self.asset(ticker));
        Some(Reading { indicator: indicator?, asset: asset? })
    }

    fn condition(&mut self, object: &Fields) -> Option<Condition> {
        match object.get("condition-type").and_then(Value::as_str) {
            Some("binary") => {
                self.closed(object, &["condition-type", "lhs", "comparator", "rhs"]);
                match self.pattern(object)?.on(None) {
                    Some(comparison) => Some(Condition::Compare(comparison)),
                    None => self.refuse(
                        IssueCode::InvalidCondition,
                        "This comparison uses % but lists no tickers for it to stand for.",
                        "Name the ticker, or list tickers in a binary-compound condition.",
                    ),
                }
            }
            Some("compound") => self.compound(object),
            Some("binary-compound") => self.over_tickers(object),
            _ => self.refuse(
                IssueCode::InvalidCondition,
                "This condition has no condition-type that is known.",
                "Use one of: binary, compound, binary-compound.",
            ),
        }
    }

    fn compound(&mut self, object: &Fields) -> Option<Condition> {
        self.closed(object, &["condition-type", "operator", "conditions"]);
        let group = self.word(object, "operator", &GROUPS);
        let items = self.list(object, "conditions")?;
        if items.is_empty() {
            return self.refuse(IssueCode::IncompleteCondition, "This any/all group has no conditions.", "Add at least one condition to it.");
        }
        let mut conditions = Vec::with_capacity(items.len());
        for (index, item) in items.iter().enumerate() {
            self.trail.enter_item("conditions", index);
            conditions.push(match item {
                Value::Object(inner) => self.condition(inner),
                other => self.refuse(
                    IssueCode::InvalidCondition,
                    format!("This condition is {}, not an object.", shown(other)),
                    "Write each condition as an object.",
                ),
            });
            self.trail.leave();
        }
        Some(group?(conditions.into_iter().collect::<Option<_>>()?))
    }

    /// One comparison made on each of a list of tickers, joined by any or
    /// all. `%` in the comparison stands for the ticker.
    fn over_tickers(&mut self, object: &Fields) -> Option<Condition> {
        self.closed(object, &["condition-type", "operator", "tickers", "lhs", "comparator", "rhs"]);
        let group = self.word(object, "operator", &GROUPS);
        let pattern = self.pattern(object);
        let tickers = self.list(object, "tickers")?;
        if tickers.is_empty() {
            return self.refuse(IssueCode::IncompleteCondition, "This condition lists no tickers.", "List at least one ticker.");
        }
        if tickers.len() > MAX_TICKERS {
            return self.refuse(
                IssueCode::ExcessiveConditions,
                format!("This condition lists {} tickers; the most allowed is {MAX_TICKERS}.", tickers.len()),
                "List fewer tickers.",
            );
        }
        let mut assets = Vec::with_capacity(tickers.len());
        for (index, ticker) in tickers.iter().enumerate() {
            self.trail.enter_item("tickers", index);
            assets.push(match ticker {
                Value::String(ticker) => self.asset(ticker),
                other => self.refuse(IssueCode::InvalidAsset, format!("This ticker is {}, not text.", shown(other)), "Write it as text, such as \"SPY\"."),
            });
            self.trail.leave();
        }
        let (assets, pattern, group) = (assets.into_iter().collect::<Option<Vec<Asset>>>()?, pattern?, group?);
        let mut comparisons: Vec<Condition> = assets.iter().filter_map(|asset| pattern.on(Some(asset))).map(Condition::Compare).collect();
        // One ticker needs no group around its comparison.
        if comparisons.len() == 1 {
            return comparisons.pop();
        }
        Some(group(comparisons))
    }

    fn pattern(&mut self, object: &Fields) -> Option<Pattern> {
        let left = self.template(object, "lhs");
        let comparator = self.word(object, "comparator", &COMPARATORS);
        let right = self.template(object, "rhs");
        match left? {
            Template::Read(indicator, asset) => Some(Pattern { indicator, asset, comparator: comparator?, right: right? }),
            // The left indicator sets the unit of a fixed value on the right.
            // With a number on the left there is no unit to scale by.
            Template::Fixed(_) => self.refuse(
                IssueCode::InvalidCondition,
                "The left side of this comparison is a fixed number.",
                "Put the indicator on the left and the number on the right.",
            ),
        }
    }

    fn template(&mut self, object: &Fields, key: &'static str) -> Option<Template> {
        self.trail.enter(key);
        let template = match object.get(key) {
            Some(Value::Object(side)) => self.template_of(side),
            _ => self.refuse(IssueCode::InvalidCondition, "This side of the comparison is missing.", "Give it a constant, or an indicator and a ticker."),
        };
        self.trail.leave();
        template
    }

    fn template_of(&mut self, side: &Fields) -> Option<Template> {
        if let Some(constant) = side.get("constant") {
            self.closed(side, &["constant"]);
            return self.fixed(Some(constant), "constant").map(Template::Fixed);
        }
        self.closed(side, &["fn", "params", "ticker"]);
        let indicator = self.indicator(side, "fn", "params", None);
        let asset = match self.text(side, "ticker")? {
            "%" => None,
            ticker => Some(self.asset(ticker)?),
        };
        Some(Template::Read(indicator?, asset))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn asset(ticker: &str) -> Value {
        json!({ "id": format!("asset-{ticker}"), "step": "asset", "ticker": ticker })
    }

    fn equal(children: Vec<Value>) -> Value {
        json!({ "id": "w", "step": "wt-cash-equal", "children": children })
    }

    fn document(child: Value) -> Value {
        json!({ "id": "doc", "step": "root", "name": "Test", "description": "", "rebalance": "daily", "children": [child] })
    }

    /// An if block that holds SPY when `branch` is true and BIL when it is not.
    fn when(mut branch: Value) -> Value {
        branch["step"] = json!("if-child");
        branch["is-else-condition?"] = json!(false);
        branch["children"] = json!([asset("SPY")]);
        json!({ "id": "if", "step": "if", "children": [branch, { "step": "if-child", "is-else-condition?": true, "children": [asset("BIL")] }] })
    }

    /// The flat form: an indicator on QQQ against a fixed value.
    fn flat(function: &str, comparator: &str, fixed: &str) -> Value {
        json!({ "lhs-fn": function, "lhs-val": "QQQ", "lhs-fn-params": { "window": 10 }, "comparator": comparator, "rhs-fixed-value?": true, "rhs-val": fixed })
    }

    /// The object form: an indicator on a ticker against a constant.
    fn binary(function: &str, ticker: &str, comparator: &str, constant: f64) -> Value {
        json!({ "condition-type": "binary", "lhs": { "fn": function, "params": { "window": 10 }, "ticker": ticker }, "comparator": comparator, "rhs": { "constant": constant } })
    }

    fn listed(symbol: &str) -> Asset {
        Asset::Listed { market: Market::Equities, symbol: symbol.to_string() }
    }

    fn condition(branch: Value) -> Condition {
        match import(&document(when(branch))).expect("imports").root {
            Node::If { condition, .. } => condition,
            other => panic!("not an if: {other:?}"),
        }
    }

    fn comparison(branch: Value) -> Comparison {
        match condition(branch) {
            Condition::Compare(comparison) => comparison,
            other => panic!("not one comparison: {other:?}"),
        }
    }

    fn codes(document: &Value) -> Vec<IssueCode> {
        import(document).expect_err("refused").into_iter().map(|issue| issue.code).collect()
    }

    #[test]
    fn a_fixed_value_is_scaled_by_the_unit_of_the_indicator_on_its_left() {
        for function in ["cumulative-return", "moving-average-return", "standard-deviation-return", "max-drawdown", "relative-strength-index"] {
            let comparison = comparison(flat(function, "gt", "79"));
            assert_eq!(comparison.left.indicator.unit(), Unit::Percent, "{function}");
            assert_eq!(comparison.right, Operand::Fixed(0.79), "{function}");
        }
        for function in ["current-price", "moving-average-price", "exponential-moving-average-price", "standard-deviation-price"] {
            let comparison = comparison(flat(function, "gt", "79"));
            assert_eq!(comparison.left.indicator.unit(), Unit::Dollars, "{function}");
            assert_eq!(comparison.right, Operand::Fixed(79.0), "{function}");
        }
    }

    #[test]
    fn a_percent_is_divided_by_one_hundred_and_keeps_the_digits_division_gives() {
        for (typed, held) in [("99.9", 0.9990000000000001), ("2.9", 0.028999999999999998), ("70", 0.7), ("0.33", 0.0033), ("-13", -0.13), ("0", 0.0)] {
            assert_eq!(comparison(flat("relative-strength-index", "gt", typed)).right, Operand::Fixed(held), "{typed}");
        }
        // A shift of the decimal point, or a multiplication by 0.01, gives other digits.
        assert_ne!(99.9 / 100.0, 0.999);
        assert_ne!(70.0 / 100.0, 70.0 * 0.01);
        assert_eq!(comparison(flat("moving-average-price", "lt", "-1")).right, Operand::Fixed(-1.0));
        let mut number = flat("relative-strength-index", "gt", "");
        number["rhs-val"] = json!(79);
        assert_eq!(comparison(number).right, Operand::Fixed(0.79));
    }

    #[test]
    fn a_constant_in_a_condition_object_is_scaled_the_same_way() {
        let percent = comparison(json!({ "condition": binary("cumulative-return", "QQQ", "gt", 99.9) }));
        assert_eq!(percent.right, Operand::Fixed(0.9990000000000001));
        let dollars = comparison(json!({ "condition": binary("moving-average-price", "QQQ", "gt", 99.9) }));
        assert_eq!(dollars.right, Operand::Fixed(99.9));
        let mut over_tickers = binary("max-drawdown", "%", "lt", 6.0);
        over_tickers["condition-type"] = json!("binary-compound");
        over_tickers["operator"] = json!("all");
        over_tickers["tickers"] = json!(["QQQ"]);
        assert_eq!(comparison(json!({ "condition": over_tickers })).right, Operand::Fixed(0.06));
    }

    #[test]
    fn the_condition_object_wins_over_the_flat_fields_beside_it() {
        // As stored documents do it: the flat fields repeat one comparison of the object.
        let mut branch = flat("relative-strength-index", "lt", "32");
        branch["lhs-val"] = json!("TQQQ");
        branch["condition"] = json!({ "condition-type": "compound", "operator": "any", "conditions": [
            binary("relative-strength-index", "TQQQ", "lt", 32.0),
            binary("relative-strength-index", "SOXL", "lt", 28.0),
        ] });
        let reading = |ticker| Reading { indicator: Indicator::RelativeStrength { window: 10 }, asset: listed(ticker) };
        let below = |ticker, value| Condition::Compare(Comparison { left: reading(ticker), comparator: Comparator::Below, right: Operand::Fixed(value) });
        assert_eq!(condition(branch.clone()), Condition::Any(vec![below("TQQQ", 0.32), below("SOXL", 0.28)]));

        // The two forms may disagree outright. The object still wins.
        branch["condition"] = binary("cumulative-return", "XLF", "gt", 5.0);
        let comparison = comparison(branch.clone());
        assert_eq!(comparison.left, Reading { indicator: Indicator::CumulativeReturn { window: 10 }, asset: listed("XLF") });
        assert_eq!((comparison.comparator, comparison.right), (Comparator::Above, Operand::Fixed(0.05)));

        // With no object to read, the flat fields are the condition.
        for absent in [json!(null), json!({})] {
            branch["condition"] = absent;
            assert_eq!(condition(branch.clone()), below("TQQQ", 0.32));
        }
    }

    #[test]
    fn a_window_is_read_from_the_newer_field_first_and_the_older_one_second() {
        let window = |branch: Value| comparison(branch).left.indicator.window();
        let mut branch = flat("relative-strength-index", "gt", "50");
        assert_eq!(window(branch.clone()), Some(10));
        branch["lhs-window-days"] = json!("21");
        assert_eq!(window(branch.clone()), Some(10));
        branch.as_object_mut().unwrap().remove("lhs-fn-params");
        assert_eq!(window(branch.clone()), Some(21));
        branch["lhs-fn-params"] = json!({ "window": "14" });
        assert_eq!(window(branch.clone()), Some(14));
        // The latest price has no window, and a leftover one is not read.
        branch["lhs-fn"] = json!("current-price");
        assert_eq!(comparison(branch.clone()).left.indicator, Indicator::Price);

        branch["lhs-fn"] = json!("max-drawdown");
        branch.as_object_mut().unwrap().remove("lhs-fn-params");
        branch.as_object_mut().unwrap().remove("lhs-window-days");
        assert_eq!(codes(&document(when(branch.clone()))), vec![IssueCode::MissingField]);
        for bad in [json!(10.5), json!("ten"), json!(-3), json!(true)] {
            branch["lhs-fn-params"] = json!({ "window": bad });
            assert_eq!(codes(&document(when(branch.clone()))), vec![IssueCode::InvalidWindow], "{bad}");
        }
    }

    #[test]
    fn every_one_of_the_nine_indicators_is_read_and_no_other() {
        let names = [
            ("current-price", Indicator::Price),
            ("moving-average-price", Indicator::AveragePrice { window: 10 }),
            ("exponential-moving-average-price", Indicator::ExponentialAveragePrice { window: 10 }),
            ("standard-deviation-price", Indicator::PriceDeviation { window: 10 }),
            ("cumulative-return", Indicator::CumulativeReturn { window: 10 }),
            ("moving-average-return", Indicator::AverageReturn { window: 10 }),
            ("standard-deviation-return", Indicator::ReturnDeviation { window: 10 }),
            ("max-drawdown", Indicator::MaxDrawdown { window: 10 }),
            ("relative-strength-index", Indicator::RelativeStrength { window: 10 }),
        ];
        for (name, indicator) in names {
            let filter = json!({ "step": "filter", "sort-by-fn": name, "sort-by-fn-params": { "window": 10 }, "select-fn": "bottom", "select-n": "1", "children": [asset("SPY"), asset("QQQ")] });
            let expected = Node::Filter { sort_by: indicator, take: Take::Bottom, count: 1, children: vec![Node::Asset(listed("SPY")), Node::Asset(listed("QQQ"))] };
            assert_eq!(import(&document(filter)).expect("imports").root, expected, "{name}");
        }
        for name in ["moving-average-convergence-divergence", "upper-bollinger", "percentage-price-oscillator", "rsi"] {
            let issues = import(&document(when(flat(name, "gt", "1")))).expect_err("refused");
            assert_eq!(issues.len(), 1, "{name}");
            assert_eq!(issues[0].code, IssueCode::InvalidIndicator, "{name}");
            assert!(issues[0].fix.contains("relative-strength-index"));
        }
    }

    #[test]
    fn a_ticker_is_read_in_its_short_and_its_long_form() {
        let held = |ticker: &str| import(&document(asset(ticker))).map(|strategy| strategy.root);
        for same in ["SPY", "EQUITIES::SPY//USD", "SPY@ARCX", "EQUITIES::SPY//USD@ARCX"] {
            assert_eq!(held(same), Ok(Node::Asset(listed("SPY"))), "{same}");
        }
        assert_eq!(held("BRK/B"), Ok(Node::Asset(listed("BRK/B"))));
        assert_eq!(held("CRYPTO::BTC//USD"), Ok(Node::Asset(Asset::Listed { market: Market::Crypto, symbol: "BTC".to_string() })));
        for bad in ["", "spy", "BRK.B", "^VIX", "$USD", "OPTIONS::SPY//USD", "SPY@X", "SPY//EUR", "BRK//B", "S PY"] {
            assert_eq!(codes(&document(asset(bad))), vec![IssueCode::InvalidAsset], "{bad:?}");
        }
    }

    #[test]
    fn market_cap_weighting_is_refused_in_plain_words() {
        let issues = import(&document(json!({ "step": "wt-marketcap", "children": [asset("AAPL"), asset("MSFT")] }))).expect_err("refused");
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, IssueCode::UnsupportedBlock);
        assert_eq!(issues[0].path, "root.children[0]");
        assert_eq!(issues[0].message, "Market cap weighting is no longer supported.");
    }

    #[test]
    fn keys_that_are_not_known_are_skipped_below_the_root() {
        let plain = document(equal(vec![when(flat("relative-strength-index", "gt", "79")), asset("GLD")]));
        let mut noisy = plain.clone();
        noisy["children"][0]["collapsed?"] = json!(true);
        noisy["children"][0]["window-days"] = json!("45");
        noisy["children"][0]["children"][0]["suppress_incomplete_warnings"] = json!(false);
        noisy["children"][0]["children"][0]["children"][0]["made-up"] = json!({ "any": ["thing"] });
        noisy["children"][0]["children"][0]["children"][0]["children"][0]["price"] = json!(412.5);
        noisy["children"][0]["children"][1]["dollar_volume"] = json!(1e9);
        noisy["children"][0]["children"][1]["has_marketcap"] = json!(true);
        noisy["children"][0]["children"][1]["exchange"] = json!("ARCX");
        assert_eq!(import(&noisy), import(&plain));
        assert!(import(&plain).is_ok());
    }

    #[test]
    fn the_root_and_the_condition_objects_are_closed() {
        // What a stored root may carry for display is known, and is not read.
        let mut stored = document(asset("SPY"));
        for (key, value) in [("asset_class", json!(null)), ("asset_classes", json!(["EQUITIES"])), ("color", json!("#AEC3C6")), ("sparkgraph_url", json!(null)), ("collapsed?", json!(false))] {
            stored[key] = value;
        }
        assert_eq!(import(&stored), import(&document(asset("SPY"))));

        // Anything else on the root is a mistake: here, the other spelling of the corridor width.
        stored["rebalance_corridor_width"] = json!(0.05);
        let issues = import(&stored).expect_err("refused");
        assert_eq!(issues.iter().map(|issue| (issue.code, issue.path.as_str())).collect::<Vec<_>>(), vec![(IssueCode::UnknownField, "root")]);
        assert!(issues[0].message.contains("rebalance_corridor_width"));

        let mut object = binary("relative-strength-index", "QQQ", "gt", 70.0);
        object["note"] = json!("x");
        assert_eq!(codes(&document(when(json!({ "condition": object })))), vec![IssueCode::UnknownField]);
        let mut side = binary("relative-strength-index", "QQQ", "gt", 70.0);
        side["rhs"]["ticker"] = json!("SPY");
        assert_eq!(codes(&document(when(json!({ "condition": side })))), vec![IssueCode::UnknownField]);
    }

    #[test]
    fn a_share_is_one_number_over_another_in_any_spelling() {
        let child = |ticker: &str, weight: Value| {
            let mut child = asset(ticker);
            child["weight"] = weight;
            child
        };
        let specified = |weights: Vec<Value>| {
            let children: Vec<Value> = weights.into_iter().enumerate().map(|(index, weight)| child(["SPY", "TLT", "GLD", "BIL"][index], weight)).collect();
            document(json!({ "step": "wt-cash-specified", "name": "not kept", "children": children }))
        };
        let shares = |document: Value| match import(&document).expect("imports").root {
            Node::Weight { name: None, weighting: Weighting::Specified { shares }, .. } => shares,
            other => panic!("{other:?}"),
        };
        let spellings = vec![json!({ "num": "60", "den": 100 }), json!({ "num": 40, "den": "100" }), json!({ "num": "67.00", "den": 100.0 }), json!({ "num": 1, "den": 3 })];
        assert_eq!(shares(specified(spellings)), vec![0.6, 0.4, 0.67, 1.0 / 3.0]);
        assert_eq!(shares(specified(vec![json!({ "num": "0", "den": 100 }), json!({ "num": "100", "den": 100 })])), vec![0.0, 1.0]);

        for bad in [json!(null), json!({ "num": 50 }), json!({ "num": "half", "den": 100 }), json!({ "num": 50, "den": 0 }), json!({ "num": true, "den": 100 })] {
            let issues = import(&specified(vec![json!({ "num": 100, "den": 100 }), bad.clone()])).expect_err("refused");
            assert_eq!(issues.iter().map(|issue| (issue.code, issue.path.as_str())).collect::<Vec<_>>(), vec![(IssueCode::InvalidWeightValue, "root.children[0].children[1]")], "{bad}");
        }
    }

    #[test]
    fn a_weight_left_over_from_an_earlier_parent_is_not_read() {
        let mut stale = asset("SPY");
        stale["weight"] = json!({ "num": "not a number", "den": 0 });
        let expected = Node::Weight { name: None, weighting: Weighting::Equal, children: vec![Node::Asset(listed("SPY")), Node::Asset(listed("TLT"))] };
        assert_eq!(import(&document(equal(vec![stale, asset("TLT")]))).expect("imports").root, expected);
    }

    #[test]
    fn a_group_is_an_equal_weight_that_keeps_its_name() {
        let group = json!({ "step": "group", "name": "Bonds", "collapsed?": true, "children": [equal(vec![asset("TLT")])] });
        let inner = Node::Weight { name: None, weighting: Weighting::Equal, children: vec![Node::Asset(listed("TLT"))] };
        assert_eq!(import(&document(group)).expect("imports").root, Node::Weight { name: Some("Bonds".to_string()), weighting: Weighting::Equal, children: vec![inner] });

        let inverse = json!({ "step": "wt-inverse-vol", "window-days": "30", "children": [asset("TLT"), asset("GLD")] });
        let children = vec![Node::Asset(listed("TLT")), Node::Asset(listed("GLD"))];
        assert_eq!(import(&document(inverse)).expect("imports").root, Node::Weight { name: None, weighting: Weighting::InverseVolatility { window: 30 }, children });
    }

    #[test]
    fn an_if_needs_its_two_branches_with_the_condition_first() {
        let good = when(flat("relative-strength-index", "gt", "79"));
        assert!(import(&document(good.clone())).is_ok());

        let mut one = good.clone();
        one["children"].as_array_mut().unwrap().pop();
        let mut three = good.clone();
        three["children"].as_array_mut().unwrap().push(json!({ "step": "if-child", "is-else-condition?": true, "children": [asset("GLD")] }));
        let mut not_a_branch = good.clone();
        not_a_branch["children"][1] = asset("GLD");
        let mut else_first = good.clone();
        else_first["children"].as_array_mut().unwrap().reverse();
        let mut two_conditions = good.clone();
        two_conditions["children"][1] = good["children"][0].clone();
        for bad in [one, three, not_a_branch, else_first, two_conditions] {
            assert_eq!(codes(&document(bad.clone())), vec![IssueCode::InvalidStructure], "{bad}");
        }
        assert_eq!(codes(&document(good["children"][0].clone())), vec![IssueCode::InvalidStructure]);
    }

    #[test]
    fn a_branch_may_hold_several_blocks_or_another_if() {
        let mut outer = when(flat("relative-strength-index", "gt", "79"));
        outer["children"][0]["children"] = json!([asset("SPXL"), asset("TQQQ")]);
        outer["children"][1]["children"] = json!([when(flat("current-price", "gt", "0"))]);
        match import(&document(outer)).expect("imports").root {
            Node::If { then, otherwise, .. } => {
                assert_eq!(then, vec![Node::Asset(listed("SPXL")), Node::Asset(listed("TQQQ"))]);
                assert!(matches!(otherwise.as_slice(), [Node::If { .. }]));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_comparison_over_a_ticker_list_is_made_once_for_each_ticker() {
        let over = |operator: &str, tickers: Value, rhs: Value| {
            json!({ "condition": { "condition-type": "binary-compound", "operator": operator, "tickers": tickers,
                "lhs": { "fn": "cumulative-return", "params": { "window": 10 }, "ticker": "%" }, "comparator": "gt", "rhs": rhs } })
        };
        let left = |ticker| Reading { indicator: Indicator::CumulativeReturn { window: 10 }, asset: listed(ticker) };
        let above = |ticker, right| Condition::Compare(Comparison { left: left(ticker), comparator: Comparator::Above, right });

        let any = condition(over("any", json!(["NVDA", "TSLA", "AAPL"]), json!({ "constant": 10 })));
        assert_eq!(any, Condition::Any(["NVDA", "TSLA", "AAPL"].map(|ticker| above(ticker, Operand::Fixed(0.1))).to_vec()));

        // One ticker needs no group around it.
        assert_eq!(condition(over("all", json!(["NVDA"]), json!({ "constant": 10 }))), above("NVDA", Operand::Fixed(0.1)));

        // The right side may read a named ticker, or the listed one in its turn.
        let average = |ticker| Operand::Reading(Reading { indicator: Indicator::AverageReturn { window: 5 }, asset: listed(ticker) });
        let against = |ticker: &str| json!({ "fn": "moving-average-return", "params": { "window": 5 }, "ticker": ticker });
        assert_eq!(condition(over("all", json!(["XLK", "XLE"]), against("SPY"))), Condition::All(vec![above("XLK", average("SPY")), above("XLE", average("SPY"))]));
        assert_eq!(condition(over("all", json!(["XLK", "XLE"]), against("%"))), Condition::All(vec![above("XLK", average("XLK")), above("XLE", average("XLE"))]));

        assert_eq!(codes(&document(when(over("any", json!([]), json!({ "constant": 10 }))))), vec![IssueCode::IncompleteCondition]);
        let many: Vec<String> = (0..51).map(|_| "SPY".to_string()).collect();
        assert_eq!(codes(&document(when(over("any", json!(many), json!({ "constant": 10 }))))), vec![IssueCode::ExcessiveConditions]);
        assert_eq!(codes(&document(when(over("any", json!(["SPY", 7]), json!({ "constant": 10 }))))), vec![IssueCode::InvalidAsset]);
        assert_eq!(codes(&document(when(over("some", json!(["SPY"]), json!({ "constant": 10 }))))), vec![IssueCode::InvalidField]);
    }

    #[test]
    fn a_condition_object_must_be_whole_and_put_its_indicator_on_the_left() {
        let refused = |object: Value| codes(&document(when(json!({ "condition": object }))));
        // `%` stands for a listed ticker, and a plain comparison lists none.
        assert_eq!(refused(binary("relative-strength-index", "%", "gt", 70.0)), vec![IssueCode::InvalidCondition]);
        // A number on the left has no unit to scale the comparison by.
        let mut number_first = binary("relative-strength-index", "QQQ", "lt", 70.0);
        let (lhs, rhs) = (number_first["lhs"].take(), number_first["rhs"].take());
        number_first["lhs"] = rhs;
        number_first["rhs"] = lhs;
        assert_eq!(refused(number_first), vec![IssueCode::InvalidCondition]);

        assert_eq!(refused(json!({ "condition-type": "ternary" })), vec![IssueCode::InvalidCondition]);
        assert_eq!(refused(json!({ "condition-type": "compound", "operator": "any", "conditions": [] })), vec![IssueCode::IncompleteCondition]);
        assert_eq!(refused(json!({ "condition-type": "compound", "operator": "any", "conditions": ["x"] })), vec![IssueCode::InvalidCondition]);
        let mut no_right = binary("relative-strength-index", "QQQ", "gt", 70.0);
        no_right.as_object_mut().unwrap().remove("rhs");
        assert_eq!(refused(no_right), vec![IssueCode::InvalidCondition]);
        let mut words = binary("relative-strength-index", "QQQ", "gt", 70.0);
        words["rhs"] = json!({ "constant": "seventy" });
        assert_eq!(refused(words), vec![IssueCode::InvalidFixedValue]);
        assert_eq!(codes(&document(when(json!({ "condition": "rsi > 70" })))), vec![IssueCode::InvalidField]);
    }

    #[test]
    fn groups_of_conditions_nest_as_they_are_written() {
        let rsi = |ticker| binary("relative-strength-index", ticker, "lt", 30.0);
        let nested = json!({ "condition-type": "compound", "operator": "all", "conditions": [
            rsi("SPY"),
            { "condition-type": "compound", "operator": "any", "conditions": [rsi("QQQ"), rsi("IWM")] },
        ] });
        let below = |ticker| Condition::Compare(Comparison { left: Reading { indicator: Indicator::RelativeStrength { window: 10 }, asset: listed(ticker) }, comparator: Comparator::Below, right: Operand::Fixed(0.3) });
        assert_eq!(condition(json!({ "condition": nested })), Condition::All(vec![below("SPY"), Condition::Any(vec![below("QQQ"), below("IWM")])]));
    }

    #[test]
    fn the_flat_form_names_what_it_is_missing() {
        let mut against_ticker = flat("current-price", "gt", "");
        against_ticker["rhs-fixed-value?"] = json!(false);
        against_ticker["rhs-fn"] = json!("moving-average-price");
        against_ticker["rhs-fn-params"] = json!({ "window": 200 });
        against_ticker["rhs-val"] = json!("SPY");
        let comparison = comparison(against_ticker.clone());
        assert_eq!(comparison.right, Operand::Reading(Reading { indicator: Indicator::AveragePrice { window: 200 }, asset: listed("SPY") }));
        // Without the flag the right side is still an indicator on a ticker.
        against_ticker.as_object_mut().unwrap().remove("rhs-fixed-value?");
        assert_eq!(super::tests::comparison(against_ticker.clone()), comparison);

        for (key, code) in [("lhs-fn", IssueCode::MissingField), ("lhs-val", IssueCode::MissingField), ("comparator", IssueCode::MissingField), ("rhs-fn", IssueCode::MissingField), ("rhs-val", IssueCode::MissingField)] {
            let mut branch = against_ticker.clone();
            branch.as_object_mut().unwrap().remove(key);
            assert_eq!(codes(&document(when(branch))), vec![code], "{key}");
        }
        let mut comparator = against_ticker.clone();
        comparator["comparator"] = json!("ne");
        let issues = import(&document(when(comparator))).expect_err("refused");
        assert_eq!((issues[0].code, issues[0].fix.as_str()), (IssueCode::InvalidField, "Use one of: gt, gte, eq, lt, lte."));
        assert_eq!(codes(&document(when(flat("relative-strength-index", "gt", "high")))), vec![IssueCode::InvalidFixedValue]);
        let mut flag = flat("relative-strength-index", "gt", "70");
        flag["rhs-fixed-value?"] = json!("yes");
        assert_eq!(codes(&document(when(flag))), vec![IssueCode::InvalidField]);
    }

    #[test]
    fn every_comparator_has_its_word() {
        for (word, comparator) in [("gt", Comparator::Above), ("gte", Comparator::AtLeast), ("eq", Comparator::Equal), ("lt", Comparator::Below), ("lte", Comparator::AtMost)] {
            assert_eq!(comparison(flat("relative-strength-index", word, "50")).comparator, comparator);
        }
    }

    #[test]
    fn the_threshold_mode_is_the_word_none_with_a_corridor_width() {
        let mut document = document(asset("SPY"));
        document["rebalance"] = json!("none");
        document["rebalance-corridor-width"] = json!(0.05);
        assert_eq!(import(&document).expect("imports").rebalance, Rebalance::Threshold { corridor: 0.05 });

        document.as_object_mut().unwrap().remove("rebalance-corridor-width");
        assert_eq!(codes(&document), vec![IssueCode::InvalidRebalance]);
        document["rebalance-corridor-width"] = json!("0.05");
        assert_eq!(codes(&document), vec![IssueCode::InvalidRebalance]);

        // A width beside a calendar word is not read.
        for (word, frequency) in [("daily", Frequency::Daily), ("weekly", Frequency::Weekly), ("monthly", Frequency::Monthly), ("quarterly", Frequency::Quarterly), ("yearly", Frequency::Yearly)] {
            document["rebalance"] = json!(word);
            assert_eq!(import(&document).expect("imports").rebalance, Rebalance::Calendar(frequency));
        }
        document["rebalance"] = json!("hourly");
        let issues = import(&document).expect_err("refused");
        assert_eq!((issues[0].code, issues[0].fix.as_str()), (IssueCode::InvalidField, "Use one of: none, daily, weekly, monthly, quarterly, yearly."));
    }

    #[test]
    fn the_root_holds_a_name_a_rebalance_rule_and_exactly_one_block() {
        let strategy = import(&document(asset("SPY"))).expect("imports");
        assert_eq!((strategy.name.as_str(), strategy.rebalance), ("Test", Rebalance::Calendar(Frequency::Daily)));

        let mut empty = document(asset("SPY"));
        empty["children"] = json!([]);
        assert_eq!(codes(&empty), vec![IssueCode::IncompleteBlock]);
        let mut two = document(asset("SPY"));
        two["children"] = json!([asset("SPY"), asset("TLT")]);
        assert_eq!(codes(&two), vec![IssueCode::InvalidStructure]);
        for missing in ["name", "rebalance"] {
            let mut document = document(asset("SPY"));
            document.as_object_mut().unwrap().remove(missing);
            assert_eq!(codes(&document), vec![IssueCode::MissingField], "{missing}");
        }
        let mut class = document(asset("SPY"));
        class["asset_class"] = json!("BONDS");
        assert_eq!(codes(&class), vec![IssueCode::InvalidField]);
        for not_a_root in [json!([]), json!("root"), json!({ "step": "wt-cash-equal", "children": [] }), json!({ "type": "node_root", "score_version": "v2" })] {
            assert_eq!(codes(&not_a_root), vec![IssueCode::InvalidStructure], "{not_a_root}");
        }
    }

    #[test]
    fn blocks_that_are_unfinished_or_unknown_are_named() {
        assert_eq!(codes(&document(equal(vec![json!({ "step": "empty" })]))), vec![IssueCode::IncompleteStrategy]);
        assert_eq!(codes(&document(equal(vec![json!({ "step": "wt-risk-equal", "children": [] })]))), vec![IssueCode::InvalidBlockType]);
        assert_eq!(codes(&document(equal(vec![json!("SPY")]))), vec![IssueCode::InvalidBlockType]);
        assert_eq!(codes(&document(equal(vec![json!({ "ticker": "SPY" })]))), vec![IssueCode::MissingField]);
        assert_eq!(codes(&document(equal(vec![document(asset("SPY"))]))), vec![IssueCode::InvalidStructure]);
        assert_eq!(codes(&document(json!({ "step": "wt-cash-equal", "children": "SPY" }))), vec![IssueCode::InvalidField]);
        assert_eq!(codes(&document(json!({ "step": "wt-inverse-vol", "children": [asset("SPY")] }))), vec![IssueCode::MissingField]);

        let filter = |select: Value, count: Value| json!({ "step": "filter", "sort-by-fn": "cumulative-return", "sort-by-window-days": "20", "select-fn": select, "select-n": count, "children": [asset("SPY")] });
        assert!(import(&document(filter(json!("top"), json!(1)))).is_ok());
        assert_eq!(codes(&document(filter(json!("middle"), json!("1")))), vec![IssueCode::InvalidField]);
        assert_eq!(codes(&document(filter(json!("top"), json!("a few")))), vec![IssueCode::InvalidFilter]);
        assert_eq!(codes(&document(filter(json!("top"), json!(null)))), vec![IssueCode::MissingField]);
    }

    #[test]
    fn every_problem_in_a_document_is_reported_with_its_place() {
        let mut bad_branch = flat("made-up-index", "gt", "70");
        bad_branch["lhs-val"] = json!("qqq");
        let document = document(equal(vec![asset("SPY"), json!({ "step": "empty" }), when(bad_branch), json!({ "step": "wt-marketcap", "children": [asset("$USD")] })]));
        let found: Vec<(IssueCode, String)> = import(&document).expect_err("refused").into_iter().map(|issue| (issue.code, issue.path)).collect();
        let expected = [
            (IssueCode::IncompleteStrategy, "root.children[0].children[1]"),
            (IssueCode::InvalidIndicator, "root.children[0].children[2].children[0]"),
            (IssueCode::InvalidAsset, "root.children[0].children[2].children[0]"),
            (IssueCode::InvalidAsset, "root.children[0].children[3].children[0]"),
            (IssueCode::UnsupportedBlock, "root.children[0].children[3]"),
        ];
        assert_eq!(found, expected.map(|(code, path)| (code, path.to_string())));
    }

    #[test]
    fn text_that_is_not_a_document_is_a_problem_and_never_a_panic() {
        for bad in ["", "{", "{\"step\": \"root\",}", "nul", "{} {}"] {
            let issues = import_json(bad).expect_err("refused");
            assert_eq!(issues.iter().map(|issue| issue.code).collect::<Vec<_>>(), vec![IssueCode::InvalidJson], "{bad:?}");
        }
        assert_eq!(import_json("[1, 2]").expect_err("refused")[0].code, IssueCode::InvalidStructure);
        let text = document(asset("SPY")).to_string();
        assert_eq!(import_json(&text), import(&document(asset("SPY"))));
    }

    /// `levels` equal weights, one inside the other, around one asset.
    fn nested_weights(levels: usize) -> String {
        let mut text = String::from(r#"{"step":"root","name":"Deep","rebalance":"daily","children":["#);
        text.push_str(&r#"{"step":"wt-cash-equal","children":["#.repeat(levels));
        text.push_str(r#"{"step":"asset","ticker":"SPY"}"#);
        text.push_str(&"]}".repeat(levels + 1));
        text
    }

    /// `levels` ifs, each inside the first branch of the one before.
    fn nested_ifs(levels: usize) -> String {
        let mut text = String::from(r#"{"step":"root","name":"Deep","rebalance":"daily","children":["#);
        text.push_str(&r#"{"step":"if","children":[{"step":"if-child","lhs-fn":"current-price","lhs-val":"SPY","comparator":"gt","rhs-fixed-value?":true,"rhs-val":"1","children":["#.repeat(levels));
        text.push_str(r#"{"step":"asset","ticker":"SPY"}"#);
        text.push_str(&r#"]},{"step":"if-child","is-else-condition?":true,"children":[{"step":"asset","ticker":"BIL"}]}]}"#.repeat(levels));
        text.push_str("]}");
        text
    }

    #[test]
    fn a_document_nests_as_deep_as_the_limit_and_no_deeper() {
        // The root and its list are two levels, a weight two more, an if four, the asset one.
        assert_eq!(text_nesting(&nested_weights(254)), 511);
        assert_eq!(import_json(&nested_weights(254)).expect("imports").node_count(), 256);
        assert_eq!(text_nesting(&nested_ifs(127)), 511);
        assert_eq!(import_json(&nested_ifs(127)).expect("imports").node_count(), 2 + 3 * 127);

        // Past the limit the answer is one problem, however deep the text goes.
        for levels in [255, 5_000, 300_000] {
            let issues = import_json(&nested_weights(levels)).expect_err("refused");
            assert_eq!(issues.iter().map(|issue| issue.code).collect::<Vec<_>>(), vec![IssueCode::ExcessiveDepth], "{levels}");
        }
        assert_eq!(import_json(&"[".repeat(1_000_000)).expect_err("refused")[0].code, IssueCode::ExcessiveDepth);

        // Brackets inside text are not nesting.
        let quoted = document(asset("SPY")).to_string().replace("Test", &format!("a \\\"{}\\\" b", "[{".repeat(600)));
        assert!(import_json(&quoted).is_ok());

        // A value that was parsed elsewhere is held to the same limit.
        let mut value = asset("SPY");
        for _ in 0..300 {
            value = equal(vec![value]);
        }
        assert_eq!(codes(&document(value)), vec![IssueCode::ExcessiveDepth]);
    }
}
