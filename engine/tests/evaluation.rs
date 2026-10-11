//! Evaluating a strategy at one close, on made-up prices small enough to
//! work out by hand. The cases in `tests/cases` cover the same rules over
//! long runs, against numbers worked out apart from this engine.

mod support;

use compounder_engine::evaluate::{evaluate, Evaluator, Read, ReadOn};
use compounder_engine::rules::{Asset, Comparator, Condition, Indicator, IssueCode, Node, Take};
use support::*;

const RETURN_1: Indicator = Indicator::CumulativeReturn { window: 1 };
const RETURN_2: Indicator = Indicator::CumulativeReturn { window: 2 };

/// Four assets over six days. AAA rises, BBB falls, CCC is flat and DDD jumps about.
fn four() -> (Vec<compounder_prices::day::Day>, compounder_engine::history::History) {
    let days = weekdays("2025-03-03", 6);
    let history = history(
        &days,
        &[
            ("AAA", 0, &[100.0, 101.0, 102.0, 104.0, 105.0, 108.0]),
            ("BBB", 0, &[50.0, 49.5, 49.0, 48.0, 47.5, 47.0]),
            ("CCC", 0, &[20.0, 20.0, 20.0, 20.0, 20.0, 20.0]),
            ("DDD", 0, &[10.0, 12.0, 9.0, 9.0, 11.0, 10.0]),
        ],
    );
    (days, history)
}

/// The weights of a strategy on one day, by symbol, without the assets it holds none of.
fn held(root: Node, at: usize) -> Vec<(String, f64)> {
    let (days, history) = four();
    let evaluator = Evaluator::new(&daily(root), &history, false).expect("a sound strategy");
    let evaluation = evaluator.on(days[at]).expect("a day it can be evaluated on");
    assert!(close(evaluation.weights.iter().sum::<f64>(), 1.0, 1e-15), "the weights sum to {}", evaluation.weights.iter().sum::<f64>());
    let symbol = |asset: &Asset| match asset {
        Asset::Listed { symbol, .. } => symbol.clone(),
        Asset::Token { id } => id.clone(),
    };
    evaluator.assets().iter().map(symbol).zip(evaluation.weights).filter(|(_, weight)| *weight != 0.0).collect()
}

fn holds(root: Node, at: usize, expected: &[(&str, f64)]) {
    let got = held(root, at);
    let same = got.len() == expected.len() && got.iter().zip(expected).all(|((symbol, weight), (wanted, share))| symbol == wanted && close(*weight, *share, 1e-15));
    assert!(same, "{got:?}, not {expected:?}");
}

#[test]
fn shares_multiply_down_the_tree_and_an_asset_reached_twice_adds_up() {
    // A third each to: 60/40 of AAA and BBB, a half each of CCC and a pair of AAA and DDD, and BBB.
    let root = equal(vec![specified(&[0.6, 0.4], vec![asset("AAA"), asset("BBB")]), equal(vec![asset("CCC"), equal(vec![asset("AAA"), asset("DDD")])]), asset("BBB")]);
    let third = 1.0 / 3.0;
    holds(root, 0, &[("AAA", third * 0.6 + third * 0.5 * 0.5), ("BBB", third * 0.4 + third), ("CCC", third * 0.5), ("DDD", third * 0.5 * 0.5)]);
    holds(asset("CCC"), 0, &[("CCC", 1.0)]);
    // A share of nothing holds nothing.
    holds(specified(&[0.0, 1.0], vec![asset("AAA"), asset("BBB")]), 0, &[("BBB", 1.0)]);
}

#[test]
fn an_if_holds_one_branch_and_several_blocks_in_a_branch_share_it_equally() {
    let rose = |symbol: &str| against(RETURN_1, symbol, Comparator::Above, 0.0);
    let tree = || when(rose("DDD"), vec![asset("AAA"), asset("BBB")], vec![when(rose("BBB"), vec![asset("BBB")], vec![asset("CCC"), asset("AAA"), asset("DDD")])]);
    // DDD rose on day 1 and fell on day 2. BBB never rose.
    holds(tree(), 1, &[("AAA", 0.5), ("BBB", 0.5)]);
    holds(tree(), 2, &[("AAA", 1.0 / 3.0), ("CCC", 1.0 / 3.0), ("DDD", 1.0 / 3.0)]);
    // On day 3 DDD is flat: a return of exactly zero is not above zero.
    holds(tree(), 3, &[("AAA", 1.0 / 3.0), ("CCC", 1.0 / 3.0), ("DDD", 1.0 / 3.0)]);
    holds(tree(), 4, &[("AAA", 0.5), ("BBB", 0.5)]);
}

#[test]
fn a_condition_is_held_to_a_small_fixed_margin() {
    // AAA closes at 104 on day 3.
    let against_104 = |comparator, fixed: f64| when(against(Indicator::Price, "AAA", comparator, fixed), vec![asset("AAA")], vec![asset("BBB")]);
    let taken = |comparator, fixed: f64| held(against_104(comparator, fixed), 3)[0].0 == "AAA";
    // At the same value: at least, at most and equal hold. Above and below do not.
    assert_eq!([Comparator::Above, Comparator::AtLeast, Comparator::Equal, Comparator::AtMost, Comparator::Below].map(|comparator| taken(comparator, 104.0)), [false, true, true, true, false]);
    // Half a millionth of a millionth away is still the same value. Two of them is not.
    let (near, far) = (104.0 - 5e-13, 104.0 - 2e-12);
    assert!(104.0 - near > 0.0 && 104.0 - near < 1e-12 && 104.0 - far > 1e-12);
    assert!(!taken(Comparator::Above, near) && taken(Comparator::Equal, near) && taken(Comparator::AtMost, near));
    assert!(taken(Comparator::Above, far) && !taken(Comparator::Equal, far) && !taken(Comparator::AtMost, far));
    // One reading against another: CCC's price is its own two day average, to the last bit and so within the margin.
    let flat = between(reading(Indicator::Price, "CCC"), Comparator::Above, reading(Indicator::AveragePrice { window: 2 }, "CCC"));
    holds(when(flat, vec![asset("AAA")], vec![asset("BBB")]), 3, &[("BBB", 1.0)]);
}

#[test]
fn any_is_true_when_one_member_is_and_all_when_every_member_is() {
    let rose = |symbol: &str| against(RETURN_1, symbol, Comparator::Above, 0.0);
    let pick = |condition: Condition, at: usize| held(when(condition, vec![asset("AAA")], vec![asset("BBB")]), at)[0].0.clone();
    // On day 1 AAA and DDD rose and BBB fell. On day 2 only AAA rose.
    assert_eq!(pick(Condition::Any(vec![rose("BBB"), rose("DDD")]), 1), "AAA");
    assert_eq!(pick(Condition::Any(vec![rose("BBB"), rose("DDD")]), 2), "BBB");
    assert_eq!(pick(Condition::All(vec![rose("AAA"), rose("DDD")]), 1), "AAA");
    assert_eq!(pick(Condition::All(vec![rose("AAA"), rose("DDD")]), 2), "BBB");
    // Groups nest as they are written.
    let nested = |at| pick(Condition::All(vec![rose("AAA"), Condition::Any(vec![rose("BBB"), Condition::All(vec![rose("DDD")])])]), at);
    assert_eq!((nested(1), nested(2)), ("AAA".to_string(), "BBB".to_string()));
}

#[test]
fn a_filter_ranks_by_the_raw_value_and_ties_keep_list_order() {
    let four = || vec![asset("CCC"), asset("AAA"), asset("DDD"), asset("BBB")];
    // Two day returns on day 3: AAA +2.97%, BBB -3.03%, CCC 0, DDD -25%.
    holds(filter(RETURN_2, Take::Top, 1, four()), 3, &[("AAA", 1.0)]);
    holds(filter(RETURN_2, Take::Top, 2, four()), 3, &[("AAA", 0.5), ("CCC", 0.5)]);
    holds(filter(RETURN_2, Take::Bottom, 2, four()), 3, &[("BBB", 0.5), ("DDD", 0.5)]);
    holds(filter(RETURN_2, Take::Top, 4, four()), 3, &[("AAA", 0.25), ("BBB", 0.25), ("CCC", 0.25), ("DDD", 0.25)]);
    // The deepest fall in three closes on day 3: DDD 25%, BBB about 3%, AAA and CCC none.
    // Top keeps the largest value, so the deepest fall. AAA and CCC tie at zero, and the first in the list wins.
    let drawdown = Indicator::MaxDrawdown { window: 3 };
    holds(filter(drawdown, Take::Top, 1, four()), 3, &[("DDD", 1.0)]);
    holds(filter(drawdown, Take::Bottom, 1, four()), 3, &[("CCC", 1.0)]);
    holds(filter(drawdown, Take::Bottom, 1, vec![asset("AAA"), asset("CCC"), asset("DDD")]), 3, &[("AAA", 1.0)]);
    holds(filter(drawdown, Take::Bottom, 3, four()), 3, &[("AAA", 1.0 / 3.0), ("BBB", 1.0 / 3.0), ("CCC", 1.0 / 3.0)]);
    // An asset listed twice ties with itself and can be kept twice.
    holds(filter(RETURN_2, Take::Top, 2, vec![asset("AAA"), asset("BBB"), asset("AAA")]), 3, &[("AAA", 1.0)]);
}

#[test]
fn inverse_volatility_gives_the_steadier_child_more() {
    // Three daily returns to day 3. Their sample deviations, by the formula written out.
    let deviation = |closes: [f64; 4]| {
        let returns = [closes[1] / closes[0] - 1.0, closes[2] / closes[1] - 1.0, closes[3] / closes[2] - 1.0];
        let mean = (returns[0] + returns[1] + returns[2]) / 3.0;
        (returns.iter().map(|change| (change - mean) * (change - mean)).sum::<f64>() / 2.0).sqrt()
    };
    let (aaa, bbb, ddd) = (deviation([100.0, 101.0, 102.0, 104.0]), deviation([50.0, 49.5, 49.0, 48.0]), deviation([10.0, 12.0, 9.0, 9.0]));
    let total = 1.0 / aaa + 1.0 / bbb + 1.0 / ddd;
    holds(inverse_volatility(3, vec![asset("AAA"), asset("BBB"), asset("DDD")]), 3, &[("AAA", 1.0 / aaa / total), ("BBB", 1.0 / bbb / total), ("DDD", 1.0 / ddd / total)]);
    assert!(1.0 / aaa / total > 0.4 && 1.0 / ddd / total < 0.02);
    // CCC has not moved at all, so it takes the whole block.
    holds(inverse_volatility(3, vec![asset("AAA"), asset("CCC"), asset("DDD")]), 3, &[("CCC", 1.0)]);
}

/// A block that switches: AAA while DDD is above 10, else BBB.
fn switching() -> Node {
    when(against(Indicator::Price, "DDD", Comparator::Above, 10.0), vec![asset("AAA")], vec![asset("BBB")])
}

#[test]
fn a_block_is_read_on_what_it_would_have_earned_holding_its_own_choices() {
    // DDD closes at 10, 12, 9, 9, 11, 10: the block picks BBB, AAA, BBB, BBB, AAA, BBB.
    // Each day it earns the return of what it picked at the close before.
    let day_2 = 102.0 / 101.0 - 1.0; // AAA, picked on day 1
    let day_3 = 48.0 / 49.0 - 1.0; // BBB, picked on day 2
    let day_4 = 47.5 / 48.0 - 1.0; // BBB, picked on day 3
    let day_5 = 108.0 / 105.0 - 1.0; // AAA, picked on day 4
    let (level_1, level_2) = (1.0 + (49.5 / 50.0 - 1.0), (1.0 + (49.5 / 50.0 - 1.0)) * (1.0 + day_2));
    let (level_3, level_4) = (level_2 * (1.0 + day_3), level_2 * (1.0 + day_3) * (1.0 + day_4));
    let level_5 = level_4 * (1.0 + day_5);

    // The block against CCC, which never moves: the block wins when its own two day return is above zero.
    let ranked = || filter(RETURN_2, Take::Top, 1, vec![switching(), asset("CCC")]);
    let (days, history) = four();
    let evaluator = Evaluator::new(&daily(ranked()), &history, false).unwrap();
    // The block can be evaluated from day 0, so its two day return is first there on day 2.
    assert_eq!(evaluator.days(), Some((days[2], days[5])));
    for (at, block_return) in [(2, level_2 - 1.0), (3, level_3 / level_1 - 1.0), (4, level_4 / level_2 - 1.0), (5, level_5 / level_3 - 1.0)] {
        let evaluation = evaluator.on(days[at]).unwrap();
        let read = &evaluation.readings[0];
        assert_eq!((read.indicator, &read.on), (RETURN_2, &ReadOn::Block("root.children[0]".to_string())));
        assert!(close(read.value, block_return, 1e-13), "day {at}: {} is not {block_return}", read.value);
        // When the block wins the strategy holds what the block picks today, not what it held to get here.
        let (wins, picks_aaa) = (block_return > 0.0, [12.0, 9.0, 9.0, 11.0, 10.0][at - 1] > 10.0);
        let expected = match (wins, picks_aaa) {
            (false, _) => "CCC",
            (true, true) => "AAA",
            (true, false) => "BBB",
        };
        assert_eq!(held(ranked(), at), [(expected.to_string(), 1.0)], "day {at}");
    }
    // The block lost to CCC on days 2, 3 and 4. On day 5 it won on what AAA earned it, and holds BBB.
    assert!(level_2 < 1.0 && level_3 < level_1 && level_4 < level_2 && level_5 > level_3);
    assert_eq!(held(ranked(), 5), [("BBB".to_string(), 1.0)]);
}

#[test]
fn warm_up_adds_through_a_block_and_a_branch_never_taken_still_counts() {
    let days = weekdays("2025-03-03", 30);
    let (old, young): (Vec<f64>, Vec<f64>) = ((0..30).map(|index| 50.0 + f64::from(index)).collect(), (0..20).map(|index| 8.0 + f64::from(index % 3)).collect());
    let history = history(&days, &[("OLD", 0, &old), ("YOUNG", 10, &young)]);
    let first = |root: Node| Evaluator::new(&daily(root), &history, false).unwrap().days().map(|(first, _)| first);
    // YOUNG's first close is day 10.
    assert_eq!(first(asset("OLD")), Some(days[0]));
    assert_eq!(first(equal(vec![asset("OLD"), asset("YOUNG")])), Some(days[10]));
    // A three day strength of YOUNG has its first value three closes later.
    let strong = against(Indicator::RelativeStrength { window: 3 }, "YOUNG", Comparator::Above, 0.5);
    assert_eq!(first(when(strong.clone(), vec![asset("OLD")], vec![asset("OLD")])), Some(days[13]));
    // A five day return on a block that reads it needs five closes of the block on top: 3 + 5.
    let block = || when(strong.clone(), vec![asset("OLD")], vec![asset("OLD")]);
    let ranked = filter(Indicator::CumulativeReturn { window: 5 }, Take::Top, 1, vec![block(), asset("OLD")]);
    assert_eq!(first(ranked), Some(days[18]));
    // And a four day average return on a block that holds that filter: 3 + 5 + 4.
    let inner = || filter(Indicator::CumulativeReturn { window: 5 }, Take::Top, 1, vec![block(), asset("OLD")]);
    let outer = filter(Indicator::AverageReturn { window: 4 }, Take::Bottom, 1, vec![equal(vec![inner()]), asset("OLD")]);
    assert_eq!(first(outer), Some(days[22]));
    // YOUNG sits in a branch OLD's price never lets the strategy take. It still holds the start back.
    let never = against(Indicator::Price, "OLD", Comparator::Below, 0.0);
    assert_eq!(first(when(never, vec![asset("YOUNG")], vec![asset("OLD")])), Some(days[10]));
}

#[test]
fn every_reading_taken_is_reported_once_with_its_value() {
    let (days, history) = four();
    let rose = |symbol: &str| against(RETURN_1, symbol, Comparator::Above, 0.0);
    // BBB did not rise, so the else branch is taken. Its filter reads three two day returns.
    let root = when(
        Condition::Any(vec![rose("BBB"), rose("BBB"), against(Indicator::Price, "CCC", Comparator::Above, 25.0)]),
        vec![inverse_volatility(2, vec![asset("AAA"), asset("DDD")])],
        vec![filter(RETURN_2, Take::Top, 1, vec![asset("AAA"), asset("BBB"), asset("DDD")])],
    );
    let evaluation = evaluate(&daily(root), &history, days[3]).unwrap();
    let on = |symbol: &str| ReadOn::Asset(listed(symbol));
    let expected = [
        Read { indicator: RETURN_1, on: on("BBB"), value: 48.0 / 49.0 - 1.0 },
        Read { indicator: Indicator::Price, on: on("CCC"), value: 20.0 },
        Read { indicator: RETURN_2, on: on("AAA"), value: 104.0 / 101.0 - 1.0 },
        Read { indicator: RETURN_2, on: on("BBB"), value: 48.0 / 49.5 - 1.0 },
        Read { indicator: RETURN_2, on: on("DDD"), value: 9.0 / 12.0 - 1.0 },
    ];
    // The one reading asked for twice is there once. Every member of the group was read, though
    // none could change the answer after the first. The deviations in the branch not taken were not.
    assert_eq!(evaluation.readings, expected);
    assert_eq!(evaluation.day, days[3]);
}

#[test]
fn the_one_step_gives_what_a_prepared_strategy_gives() {
    let (days, history) = four();
    let strategy = daily(equal(vec![filter(RETURN_2, Take::Top, 1, vec![switching(), asset("CCC")]), inverse_volatility(2, vec![asset("AAA"), asset("DDD")])]));
    let evaluator = Evaluator::new(&strategy, &history, false).unwrap();
    for day in &days[2..] {
        assert_eq!(evaluate(&strategy, &history, *day), evaluator.on(*day));
        // Asking again changes nothing.
        assert_eq!(evaluator.on(*day), evaluator.on(*day));
    }
}

#[test]
fn a_day_the_strategy_has_no_value_on_is_refused() {
    let (days, history) = four();
    let strategy = daily(filter(RETURN_2, Take::Top, 1, vec![asset("AAA"), asset("BBB")]));
    let refused = |day: &str| evaluate(&strategy, &history, day.parse().unwrap()).expect_err("refused");
    // The window is not full on day 1, 2025-03-08 is a Saturday, and the prices end on 2025-03-10.
    for day in ["2025-03-04", "2025-03-08", "2025-03-11", "2024-01-02"] {
        let issues = refused(day);
        assert_eq!(issues.iter().map(|issue| (issue.code, issue.path.as_str())).collect::<Vec<_>>(), [(IssueCode::NoTradingDay, "day")], "{day}");
        assert_eq!(issues[0].message, format!("The strategy cannot be evaluated on {day}. It can be from 2025-03-05 to 2025-03-10, on trading days."));
    }
    assert!(evaluate(&strategy, &history, days[2]).is_ok());

    // A window longer than the history has no day at all.
    let long = daily(filter(Indicator::CumulativeReturn { window: 6 }, Take::Top, 1, vec![asset("AAA"), asset("BBB")]));
    assert!(Evaluator::new(&long, &history, false).unwrap().days().is_none());
    let issues = evaluate(&long, &history, days[5]).expect_err("refused");
    assert_eq!(issues[0].message, "There is no day on which every reading in the strategy has a value.");
}

#[test]
fn a_strategy_with_problems_or_an_asset_with_no_prices_is_refused() {
    let (days, history) = four();
    // Validation comes first, and reports every problem.
    let broken = daily(equal(vec![filter(RETURN_2, Take::Top, 0, vec![asset("AAA")]), equal(vec![])]));
    let codes = |issues: Vec<compounder_engine::rules::Issue>| issues.into_iter().map(|issue| issue.code).collect::<Vec<_>>();
    assert_eq!(codes(evaluate(&broken, &history, days[3]).expect_err("refused")), [IssueCode::InvalidFilter, IssueCode::IncompleteBlock]);

    // Then prices: every asset held or read needs some.
    let unknown = daily(when(against(Indicator::Price, "ZZZ", Comparator::Above, 1.0), vec![asset("AAA")], vec![asset("YYY")]));
    let issues = evaluate(&unknown, &history, days[3]).expect_err("refused");
    assert_eq!(issues.iter().map(|issue| (issue.code, issue.message.as_str())).collect::<Vec<_>>(), [(IssueCode::MissingHistory, "YYY has no price history."), (IssueCode::MissingHistory, "ZZZ has no price history.")]);
}

#[test]
fn a_token_is_evaluated_on_the_series_it_is_given() {
    let days = weekdays("2025-03-03", 3);
    let mut history = history(&days, &[("AAA", 0, &[10.0, 11.0, 12.0])]);
    let token = Asset::Token { id: "0x00000000000000000000000000000000000000aa:56".to_string() };
    let bars: Vec<_> = days.iter().zip([5.0, 4.0, 6.0]).map(|(day, close)| compounder_prices::adjust::Bar::new(*day, close)).collect();
    history.add(token.clone(), &compounder_prices::adjust::adjust(&bars).unwrap());
    // Hold the token while it rose yesterday, else AAA.
    let rose = Condition::Compare(compounder_engine::rules::Comparison {
        left: compounder_engine::rules::Reading { indicator: RETURN_1, asset: token.clone() },
        comparator: Comparator::Above,
        right: compounder_engine::rules::Operand::Fixed(0.0),
    });
    let strategy = daily(when(rose, vec![Node::Asset(token.clone())], vec![asset("AAA")]));
    let evaluator = Evaluator::new(&strategy, &history, false).unwrap();
    // Listed assets come before tokens.
    assert_eq!(evaluator.assets(), [listed("AAA"), token]);
    assert_eq!(evaluator.on(days[1]).unwrap().weights, [1.0, 0.0]);
    assert_eq!(evaluator.on(days[2]).unwrap().weights, [0.0, 1.0]);
}

#[test]
fn the_days_a_run_would_cover_can_be_asked_without_running_it() {
    let (days, history) = four();
    let strategy = daily(filter(RETURN_2, Take::Top, 1, vec![asset("AAA"), asset("BBB")]));
    let evaluator = Evaluator::new(&strategy, &history, false).unwrap();
    // Asked for everything, it gets what the readings allow, and is told which assets held it back.
    let whole = evaluator.window(None, None).unwrap();
    assert_eq!((whole.first, whole.last), (days[2], days[5]));
    assert_eq!(whole.warnings.iter().map(|warning| (warning.asset.clone(), warning.day)).collect::<Vec<_>>(), [(listed("AAA"), days[2]), (listed("BBB"), days[2])]);
    // A start on a Saturday moves to the Monday. An end on a Sunday moves back to the Friday.
    let weekend = evaluator.window(Some("2025-03-08".parse().unwrap()), None).unwrap();
    assert_eq!((weekend.first, weekend.last, weekend.warnings.len()), (days[5], days[5], 0));
    let friday = evaluator.window(Some(days[3]), Some("2025-03-09".parse().unwrap())).unwrap();
    assert_eq!((friday.first, friday.last, friday.warnings.len()), (days[3], days[4], 0));
    // The closes of the assets it may hold, in their order. A closed day has none.
    assert_eq!(evaluator.closes(days[3]), Some(vec![104.0, 48.0]));
    assert_eq!(evaluator.closes("2025-03-08".parse().unwrap()), None);
}
