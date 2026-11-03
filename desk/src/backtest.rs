//! A fund over past bars, through the same strategy, account and cost model
//! the live run uses. A decision at a bar's close fills at the next bar's open.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    account::Account,
    fill::{at_open, orders, Costs},
    fund::FundSpec,
    market::{Bar, View, HOUR_MS},
    stats::{figures, Figures, Point},
};

/// Past bars and hourly funding rates, per coin, oldest first.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct History {
    pub bars: BTreeMap<String, Vec<Bar>>,
    pub funding: BTreeMap<String, Vec<(i64, f64)>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Tested {
    pub figures: Figures,
    pub curve: Vec<Point>,
    pub fills: usize,
    pub fees: f64,
    pub funding: f64,
    /// Notional traded over average equity, per year.
    pub turnover: f64,
}

pub fn run(fund: &FundSpec, history: &History, costs: &Costs) -> Tested {
    let interval = fund.interval.ms();
    let coins = fund.coins();
    let mut times: Vec<i64> = coins.iter().filter_map(|c| history.bars.get(c)).flatten().map(|b| b.t).collect();
    times.sort_unstable();
    times.dedup();
    if fund.strategy.needs_all() {
        let start = coins.iter().filter_map(|c| history.bars.get(c)?.first().map(|b| b.t)).max().unwrap_or(0);
        times.retain(|&t| t >= start);
    }

    let mut strategy = fund.strategy.build(fund.interval);
    let mut account = Account::new(fund.capital);
    let mut marks: BTreeMap<String, f64> = BTreeMap::new();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    let mut pending: Vec<(String, f64)> = Vec::new();
    let mut funding_at: BTreeMap<String, usize> = BTreeMap::new();
    let mut curve = Vec::new();
    let mut fills = 0;
    let warmup = fund.strategy.lookback();

    for (step, &t) in times.iter().enumerate() {
        // The orders decided at the last close fill at this bar's open.
        for (coin, qty) in pending.drain(..) {
            if let Some(bar) = bar_at(history, &coin, t) {
                account.apply_fill(&at_open(&coin, qty, bar.o, costs));
                fills += 1;
            }
        }
        // Funding falls due each hour of the bar.
        let close = t + interval;
        for coin in account.qty.keys().cloned().collect::<Vec<_>>() {
            let Some(rates) = history.funding.get(&coin) else {
                continue;
            };
            let at = funding_at.entry(coin.clone()).or_insert(0);
            while *at < rates.len() && rates[*at].0 < close {
                let (when, rate) = rates[*at];
                if when >= t {
                    let px = marks.get(&coin).copied().unwrap_or(0.0);
                    account.apply_funding(&coin, rate, px);
                }
                *at += 1;
            }
        }
        for coin in &coins {
            if let Some(index) = index_at(history, coin, t) {
                seen.insert(coin.clone(), index + 1);
                marks.insert(coin.clone(), history.bars[coin][index].c);
            }
        }
        if step + 1 < warmup {
            continue;
        }
        let equity = account.equity(&marks);
        curve.push(Point { t: close, equity });
        if equity <= 0.0 {
            break;
        }
        let lookback = fund.strategy.lookback();
        let bars = seen.iter().map(|(coin, &n)| (coin.clone(), &history.bars[coin][n.saturating_sub(lookback)..n])).collect();
        let view = View { now: close, bars, funding: BTreeMap::new(), weights: account.weights(&marks) };
        let decision = strategy.decide(&view);
        pending = orders(&account, &decision.targets, &marks, fund.min_trade);
    }

    let years = curve.last().zip(curve.first()).map(|(l, f)| (l.t - f.t) as f64 / (365.0 * 24.0 * HOUR_MS as f64));
    let mean_equity = curve.iter().map(|p| p.equity).sum::<f64>() / curve.len().max(1) as f64;
    Tested {
        figures: figures(&curve),
        fills,
        fees: account.fees,
        funding: account.funding,
        turnover: match years {
            Some(y) if y > 0.0 && mean_equity > 0.0 => account.traded / mean_equity / y,
            _ => 0.0,
        },
        curve,
    }
}

fn index_at(history: &History, coin: &str, t: i64) -> Option<usize> {
    history.bars.get(coin)?.binary_search_by_key(&t, |b| b.t).ok()
}

fn bar_at<'a>(history: &'a History, coin: &str, t: i64) -> Option<&'a Bar> {
    index_at(history, coin, t).map(|i| &history.bars[coin][i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fund::Speed, market::Interval, strategy::Spec};

    fn fund(spec: Spec) -> FundSpec {
        FundSpec {
            id: "t".into(),
            name: "t".into(),
            speed: Speed::Compounding,
            thesis: String::new(),
            rules: String::new(),
            interval: Interval::H1,
            capital: 1000.0,
            min_trade: 0.0,
            strategy: spec,
        }
    }

    #[test]
    fn a_full_long_basket_tracks_the_market_less_its_costs() {
        let bars: Vec<Bar> = (0..10)
            .map(|i| {
                let c = 100.0 + i as f64;
                Bar { t: i * HOUR_MS, o: c - 1.0, h: c, l: c - 1.0, c, v: 1.0 }
            })
            .collect();
        let history = History { bars: BTreeMap::from([("X".into(), bars)]), funding: BTreeMap::new() };
        let spec = Spec::Basket { weights: BTreeMap::from([("X".into(), 1.0)]) };
        let free = run(&fund(spec.clone()), &history, &Costs { taker: 0.0, slippage: 0.0 });
        // Filled at the second bar's open (100) and marked at the last close (109).
        let last = free.curve.last().unwrap().equity;
        assert!((last - 1090.0).abs() < 1e-6, "{last}");
        let paid = run(&fund(spec), &history, &Costs::default());
        assert!(paid.curve.last().unwrap().equity < last);
        assert!(paid.fees > 0.0);
    }
}
