//! Validation: what a tree must satisfy before it can be evaluated.
//!
//! The limits are kept loose where real strategies are loose. A block may
//! hold any number of children, an if may sit anywhere, and a branch may
//! hold several blocks. What is refused is what has no clear meaning: an
//! empty block, shares that do not add up, a filter that keeps more than it
//! holds, a price ranked across blocks that have no price.

use super::{
    is_symbol, Asset, Comparison, Condition, Indicator, Issue, IssueCode, Node, Operand, Reading, Rebalance, RuleStrategy, Trail, Unit, Weighting,
    MAX_CONDITIONS, MAX_CONDITION_DEPTH, MAX_DEPTH, MAX_WINDOW,
};
use crate::strategy::parse_asset_id;

/// How far the shares of a specified weight may sit from 1 in total.
const SHARE_TOTAL_TOLERANCE: f64 = 0.00009;

impl RuleStrategy {
    /// Every problem in the strategy. Empty means it can be evaluated.
    /// `node_limit` is the most it may count, as `node_count` counts.
    pub fn validate(&self, node_limit: usize) -> Vec<Issue> {
        let mut check = Check::default();
        if let Rebalance::Threshold { corridor } = self.rebalance {
            if !corridor.is_finite() || corridor <= 0.0 || corridor > 1.0 {
                check.trail.report_at(
                    "rebalance",
                    IssueCode::InvalidRebalance,
                    format!("The rebalance corridor is {corridor}, which is not above 0 and at most 1."),
                    "Set a corridor as a fraction, such as 0.05 for 5%.",
                );
            }
        }
        check.node(&self.root, 1);
        let nodes = self.node_count();
        if nodes > node_limit {
            check.trail.report(
                IssueCode::ExcessiveNodes,
                format!("The strategy counts {nodes} nodes; the most allowed is {node_limit}."),
                "Remove blocks, or join nested ifs into one any/all condition.",
            );
        }
        check.trail.issues
    }
}

#[derive(Default)]
struct Check {
    trail: Trail,
}

impl Check {
    fn node(&mut self, node: &Node, depth: usize) {
        if depth > MAX_DEPTH {
            self.trail.report(IssueCode::ExcessiveDepth, format!("The strategy has more than {MAX_DEPTH} nested levels."), "Reduce the nesting.");
            return;
        }
        match node {
            Node::Asset(asset) => self.asset(asset),
            Node::Weight { weighting, children, .. } => {
                self.weighting(weighting, children.len());
                self.children("children", children, depth, "This weight block is empty.");
            }
            Node::If { condition, then, otherwise } => {
                self.trail.enter("condition");
                self.condition(condition, 1);
                self.trail.leave();
                self.children("then", then, depth, "The branch this if block takes when its condition is true is empty.");
                self.children("else", otherwise, depth, "The else branch of this if block is empty.");
            }
            Node::Filter { sort_by, count, children, .. } => {
                self.indicator(sort_by);
                if !children.is_empty() && (*count == 0 || *count as usize > children.len()) {
                    self.trail.report(
                        IssueCode::InvalidFilter,
                        format!("This filter keeps {count} of the {} items it holds.", children.len()),
                        "Keep at least one item and no more than it holds.",
                    );
                }
                // A block has a value over time but no price per share, so a
                // price cannot rank it against anything.
                if sort_by.unit() == Unit::Dollars && children.iter().any(|child| !matches!(child, Node::Asset(_))) {
                    self.trail.report(
                        IssueCode::InvalidFilter,
                        "This filter sorts by a price but holds blocks that are not assets.",
                        "Sort by a return, or hold only assets in it.",
                    );
                }
                self.children("children", children, depth, "This filter is empty.");
            }
        }
    }

    fn children(&mut self, field: &'static str, children: &[Node], depth: usize, when_empty: &str) {
        if children.is_empty() {
            self.trail.report(IssueCode::IncompleteBlock, when_empty, "Add at least one block to it.");
        }
        for (index, child) in children.iter().enumerate() {
            self.trail.enter_item(field, index);
            self.node(child, depth + 1);
            self.trail.leave();
        }
    }

    fn weighting(&mut self, weighting: &Weighting, children: usize) {
        match weighting {
            Weighting::Equal => {}
            // It is the deviation of returns that weighs the children.
            Weighting::InverseVolatility { window } => self.window(*window, Indicator::ReturnDeviation { window: *window }.least_window()),
            Weighting::Specified { shares } => {
                if shares.len() != children {
                    self.trail.report(
                        IssueCode::InconsistentWeights,
                        format!("This block has {children} items but {} shares.", shares.len()),
                        "Give every item exactly one share.",
                    );
                }
                let mut total = Some(0.0);
                for (index, share) in shares.iter().enumerate() {
                    if share.is_finite() && (0.0..=1.0).contains(share) {
                        total = total.map(|total| total + share);
                    } else {
                        total = None;
                        self.trail.report(
                            IssueCode::InvalidWeightValue,
                            format!("Share {} is {share}, which is not a number from 0 to 1.", index + 1),
                            "Use shares between 0 and 1.",
                        );
                    }
                }
                if let Some(total) = total.filter(|total| (total - 1.0).abs() > SHARE_TOTAL_TOLERANCE) {
                    self.trail.report(
                        IssueCode::InvalidWeightTotal,
                        format!("The shares add up to {total}, not 1."),
                        "Adjust the shares so they add up to 1.",
                    );
                }
            }
        }
    }

    /// `depth` counts the any/all groups around this condition, itself included.
    fn condition(&mut self, condition: &Condition, depth: usize) {
        let (field, parts) = match condition {
            Condition::Compare(comparison) => return self.comparison(comparison),
            Condition::Any(parts) => ("any", parts),
            Condition::All(parts) => ("all", parts),
        };
        if depth > MAX_CONDITION_DEPTH {
            self.trail.report(
                IssueCode::ExcessiveDepth,
                format!("This condition nests any/all groups more than {MAX_CONDITION_DEPTH} deep."),
                "Flatten the groups.",
            );
            return;
        }
        if parts.is_empty() {
            self.trail.report(IssueCode::IncompleteCondition, "This any/all group has no conditions.", "Add at least one condition to it.");
        }
        if parts.len() > MAX_CONDITIONS {
            self.trail.report(
                IssueCode::ExcessiveConditions,
                format!("This any/all group has {} conditions; the most allowed is {MAX_CONDITIONS}.", parts.len()),
                "Remove conditions from it.",
            );
        }
        for (index, part) in parts.iter().enumerate() {
            self.trail.enter_item(field, index);
            self.condition(part, depth + 1);
            self.trail.leave();
        }
    }

    fn comparison(&mut self, comparison: &Comparison) {
        self.trail.enter("left");
        self.reading(&comparison.left);
        self.trail.leave();
        self.trail.enter("right");
        match &comparison.right {
            Operand::Reading(reading) => self.reading(reading),
            Operand::Fixed(value) if !value.is_finite() => {
                self.trail.report(IssueCode::InvalidFixedValue, format!("The fixed value is {value}, which is not a number."), "Use a number.")
            }
            Operand::Fixed(_) => {}
        }
        self.trail.leave();
    }

    fn reading(&mut self, reading: &Reading) {
        self.indicator(&reading.indicator);
        self.asset(&reading.asset);
    }

    fn indicator(&mut self, indicator: &Indicator) {
        if let Some(window) = indicator.window() {
            self.window(window, indicator.least_window());
        }
    }

    /// `least` is the shortest window the indicator has a meaning over.
    fn window(&mut self, window: u32, least: u32) {
        if window < least || window > MAX_WINDOW {
            self.trail.report(
                IssueCode::InvalidWindow,
                format!("A window of {window} days is not from {least} to {MAX_WINDOW}."),
                format!("Use a window from {least} to {MAX_WINDOW} days."),
            );
        }
    }

    fn asset(&mut self, asset: &Asset) {
        let (sound, shown) = match asset {
            Asset::Listed { symbol, .. } => (is_symbol(symbol), symbol),
            Asset::Token { id } => (parse_asset_id(id).is_some() && !id.bytes().any(|byte| byte.is_ascii_uppercase()), id),
        };
        if !sound {
            self.trail.report(
                IssueCode::InvalidAsset,
                format!("{shown:?} is not a valid asset."),
                "Use a market symbol such as SPY, or a token address and chain id in lower case, like 0x…:8453.",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{Comparator, Frequency, Market, Take, MAX_NODES};

    fn asset(symbol: &str) -> Node {
        Node::Asset(Asset::Listed { market: Market::Equities, symbol: symbol.to_string() })
    }

    fn equal(children: Vec<Node>) -> Node {
        Node::Weight { name: None, weighting: Weighting::Equal, children }
    }

    fn specified(shares: &[f64], children: Vec<Node>) -> Node {
        Node::Weight { name: None, weighting: Weighting::Specified { shares: shares.to_vec() }, children }
    }

    fn filter(sort_by: Indicator, count: u32, children: Vec<Node>) -> Node {
        Node::Filter { sort_by, take: Take::Top, count, children }
    }

    fn reading(indicator: Indicator, symbol: &str) -> Reading {
        Reading { indicator, asset: Asset::Listed { market: Market::Equities, symbol: symbol.to_string() } }
    }

    /// The indicator on SPY is above a fixed value.
    fn above(indicator: Indicator, fixed: f64) -> Condition {
        Condition::Compare(Comparison { left: reading(indicator, "SPY"), comparator: Comparator::Above, right: Operand::Fixed(fixed) })
    }

    fn rsi_above_half() -> Condition {
        above(Indicator::RelativeStrength { window: 14 }, 0.5)
    }

    /// Hold SPY when the condition is true and BIL when it is not.
    fn when(condition: Condition) -> Node {
        Node::If { condition, then: vec![asset("SPY")], otherwise: vec![asset("BIL")] }
    }

    fn strategy(root: Node) -> RuleStrategy {
        RuleStrategy { name: "Test".to_string(), rebalance: Rebalance::Calendar(Frequency::Daily), root }
    }

    fn found(root: Node) -> Vec<(IssueCode, String)> {
        strategy(root).validate(MAX_NODES).into_iter().map(|issue| (issue.code, issue.path)).collect()
    }

    fn codes(root: Node) -> Vec<IssueCode> {
        found(root).into_iter().map(|(code, _)| code).collect()
    }

    fn at(code: IssueCode, path: &str) -> Vec<(IssueCode, String)> {
        vec![(code, path.to_string())]
    }

    #[test]
    fn the_corridor_is_a_fraction_above_zero_and_at_most_one() {
        let with = |corridor| RuleStrategy { rebalance: Rebalance::Threshold { corridor }, ..strategy(asset("SPY")) };
        for good in [0.0001, 0.05, 1.0] {
            assert!(with(good).validate(MAX_NODES).is_empty(), "{good}");
        }
        for bad in [0.0, -0.05, 1.01, 5.0, f64::NAN, f64::INFINITY] {
            let issues = with(bad).validate(MAX_NODES);
            assert_eq!(issues.iter().map(|issue| (issue.code, issue.path.as_str())).collect::<Vec<_>>(), vec![(IssueCode::InvalidRebalance, "rebalance")], "{bad}");
        }
    }

    #[test]
    fn a_strategy_counts_no_more_nodes_than_its_limit() {
        // One for the root and one for each block, but two for the if, and
        // nothing for the three comparisons in its condition.
        let condition = Condition::Any(vec![rsi_above_half(), rsi_above_half(), rsi_above_half()]);
        let tree = strategy(equal(vec![when(condition), asset("GLD"), asset("TLT")]));
        assert_eq!(tree.node_count(), 1 + 1 + (2 + 1 + 1) + 1 + 1);
        assert!(tree.validate(8).is_empty());
        let issues = tree.validate(7);
        assert_eq!(issues.iter().map(|issue| (issue.code, issue.path.as_str())).collect::<Vec<_>>(), vec![(IssueCode::ExcessiveNodes, "root")]);
        assert_eq!(issues[0].message, "The strategy counts 8 nodes; the most allowed is 7.");
    }

    #[test]
    fn blocks_nest_one_hundred_deep_and_no_deeper() {
        let nest = |levels: usize| (1..levels).fold(asset("SPY"), |inner, _| equal(vec![inner]));
        assert!(found(nest(100)).is_empty());
        assert_eq!(codes(nest(101)), vec![IssueCode::ExcessiveDepth]);
        // A tree far past the limit is reported once, where the limit is crossed.
        assert_eq!(codes(nest(250)), vec![IssueCode::ExcessiveDepth]);
    }

    #[test]
    fn a_block_that_holds_nothing_is_incomplete() {
        assert_eq!(found(equal(vec![])), at(IssueCode::IncompleteBlock, "root"));
        assert_eq!(found(equal(vec![asset("SPY"), equal(vec![])])), at(IssueCode::IncompleteBlock, "root.children[1]"));
        assert_eq!(found(filter(Indicator::CumulativeReturn { window: 20 }, 1, vec![])), at(IssueCode::IncompleteBlock, "root"));
        let no_then = Node::If { condition: rsi_above_half(), then: vec![], otherwise: vec![asset("BIL")] };
        let no_else = Node::If { condition: rsi_above_half(), then: vec![asset("SPY")], otherwise: vec![] };
        for empty in [no_then, no_else] {
            assert_eq!(found(equal(vec![empty])), at(IssueCode::IncompleteBlock, "root.children[0]"));
        }
    }

    #[test]
    fn every_child_of_a_specified_weight_has_one_share() {
        assert!(found(specified(&[0.6, 0.4], vec![asset("SPY"), asset("TLT")])).is_empty());
        for shares in [vec![1.0], vec![0.5, 0.25, 0.25]] {
            assert_eq!(codes(specified(&shares, vec![asset("SPY"), asset("TLT")])), vec![IssueCode::InconsistentWeights], "{shares:?}");
        }
    }

    #[test]
    fn a_share_is_a_number_from_zero_to_one() {
        assert!(found(specified(&[0.0, 1.0], vec![asset("SPY"), asset("TLT")])).is_empty());
        for bad in [-0.1, 1.5, f64::NAN, f64::INFINITY] {
            assert_eq!(codes(specified(&[bad, 1.0], vec![asset("SPY"), asset("TLT")])), vec![IssueCode::InvalidWeightValue], "{bad}");
        }
    }

    #[test]
    fn shares_add_up_to_one() {
        let three = || vec![asset("SPY"), asset("TLT"), asset("GLD")];
        assert!(found(specified(&[1.0 / 3.0, 1.0 / 3.0, 1.0 / 3.0], three())).is_empty());
        assert!(found(specified(&[0.3334, 0.3333, 0.3333], three())).is_empty());
        // Thirds rounded to two decimals of a percent fall one part in ten thousand short.
        let issues = strategy(specified(&[0.3333, 0.3333, 0.3333], three())).validate(MAX_NODES);
        assert_eq!(issues.iter().map(|issue| issue.code).collect::<Vec<_>>(), vec![IssueCode::InvalidWeightTotal]);
        assert_eq!(issues[0].message, "The shares add up to 0.9999, not 1.");
        assert_eq!(codes(specified(&[0.6, 0.3], vec![asset("SPY"), asset("TLT")])), vec![IssueCode::InvalidWeightTotal]);
        assert_eq!(codes(specified(&[0.7, 0.7], vec![asset("SPY"), asset("TLT")])), vec![IssueCode::InvalidWeightTotal]);
    }

    #[test]
    fn a_window_runs_from_one_day_to_one_thousand() {
        let inverse = |window| Node::Weight { name: None, weighting: Weighting::InverseVolatility { window }, children: vec![asset("SPY"), asset("TLT")] };
        for good in [2, 20, 1000] {
            assert!(found(inverse(good)).is_empty(), "{good}");
            assert!(found(filter(Indicator::CumulativeReturn { window: good }, 1, vec![asset("SPY"), asset("TLT")])).is_empty(), "{good}");
            assert!(found(when(above(Indicator::MaxDrawdown { window: good }, 0.1))).is_empty(), "{good}");
        }
        for bad in [0, 1001] {
            assert_eq!(found(inverse(bad)), at(IssueCode::InvalidWindow, "root"), "{bad}");
            assert_eq!(found(filter(Indicator::CumulativeReturn { window: bad }, 1, vec![asset("SPY"), asset("TLT")])), at(IssueCode::InvalidWindow, "root"), "{bad}");
            assert_eq!(found(when(above(Indicator::MaxDrawdown { window: bad }, 0.1))), at(IssueCode::InvalidWindow, "root.condition.left"), "{bad}");
            let right = Operand::Reading(reading(Indicator::AveragePrice { window: bad }, "SPY"));
            let against = Condition::Compare(Comparison { left: reading(Indicator::Price, "SPY"), comparator: Comparator::Above, right });
            assert_eq!(found(when(against)), at(IssueCode::InvalidWindow, "root.condition.right"), "{bad}");
        }
    }

    #[test]
    fn a_deviation_a_drawdown_and_relative_strength_need_two_days() {
        // A one day average is the price and a one day return is a return.
        for one_day in [Indicator::AveragePrice { window: 1 }, Indicator::ExponentialAveragePrice { window: 1 }, Indicator::CumulativeReturn { window: 1 }, Indicator::AverageReturn { window: 1 }] {
            assert!(found(when(above(one_day, 0.0))).is_empty(), "{one_day:?}");
        }
        for two_days in [Indicator::PriceDeviation { window: 1 }, Indicator::ReturnDeviation { window: 1 }, Indicator::MaxDrawdown { window: 1 }, Indicator::RelativeStrength { window: 1 }] {
            let issues = strategy(when(above(two_days, 0.0))).validate(MAX_NODES);
            assert_eq!(issues.iter().map(|issue| (issue.code, issue.path.as_str())).collect::<Vec<_>>(), vec![(IssueCode::InvalidWindow, "root.condition.left")], "{two_days:?}");
            assert_eq!(issues[0].message, "A window of 1 days is not from 2 to 1000.");
            assert_eq!(codes(filter(two_days, 1, vec![asset("SPY"), asset("TLT")])), vec![IssueCode::InvalidWindow], "{two_days:?}");
        }
        // Inverse volatility weighs by the deviation of returns.
        let inverse = Node::Weight { name: None, weighting: Weighting::InverseVolatility { window: 1 }, children: vec![asset("SPY"), asset("TLT")] };
        assert_eq!(found(inverse), at(IssueCode::InvalidWindow, "root"));
    }

    #[test]
    fn a_filter_keeps_at_least_one_item_and_no_more_than_it_holds() {
        let over = |count| filter(Indicator::RelativeStrength { window: 10 }, count, vec![asset("SPY"), asset("QQQ"), asset("IWM")]);
        for good in [1, 2, 3] {
            assert!(found(over(good)).is_empty(), "{good}");
        }
        for bad in [0, 4] {
            assert_eq!(codes(over(bad)), vec![IssueCode::InvalidFilter], "{bad}");
        }
    }

    #[test]
    fn a_filter_ranks_blocks_by_a_return_but_only_assets_by_a_price() {
        let mixed = || vec![asset("SPY"), equal(vec![asset("TLT"), asset("GLD")])];
        for by_return in [Indicator::CumulativeReturn { window: 20 }, Indicator::AverageReturn { window: 20 }, Indicator::ReturnDeviation { window: 20 }, Indicator::MaxDrawdown { window: 20 }, Indicator::RelativeStrength { window: 20 }] {
            assert!(found(filter(by_return, 1, mixed())).is_empty(), "{by_return:?}");
        }
        for by_price in [Indicator::Price, Indicator::AveragePrice { window: 20 }, Indicator::ExponentialAveragePrice { window: 20 }, Indicator::PriceDeviation { window: 20 }] {
            assert_eq!(codes(filter(by_price, 1, mixed())), vec![IssueCode::InvalidFilter], "{by_price:?}");
            assert!(found(filter(by_price, 1, vec![asset("SPY"), asset("TLT")])).is_empty(), "{by_price:?}");
        }
    }

    #[test]
    fn an_any_or_all_group_holds_from_one_to_fifty_conditions() {
        let group = |size: usize| Condition::All(vec![rsi_above_half(); size]);
        for good in [1, 2, 50] {
            assert!(found(when(group(good))).is_empty(), "{good}");
        }
        assert_eq!(found(when(group(0))), at(IssueCode::IncompleteCondition, "root.condition"));
        assert_eq!(found(when(group(51))), at(IssueCode::ExcessiveConditions, "root.condition"));
        let inner_empty = Condition::Any(vec![rsi_above_half(), Condition::All(vec![])]);
        assert_eq!(found(when(inner_empty)), at(IssueCode::IncompleteCondition, "root.condition.any[1]"));
    }

    #[test]
    fn any_and_all_groups_nest_twenty_deep_and_no_deeper() {
        let nest = |levels: usize| (0..levels).fold(rsi_above_half(), |inner, level| if level % 2 == 0 { Condition::Any(vec![inner]) } else { Condition::All(vec![inner]) });
        assert!(found(when(nest(20))).is_empty());
        assert_eq!(codes(when(nest(21))), vec![IssueCode::ExcessiveDepth]);
        assert_eq!(codes(when(nest(200))), vec![IssueCode::ExcessiveDepth]);
    }

    #[test]
    fn a_fixed_value_is_a_real_number() {
        for good in [0.0, -0.13, 412.5] {
            assert!(found(when(above(Indicator::Price, good))).is_empty(), "{good}");
        }
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(found(when(above(Indicator::Price, bad))), at(IssueCode::InvalidFixedValue, "root.condition.right"), "{bad}");
        }
    }

    #[test]
    fn an_asset_is_a_market_symbol_or_a_token_on_a_chain() {
        let token = |id: &str| Node::Asset(Asset::Token { id: id.to_string() });
        for good in [asset("SPY"), asset("BRK/B"), token("0x00000000000000000000000000000000000000aa:56")] {
            assert!(found(good.clone()).is_empty(), "{good:?}");
        }
        let mixed_case = token("0x00000000000000000000000000000000000000AA:56");
        for bad in [asset(""), asset("spy"), asset("BRK.B"), asset("BRK/"), asset("$USD"), token("0x1234:56"), token("SPY"), mixed_case] {
            assert_eq!(found(equal(vec![asset("TLT"), bad.clone()])), at(IssueCode::InvalidAsset, "root.children[1]"), "{bad:?}");
        }
        let on_bad_asset = Condition::Compare(Comparison { left: reading(Indicator::Price, "spy"), comparator: Comparator::Above, right: Operand::Fixed(1.0) });
        assert_eq!(found(when(on_bad_asset)), at(IssueCode::InvalidAsset, "root.condition.left"));
    }

    #[test]
    fn the_loose_shapes_real_strategies_use_are_valid() {
        let wide = equal((0..65).map(|_| asset("SPY")).collect());
        let branch_of_several = Node::If { condition: rsi_above_half(), then: vec![asset("SPXL"), asset("TQQQ")], otherwise: vec![when(rsi_above_half())] };
        let filter_in_a_branch = Node::If { condition: rsi_above_half(), then: vec![filter(Indicator::CumulativeReturn { window: 5 }, 1, vec![asset("XLE"), asset("XLV")])], otherwise: vec![asset("BIL")] };
        let named_block_of_one_asset = Node::Weight { name: Some("g".to_string()), weighting: Weighting::Equal, children: vec![asset("XLI")] };
        let blocks_under_inverse_volatility = Node::Weight { name: None, weighting: Weighting::InverseVolatility { window: 30 }, children: vec![equal(vec![asset("TLT")]), asset("GLD")] };
        let zero_share = specified(&[1.0, 0.0], vec![asset("SPY"), asset("BIL")]);
        for loose in [wide, branch_of_several, filter_in_a_branch, named_block_of_one_asset, blocks_under_inverse_volatility, zero_share, asset("SPY")] {
            assert!(found(loose.clone()).is_empty(), "{loose:?}");
        }
    }

    #[test]
    fn every_problem_is_found_and_placed_in_the_tree() {
        let bad_group = Condition::All(vec![rsi_above_half(), Condition::Any(vec![above(Indicator::CumulativeReturn { window: 0 }, f64::NAN)])]);
        let tree = equal(vec![
            specified(&[0.5, 0.6], vec![asset("SPY"), asset("spy")]),
            Node::If { condition: bad_group, then: vec![filter(Indicator::Price, 5, vec![asset("QQQ"), equal(vec![])])], otherwise: vec![] },
        ]);
        let expected = [
            (IssueCode::InvalidWeightTotal, "root.children[0]"),
            (IssueCode::InvalidAsset, "root.children[0].children[1]"),
            (IssueCode::InvalidWindow, "root.children[1].condition.all[1].any[0].left"),
            (IssueCode::InvalidFixedValue, "root.children[1].condition.all[1].any[0].right"),
            (IssueCode::InvalidFilter, "root.children[1].then[0]"),
            (IssueCode::InvalidFilter, "root.children[1].then[0]"),
            (IssueCode::IncompleteBlock, "root.children[1].then[0].children[1]"),
            (IssueCode::IncompleteBlock, "root.children[1]"),
        ];
        assert_eq!(found(tree), expected.map(|(code, path)| (code, path.to_string())));
    }
}
