//! Each starter played over the price store, kept for a while so that a page
//! never waits for a test it has already run.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Instant,
};

use axum::body::Bytes;
use compounder_engine::rules::RuleStrategy;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    backtest,
    prices::{Prices, KEEP},
};

/// What every starter is set beside.
const BENCHMARK: &str = "SPY";

/// The figures of a test that a card and a page lead with.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Figures {
    pub first_day: String,
    pub last_day: String,
    /// The yearly gain over the tested days.
    pub per_year: Option<f64>,
    /// The largest fall from a high to a later low, as a share of the high.
    pub worst_drop: Option<f64>,
    pub sharpe: Option<f64>,
    /// The benchmark's yearly gain, when it has prices on every tested day.
    pub benchmark_per_year: Option<f64>,
    /// The share of each trade that the test took as its cost.
    pub cost: f64,
}

/// One starter's test: its leading figures, and the whole answer as it is sent.
pub struct Test {
    pub figures: Figures,
    pub body: Bytes,
}

/// A test and when it was run. None is a test that could not be run.
type Kept = (Instant, Option<Arc<Test>>);

/// The starters' tests on one price store.
pub struct Tested {
    prices: Arc<Prices>,
    kept: Mutex<HashMap<String, Kept>>,
}

impl Tested {
    pub fn on(prices: Arc<Prices>) -> Tested {
        Tested { prices, kept: Mutex::new(HashMap::new()) }
    }

    /// The test of the starter `id`, run again once the prices it was run on have been let go.
    /// None when it cannot be run, which is said once in the log and then kept as well.
    pub fn of(&self, id: &str, rules: &RuleStrategy) -> Option<Arc<Test>> {
        if let Some((at, test)) = self.kept.lock().expect("no test panics while it is kept").get(id) {
            if at.elapsed() <= KEEP {
                return test.clone();
            }
        }
        // Run outside the lock. Two askers at once may both run it, and either answer will do.
        let test = match run(&self.prices, rules) {
            Ok(test) => Some(Arc::new(test)),
            Err(reason) => {
                eprintln!("the starter {id} could not be tested: {reason}");
                None
            }
        };
        self.kept.lock().expect("no test panics while it is kept").insert(id.to_string(), (Instant::now(), test.clone()));
        test
    }
}

fn run(prices: &Prices, rules: &RuleStrategy) -> Result<Test, String> {
    let ask = backtest::Ask::plain(rules.clone(), BENCHMARK);
    let answer = backtest::run(prices, &ask).map_err(|error| error.message)?;
    let text = |value: &Value| value.as_str().map(str::to_string).ok_or("the answer has no day where one belongs");
    let (first_day, last_day) = (text(&answer["first_day"])?, text(&answer["last_day"])?);
    // The benchmark's figure may stand beside the strategy's only when both cover the same days.
    let benchmark = &answer["benchmark"];
    let same_days = benchmark["first_day"] == answer["first_day"] && benchmark["last_day"] == answer["last_day"];
    let figures = Figures {
        first_day,
        last_day,
        per_year: answer["stats"]["annual_return"].as_f64(),
        worst_drop: answer["stats"]["max_drawdown"].as_f64(),
        sharpe: answer["stats"]["sharpe"].as_f64(),
        benchmark_per_year: benchmark["stats"]["annual_return"].as_f64().filter(|_| same_days),
        cost: answer["slippage"].as_f64().ok_or("the answer does not say what a trade cost")?,
    };
    Ok(Test { figures, body: Bytes::from(answer.to_string()) })
}
