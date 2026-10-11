//! Fixed-weight funds as rule trees, so one evaluator can serve both kinds.

use super::{Asset, Frequency, Node, Rebalance, RuleStrategy, Weighting};
use crate::strategy::{Block, Issue, IssueCode, Strategy, TradingSettings, WeightType};

impl RuleStrategy {
    /// The same fund as a rule tree: its weight blocks and assets, with
    /// percentages as fractions. Returns the document's problems instead
    /// when it has any.
    pub fn from_fixed(strategy: &Strategy) -> Result<RuleStrategy, Vec<Issue>> {
        let mut issues = strategy.validate();
        let rebalance = match &strategy.trading_settings {
            TradingSettings::Threshold { trigger_percentage } => Some(Rebalance::Threshold { corridor: trigger_percentage / 100.0 }),
            TradingSettings::Calendar { frequency } => {
                let word = frequency.trim().to_ascii_lowercase();
                let known = Frequency::named(&word);
                // An empty schedule is already among the problems.
                if known.is_none() && !word.is_empty() {
                    issues.push(Issue {
                        code: IssueCode::InvalidTradingSettings,
                        block_id: "root".to_string(),
                        message: format!("The rebalance schedule is {frequency:?}, which is not one this engine runs."),
                        fix: "Use one of: daily, weekly, monthly, quarterly, yearly.".to_string(),
                    });
                }
                known.map(Rebalance::Calendar)
            }
        };
        let tree = match &strategy.entry {
            Block::TopLevel { name, children: Some(child), .. } => node(child).map(|root| (name, root)),
            _ => None,
        };
        match (tree, rebalance) {
            (Some((name, root)), Some(rebalance)) if issues.is_empty() => Ok(RuleStrategy { name: name.clone(), rebalance, root }),
            _ => Err(issues),
        }
    }
}

/// One block of a document. `validate` lets only these three kinds through.
fn node(block: &Block) -> Option<Node> {
    let nodes = |children: &[Block]| children.iter().map(node).collect::<Option<Vec<Node>>>();
    match block {
        Block::Asset { asset_id, .. } => Some(Node::Asset(Asset::Token { id: asset_id.to_ascii_lowercase() })),
        Block::Weight { weight_type: WeightType::Equal, children, .. } => {
            Some(Node::Weight { name: None, weighting: Weighting::Equal, children: nodes(children)? })
        }
        Block::Weight { weight_type: WeightType::SpecifiedPercentage, weightings, children, .. } => {
            let shares = weightings.iter().map(|percent| percent.trim().parse::<f64>().ok().map(|percent| percent / 100.0)).collect::<Option<_>>()?;
            Some(Node::Weight { name: None, weighting: Weighting::Specified { shares }, children: nodes(children)? })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::MAX_NODES;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;

    fn address(n: usize) -> String {
        format!("0x{n:040x}:56")
    }

    fn asset(n: usize) -> Value {
        json!({ "blockType": "asset", "assetId": address(n), "blockIdForInstance": format!("asset-{n}") })
    }

    fn weight(id: &str, kind: &str, weightings: &[&str], children: Vec<Value>) -> Value {
        json!({ "blockType": "weight", "weightType": kind, "weightings": weightings, "children": children, "blockIdForInstance": id })
    }

    fn document(child: Value) -> Value {
        json!({
            "entry": { "blockType": "topLevel", "name": "Steady", "children": child, "blockIdForInstance": "top" },
            "version": "1.0.0",
            "tradingSettings": { "type": "threshold", "triggerPercentage": 10 }
        })
    }

    fn parse(value: Value) -> Strategy {
        serde_json::from_value(value).expect("document parses")
    }

    fn token(n: usize) -> Node {
        Node::Asset(Asset::Token { id: address(n) })
    }

    /// Hand `share` down a tree of weights and assets, the only blocks a
    /// fixed-weight fund turns into.
    fn spread(node: &Node, share: f64, targets: &mut BTreeMap<String, f64>) {
        match node {
            Node::Asset(Asset::Token { id }) => *targets.entry(id.clone()).or_insert(0.0) += share,
            Node::Weight { weighting: Weighting::Equal, children, .. } => children.iter().for_each(|child| spread(child, share / children.len() as f64, targets)),
            Node::Weight { weighting: Weighting::Specified { shares }, children, .. } => {
                children.iter().zip(shares).for_each(|(child, part)| spread(child, share * part, targets))
            }
            other => panic!("a fixed-weight fund does not hold {other:?}"),
        }
    }

    #[test]
    fn a_fund_becomes_the_same_tree_with_percentages_as_fractions() {
        let inner = weight("inner", "equal", &["100", "0"], vec![asset(2), asset(3)]);
        let fund = parse(document(weight("w", "specified-percentage", &["60", " 40 "], vec![asset(1), inner])));
        let tree = RuleStrategy::from_fixed(&fund).expect("converts");
        let expected = Node::Weight {
            name: None,
            weighting: Weighting::Specified { shares: vec![0.6, 0.4] },
            children: vec![token(1), Node::Weight { name: None, weighting: Weighting::Equal, children: vec![token(2), token(3)] }],
        };
        assert_eq!(tree, RuleStrategy { name: "Steady".to_string(), rebalance: Rebalance::Threshold { corridor: 0.1 }, root: expected });
        assert!(tree.validate(MAX_NODES).is_empty());
        assert_eq!(tree.node_count(), 6);
    }

    #[test]
    fn a_fund_of_one_asset_converts_with_no_weight_block() {
        let tree = RuleStrategy::from_fixed(&parse(document(asset(1)))).expect("converts");
        assert_eq!(tree.root, token(1));
        assert!(tree.validate(MAX_NODES).is_empty());
    }

    /// Any fund the engine runs today resolves to the same targets from its
    /// rule tree as from the document.
    #[test]
    fn the_tree_resolves_to_the_targets_the_document_does() {
        let mut state: u64 = 0x5EED;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        fn block(depth: usize, counter: &mut usize, next: &mut dyn FnMut() -> u64) -> Value {
            *counter += 1;
            let id = format!("b{counter}");
            if depth >= 4 || next().is_multiple_of(3) {
                return json!({ "blockType": "asset", "assetId": address((next() % 12) as usize + 1), "blockIdForInstance": id });
            }
            let size = (next() % 4) as usize + 1;
            let children: Vec<Value> = (0..size).map(|_| block(depth + 1, counter, next)).collect();
            if next().is_multiple_of(2) {
                return weight(&id, "equal", &[], children);
            }
            // Whole percentages that total exactly 100.
            let mut left = 100u64;
            let weightings: Vec<String> = (0..size)
                .map(|index| {
                    let take = if index + 1 == size { left } else { next() % (left + 1) };
                    left -= take;
                    take.to_string()
                })
                .collect();
            weight(&id, "specified-percentage", &weightings.iter().map(String::as_str).collect::<Vec<_>>(), children)
        }
        for case in 0..300 {
            let fund = parse(document(block(2, &mut 0, &mut next)));
            let tree = RuleStrategy::from_fixed(&fund).unwrap_or_else(|issues| panic!("case {case}: {issues:?}"));
            assert!(tree.validate(MAX_NODES).is_empty(), "case {case}");
            let mut from_tree = BTreeMap::new();
            spread(&tree.root, 1.0, &mut from_tree);
            let from_document = fund.targets().expect("valid");
            assert_eq!(from_tree.keys().collect::<Vec<_>>(), from_document.keys().collect::<Vec<_>>(), "case {case}");
            for (asset_id, share) in &from_document {
                assert!((from_tree[asset_id] - share).abs() < 1e-12, "case {case}: {asset_id}");
            }
        }
    }

    #[test]
    fn the_rebalance_rule_carries_over() {
        let with = |settings: Value| {
            let mut fund = document(asset(1));
            fund["tradingSettings"] = settings;
            RuleStrategy::from_fixed(&parse(fund))
        };
        let rebalance = |settings: Value| with(settings).map(|tree| tree.rebalance);
        assert_eq!(rebalance(json!({ "type": "threshold", "triggerPercentage": 5 })), Ok(Rebalance::Threshold { corridor: 0.05 }));
        for (word, frequency) in [("daily", Frequency::Daily), ("weekly", Frequency::Weekly), (" Monthly ", Frequency::Monthly), ("quarterly", Frequency::Quarterly), ("YEARLY", Frequency::Yearly)] {
            assert_eq!(rebalance(json!({ "type": "calendar", "frequency": word })), Ok(Rebalance::Calendar(frequency)), "{word}");
        }
        let issues = with(json!({ "type": "calendar", "frequency": "fortnightly" })).expect_err("refused");
        assert_eq!(issues.iter().map(|issue| (issue.code, issue.block_id.as_str())).collect::<Vec<_>>(), vec![(IssueCode::InvalidTradingSettings, "root")]);
        // An empty schedule is the document's own problem, reported once.
        let issues = with(json!({ "type": "calendar", "frequency": " " })).expect_err("refused");
        assert_eq!(issues.iter().map(|issue| issue.code).collect::<Vec<_>>(), vec![IssueCode::InvalidTradingSettings]);
    }

    #[test]
    fn a_fund_with_problems_gives_its_problems_and_no_tree() {
        let short = parse(document(weight("w", "specified-percentage", &["60", "30"], vec![asset(1), asset(2)])));
        assert_eq!(RuleStrategy::from_fixed(&short), Err(short.validate()));
        assert_eq!(short.validate().len(), 1);

        let market_cap = parse(document(weight("w", "market-cap", &[], vec![asset(1), asset(2)])));
        assert_eq!(RuleStrategy::from_fixed(&market_cap).expect_err("refused")[0].code, IssueCode::UnsupportedBlock);
        let conditional = parse(document(json!({ "blockType": "conditional", "blockIdForInstance": "if" })));
        assert_eq!(RuleStrategy::from_fixed(&conditional).expect_err("refused")[0].code, IssueCode::UnsupportedBlock);
        let mut empty = document(asset(1));
        empty["entry"]["children"] = Value::Null;
        assert_eq!(RuleStrategy::from_fixed(&parse(empty)).expect_err("refused")[0].code, IssueCode::IncompleteBlock);
    }

    #[test]
    fn two_funds_that_differ_only_in_ids_names_and_case_hash_the_same() {
        let fund = |name: &str, id: &str, asset_id: String| {
            let mut document = document(weight(id, "equal", &[], vec![json!({ "blockType": "asset", "assetId": asset_id, "blockIdForInstance": format!("{id}-asset") })]));
            document["entry"]["name"] = json!(name);
            RuleStrategy::from_fixed(&parse(document)).expect("converts")
        };
        let one = fund("One", "w1", address(0xabc));
        assert_eq!(fund("Two", "w2", address(0xabc).to_uppercase().replace("0X", "0x")).hash(), one.hash());
        assert_ne!(fund("One", "w1", address(0xabd)).hash(), one.hash());
        assert_eq!(one.root, Node::Weight { name: None, weighting: Weighting::Equal, children: vec![token(0xabc)] });
    }
}
