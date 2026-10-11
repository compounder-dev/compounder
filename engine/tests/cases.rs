//! Cases: a strategy, a run and the answer the run must give, on prices
//! kept in files.
//!
//! ```text
//! cases/<name>/strategy.json   the strategy as its canonical tree
//!              or input.json   a block-tree document, when the case is about import
//!              run.json        start, end, capital, costs, conventions
//!              expected.json   what the run must give, and how closely
//! prices/<SYMBOL>.csv          day,price: adjusted closes. A slash in a symbol is written _
//! ```
//!
//! The cases in `tests/cases` are on made-up prices and run every time. Their
//! expected files were worked out apart from this engine, so a mistake here
//! cannot hide in them. `COMPOUNDER_CASES` may name another folder laid out
//! the same way. Its cases are run too, and a table says how each one did.
//!
//! An expected file holds any of `first_day`, `trading_days`, `values` and
//! `weights`, and the `tolerance` the last two are held to: an absolute gap
//! for a weight, a share of the value for a value. A weight that is not
//! listed for a day is zero. `{"refused": true}` says the engine must refuse
//! the case instead. `known_gap` names days on which the case is known not to
//! be met, and why. They are counted and shown, and do not fail the case.

use compounder_engine::backtest::{backtest, Backtest, Run};
use compounder_engine::history::History;
use compounder_engine::rules::{
    import_json, Asset, Comparator, Comparison, Condition, Frequency, Indicator, Issue, Market, Node, Operand, Reading, Rebalance, RuleStrategy, Take, Weighting,
};
use compounder_prices::{
    adjust::{adjust, Bar},
    calendar::Calendar,
    day::Day,
};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

// ---- reading a case ----

fn json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

fn text<'a>(value: &'a Value, what: &str) -> &'a str {
    value.as_str().unwrap_or_else(|| panic!("{what} is {value}, not text"))
}

fn day(value: &Value) -> Day {
    text(value, "a day").parse().unwrap_or_else(|error| panic!("{error}"))
}

fn number(value: &Value, what: &str) -> f64 {
    value.as_f64().unwrap_or_else(|| panic!("{what} is {value}, not a number"))
}

fn list<'a>(value: &'a Value, what: &str) -> &'a Vec<Value> {
    value.as_array().unwrap_or_else(|| panic!("{what} is {value}, not a list"))
}

fn asset(value: &Value) -> Asset {
    if let Some(id) = value.get("token") {
        return Asset::Token { id: text(id, "a token").to_string() };
    }
    let market = match text(&value["market"], "a market") {
        "equities" => Market::Equities,
        "crypto" => Market::Crypto,
        other => panic!("{other:?} is not a market"),
    };
    Asset::Listed { market, symbol: text(&value["symbol"], "a symbol").to_string() }
}

fn indicator(value: &Value) -> Indicator {
    let window = || number(&value["window"], "a window") as u32;
    match text(&value["indicator"], "an indicator") {
        "price" => Indicator::Price,
        "average_price" => Indicator::AveragePrice { window: window() },
        "exponential_average_price" => Indicator::ExponentialAveragePrice { window: window() },
        "price_deviation" => Indicator::PriceDeviation { window: window() },
        "cumulative_return" => Indicator::CumulativeReturn { window: window() },
        "average_return" => Indicator::AverageReturn { window: window() },
        "return_deviation" => Indicator::ReturnDeviation { window: window() },
        "max_drawdown" => Indicator::MaxDrawdown { window: window() },
        "relative_strength" => Indicator::RelativeStrength { window: window() },
        other => panic!("{other:?} is not an indicator"),
    }
}

fn reading(value: &Value) -> Reading {
    Reading { indicator: indicator(value), asset: asset(&value["asset"]) }
}

fn condition(value: &Value) -> Condition {
    for (word, group) in [("any", Condition::Any as fn(Vec<Condition>) -> Condition), ("all", Condition::All)] {
        if let Some(parts) = value.get(word) {
            return group(list(parts, word).iter().map(condition).collect());
        }
    }
    let comparator = match text(&value["compare"], "a comparator") {
        "above" => Comparator::Above,
        "at_least" => Comparator::AtLeast,
        "equal" => Comparator::Equal,
        "at_most" => Comparator::AtMost,
        "below" => Comparator::Below,
        other => panic!("{other:?} is not a comparator"),
    };
    let right = match value["right"].get("fixed") {
        Some(fixed) => Operand::Fixed(number(fixed, "a fixed value")),
        None => Operand::Reading(reading(&value["right"])),
    };
    Condition::Compare(Comparison { left: reading(&value["left"]), comparator, right })
}

fn nodes(value: &Value) -> Vec<Node> {
    list(value, "a list of blocks").iter().map(node).collect()
}

fn node(value: &Value) -> Node {
    match text(&value["block"], "a block") {
        "asset" => Node::Asset(asset(&value["asset"])),
        "weight" => {
            let weighting = match text(&value["weighting"]["type"], "a weighting") {
                "equal" => Weighting::Equal,
                "specified" => Weighting::Specified { shares: list(&value["weighting"]["shares"], "shares").iter().map(|share| number(share, "a share")).collect() },
                "inverse_volatility" => Weighting::InverseVolatility { window: number(&value["weighting"]["window"], "a window") as u32 },
                other => panic!("{other:?} is not a weighting"),
            };
            Node::Weight { name: None, weighting, children: nodes(&value["children"]) }
        }
        "if" => Node::If { condition: condition(&value["condition"]), then: nodes(&value["then"]), otherwise: nodes(&value["else"]) },
        "filter" => {
            let take = match text(&value["take"], "a take") {
                "top" => Take::Top,
                "bottom" => Take::Bottom,
                other => panic!("{other:?} is not top or bottom"),
            };
            Node::Filter { sort_by: indicator(&value["sort_by"]), take, count: number(&value["count"], "a count") as u32, children: nodes(&value["children"]) }
        }
        other => panic!("{other:?} is not a block"),
    }
}

/// A strategy from its canonical tree.
fn strategy(name: &str, value: &Value) -> RuleStrategy {
    let rebalance = match text(&value["rebalance"]["type"], "a rebalance rule") {
        "threshold" => Rebalance::Threshold { corridor: number(&value["rebalance"]["corridor"], "a corridor") },
        "calendar" => Rebalance::Calendar(match text(&value["rebalance"]["frequency"], "a frequency") {
            "daily" => Frequency::Daily,
            "weekly" => Frequency::Weekly,
            "monthly" => Frequency::Monthly,
            "quarterly" => Frequency::Quarterly,
            "yearly" => Frequency::Yearly,
            other => panic!("{other:?} is not a frequency"),
        }),
        other => panic!("{other:?} is not a rebalance rule"),
    };
    RuleStrategy { name: name.to_string(), rebalance, root: node(&value["root"]) }
}

/// Every price file of a folder, on the calendar of the days any of them has.
fn history(folder: &Path) -> History {
    let mut series = Vec::new();
    for entry in fs::read_dir(folder.join("prices")).unwrap_or_else(|error| panic!("{}: {error}", folder.display())) {
        let path = entry.expect("a folder entry").path();
        let Some(symbol) = path.file_name().and_then(|name| name.to_str()).and_then(|name| name.strip_suffix(".csv")) else { continue };
        let rows = fs::read_to_string(&path).expect("a price file");
        let bars: Vec<Bar> = rows
            .lines()
            .skip(1)
            .filter(|line| !line.is_empty())
            .map(|line| {
                let (day, price) = line.split_once(',').unwrap_or_else(|| panic!("{}: {line:?} is not day,price", path.display()));
                Bar::new(day.parse().unwrap_or_else(|error| panic!("{error}")), price.parse().unwrap_or_else(|_| panic!("{}: {price:?} is not a price", path.display())))
            })
            .collect();
        series.push((symbol.replace('_', "/"), adjust(&bars).unwrap_or_else(|error| panic!("{}: {error}", path.display()))));
    }
    let days = series.iter().flat_map(|(_, series)| series.days().iter().copied());
    let mut history = History::new(Calendar::new(days).expect("some prices"));
    for (symbol, series) in &series {
        history.add(Asset::Listed { market: Market::Equities, symbol: symbol.clone() }, series);
    }
    history
}

// ---- running a case ----

/// How one case did.
struct Report {
    name: String,
    /// `None` where the case does not say what to expect.
    first_day: Option<bool>,
    trading_days: Option<bool>,
    weight_gap: Option<f64>,
    value_gap: Option<f64>,
    /// Days outside a tolerance that the case names as known.
    known: usize,
    /// The codes it was refused with, when it was.
    refused: Option<String>,
    /// What is wrong, when something is.
    failure: Option<String>,
}

impl Report {
    fn new(name: &str) -> Report {
        Report { name: name.to_string(), first_day: None, trading_days: None, weight_gap: None, value_gap: None, known: 0, refused: None, failure: None }
    }

    fn fail(&mut self, why: String) {
        self.failure.get_or_insert(why);
    }

    fn line(&self) -> String {
        let word = |equal: Option<bool>| equal.map_or("-", |equal| if equal { "equal" } else { "DIFFER" });
        let gap = |gap: Option<f64>| gap.map_or("-".to_string(), |gap| format!("{gap:.1e}"));
        let verdict = match (&self.failure, &self.refused, self.known) {
            (Some(why), _, _) => format!("FAIL: {why}"),
            (None, Some(codes), 0) => format!("pass: refused, {codes}"),
            (None, Some(codes), _) => format!("known: refused, {codes}, and the case is a run"),
            (None, None, 0) => "pass".to_string(),
            (None, None, days) => format!("pass, with {days} known days"),
        };
        format!("{:<44} {:<7} {:<7} {:>8} {:>8}  {verdict}", self.name, word(self.first_day), word(self.trading_days), gap(self.weight_gap), gap(self.value_gap))
    }
}

fn run(folder: &Path, name: &str, history: &History) -> Report {
    let mut report = Report::new(name);
    let settings = json(&folder.join("run.json"));
    let expected = json(&folder.join("expected.json"));
    let refused = expected.get("refused").and_then(Value::as_bool).unwrap_or(false);
    let optional_day = |value: &Value| if value.is_null() { None } else { Some(day(value)) };
    let run = Run {
        start: optional_day(&settings["start"]),
        end: optional_day(&settings["end"]),
        capital: number(&settings["capital"], "the capital"),
        slippage: number(&settings["costs"]["slippage"], "the slippage"),
        min_order: number(&settings["costs"]["min_order"], "the minimum order"),
        run_in: settings["conventions"]["run_in"].as_bool().unwrap_or(false),
    };
    let strategy = if folder.join("strategy.json").exists() {
        Ok(strategy(name, &json(&folder.join("strategy.json"))))
    } else {
        import_json(&fs::read_to_string(folder.join("input.json")).unwrap_or_else(|error| panic!("{name}: {error}")))
    };
    let outcome = strategy.and_then(|strategy| backtest(&strategy, history, &run));
    let said = |issues: &[Issue]| issues.iter().map(|issue| format!("{:?}: {}", issue.code, issue.message)).collect::<Vec<_>>().join(" | ");
    match (outcome, refused) {
        // A refusal must be for the reason the case gives, when it gives one.
        (Err(issues), true) => {
            let wanted = expected.get("issue").map(|code| text(code, "an issue code"));
            report.refused = Some(codes(&issues));
            if wanted.is_some_and(|wanted| !codes(&issues).split(' ').any(|code| code == wanted)) {
                report.fail(format!("refused, but not as {}: {}", wanted.unwrap_or_default(), said(&issues)));
            }
        }
        (Ok(_), true) => report.fail("it ran, and it must be refused".to_string()),
        (Err(issues), false) if expected["known_gap"]["refused"].as_bool() == Some(true) => {
            report.refused = Some(codes(&issues));
            report.known = 1;
        }
        (Err(issues), false) => report.fail(format!("refused: {}", said(&issues))),
        (Ok(backtest), false) => compare(&backtest, &expected, &mut report),
    }
    report
}

/// The distinct codes of some issues, as they are written in JSON.
fn codes(issues: &[Issue]) -> String {
    let codes: BTreeSet<String> = issues.iter().filter_map(|issue| serde_json::to_value(issue.code).ok()?.as_str().map(str::to_string)).collect();
    codes.into_iter().collect::<Vec<_>>().join(" ")
}

fn compare(backtest: &Backtest, expected: &Value, report: &mut Report) {
    let known: BTreeSet<String> = expected["known_gap"]["days"].as_array().map(|days| days.iter().map(|day| text(day, "a day").to_string()).collect()).unwrap_or_default();
    let mut known_met = BTreeSet::new();
    if let Some(first) = expected.get("first_day") {
        report.first_day = Some(backtest.first_day == day(first));
        if report.first_day == Some(false) {
            report.fail(format!("it starts on {}, not {}", backtest.first_day, day(first)));
        }
    }
    if let Some(days) = expected.get("trading_days") {
        let wanted: Vec<Day> = list(days, "trading days").iter().map(day).collect();
        let traded: Vec<Day> = backtest.days.iter().filter(|day| day.traded).map(|day| day.day).collect();
        report.trading_days = Some(traded == wanted);
        if traded != wanted {
            let odd = traded.iter().find(|day| !wanted.contains(day)).or(wanted.iter().find(|day| !traded.contains(day)));
            report.fail(format!("it trades on {} days, not {}: {} is in one list only", traded.len(), wanted.len(), odd.map(Day::to_string).unwrap_or_default()));
        }
    }
    if let Some(values) = expected.get("values").and_then(Value::as_object) {
        let tolerance = number(&expected["tolerance"]["values"], "the value tolerance");
        if values.len() != backtest.days.len() {
            report.fail(format!("it runs for {} days, not {}", backtest.days.len(), values.len()));
        }
        let mut worst = 0.0_f64;
        for result in &backtest.days {
            let Some(wanted) = values.get(&result.day.to_string()) else { continue };
            let wanted = number(wanted, "a value");
            let gap = (result.value - wanted).abs() / wanted.abs();
            worst = worst.max(gap);
            if gap > tolerance && report.failure.is_none() {
                report.fail(format!("the value on {} is {}, not {wanted}", result.day, result.value));
            }
        }
        report.value_gap = Some(worst);
    }
    if let Some(weights) = expected.get("weights").and_then(Value::as_object) {
        let tolerance = number(&expected["tolerance"]["weights"], "the weight tolerance");
        let ours: BTreeMap<String, usize> = backtest.assets.iter().enumerate().map(|(column, asset)| (symbol(asset), column)).collect();
        let symbols: BTreeSet<&str> = ours.keys().map(String::as_str).chain(weights.keys().map(String::as_str)).collect();
        let mut worst = 0.0_f64;
        for result in &backtest.days {
            let day = result.day.to_string();
            for symbol in &symbols {
                let held = ours.get(*symbol).map_or(0.0, |column| result.weights[*column]);
                let wanted = weights.get(*symbol).and_then(|days| days.get(&day)).map_or(0.0, |weight| number(weight, "a weight"));
                let gap = (held - wanted).abs();
                if gap > tolerance && known.contains(&day) {
                    known_met.insert(day.clone());
                    continue;
                }
                worst = worst.max(gap);
                if gap > tolerance && report.failure.is_none() {
                    report.fail(format!("the weight of {symbol} on {day} is {held:.6}, not {wanted}"));
                }
            }
        }
        report.weight_gap = Some(worst);
    }
    report.known = known_met.len();
}

fn symbol(asset: &Asset) -> String {
    match asset {
        Asset::Listed { symbol, .. } => symbol.clone(),
        Asset::Token { id } => id.clone(),
    }
}

/// Run every case of a folder and print how each did.
fn run_all(folder: &Path) -> Vec<Report> {
    let history = history(folder);
    let mut names: Vec<String> = fs::read_dir(folder.join("cases"))
        .unwrap_or_else(|error| panic!("{}: {error}", folder.display()))
        .map(|entry| entry.expect("a folder entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let reports: Vec<Report> = names.iter().map(|name| run(&folder.join("cases").join(name), name, &history)).collect();
    println!("{:<44} {:<7} {:<7} {:>8} {:>8}", "case", "first", "trading", "weights", "values");
    for report in &reports {
        println!("{}", report.line());
    }
    let largest = |gap: fn(&Report) -> Option<f64>| reports.iter().filter(|report| report.failure.is_none()).filter_map(gap).fold(0.0, f64::max);
    println!(
        "{} cases, {} pass, {} fail. Largest gap among those that pass: weights {:.1e}, values {:.1e}.",
        reports.len(),
        reports.iter().filter(|report| report.failure.is_none()).count(),
        reports.iter().filter(|report| report.failure.is_some()).count(),
        largest(|report| report.weight_gap),
        largest(|report| report.value_gap),
    );
    reports
}

fn failures(reports: &[Report]) -> Vec<String> {
    reports.iter().filter(|report| report.failure.is_some()).map(Report::line).collect()
}

#[test]
fn every_case_in_the_repo_is_met() {
    let reports = run_all(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases"));
    assert!(reports.len() >= 20, "cases went missing");
    assert_eq!(failures(&reports), Vec::<String>::new());
}

/// `COMPOUNDER_CASES=<folder> cargo test --release -p compounder-engine --test cases -- --nocapture`
#[test]
fn every_case_in_the_folder_the_environment_names_is_met() {
    let Some(folder) = std::env::var_os("COMPOUNDER_CASES") else { return };
    let reports = run_all(Path::new(&folder));
    assert_eq!(failures(&reports), Vec::<String>::new());
}
