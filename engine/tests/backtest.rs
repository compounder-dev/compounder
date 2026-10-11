//! The backtest loop on made-up prices small enough to work out by hand:
//! money, costs, the minimum order, drift, the corridor, and where a run
//! starts and ends. The cases in `tests/cases` cover the schedules and the
//! rest over long runs.

mod support;

use compounder_engine::backtest::{backtest, Backtest, Run};
use compounder_engine::evaluate::WarningCode;
use compounder_engine::history::History;
use compounder_engine::rules::{Comparator, Frequency, Indicator, IssueCode, RuleStrategy, Take};
use compounder_prices::day::Day;
use support::*;

/// A run of the whole history at no cost.
fn free(capital: f64) -> Run {
    Run { start: None, end: None, capital, slippage: 0.0, min_order: 0.0, run_in: false }
}

fn ran(strategy: &RuleStrategy, history: &History, run: &Run) -> Backtest {
    backtest(strategy, history, run).unwrap_or_else(|issues| panic!("{issues:#?}"))
}

fn traded_on(backtest: &Backtest) -> Vec<String> {
    backtest.days.iter().filter(|day| day.traded).map(|day| day.day.to_string()).collect()
}

/// AAA goes 100, 110, 110, 99 and BBB stays at 50, then 55.
fn two(days: &[Day]) -> History {
    history(days, &[("AAA", 0, &[100.0, 110.0, 110.0, 99.0]), ("BBB", 0, &[50.0, 50.0, 50.0, 55.0])])
}

#[test]
fn the_first_day_invests_everything_and_earns_nothing() {
    let days = weekdays("2025-03-03", 4);
    let result = ran(&daily(equal(vec![asset("AAA"), asset("BBB")])), &two(&days), &free(10_000.0));
    assert_eq!((result.first_day, result.last_day), (days[0], days[3]));
    assert_eq!(result.assets, [listed("AAA"), listed("BBB")]);
    let first = &result.days[0];
    assert_eq!((first.day, first.value, first.traded), (days[0], 10_000.0, true));
    assert_eq!((first.bought, first.sold, first.cost), (10_000.0, 0.0, 0.0));
    assert_eq!(first.weights, [0.5, 0.5]);
    // 50 shares of AAA and 100 of BBB. On day 1 AAA is up a tenth: 5,500 and 5,000, and 250 changes hands.
    let second = &result.days[1];
    assert!(close(second.value, 10_500.0, 1e-12) && close(second.sold, 250.0, 1e-12) && close(second.bought, 250.0, 1e-12));
    assert!(all_close(&second.weights, &[0.5, 0.5], 1e-12));
    // Nothing moves on day 2, so nothing changes hands, though it is a trading day.
    let third = &result.days[2];
    assert!(third.traded && close(third.value, 10_500.0, 1e-12) && third.bought.abs() < 1e-9 && third.sold.abs() < 1e-9);
    // Day 3: 5,250 of AAA becomes 4,725 and 5,250 of BBB becomes 5,775.
    assert!(close(result.days[3].value, 10_500.0, 1e-12) && close(result.days[3].sold, 525.0, 1e-9));
    assert!(all_close(&result.holdings.shares, &[5_250.0 / 99.0, 5_250.0 / 55.0], 1e-12));
    assert!(result.holdings.cash.abs() < 1e-6);
    assert!(result.warnings.is_empty());
}

#[test]
fn slippage_is_charged_on_every_dollar_traded_and_buys_are_scaled_to_the_money() {
    let days = weekdays("2025-03-03", 4);
    let run = Run { slippage: 0.01, min_order: 1.0, ..free(10_000.0) };
    let result = ran(&daily(equal(vec![asset("AAA"), asset("BBB")])), &two(&days), &run);
    // Day 0. The targets come to 10,000, which costs 10,100 to buy. Every buy is cut by the same
    // factor, so 10,000 / 1.01 goes into the two assets and the rest is the cost. No cash is left.
    let invested = 10_000.0 / 1.01;
    let first = &result.days[0];
    assert!(close(first.value, invested, 1e-12) && close(first.bought, invested, 1e-12) && close(first.cost, invested * 0.01, 1e-12));
    assert!(all_close(&first.weights, &[0.5, 0.5], 1e-12) && first.sold == 0.0);
    // Day 1. AAA is up a tenth: 0.55 and 0.5 of what was invested, 1.05 in all, so each target is 0.525.
    // AAA sells 0.025 and brings in 99% of it. BBB wants 0.025, which would cost 101%: its buy is cut to 99/101.
    let sold = invested * 0.025;
    let bought = sold * 0.99 / 1.01;
    let cost = (sold + bought) * 0.01;
    let second = &result.days[1];
    assert!(close(second.sold, sold, 1e-11) && close(second.bought, bought, 1e-11) && close(second.cost, cost, 1e-11));
    assert!(close(second.value, invested * 1.05 - cost, 1e-12));
    assert!(all_close(&second.weights, &[invested * 0.525 / second.value, (invested * 0.5 + bought) / second.value], 1e-12));
    // Every day the value falls by exactly what trading cost, and never by more.
    let mut shares = [0.0, 0.0];
    let mut cash = 10_000.0;
    for (day, prices) in result.days.iter().zip([[100.0, 50.0], [110.0, 50.0], [110.0, 50.0], [99.0, 55.0]]) {
        let value_before = cash + shares[0] * prices[0] + shares[1] * prices[1];
        assert!(close(day.value, value_before - day.cost, 1e-12), "{}", day.day);
        assert!(close(day.cost, (day.bought + day.sold) * 0.01, 1e-12), "{}", day.day);
        shares = [day.weights[0] * day.value / prices[0], day.weights[1] * day.value / prices[1]];
        cash = day.value - shares[0] * prices[0] - shares[1] * prices[1];
        assert!(cash.abs() < 1e-6, "{}: {cash} left in cash", day.day);
    }
}

#[test]
fn an_order_under_the_minimum_is_not_placed() {
    let days = weekdays("2025-03-03", 3);
    let history = history(&days, &[("AAA", 0, &[10.0, 10.1, 10.5]), ("BBB", 0, &[10.0, 10.0, 10.0])]);
    let strategy = daily(equal(vec![asset("AAA"), asset("BBB")]));
    let result = ran(&strategy, &history, &Run { min_order: 1.0, ..free(100.0) });
    // Day 1: 50.50 and 50.00. Evening them out is 25 cents each way, and neither order is placed.
    let second = &result.days[1];
    assert!(second.traded && second.bought == 0.0 && second.sold == 0.0);
    assert!(all_close(&second.weights, &[50.5 / 100.5, 50.0 / 100.5], 1e-12));
    // Day 2: 52.50 and 50.00. Now it is 1.25 each way.
    let third = &result.days[2];
    assert!(close(third.sold, 1.25, 1e-12) && close(third.bought, 1.25, 1e-12) && all_close(&third.weights, &[0.5, 0.5], 1e-12));
    // With no minimum the 25 cents are traded too.
    let exact = ran(&strategy, &history, &free(100.0));
    assert!(close(exact.days[1].sold, 0.25, 1e-10) && all_close(&exact.days[1].weights, &[0.5, 0.5], 1e-12));

    // A first purchase too small to place stays in cash, and the weights say so.
    let tiny = specified(&[0.995, 0.005], vec![asset("AAA"), asset("BBB")]);
    let result = ran(&daily(tiny), &history, &Run { min_order: 1.0, ..free(100.0) });
    assert!(all_close(&result.days[0].weights, &[0.995, 0.0], 1e-12) && close(result.days[0].bought, 99.5, 1e-12));
    assert!(close(result.days[0].value, 100.0, 1e-12));
    // Day 2: AAA is worth 104.475 of 104.975, and BBB is still owed half a percent: 52 cents, not placed.
    assert!(close(result.holdings.cash, 0.5, 1e-9) && result.holdings.shares[1] == 0.0);
}

#[test]
fn between_trading_days_holdings_drift() {
    // A month end in the middle: two days of March and two of April.
    let days = [day("2025-03-28"), day("2025-03-31"), day("2025-04-01"), day("2025-04-02")];
    let result = ran(&scheduled(Frequency::Monthly, equal(vec![asset("AAA"), asset("BBB")])), &two(&days), &free(10_000.0));
    assert_eq!(traded_on(&result), ["2025-03-28", "2025-04-01"]);
    // 50 shares of AAA at 110 and 100 of BBB at 50 on the last day of March.
    assert!(all_close(&result.days[1].weights, &[5_500.0 / 10_500.0, 5_000.0 / 10_500.0], 1e-12) && !result.days[1].traded);
    assert!(all_close(&result.days[2].weights, &[0.5, 0.5], 1e-12) && close(result.days[2].sold, 250.0, 1e-12));
    // The last day does not trade: 5,250 of AAA is now 4,725 and 5,250 of BBB is 5,775.
    let last = &result.days[3];
    assert!(!last.traded && close(last.value, 10_500.0, 1e-12) && all_close(&last.weights, &[4_725.0 / 10_500.0, 5_775.0 / 10_500.0], 1e-12));
    // The picks are what the strategy wants on the last day, which is not what it holds.
    assert_eq!(result.picks, [0.5, 0.5]);
}

#[test]
fn a_threshold_strategy_trades_on_its_first_day_and_then_only_on_a_breach() {
    let days = weekdays("2025-03-03", 5);
    let history = history(&days, &[("AAA", 0, &[100.0, 200.0, 300.0, 300.0, 310.0]), ("BBB", 0, &[100.0, 100.0, 100.0, 100.0, 100.0])]);
    let half_each = || equal(vec![asset("AAA"), asset("BBB")]);
    // One share of each. AAA at 200 is two thirds of the value: 16.7 points out, inside a corridor of 25.
    // AAA at 300 is three quarters: exactly 25 points out, and a gap of exactly the corridor trades.
    let result = ran(&threshold(0.25, half_each()), &history, &free(200.0));
    assert_eq!(traded_on(&result), ["2025-03-03", "2025-03-05"]);
    assert_eq!(result.days[1].weights, [200.0 / 300.0, 100.0 / 300.0]);
    assert!(all_close(&result.days[2].weights, &[0.5, 0.5], 1e-15));
    assert!(close(result.days[2].sold, 100.0, 1e-12));
    // A corridor wider than any weight still lets the first day trade.
    let wide = ran(&threshold(0.9, half_each()), &history, &free(200.0));
    assert_eq!(traded_on(&wide), ["2025-03-03"]);
    assert_eq!(wide.days[0].weights, [0.5, 0.5]);
    // A switch of branch is a gap of the whole weight, and trades at once.
    let switch = when(against(Indicator::Price, "AAA", Comparator::Above, 305.0), vec![asset("BBB")], vec![asset("AAA")]);
    let switched = ran(&threshold(0.9, switch), &history, &free(200.0));
    assert_eq!(traded_on(&switched), ["2025-03-03", "2025-03-07"]);
    assert_eq!(switched.days[4].weights, [0.0, 1.0]);
}

#[test]
fn a_run_starts_and_ends_where_prices_allow_and_says_which_asset_held_it() {
    let days = weekdays("2025-03-03", 30);
    let old: Vec<f64> = (0..30).map(|index| 50.0 + f64::from(index)).collect();
    let young: Vec<f64> = (0..20).map(|index| 8.0 + f64::from(index % 3)).collect();
    let gone: Vec<f64> = (0..25).map(|index| 30.0 - f64::from(index % 4)).collect();
    let history = history(&days, &[("OLD", 0, &old), ("YOUNG", 10, &young), ("GONE", 0, &gone)]);
    // The five day return of YOUNG ranks it against OLD, and GONE is always held beside the winner.
    let strategy = daily(equal(vec![filter(Indicator::CumulativeReturn { window: 5 }, Take::Top, 1, vec![asset("YOUNG"), asset("OLD")]), asset("GONE")]));
    let run = Run { start: Some(days[3]), end: Some(days[28]), ..free(1_000.0) };
    let result = ran(&strategy, &history, &run);
    // YOUNG's first close is day 10 and its five day return is first there on day 15. GONE's last close is day 24.
    assert_eq!((result.first_day, result.last_day), (days[15], days[24]));
    assert_eq!(result.days.len(), 10);
    let warnings: Vec<_> = result.warnings.iter().map(|warning| (warning.code, warning.asset.clone(), warning.day, warning.message.as_str())).collect();
    let expected = [
        (WarningCode::LateStart, listed("OLD"), days[5], "OLD has prices from 2025-03-03, and the readings that rest on it first have a value on 2025-03-10, so the run cannot start before then."),
        (WarningCode::LateStart, listed("YOUNG"), days[15], "YOUNG has prices from 2025-03-17, and the readings that rest on it first have a value on 2025-03-24, so the run cannot start before then."),
        (WarningCode::EarlyEnd, listed("GONE"), days[24], "GONE has no price after 2025-04-04, so the run cannot go on past then."),
    ];
    assert_eq!(warnings, expected);

    // Asked for days inside what the prices allow, a run says nothing.
    let inside = ran(&strategy, &history, &Run { start: Some(days[16]), end: Some(days[20]), ..free(1_000.0) });
    assert_eq!((inside.first_day, inside.last_day, inside.warnings.len()), (days[16], days[20], 0));
    // A start on a weekend moves to the Monday, and an end on one moves back to the Friday.
    let weekend = ran(&strategy, &history, &Run { start: Some(day("2025-03-29")), end: Some(day("2025-04-05")), ..free(1_000.0) });
    assert_eq!((weekend.first_day.to_string(), weekend.last_day.to_string(), weekend.warnings.len()), ("2025-03-31".to_string(), "2025-04-04".to_string(), 0));
    // An asset with no bar yet, held with no reading on it, holds the start to its first bar.
    let plain = ran(&daily(equal(vec![asset("YOUNG"), asset("OLD")])), &history, &free(1_000.0));
    assert_eq!(plain.first_day, days[10]);
    assert_eq!(plain.warnings.iter().map(|warning| warning.message.as_str()).collect::<Vec<_>>(), ["YOUNG has no price before 2025-03-17, so the run cannot start before then."]);

    // With no days asked for, the run takes what the prices allow and names only the assets that set its two ends:
    // YOUNG and GONE here. OLD's own five day window is filled long before and moved nothing.
    let whole = ran(&strategy, &history, &free(1_000.0));
    assert_eq!((whole.first_day, whole.last_day), (days[15], days[24]));
    assert_eq!(whole.warnings.iter().map(|warning| (warning.code, warning.asset.clone(), warning.day)).collect::<Vec<_>>(), [(WarningCode::LateStart, listed("YOUNG"), days[15]), (WarningCode::EarlyEnd, listed("GONE"), days[24])]);
    // Assets as old as the calendar, with no reading on them, move nothing and say nothing.
    assert!(ran(&daily(equal(vec![asset("OLD")])), &history, &free(1_000.0)).warnings.is_empty());
}

#[test]
fn a_run_that_cannot_be_made_is_refused_with_the_reason() {
    // A Thursday and a Friday, then a Monday and a Tuesday.
    let days = weekdays("2025-03-06", 4);
    let history = two(&days);
    let strategy = daily(equal(vec![asset("AAA"), asset("BBB")]));
    let refused = |run: Run| backtest(&strategy, &history, &run).expect_err("refused").into_iter().map(|issue| (issue.code, issue.path)).collect::<Vec<_>>();
    let at = |code: IssueCode, path: &str| vec![(code, path.to_string())];
    for capital in [0.0, -100.0, f64::NAN, f64::INFINITY] {
        assert_eq!(refused(free(capital)), at(IssueCode::InvalidRun, "run.capital"), "{capital}");
    }
    for slippage in [-0.01, 1.0, 1.5, f64::NAN] {
        assert_eq!(refused(Run { slippage, ..free(100.0) }), at(IssueCode::InvalidRun, "run.slippage"), "{slippage}");
    }
    for min_order in [-1.0, f64::NAN] {
        assert_eq!(refused(Run { min_order, ..free(100.0) }), at(IssueCode::InvalidRun, "run.min_order"), "{min_order}");
    }
    // Every bad setting is reported, not only the first.
    assert_eq!(refused(Run { slippage: 2.0, min_order: -1.0, ..free(0.0) }).len(), 3);
    assert_eq!(refused(Run { start: Some(days[3]), end: Some(days[1]), ..free(100.0) }), at(IssueCode::NoTradingDay, "run.end"));
    assert_eq!(refused(Run { start: Some(day("2025-03-08")), end: Some(day("2025-03-09")), ..free(100.0) }), at(IssueCode::NoTradingDay, "run"));
    assert_eq!(refused(Run { start: Some(day("2026-01-05")), ..free(100.0) }), at(IssueCode::NoTradingDay, "run.start"));
    assert_eq!(refused(Run { end: Some(day("2024-01-05")), ..free(100.0) }), at(IssueCode::NoTradingDay, "run.end"));
    // A strategy with problems is refused before anything is run, and its problems come back beside the run's.
    let empty = daily(equal(vec![]));
    assert_eq!(backtest(&empty, &history, &free(100.0)).expect_err("refused")[0].code, IssueCode::IncompleteBlock);
    let both = backtest(&empty, &history, &free(0.0)).expect_err("refused");
    assert_eq!(both.iter().map(|issue| issue.code).collect::<Vec<_>>(), [IssueCode::InvalidRun, IssueCode::IncompleteBlock]);
}

#[test]
fn the_same_run_gives_the_same_answer_and_writes_as_json() {
    let days = weekdays("2025-03-03", 4);
    let history = two(&days);
    let strategy = scheduled(Frequency::Weekly, equal(vec![asset("AAA"), asset("BBB")]));
    let run = Run { slippage: 0.0015, min_order: 1.0, ..free(10_000.0) };
    let result = ran(&strategy, &history, &run);
    assert_eq!(result, ran(&strategy, &history, &run));
    let written = serde_json::to_value(&result).expect("it serialises");
    assert_eq!(serde_json::to_string(&written).ok(), serde_json::to_string(&serde_json::to_value(ran(&strategy, &history, &run)).unwrap()).ok());
    assert_eq!(written["first_day"], "2025-03-03");
    assert_eq!(written["last_day"], "2025-03-06");
    assert_eq!(written["assets"], serde_json::json!([{ "market": "equities", "symbol": "AAA" }, { "market": "equities", "symbol": "BBB" }]));
    let first = &written["days"][0];
    assert_eq!(first["day"], "2025-03-03");
    assert_eq!(first["traded"], true);
    for field in ["value", "bought", "sold", "cost"] {
        assert!(first[field].is_number(), "{field}");
    }
    assert_eq!(first["weights"].as_array().map(Vec::len), Some(2));
    assert_eq!(written["days"].as_array().map(Vec::len), Some(4));
    assert_eq!(written["days"][1]["traded"], false);
    assert_eq!(written["holdings"]["shares"].as_array().map(Vec::len), Some(2));
    assert!(written["holdings"]["cash"].is_number());
    assert_eq!(written["picks"], serde_json::json!([0.5, 0.5]));
    assert_eq!(written["warnings"], serde_json::json!([]));
}
