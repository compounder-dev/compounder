//! Strategy documents: a tree of blocks that resolves to target weights.
//!
//! A document has one top-level block holding one child. Weight blocks split
//! their share among their children and asset blocks are the leaves.
//! `validate` returns every problem in a document; `targets` resolves a clean
//! document to one weight per asset.
//!
//! The typed tree is for checking and evaluating. Store the original JSON:
//! block kinds this engine does not run yet keep only their id here.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Deepest nesting a document may have, counting the top-level block.
pub const MAX_DEPTH: usize = 10;
/// Most children one weight block may hold.
pub const MAX_CHILDREN: usize = 20;
/// Most distinct assets one document may allocate to.
pub const MAX_UNIQUE_ASSETS: usize = 50;
/// How far a percentage block's weights may sit from 100 in total.
const WEIGHT_TOTAL_TOLERANCE: f64 = 0.001;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Strategy {
    pub entry: Block,
    /// Absent in documents written before the format was versioned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(rename = "tradingSettings")]
    pub trading_settings: TradingSettings,
}

/// When a portfolio that follows the strategy is rebalanced.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TradingSettings {
    /// Rebalance once a holding has drifted this far from its target, in percent.
    #[serde(rename_all = "camelCase")]
    Threshold { trigger_percentage: f64 },
    /// Rebalance on a fixed schedule, such as `weekly`.
    Calendar { frequency: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WeightType {
    /// Every child gets the same share. `weightings` is ignored.
    Equal,
    /// Child `i` gets `weightings[i]` percent.
    SpecifiedPercentage,
    /// Children are weighted by market value. Needs market data.
    MarketCap,
    /// Hold whatever the portfolio already holds. Top level only.
    Maintain,
}

/// A block kind the document format defines that this engine does not run yet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unsupported {
    pub block_id_for_instance: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "blockType", rename_all = "camelCase")]
pub enum Block {
    #[serde(rename_all = "camelCase")]
    TopLevel {
        name: String,
        children: Option<Box<Block>>,
        block_id_for_instance: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_node_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Weight {
        weight_type: WeightType,
        /// Percentages as decimal strings, one per child.
        weightings: Vec<String>,
        children: Vec<Block>,
        block_id_for_instance: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_node_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Asset {
        /// `0x<token address>:<chain id>`.
        asset_id: String,
        block_id_for_instance: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_node_id: Option<String>,
    },
    /// An empty slot left by the editor. Never valid in a finished document.
    Placeholder(Unsupported),
    Conditional(Unsupported),
    Filter(Unsupported),
    Dynamic(Unsupported),
    Group(Unsupported),
    Lend(Unsupported),
    Remote(Unsupported),
}

impl Block {
    pub fn id(&self) -> &str {
        match self {
            Block::TopLevel { block_id_for_instance, .. }
            | Block::Weight { block_id_for_instance, .. }
            | Block::Asset { block_id_for_instance, .. } => block_id_for_instance,
            Block::Placeholder(b)
            | Block::Conditional(b)
            | Block::Filter(b)
            | Block::Dynamic(b)
            | Block::Group(b)
            | Block::Lend(b)
            | Block::Remote(b) => &b.block_id_for_instance,
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Block::TopLevel { .. } => "topLevel",
            Block::Weight { .. } => "weight",
            Block::Asset { .. } => "asset",
            Block::Placeholder(_) => "placeholder",
            Block::Conditional(_) => "conditional",
            Block::Filter(_) => "filter",
            Block::Dynamic(_) => "dynamic",
            Block::Group(_) => "group",
            Block::Lend(_) => "lend",
            Block::Remote(_) => "remote",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum IssueCode {
    InvalidWeightValue,
    InvalidWeightTotal,
    InconsistentWeights,
    InvalidMaintainBlock,
    InvalidAsset,
    InvalidAssetId,
    MixedChainAssets,
    DuplicateBlockId,
    ExcessiveDepth,
    ExcessiveChildren,
    IncompleteBlock,
    IncompleteStrategy,
    InvalidBlockType,
    UnsupportedBlock,
    InsufficientDiversification,
    ExcessiveUniqueAssets,
    InvalidTradingSettings,
}

/// One problem in a document, tied to the block it is in.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Issue {
    pub code: IssueCode,
    /// The block's id, or `root` for a document-wide problem.
    pub block_id: String,
    /// What is wrong, in plain words.
    pub message: String,
    /// What to change.
    pub fix: String,
}

/// Split an asset id into its lowercased token address and chain id.
pub fn parse_asset_id(id: &str) -> Option<(String, u64)> {
    let (address, chain) = id.split_once(':')?;
    let hex = address.strip_prefix("0x")?;
    if hex.len() != 40 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    if chain.is_empty() || !chain.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((address.to_ascii_lowercase(), chain.parse().ok()?))
}

fn canonical_asset_id(id: &str) -> String {
    id.to_ascii_lowercase()
}

#[derive(Default)]
struct Check {
    issues: Vec<Issue>,
    block_ids: BTreeSet<String>,
    assets: BTreeSet<String>,
    chains: BTreeSet<u64>,
    holds_current: bool,
}

impl Check {
    fn report(&mut self, code: IssueCode, block_id: &str, message: impl Into<String>, fix: &str) {
        self.issues.push(Issue {
            code,
            block_id: block_id.to_string(),
            message: message.into(),
            fix: fix.to_string(),
        });
    }

    fn block(&mut self, block: &Block, depth: usize) {
        let id = block.id();
        if depth > MAX_DEPTH {
            self.report(
                IssueCode::ExcessiveDepth,
                id,
                format!("The portfolio has more than {MAX_DEPTH} nested levels."),
                "Reduce the nesting.",
            );
            return;
        }
        if !self.block_ids.insert(id.to_string()) {
            self.report(
                IssueCode::DuplicateBlockId,
                id,
                format!("Two blocks share the id {id}."),
                "Give every block its own id.",
            );
        }
        match block {
            Block::TopLevel { .. } => self.report(
                IssueCode::InvalidBlockType,
                id,
                "A top-level block is nested inside another block.",
                "Keep one top-level block, at the root.",
            ),
            Block::Weight { weight_type, weightings, children, .. } => {
                self.weight(id, *weight_type, weightings, children, depth)
            }
            Block::Asset { asset_id, .. } => self.asset(id, asset_id),
            Block::Placeholder(_) => self.report(
                IssueCode::IncompleteStrategy,
                id,
                "The portfolio contains an empty block.",
                "Replace it with an asset or a weight block.",
            ),
            other => self.report(
                IssueCode::UnsupportedBlock,
                id,
                format!("{} blocks do not run in this engine yet.", other.kind()),
                "Use weight and asset blocks.",
            ),
        }
    }

    fn weight(&mut self, id: &str, weight_type: WeightType, weightings: &[String], children: &[Block], depth: usize) {
        match weight_type {
            WeightType::Maintain => {
                if depth > 2 {
                    self.report(
                        IssueCode::InvalidMaintainBlock,
                        id,
                        "A maintain block is nested inside another block.",
                        "Move it directly under the top level.",
                    );
                }
                if !children.is_empty() {
                    self.report(
                        IssueCode::InvalidMaintainBlock,
                        id,
                        "A maintain block holds current positions and cannot have children.",
                        "Remove its children or choose another weight type.",
                    );
                }
                self.holds_current = true;
                self.report(
                    IssueCode::UnsupportedBlock,
                    id,
                    "Maintain weighting does not run in this engine yet.",
                    "Use equal or specified-percentage weighting.",
                );
                return;
            }
            WeightType::MarketCap => self.report(
                IssueCode::UnsupportedBlock,
                id,
                "Market-cap weighting needs market data this engine does not load yet.",
                "Use equal or specified-percentage weighting.",
            ),
            WeightType::SpecifiedPercentage => self.percentages(id, weightings, children.len()),
            WeightType::Equal => {}
        }
        if children.is_empty() {
            self.report(
                IssueCode::IncompleteBlock,
                id,
                "This weight block is empty.",
                "Add at least one asset or weight block to it.",
            );
        }
        if children.len() > MAX_CHILDREN {
            self.report(
                IssueCode::ExcessiveChildren,
                id,
                format!("This weight block has {} items; the most allowed is {MAX_CHILDREN}.", children.len()),
                "Remove items or split them into nested weight blocks.",
            );
        }
        for child in children {
            if let Block::Placeholder(_) = child {
                self.report(
                    IssueCode::IncompleteBlock,
                    child.id(),
                    "There is an empty slot in this weight block.",
                    "Replace it with an asset or a weight block.",
                );
            } else {
                self.block(child, depth + 1);
            }
        }
    }

    fn percentages(&mut self, id: &str, weightings: &[String], children: usize) {
        if weightings.len() != children {
            self.report(
                IssueCode::InconsistentWeights,
                id,
                format!("This block has {children} items but {} weights.", weightings.len()),
                "Give every item exactly one weight.",
            );
        }
        let mut total = 0.0;
        for (index, weighting) in weightings.iter().enumerate() {
            match weighting.trim().parse::<f64>() {
                Ok(percent) if percent.is_finite() && (0.0..=100.0).contains(&percent) => total += percent,
                _ => self.report(
                    IssueCode::InvalidWeightValue,
                    id,
                    format!("Weight {} is {weighting:?}, which is not a number from 0 to 100.", index + 1),
                    "Use numbers between 0 and 100.",
                ),
            }
        }
        if (total - 100.0).abs() > WEIGHT_TOTAL_TOLERANCE {
            self.report(
                IssueCode::InvalidWeightTotal,
                id,
                format!("The weights add up to {total}%, not 100%."),
                "Adjust the weights so they add up to 100%.",
            );
        }
    }

    fn asset(&mut self, id: &str, asset_id: &str) {
        if asset_id.is_empty() {
            self.report(IssueCode::InvalidAsset, id, "This asset block has no asset.", "Select an asset.");
            return;
        }
        match parse_asset_id(asset_id) {
            Some((_, chain)) => {
                self.assets.insert(canonical_asset_id(asset_id));
                self.chains.insert(chain);
            }
            None => self.report(
                IssueCode::InvalidAssetId,
                id,
                format!("{asset_id:?} is not a valid asset id."),
                "Use a token address and chain id, like 0x…:8453.",
            ),
        }
    }
}

impl Strategy {
    /// Every problem in the document. Empty means it can be evaluated.
    pub fn validate(&self) -> Vec<Issue> {
        let mut check = Check::default();
        match &self.entry {
            Block::TopLevel { children, block_id_for_instance, .. } => {
                check.block_ids.insert(block_id_for_instance.clone());
                match children {
                    Some(child) => check.block(child, 2),
                    None => check.report(
                        IssueCode::IncompleteBlock,
                        block_id_for_instance,
                        "The portfolio is empty.",
                        "Add a weight block or an asset.",
                    ),
                }
            }
            other => check.report(
                IssueCode::InvalidBlockType,
                other.id(),
                "The document does not start with a top-level block.",
                "Wrap the portfolio in a top-level block.",
            ),
        }
        // Only count assets once the tree itself is sound, so one missing
        // block does not also read as "no assets".
        if check.issues.is_empty() && !check.holds_current {
            if check.assets.is_empty() {
                check.report(
                    IssueCode::InsufficientDiversification,
                    "root",
                    "The portfolio allocates to no assets.",
                    "Add at least one asset.",
                );
            }
            if check.assets.len() > MAX_UNIQUE_ASSETS {
                check.report(
                    IssueCode::ExcessiveUniqueAssets,
                    "root",
                    format!("The portfolio uses {} assets; the most allowed is {MAX_UNIQUE_ASSETS}.", check.assets.len()),
                    "Reduce the number of different assets.",
                );
            }
            if check.chains.len() > 1 {
                check.report(
                    IssueCode::MixedChainAssets,
                    "root",
                    "The portfolio mixes assets from different chains.",
                    "Keep every asset on one chain.",
                );
            }
        }
        match &self.trading_settings {
            TradingSettings::Threshold { trigger_percentage } => {
                if !trigger_percentage.is_finite() || *trigger_percentage <= 0.0 || *trigger_percentage > 100.0 {
                    check.report(
                        IssueCode::InvalidTradingSettings,
                        "root",
                        format!("The rebalance threshold is {trigger_percentage}%, which is not above 0 and at most 100."),
                        "Set a threshold between 0 and 100 percent.",
                    );
                }
            }
            TradingSettings::Calendar { frequency } => {
                if frequency.trim().is_empty() {
                    check.report(
                        IssueCode::InvalidTradingSettings,
                        "root",
                        "The rebalance schedule has no frequency.",
                        "Choose how often to rebalance.",
                    );
                }
            }
        }
        check.issues
    }

    /// The share of the portfolio each asset should hold, summing to 1.
    /// Returns the document's problems instead when it has any.
    pub fn targets(&self) -> Result<BTreeMap<String, f64>, Vec<Issue>> {
        let issues = self.validate();
        if !issues.is_empty() {
            return Err(issues);
        }
        let mut weights = BTreeMap::new();
        if let Block::TopLevel { children: Some(child), .. } = &self.entry {
            spread(child, 1.0, &mut weights);
        }
        Ok(weights)
    }
}

/// Hand `share` of the portfolio down a validated subtree.
fn spread(block: &Block, share: f64, weights: &mut BTreeMap<String, f64>) {
    match block {
        Block::Asset { asset_id, .. } => {
            *weights.entry(canonical_asset_id(asset_id)).or_insert(0.0) += share;
        }
        Block::Weight { weight_type: WeightType::Equal, children, .. } => {
            let each = share / children.len() as f64;
            for child in children {
                spread(child, each, weights);
            }
        }
        Block::Weight { weight_type: WeightType::SpecifiedPercentage, weightings, children, .. } => {
            for (child, weighting) in children.iter().zip(weightings) {
                let percent: f64 = weighting.trim().parse().unwrap_or(0.0);
                spread(child, share * percent / 100.0, weights);
            }
        }
        // `validate` rejects every other block before evaluation.
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn address(n: usize) -> String {
        format!("0x{n:040x}:56")
    }

    fn asset(n: usize) -> Value {
        json!({ "blockType": "asset", "assetId": address(n), "blockIdForInstance": format!("asset-{n}"), "parentNodeId": "w" })
    }

    fn weight(id: &str, kind: &str, weightings: &[&str], children: Vec<Value>) -> Value {
        json!({ "blockType": "weight", "weightType": kind, "weightings": weightings, "children": children, "blockIdForInstance": id, "parentNodeId": "top" })
    }

    fn document(child: Value) -> Value {
        json!({
            "entry": { "blockType": "topLevel", "name": "Test", "children": child, "blockIdForInstance": "top", "parentNodeId": "root" },
            "version": "1.0.0",
            "tradingSettings": { "type": "threshold", "triggerPercentage": 10 }
        })
    }

    fn parse(value: Value) -> Strategy {
        serde_json::from_value(value).expect("document parses")
    }

    fn codes(strategy: &Strategy) -> Vec<IssueCode> {
        strategy.validate().into_iter().map(|issue| issue.code).collect()
    }

    #[test]
    fn equal_weighting_ignores_the_weightings_field() {
        let children: Vec<Value> = (1..=7).map(asset).collect();
        let strategy = parse(document(weight("w", "equal", &["100", "0", "0", "0", "0", "0", "0"], children)));
        let targets = strategy.targets().expect("valid");
        assert_eq!(targets.len(), 7);
        for share in targets.values() {
            assert!((share - 1.0 / 7.0).abs() < 1e-12);
        }
    }

    #[test]
    fn nested_percentages_multiply_and_repeated_assets_add_up() {
        let inner = weight("inner", "specified-percentage", &["25", "75"], vec![asset(1), asset(2)]);
        let mut again = asset(1);
        again["blockIdForInstance"] = json!("asset-1-again");
        let strategy = parse(document(weight("w", "specified-percentage", &["60", "40"], vec![inner, again])));
        let targets = strategy.targets().expect("valid");
        assert!((targets[&address(1)] - (0.60 * 0.25 + 0.40)).abs() < 1e-12);
        assert!((targets[&address(2)] - 0.60 * 0.75).abs() < 1e-12);
        assert!((targets.values().sum::<f64>() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn asset_ids_are_matched_without_regard_to_case() {
        let upper = json!({ "blockType": "asset", "assetId": "0xABCDEF0000000000000000000000000000000001:56", "blockIdForInstance": "a" });
        let lower = json!({ "blockType": "asset", "assetId": "0xabcdef0000000000000000000000000000000001:56", "blockIdForInstance": "b" });
        let targets = parse(document(weight("w", "equal", &[], vec![upper, lower]))).targets().expect("valid");
        assert_eq!(targets.len(), 1);
        assert!((targets["0xabcdef0000000000000000000000000000000001:56"] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn percentages_must_total_one_hundred() {
        let strategy = parse(document(weight("w", "specified-percentage", &["60", "30"], vec![asset(1), asset(2)])));
        let issues = strategy.validate();
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, IssueCode::InvalidWeightTotal);
        assert_eq!(issues[0].block_id, "w");
        assert!(strategy.targets().is_err());
    }

    #[test]
    fn every_item_needs_exactly_one_weight() {
        let strategy = parse(document(weight("w", "specified-percentage", &["100"], vec![asset(1), asset(2)])));
        assert_eq!(codes(&strategy), vec![IssueCode::InconsistentWeights]);
    }

    #[test]
    fn weights_must_be_numbers_from_zero_to_one_hundred() {
        for bad in ["abc", "-1", "101", "NaN", "inf", ""] {
            let strategy = parse(document(weight("w", "specified-percentage", &[bad, "100"], vec![asset(1), asset(2)])));
            assert!(codes(&strategy).contains(&IssueCode::InvalidWeightValue), "{bad:?} accepted");
        }
    }

    #[test]
    fn a_weight_block_holds_at_most_twenty_items() {
        let strategy = parse(document(weight("w", "equal", &[], (1..=21).map(asset).collect())));
        assert_eq!(codes(&strategy), vec![IssueCode::ExcessiveChildren]);
        let strategy = parse(document(weight("w", "equal", &[], (1..=20).map(asset).collect())));
        assert!(strategy.validate().is_empty());
    }

    #[test]
    fn nesting_stops_at_ten_levels() {
        let nest = |levels: usize| {
            let mut block = asset(1);
            for level in 0..levels {
                block = weight(&format!("w{level}"), "equal", &[], vec![block]);
            }
            parse(document(block))
        };
        // Top level is 1, eight weight blocks reach 9, the asset sits at 10.
        assert!(nest(8).validate().is_empty());
        assert_eq!(codes(&nest(9)), vec![IssueCode::ExcessiveDepth]);
    }

    #[test]
    fn block_ids_are_unique() {
        let strategy = parse(document(weight("w", "equal", &[], vec![asset(1), asset(1)])));
        assert_eq!(codes(&strategy), vec![IssueCode::DuplicateBlockId]);
    }

    #[test]
    fn empty_slots_and_empty_blocks_are_incomplete() {
        let slot = json!({ "blockType": "placeholder", "blockIdForInstance": "slot" });
        assert_eq!(codes(&parse(document(weight("w", "equal", &[], vec![asset(1), slot])))), vec![IssueCode::IncompleteBlock]);
        assert_eq!(codes(&parse(document(weight("w", "equal", &[], vec![])))), vec![IssueCode::IncompleteBlock]);
        let mut empty = document(asset(1));
        empty["entry"]["children"] = Value::Null;
        assert_eq!(codes(&parse(empty)), vec![IssueCode::IncompleteBlock]);
    }

    #[test]
    fn asset_ids_need_an_address_and_a_chain() {
        for bad in ["0x1234:56", "nvda", "0x0000000000000000000000000000000000000001", "0x000000000000000000000000000000000000000g:56", "0x0000000000000000000000000000000000000001:"] {
            let block = json!({ "blockType": "asset", "assetId": bad, "blockIdForInstance": "a" });
            assert_eq!(codes(&parse(document(block))), vec![IssueCode::InvalidAssetId], "{bad:?}");
        }
        let unset = json!({ "blockType": "asset", "assetId": "", "blockIdForInstance": "a" });
        assert_eq!(codes(&parse(document(unset))), vec![IssueCode::InvalidAsset]);
    }

    #[test]
    fn a_portfolio_uses_at_most_fifty_assets_on_one_chain() {
        let group = |id: &str, from: usize| weight(id, "equal", &[], (from..from + 17).map(asset).collect());
        let strategy = parse(document(weight("w", "equal", &[], vec![group("g1", 1), group("g2", 100), group("g3", 200)])));
        assert_eq!(codes(&strategy), vec![IssueCode::ExcessiveUniqueAssets]);

        let other_chain = json!({ "blockType": "asset", "assetId": "0x0000000000000000000000000000000000000009:8453", "blockIdForInstance": "base" });
        let strategy = parse(document(weight("w", "equal", &[], vec![asset(1), other_chain])));
        assert_eq!(codes(&strategy), vec![IssueCode::MixedChainAssets]);
    }

    #[test]
    fn blocks_this_engine_does_not_run_are_named_not_guessed() {
        let conditional = json!({ "blockType": "conditional", "blockIdForInstance": "if", "children": [], "comparisonData": {} });
        let issues = parse(document(conditional)).validate();
        assert_eq!(issues[0].code, IssueCode::UnsupportedBlock);
        assert!(issues[0].message.starts_with("conditional"));

        let strategy = parse(document(weight("w", "market-cap", &[], vec![asset(1), asset(2)])));
        assert_eq!(codes(&strategy), vec![IssueCode::UnsupportedBlock]);
    }

    #[test]
    fn a_document_from_before_versioning_still_loads() {
        let mut doc = document(asset(1));
        doc.as_object_mut().unwrap().remove("version");
        let strategy = parse(doc.clone());
        assert!(strategy.validate().is_empty());
        assert_eq!(serde_json::to_value(&strategy).unwrap().get("version"), None);
    }

    #[test]
    fn an_unknown_block_type_is_refused_at_parse() {
        let unknown = json!({ "blockType": "teleport", "blockIdForInstance": "x" });
        assert!(serde_json::from_value::<Strategy>(document(unknown)).is_err());
    }

    #[test]
    fn the_rebalance_threshold_must_be_a_real_percentage() {
        for bad in [0.0, -5.0, 100.5] {
            let mut doc = document(asset(1));
            doc["tradingSettings"]["triggerPercentage"] = json!(bad);
            assert_eq!(codes(&parse(doc)), vec![IssueCode::InvalidTradingSettings], "{bad}");
        }
    }

    #[test]
    fn a_calendar_schedule_is_accepted_in_place_of_a_threshold() {
        let mut doc = document(asset(1));
        doc["tradingSettings"] = json!({ "type": "calendar", "frequency": "weekly" });
        assert!(parse(doc.clone()).validate().is_empty());
        doc["tradingSettings"]["frequency"] = json!(" ");
        assert_eq!(codes(&parse(doc)), vec![IssueCode::InvalidTradingSettings]);
    }

    #[test]
    fn a_document_survives_a_round_trip_with_its_field_names() {
        let mut original = document(weight("w", "specified-percentage", &["50", "50"], vec![asset(1), asset(2)]));
        // A threshold is a float here, so it is written back with a decimal point.
        original["tradingSettings"]["triggerPercentage"] = json!(10.0);
        let again = serde_json::to_value(parse(original.clone())).expect("serialises");
        assert_eq!(again, original);
    }

    /// Any tree built from the supported blocks resolves to weights that are
    /// never negative and sum to one.
    #[test]
    fn valid_trees_always_resolve_to_a_full_allocation() {
        let mut state: u64 = 0x5EED;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for case in 0..300 {
            let mut counter = 0usize;
            let mut build = |depth: usize, next: &mut dyn FnMut() -> u64| -> Value {
                fn node(depth: usize, counter: &mut usize, next: &mut dyn FnMut() -> u64) -> Value {
                    *counter += 1;
                    let id = format!("b{counter}");
                    if depth >= 4 || next() % 3 == 0 {
                        return json!({ "blockType": "asset", "assetId": address((next() % 12) as usize + 1), "blockIdForInstance": id });
                    }
                    let size = (next() % 4) as usize + 1;
                    let children: Vec<Value> = (0..size).map(|_| node(depth + 1, counter, next)).collect();
                    if next() % 2 == 0 {
                        return json!({ "blockType": "weight", "weightType": "equal", "weightings": [], "children": children, "blockIdForInstance": id });
                    }
                    // Whole percentages that total exactly 100.
                    let mut left = 100u64;
                    let mut weightings = Vec::new();
                    for index in 0..size {
                        let take = if index + 1 == size { left } else { next() % (left + 1) };
                        left -= take;
                        weightings.push(take.to_string());
                    }
                    json!({ "blockType": "weight", "weightType": "specified-percentage", "weightings": weightings, "children": children, "blockIdForInstance": id })
                }
                node(depth, &mut counter, next)
            };
            let strategy = parse(document(build(2, &mut next)));
            let targets = strategy.targets().unwrap_or_else(|issues| panic!("case {case}: {issues:?}"));
            assert!(targets.values().all(|share| *share >= 0.0), "case {case}");
            assert!((targets.values().sum::<f64>() - 1.0).abs() < 1e-9, "case {case}");
        }
    }
}
