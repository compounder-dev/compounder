//! Statistics: what a backtest's daily values add up to.
//!
//! Every figure is read off one series. Its first point is the starting
//! capital, and after it comes the value at each close from the second day
//! on. The first day's close is left out because the first purchase is made
//! at it: measured from the capital, the cost of that purchase sits in the
//! first daily change. So a run of 100 days has 99 daily changes.
//!
//! A year is 252 trading days, and no figure takes a risk-free rate off. A
//! ratio is held as a fraction: 0.05 is 5%.
//!
//! A figure that comes to exactly 0 is 0. A figure that cannot be worked out
//! is `None`, and `missing` gives the reason under the figure's name. A run
//! of one day has no daily change yet, so it has a concentration and nothing
//! else. Bad input is another matter: a run that is not sound is refused
//! whole.
//!
//! The same input gives the same bytes on the server and in the browser.
//! Sums run oldest to newest in plain 64-bit floats with no fused
//! multiply-add. Logarithms and powers are worked out in software: the ones
//! a machine supplies differ in the last digit from one machine to the next.
//! Nothing here reads a clock, a file or the network.

use std::collections::BTreeMap;

use compounder_prices::day::Day;
use serde::Serialize;

/// Trading days in a year: what turns a daily figure into a yearly one.
const YEAR: f64 = 252.0;

/// The widest spread of daily changes that rounding alone can leave. A change
/// is one float over another, less 1, so changes that are truly equal come
/// out a few parts in 1e16 apart. A deviation this small is counted as none:
/// a ratio that divided by it would be noise over noise.
const ROUNDING: f64 = 1e-15;

const SAME_CHANGES: &str = "Every daily change is the same, so there is no spread to measure.";
const NO_LOSS: &str = "No day lost money, so there is no downside to divide by.";
const NO_FALL: &str = "The value never fell below an earlier peak, so there is no drawdown to divide by.";
const NO_TAIL: &str = "The 5th percentile of the daily changes is 0, so there is nothing to divide by.";
const NO_GAIN: &str = "The run did not make money, so there is no gain to share out among its days.";
const TOO_LARGE: &str = "The figure is too large to write as a number.";
const STILL_BENCHMARK: &str = "The benchmark's price did not move on the days both have, so there is nothing to measure against.";
const STILL_SIDE: &str = "The run or the benchmark did not move on the days both have, so the two cannot be compared.";

/// One backtest, day by day. Every list has one entry per day, oldest first.
#[derive(Debug, Clone, Copy)]
pub struct Run<'a> {
    /// The money the run started with, before the first purchase. Above 0.
    pub capital: f64,
    /// The trading days of the run.
    pub days: &'a [Day],
    /// The value at each close, after that day's trades and costs. Above 0.
    pub values: &'a [f64],
    /// The share of the value held in each asset after each close. A day's
    /// row has one entry per asset, in the same order every day, and 0 for an
    /// asset that is not held. Cash has no entry.
    pub weights: &'a [Vec<f64>],
    /// Dollars of assets bought on each day.
    pub bought: &'a [f64],
    /// Dollars of assets sold on each day.
    pub sold: &'a [f64],
}

/// The figures of one run. In the notes below `r` is the list of daily
/// changes of the series and `n` is how many there are.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Stats {
    /// `n`: the days of the run less one.
    pub days: usize,
    /// The last value over the capital, less 1.
    pub total_return: Option<f64>,
    /// The total return as a yearly rate: `(1 + total_return) ^ (252 / (n + 1)) - 1`.
    pub annual_return: Option<f64>,
    /// The mean of `r`.
    pub mean: Option<f64>,
    /// The middle of `r`, or the mean of the middle two.
    pub median: Option<f64>,
    /// The worst day.
    pub min: Option<f64>,
    /// The best day.
    pub max: Option<f64>,
    /// The sample deviation of `r`, times the square root of 252.
    pub volatility: Option<f64>,
    /// `mean * 252 / volatility`.
    pub sharpe: Option<f64>,
    /// `mean * 252` over the yearly downside deviation. That is the square
    /// root of `sum(min(r, 0)^2) / n`, times the square root of 252. The
    /// divisor is `n`, not `n - 1`.
    pub sortino: Option<f64>,
    /// The largest fall from an earlier peak of the series, the capital
    /// included. Positive: 0.2 is a fall of 20%.
    pub max_drawdown: Option<f64>,
    /// `annual_return / max_drawdown`.
    pub calmar: Option<f64>,
    /// The share of `r` above 0.
    pub win_rate: Option<f64>,
    /// Which way `r` leans, corrected for a sample: `sqrt(n * (n - 1)) / (n - 2) * m3 / m2^1.5`,
    /// where `m2`, `m3` and `m4` are the plain central moments of `r`, each divided by `n`.
    pub skewness: Option<f64>,
    /// How heavy the tails of `r` are beyond a normal curve's, corrected for a
    /// sample: `(n - 1) / ((n - 2) * (n - 3)) * ((n + 1) * (m4 / m2^2 - 3) + 6)`.
    pub kurtosis: Option<f64>,
    /// The 95th percentile of `r` over the size of the 5th. It is negative
    /// when even the 95th is a loss. See `percentile`.
    pub tail_ratio: Option<f64>,
    /// The largest daily log change over the sum of them all.
    pub top_day_share: Option<f64>,
    /// The largest `ceil(n * 0.05)` daily log changes over the sum of them
    /// all. Above 1 when the other days lost money between them.
    pub top_5pct_days_share: Option<f64>,
    /// The same for the largest `ceil(n * 0.10)`.
    pub top_10pct_days_share: Option<f64>,
    /// The mean, over every day, of the sum of the squared weights: 1 for a
    /// run that is always in one asset, 0.25 for four equal ones, 0 for cash.
    pub concentration: Option<f64>,
    /// Dollars traded in a year, as a share of the run's value: half the
    /// dollars bought and sold after the first day, over the mean of the
    /// series, times `252 / n`. The first day's purchase is the money going
    /// in and is not counted, so a run that buys once and holds comes to 0.
    pub turnover: Option<f64>,
    /// `252 / n` times the sum, over each day after the first, of half the
    /// summed size of each asset's change in weight. A weight drifts with
    /// its price, so a run that never trades still shows some.
    pub weight_turnover: Option<f64>,
    /// The last daily change, whatever the calendar says of the day before.
    pub trailing_1d: Option<f64>,
    /// What the series made in its last week. The goal date is 7 calendar
    /// days before the last day, and the figure is the last value over the
    /// value on the first day of the run on or after that date, less 1. A
    /// goal date on which nothing traded steps forward to the next day that
    /// did, never back. A run that began after the goal date is shorter than
    /// the span and has no figure. The four below go the same way.
    pub trailing_1w: Option<f64>,
    /// From 14 days before the last day.
    pub trailing_2w: Option<f64>,
    /// From one month before: the same day of the month, or the month's last
    /// day when it has no such day.
    pub trailing_1m: Option<f64>,
    /// From three months before.
    pub trailing_3m: Option<f64>,
    /// From twelve months before.
    pub trailing_1y: Option<f64>,
    /// Why each figure that is `None` could not be worked out, by its name.
    pub missing: BTreeMap<&'static str, String>,
}

/// How a run moved against one benchmark, with the benchmark's own figures.
/// `rs` and `rb` are the daily changes of the run and of the benchmark over
/// the days both have.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Benchmark {
    /// The benchmark's own figures over those days: a run that began with
    /// its first price as capital, held nothing else and never traded again.
    pub stats: Stats,
    /// How far the run moves when the benchmark moves by one: the sample
    /// covariance of `rs` and `rb` over the sample variance of `rb`.
    pub beta: Option<f64>,
    /// What the run made in a year beyond its beta's share of the
    /// benchmark: `(mean(rs) - beta * mean(rb)) * 252`.
    pub alpha: Option<f64>,
    /// How closely the two move together, from -1 to 1: the sample covariance
    /// over the square root of the product of the two sample variances.
    pub correlation: Option<f64>,
    /// The correlation squared.
    pub r_squared: Option<f64>,
    /// The same four worked out on daily log changes.
    pub log_beta: Option<f64>,
    pub log_alpha: Option<f64>,
    pub log_correlation: Option<f64>,
    pub log_r_squared: Option<f64>,
    /// Why each of the eight that is `None` could not be worked out, by its name.
    pub missing: BTreeMap<&'static str, String>,
}

/// The figures of a run, or what is wrong with the run.
pub fn statistics(run: &Run) -> Result<Stats, String> {
    run.check()?;
    let level = run.series();
    let changes = changes(&level);
    let n = changes.len();
    let count = n as f64;
    let last = level[n];

    // With too few changes some of the numbers below are not numbers at all.
    // Each figure says how many changes it takes, and is only read with them.
    let mut sorted = changes.clone();
    sorted.sort_by(f64::total_cmp);
    let mean = sum(changes.iter().copied()) / count;
    let annual_return = libm::pow(last / run.capital, YEAR / (count + 1.0)) - 1.0;
    let drawdown = max_drawdown(&level);

    // How the changes lie around their mean.
    let offsets: Vec<f64> = changes.iter().map(|change| change - mean).collect();
    let squares = spread(sum(offsets.iter().map(|offset| offset * offset)), count);
    let m2 = squares / count;
    let m3 = sum(offsets.iter().map(|offset| offset * offset * offset)) / count;
    let m4 = sum(offsets.iter().map(|offset| offset * offset * offset * offset)) / count;
    let volatility = (squares / (count - 1.0)).sqrt() * YEAR.sqrt();
    let losses = sum(changes.iter().filter(|change| **change < 0.0).map(|change| change * change));
    let downside = (losses / count).sqrt() * YEAR.sqrt();
    let skewness = over((count * (count - 1.0)).sqrt() / (count - 2.0) * m3, m2 * m2.sqrt(), SAME_CHANGES);
    let kurtosis = over(m4, m2 * m2, SAME_CHANGES)
        .map(|tails| (count - 1.0) / ((count - 2.0) * (count - 3.0)) * ((count + 1.0) * (tails - 3.0) + 6.0));

    // What each day added to the growth of the series, largest first. Log
    // changes add up to the whole gain, which plain changes do not.
    let mut gains = log_changes(&level);
    let gain = sum(gains.iter().copied());
    gains.sort_by(|a, b| b.total_cmp(a));
    let top = |days: usize| {
        if last > run.capital {
            over(sum(gains[..days].iter().copied()), gain, NO_GAIN)
        } else {
            Err(NO_GAIN.to_string())
        }
    };

    // A trailing return runs from the first day on or after its goal date,
    // and the first day of all stands for the capital, as it does in the series.
    let end = run.days[n];
    let since = |goal: Day, span: &str| {
        if goal < run.days[0] {
            return Err(format!("The run is shorter than {span}."));
        }
        let base = run.days.partition_point(|day| *day < goal);
        Ok(last / level[base] - 1.0)
    };
    let days_before = |days: i32| Day::from_number(end.number() - days);

    let squared_weights = sum(run.weights.iter().map(|row| sum(row.iter().map(|weight| weight * weight))));
    let moved = sum(run.weights.windows(2).map(|pair| sum(pair[0].iter().zip(&pair[1]).map(|(before, after)| (after - before).abs())) / 2.0));
    let traded = sum(run.bought.iter().zip(run.sold).skip(1).map(|(bought, sold)| (bought + sold) / 2.0));
    let mean_value = sum(level.iter().copied()) / (count + 1.0);

    let mut missing = Missing::default();
    Ok(Stats {
        days: n,
        total_return: missing.keep("total_return", enough(n, 1).map(|()| last / run.capital - 1.0)),
        annual_return: missing.keep("annual_return", enough(n, 1).map(|()| annual_return)),
        mean: missing.keep("mean", enough(n, 1).map(|()| mean)),
        median: missing.keep("median", enough(n, 1).map(|()| median(&sorted))),
        min: missing.keep("min", enough(n, 1).map(|()| sorted[0])),
        max: missing.keep("max", enough(n, 1).map(|()| sorted[n - 1])),
        volatility: missing.keep("volatility", enough(n, 2).map(|()| volatility)),
        sharpe: missing.keep("sharpe", enough(n, 2).and_then(|()| over(mean * YEAR, volatility, SAME_CHANGES))),
        sortino: missing.keep("sortino", enough(n, 2).and_then(|()| over(mean * YEAR, downside, NO_LOSS))),
        max_drawdown: missing.keep("max_drawdown", enough(n, 1).map(|()| drawdown)),
        calmar: missing.keep("calmar", enough(n, 1).and_then(|()| over(annual_return, drawdown, NO_FALL))),
        win_rate: missing.keep("win_rate", enough(n, 1).map(|()| changes.iter().filter(|change| **change > 0.0).count() as f64 / count)),
        skewness: missing.keep("skewness", enough(n, 3).and(skewness)),
        kurtosis: missing.keep("kurtosis", enough(n, 4).and(kurtosis)),
        tail_ratio: missing.keep("tail_ratio", enough(n, 2).and_then(|()| over(percentile(&sorted, 0.95), percentile(&sorted, 0.05).abs(), NO_TAIL))),
        top_day_share: missing.keep("top_day_share", enough(n, 1).and_then(|()| top(1))),
        top_5pct_days_share: missing.keep("top_5pct_days_share", enough(n, 1).and_then(|()| top(n.div_ceil(20)))),
        top_10pct_days_share: missing.keep("top_10pct_days_share", enough(n, 1).and_then(|()| top(n.div_ceil(10)))),
        concentration: missing.keep("concentration", Ok(squared_weights / (count + 1.0))),
        turnover: missing.keep("turnover", enough(n, 1).map(|()| traded / mean_value * YEAR / count)),
        weight_turnover: missing.keep("weight_turnover", enough(n, 1).map(|()| moved * YEAR / count)),
        trailing_1d: missing.keep("trailing_1d", enough(n, 1).map(|()| changes[n - 1])),
        trailing_1w: missing.keep("trailing_1w", since(days_before(7), "one week")),
        trailing_2w: missing.keep("trailing_2w", since(days_before(14), "two weeks")),
        trailing_1m: missing.keep("trailing_1m", since(months_before(end, 1), "one month")),
        trailing_3m: missing.keep("trailing_3m", since(months_before(end, 3), "three months")),
        trailing_1y: missing.keep("trailing_1y", since(months_before(end, 12), "one year")),
        missing: missing.0,
    })
}

/// A run against one benchmark's prices, or what is wrong with either. The
/// prices may cover more days than the run or fewer: only the days both have
/// are read, and the benchmark's own figures start at the first of them.
pub fn benchmark(run: &Run, days: &[Day], prices: &[f64]) -> Result<Benchmark, String> {
    run.check()?;
    if prices.len() != days.len() {
        return Err(format!("the benchmark has {} days but {} prices", days.len(), prices.len()));
    }
    check_series("benchmark price", days, prices)?;

    let level = run.series();
    let (mut both, mut run_level, mut price) = (Vec::new(), Vec::new(), Vec::new());
    for (day, close) in days.iter().zip(prices) {
        if let Ok(position) = run.days.binary_search(day) {
            both.push(*day);
            run_level.push(level[position]);
            price.push(*close);
        }
    }
    if both.is_empty() {
        return Err("the benchmark has no day in common with the run".into());
    }
    let (alone, nothing) = (vec![vec![1.0]; both.len()], vec![0.0; both.len()]);
    let stats = statistics(&Run { capital: price[0], days: &both, values: &price, weights: &alone, bought: &nothing, sold: &nothing })?;

    let [beta, alpha, correlation, r_squared] = fit(&changes(&run_level), &changes(&price));
    let [log_beta, log_alpha, log_correlation, log_r_squared] = fit(&log_changes(&run_level), &log_changes(&price));
    let mut missing = Missing::default();
    Ok(Benchmark {
        stats,
        beta: missing.keep("beta", beta),
        alpha: missing.keep("alpha", alpha),
        correlation: missing.keep("correlation", correlation),
        r_squared: missing.keep("r_squared", r_squared),
        log_beta: missing.keep("log_beta", log_beta),
        log_alpha: missing.keep("log_alpha", log_alpha),
        log_correlation: missing.keep("log_correlation", log_correlation),
        log_r_squared: missing.keep("log_r_squared", log_r_squared),
        missing: missing.0,
    })
}

impl Run<'_> {
    /// Say what is wrong with a run that no figure can be read from.
    fn check(&self) -> Result<(), String> {
        let days = self.days.len();
        if days == 0 {
            return Err("a run needs at least one day".into());
        }
        for (list, entries) in [("values", self.values.len()), ("rows of weights", self.weights.len()), ("amounts bought", self.bought.len()), ("amounts sold", self.sold.len())] {
            if entries != days {
                return Err(format!("the run has {days} days but {entries} {list}"));
            }
        }
        if !(self.capital.is_finite() && self.capital > 0.0) {
            return Err(format!("a capital of {} is not above 0", self.capital));
        }
        check_series("value", self.days, self.values)?;
        let assets = self.weights[0].len();
        for (day, row) in self.days.iter().zip(self.weights) {
            if row.len() != assets {
                return Err(format!("{day} has {} weights but the first day has {assets}", row.len()));
            }
            if let Some(weight) = row.iter().find(|weight| !(weight.is_finite() && **weight >= 0.0)) {
                return Err(format!("a weight of {weight} on {day} is not 0 or more"));
            }
        }
        for (day, traded) in self.days.iter().zip(self.bought).chain(self.days.iter().zip(self.sold)) {
            if !(traded.is_finite() && *traded >= 0.0) {
                return Err(format!("{traded} dollars traded on {day} is not 0 or more"));
            }
        }
        Ok(())
    }

    /// The series every figure is read off: the capital, then the value at
    /// each close from the second day on.
    fn series(&self) -> Vec<f64> {
        std::iter::once(self.capital).chain(self.values[1..].iter().copied()).collect()
    }
}

/// Refuse days that are out of order, and a value or price that is not above
/// 0: nothing can be measured against one.
fn check_series(what: &str, days: &[Day], levels: &[f64]) -> Result<(), String> {
    if let Some(pair) = days.windows(2).find(|pair| pair[1] <= pair[0]) {
        return Err(format!("{} is listed after {} but is not a later day", pair[1], pair[0]));
    }
    match days.iter().zip(levels).find(|(_, level)| !(level.is_finite() && **level > 0.0)) {
        Some((day, level)) => Err(format!("a {what} of {level} on {day} is not above 0")),
        None => Ok(()),
    }
}

/// What one figure came to, or why it has no value.
type Figure = Result<f64, String>;

/// The reasons noted while a set of figures is put together, by figure name.
#[derive(Default)]
struct Missing(BTreeMap<&'static str, String>);

impl Missing {
    /// The value of a figure, or `None` with the reason noted under its name.
    fn keep(&mut self, name: &'static str, figure: Figure) -> Option<f64> {
        let reason = match figure {
            // Adding 0 turns a negative zero into a plain one, so a zero is
            // written one way only.
            Ok(value) if value.is_finite() => return Some(value + 0.0),
            Ok(_) => TOO_LARGE.to_string(),
            Err(reason) => reason,
        };
        self.0.insert(name, reason);
        None
    }
}

/// Refuse a figure that takes more daily changes than a run has.
fn enough(n: usize, least: usize) -> Result<(), String> {
    match n {
        _ if n >= least => Ok(()),
        0 => Err("The run has one day, so it has no daily change yet.".to_string()),
        _ => Err(format!("This takes a run of at least {} days and this one has {}.", least + 1, n + 1)),
    }
}

/// One number over another, or the reason when there is nothing to divide by.
fn over(top: f64, bottom: f64, when_nothing: &str) -> Figure {
    if bottom <= 0.0 {
        return Err(when_nothing.to_string());
    }
    // A bottom that overflowed is not a number, and neither is what it
    // gives. `keep` says so, which is the true reason.
    Ok(top / bottom)
}

/// A sum of squared distances from a mean over `count` numbers, or 0 when
/// the deviation it stands for is no more than rounding.
fn spread(squares: f64, count: f64) -> f64 {
    if (squares / count).sqrt() <= ROUNDING {
        0.0
    } else {
        squares
    }
}

/// Added up oldest to newest, so one list has one sum on every machine.
fn sum(numbers: impl IntoIterator<Item = f64>) -> f64 {
    numbers.into_iter().fold(0.0, |total, number| total + number)
}

/// Each point of a series over the one before it, less 1.
fn changes(level: &[f64]) -> Vec<f64> {
    level.windows(2).map(|pair| pair[1] / pair[0] - 1.0).collect()
}

/// The natural logarithm of each point of a series over the one before it.
fn log_changes(level: &[f64]) -> Vec<f64> {
    level.windows(2).map(|pair| libm::log(pair[1] / pair[0])).collect()
}

fn median(sorted: &[f64]) -> f64 {
    let middle = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[middle]
    } else {
        (sorted[middle - 1] + sorted[middle]) / 2.0
    }
}

/// The value a share `p` of the way up a sorted list. It sits at rank
/// `p * (n + 1)`, counting from 1, on a straight line between the two values
/// around that rank. A rank before the first value or past the last is held
/// at that value.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    let rank = p * (n + 1) as f64;
    if rank <= 1.0 {
        return sorted[0];
    }
    if rank >= n as f64 {
        return sorted[n - 1];
    }
    let below = rank.floor();
    let (low, high) = (sorted[below as usize - 1], sorted[below as usize]);
    low + (rank - below) * (high - low)
}

/// The largest fall from an earlier peak, as a share of that peak.
fn max_drawdown(level: &[f64]) -> f64 {
    let (mut peak, mut worst) = (level[0], 0.0);
    for value in level {
        if *value > peak {
            peak = *value;
        }
        let fall = 1.0 - value / peak;
        if fall > worst {
            worst = fall;
        }
    }
    worst
}

/// The same day of the month, `months` earlier. A month too short to have
/// that day gives its last day: one month before 31 March is the end of
/// February.
fn months_before(day: Day, months: i32) -> Day {
    let (year, month, mut day_of_month) = day.ymd();
    let month_count = year * 12 + month as i32 - 1 - months;
    let (year, month) = (month_count.div_euclid(12), month_count.rem_euclid(12) as u32 + 1);
    loop {
        // Every month has a 28th, so this ends.
        if let Some(found) = Day::from_ymd(year, month, day_of_month) {
            return found;
        }
        day_of_month -= 1;
    }
}

/// Beta, alpha, correlation and its square, for one list of daily changes
/// against another over the same days.
fn fit(run: &[f64], benchmark: &[f64]) -> [Figure; 4] {
    let n = run.len();
    if n < 2 {
        let few = format!("This takes at least 3 days that the run and the benchmark both have, and there {}.", ["is 1", "are 2"][n]);
        return [(); 4].map(|()| Err(few.clone()));
    }
    let count = n as f64;
    let (run_mean, benchmark_mean) = (sum(run.iter().copied()) / count, sum(benchmark.iter().copied()) / count);
    let together = sum(run.iter().zip(benchmark).map(|(change, beside)| (change - run_mean) * (beside - benchmark_mean))) / (count - 1.0);
    let run_variance = spread(sum(run.iter().map(|change| (change - run_mean) * (change - run_mean))), count) / (count - 1.0);
    let benchmark_variance = spread(sum(benchmark.iter().map(|change| (change - benchmark_mean) * (change - benchmark_mean))), count) / (count - 1.0);

    let beta = over(together, benchmark_variance, STILL_BENCHMARK);
    let alpha = beta.clone().map(|beta| (run_mean - beta * benchmark_mean) * YEAR);
    let correlation = over(together, (run_variance * benchmark_variance).sqrt(), STILL_SIDE);
    let r_squared = correlation.clone().map(|correlation| correlation * correlation);
    [beta, alpha, correlation, r_squared]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(text: &str) -> Day {
        text.parse().expect("a day")
    }

    /// Every calendar day from one date to another, both included.
    fn every_day(from: &str, to: &str) -> Vec<Day> {
        (day(from).number()..=day(to).number()).map(Day::from_number).collect()
    }

    /// A made-up run that owns its lists. It runs on consecutive days from
    /// 2025-01-01, holds one asset in full and trades nothing after its
    /// first purchase, until a test says otherwise.
    struct Made {
        capital: f64,
        days: Vec<Day>,
        values: Vec<f64>,
        weights: Vec<Vec<f64>>,
        bought: Vec<f64>,
        sold: Vec<f64>,
    }

    impl Made {
        fn new(capital: f64, values: &[f64]) -> Made {
            let start = day("2025-01-01").number();
            let mut bought = vec![0.0; values.len()];
            bought[0] = capital;
            Made {
                capital,
                days: (0..values.len() as i32).map(|offset| Day::from_number(start + offset)).collect(),
                values: values.to_vec(),
                weights: vec![vec![1.0]; values.len()],
                bought,
                sold: vec![0.0; values.len()],
            }
        }

        fn on(mut self, days: &[&str]) -> Made {
            self.days = days.iter().map(|text| day(text)).collect();
            self
        }

        fn run(&self) -> Run<'_> {
            Run { capital: self.capital, days: &self.days, values: &self.values, weights: &self.weights, bought: &self.bought, sold: &self.sold }
        }

        fn stats(&self) -> Stats {
            statistics(&self.run()).expect("a sound run")
        }
    }

    /// The figure has a value, and it is the expected one but for float noise.
    #[track_caller]
    fn near(found: Option<f64>, expected: f64) {
        let found = found.expect("the figure has a value");
        assert!((found - expected).abs() <= 1e-12 * expected.abs().max(1.0), "found {found}, expected {expected}");
    }

    /// The figure has no value, for this reason.
    #[track_caller]
    fn gap(found: Option<f64>, missing: &BTreeMap<&'static str, String>, name: &str, reason: &str) {
        assert_eq!((found, missing.get(name).map(String::as_str)), (None, Some(reason)), "{name}");
    }

    #[test]
    fn the_series_starts_at_the_capital_and_leaves_out_the_first_close() {
        // 100 goes in. The first purchase costs 2, so the first close is 98,
        // and the second close is 110. The series is 100, 110 and its one
        // change is 110 / 100 - 1 = 0.10. Measured from the first close it
        // would be 110 / 98 - 1 = 0.1224, and the cost would have vanished.
        let stats = Made::new(100.0, &[98.0, 110.0]).stats();
        assert_eq!(stats.days, 1);
        for figure in [stats.total_return, stats.mean, stats.median, stats.min, stats.max, stats.trailing_1d] {
            near(figure, 0.10);
        }
    }

    #[test]
    fn the_figures_of_a_short_run_worked_out_by_hand() {
        // The series is 100, 110, 99, 108.9, 130.68, so r = 0.1, -0.1, 0.1, 0.2 and n = 4.
        let stats = Made::new(100.0, &[99.5, 110.0, 99.0, 108.9, 130.68]).stats();
        assert_eq!(stats.days, 4);
        // 130.68 / 100 - 1.
        near(stats.total_return, 0.3068);
        // (0.1 - 0.1 + 0.1 + 0.2) / 4.
        near(stats.mean, 0.075);
        // In order: -0.1, 0.1, 0.1, 0.2. The middle two are both 0.1.
        near(stats.median, 0.1);
        near(stats.min, -0.1);
        near(stats.max, 0.2);
        // Three of the four are above 0.
        near(stats.win_rate, 0.75);
        // The peak of 110 is followed by 99, a fall of 1 - 99 / 110. Nothing after falls as far.
        near(stats.max_drawdown, 0.1);
        near(stats.trailing_1d, 0.2);

        // Distances from the mean: 0.025, -0.175, 0.025, 0.125.
        // Their squares: 0.000625 + 0.030625 + 0.000625 + 0.015625 = 0.0475.
        // Sample variance 0.0475 / 3, and for a year 0.0475 / 3 * 252 = 3.99.
        near(stats.volatility, 3.99_f64.sqrt());
        // 0.075 * 252 = 18.9, over the volatility of 1.9975: 9.4618.
        near(stats.sharpe, 18.9 / 3.99_f64.sqrt());
        // One losing day: 0.1^2 / 4 = 0.0025, whose root is 0.05.
        // 18.9 / (0.05 * sqrt(252)) = 1.5 * sqrt(252) = 23.8118.
        near(stats.sortino, 1.5 * 252.0_f64.sqrt());

        // Their cubes: 0.000015625 - 0.005359375 + 0.000015625 + 0.001953125 = -0.003375.
        // m3 = -0.003375 / 4 and m2 = 0.0475 / 4, so m3 / m2^1.5 = -0.00675 / 0.0475^1.5 = -0.6520.
        // Times sqrt(4 * 3) / 2, which is sqrt(3): -1.1293.
        near(stats.skewness, -(3.0_f64.sqrt()) * 0.00675 / (0.0475 * 0.0475_f64.sqrt()));
        // Their fourth powers: 0.000000390625 + 0.000937890625 + 0.000000390625 + 0.000244140625 = 0.0011828125.
        // m4 / m2^2 = 4 * 0.0011828125 / 0.0475^2 = 0.00473125 / 0.00225625 = 2.09695.
        // 3 / (2 * 1) * (5 * (2.09695 - 3) + 6) = 1.5 * 1.48476 = 2.2271.
        near(stats.kurtosis, 1.5 * (5.0 * (0.00473125 / 0.00225625 - 3.0) + 6.0));

        // Ranks 0.05 * 5 = 0.25 and 0.95 * 5 = 4.75 fall off both ends of four values: 0.2 over 0.1.
        near(stats.tail_ratio, 2.0);
        // The largest log is ln 1.2, and the four add up to ln 1.3068: 0.18232 / 0.26758 = 0.6814.
        // 5% of four days and 10% of four days both round up to one day.
        for share in [stats.top_day_share, stats.top_5pct_days_share, stats.top_10pct_days_share] {
            near(share, 1.2_f64.ln() / 1.3068_f64.ln());
        }
        // Five days are too few for any of the longer trailing returns, and nothing else is missing.
        assert_eq!(stats.missing.keys().copied().collect::<Vec<_>>(), ["trailing_1m", "trailing_1w", "trailing_1y", "trailing_2w", "trailing_3m"]);
    }

    #[test]
    fn the_yearly_rate_spreads_the_total_return_over_252_days() {
        // 126 days that end 10% up, by way of 120 and then 90.
        // 252 / 126 = 2, so the rate is 1.1^2 - 1.
        let mut closes = vec![90.0; 126];
        (closes[0], closes[1], closes[125]) = (100.0, 120.0, 110.0);
        let stats = Made::new(100.0, &closes).stats();
        assert_eq!(stats.days, 125);
        near(stats.total_return, 0.10);
        near(stats.annual_return, 0.21);
        // The fall from 120 to 90 is 0.25, and 0.21 / 0.25.
        near(stats.max_drawdown, 0.25);
        near(stats.calmar, 0.84);

        // 504 days that end 21% up: 252 / 504 = 0.5, and the square root of 1.21 is 1.1.
        let mut closes = vec![100.0; 504];
        closes[503] = 121.0;
        near(Made::new(100.0, &closes).stats().annual_return, 0.10);

        // Exactly 252 days: the yearly rate is the total return.
        let mut closes = vec![100.0; 252];
        closes[251] = 93.0;
        near(Made::new(100.0, &closes).stats().annual_return, -0.07);
    }

    #[test]
    fn a_percentile_sits_on_the_line_between_its_neighbours_and_stops_at_the_ends() {
        // The squares 1, 4, 9 and so on, so that a wrong neighbour shows.
        let squares = |count: usize| (1..=count).map(|number| (number * number) as f64).collect::<Vec<f64>>();
        let both = |count: usize| (percentile(&squares(count), 0.05), percentile(&squares(count), 0.95));
        // 49 values. Rank 0.05 * 50 = 2.5 is halfway from the 2nd to the 3rd: (4 + 9) / 2.
        // Rank 0.95 * 50 = 47.5 is halfway from the 47th to the 48th: (2209 + 2304) / 2.
        assert_eq!(both(49), (6.5, 2256.5));
        // 24 values. Rank 1.25 is a quarter of the way from 1 to 4.
        // Rank 23.75 is three quarters of the way from 529 to 576.
        assert_eq!(both(24), (1.75, 564.25));
        // 39 values. Ranks 2 and 38 are the 2nd and the 38th themselves.
        assert_eq!(both(39), (4.0, 1444.0));
        // 19 values. Ranks 1 and 19 are the first and the last.
        assert_eq!(both(19), (1.0, 361.0));
        // 18 values. Ranks 0.95 and 18.05 are off the ends, and are held at the first and the last.
        assert_eq!(both(18), (1.0, 324.0));
        assert_eq!(both(1), (1.0, 1.0));
    }

    #[test]
    fn the_best_days_are_measured_in_logs_against_the_whole_gain() {
        // 21 changes: the value doubles, goes up four times, goes up eight
        // times, halves, and then stays put for 17 days. In units of ln 2 the
        // logs are 1, 2, 3 and -1, and they add up to 5.
        let mut closes = vec![100.0, 200.0, 800.0, 6400.0];
        closes.resize(22, 3200.0);
        let stats = Made::new(100.0, &closes).stats();
        assert_eq!(stats.days, 21);
        near(stats.top_day_share, 3.0 / 5.0);
        // 5% of 21 days is 1.05, which rounds up to 2 days: (3 + 2) / 5.
        near(stats.top_5pct_days_share, 1.0);
        // 10% is 2.1, which rounds up to 3: (3 + 2 + 1) / 5. The halving is why it passes 1.
        near(stats.top_10pct_days_share, 6.0 / 5.0);

        // 20 changes, without the halving, so the logs add up to 6.
        // 5% is exactly 1 day and 10% exactly 2: neither rounds up.
        let mut closes = vec![100.0, 200.0, 800.0];
        closes.resize(21, 6400.0);
        let stats = Made::new(100.0, &closes).stats();
        assert_eq!(stats.days, 20);
        near(stats.top_day_share, 3.0 / 6.0);
        near(stats.top_5pct_days_share, 3.0 / 6.0);
        near(stats.top_10pct_days_share, 5.0 / 6.0);
    }

    #[test]
    fn concentration_and_weight_turnover_are_read_off_the_weights() {
        // Two assets over five days. The last day is all cash.
        let mut made = Made::new(100.0, &[100.0; 5]);
        made.weights = vec![vec![0.5, 0.5], vec![0.6, 0.4], vec![0.5, 0.5], vec![1.0, 0.0], vec![0.0, 0.0]];
        let stats = made.stats();
        // Squared weights per day: 0.5, 0.52, 0.5, 1 and 0. Over all five days, 2.52 / 5.
        near(stats.concentration, 0.504);
        // Half the summed moves on each day after the first: 0.1, 0.1, 0.5 and 0.5.
        // 1.2 in all, times 252 / 4.
        near(stats.weight_turnover, 75.6);
    }

    #[test]
    fn turnover_counts_the_dollars_traded_and_not_the_drift() {
        // The series is 100, 110, 90, 100, 100, with a mean of 100.
        let mut made = Made::new(100.0, &[99.0, 110.0, 90.0, 100.0, 100.0]);
        // The first purchase is the money going in. After it: nothing, then
        // 10 bought and 10 sold, then 30 and 30, then 20 sold into cash.
        made.bought = vec![99.0, 0.0, 10.0, 30.0, 0.0];
        made.sold = vec![0.0, 0.0, 10.0, 30.0, 20.0];
        // Half of bought plus sold on each day after the first: 0, 10, 30, 10.
        // 50 over the mean of 100, times 252 / 4.
        near(made.stats().turnover, 31.5);

        // Bought once and held. Two assets drift apart, which moves the weights and trades nothing.
        let mut held = Made::new(100.0, &[100.0, 110.0, 120.0]);
        held.weights = vec![vec![0.5, 0.5], vec![0.6, 0.4], vec![0.7, 0.3]];
        let stats = held.stats();
        assert_eq!(stats.turnover, Some(0.0));
        // Half of 0.1 + 0.1 on each of two days, times 252 / 2.
        near(stats.weight_turnover, 25.2);
    }

    #[test]
    fn a_trailing_return_starts_at_the_first_day_on_or_after_its_goal_date() {
        // A run that ends on Friday 2025-07-11. A week before is Friday the
        // 4th, when the market was closed, so the week is measured from
        // Monday the 7th: 150 / 125 - 1. Stepping back to Thursday the 3rd
        // would have given 150 / 100 - 1 = 0.5.
        let days = ["2025-06-30", "2025-07-01", "2025-07-02", "2025-07-03", "2025-07-07", "2025-07-08", "2025-07-09", "2025-07-10", "2025-07-11"];
        let stats = Made::new(100.0, &[100.0, 100.0, 100.0, 100.0, 125.0, 125.0, 125.0, 125.0, 150.0]).on(&days).stats();
        near(stats.trailing_1w, 0.2);
        // 150 / 125 - 1 again: the last change.
        near(stats.trailing_1d, 0.2);
        // The run began on 2025-06-30, after every earlier goal date.
        gap(stats.trailing_2w, &stats.missing, "trailing_2w", "The run is shorter than two weeks.");
        gap(stats.trailing_1m, &stats.missing, "trailing_1m", "The run is shorter than one month.");
        gap(stats.trailing_3m, &stats.missing, "trailing_3m", "The run is shorter than three months.");
        gap(stats.trailing_1y, &stats.missing, "trailing_1y", "The run is shorter than one year.");
    }

    #[test]
    fn a_trailing_return_from_the_first_day_is_measured_from_the_capital() {
        // Monday to the Monday after. The goal date of the week is the first
        // day itself, which stands for the capital of 100 and not for its
        // own close of 99: 110 / 100 - 1.
        let days = ["2025-07-07", "2025-07-08", "2025-07-09", "2025-07-10", "2025-07-11", "2025-07-14"];
        let stats = Made::new(100.0, &[99.0, 104.0, 104.0, 104.0, 104.0, 110.0]).on(&days).stats();
        near(stats.trailing_1w, 0.10);
        // The day before the last day is a Sunday. The one day figure is still the last change, 110 / 104 - 1.
        near(stats.trailing_1d, 110.0 / 104.0 - 1.0);
        gap(stats.trailing_2w, &stats.missing, "trailing_2w", "The run is shorter than two weeks.");
    }

    #[test]
    fn every_span_is_measured_from_its_own_goal_date() {
        // A value that trades every day of the calendar from 2024-01-01 and
        // gains a dollar a day from 1000. It ends on 2025-03-31, day 455, at 1455.
        let days = every_day("2024-01-01", "2025-03-31");
        let closes: Vec<f64> = (0..days.len()).map(|index| 1000.0 + index as f64).collect();
        let mut made = Made::new(1000.0, &closes);
        made.days = days;
        let stats = made.stats();
        assert_eq!(stats.days, 455);
        near(stats.trailing_1d, 1455.0 / 1454.0 - 1.0);
        // 7 and 14 days back.
        near(stats.trailing_1w, 1455.0 / 1448.0 - 1.0);
        near(stats.trailing_2w, 1455.0 / 1441.0 - 1.0);
        // February has no 31st, so a month back is 2025-02-28: 31 days.
        near(stats.trailing_1m, 1455.0 / 1424.0 - 1.0);
        // 2024-12-31 is 90 days back, and 2024-03-31 is 365.
        near(stats.trailing_3m, 1455.0 / 1365.0 - 1.0);
        near(stats.trailing_1y, 1455.0 / 1090.0 - 1.0);
    }

    #[test]
    fn a_month_step_keeps_the_day_or_falls_back_to_the_end_of_the_month() {
        for (from, months, goal) in [
            ("2025-01-15", 1, "2024-12-15"),
            ("2025-01-15", 3, "2024-10-15"),
            ("2025-01-15", 12, "2024-01-15"),
            ("2025-07-31", 1, "2025-06-30"),
            ("2025-12-31", 1, "2025-11-30"),
            ("2025-03-31", 1, "2025-02-28"),
            ("2024-03-31", 1, "2024-02-29"),
            ("2025-05-31", 3, "2025-02-28"),
            ("2024-02-29", 12, "2023-02-28"),
        ] {
            assert_eq!(months_before(day(from), months), day(goal), "{months} before {from}");
        }

        // A run that ends on Monday 2025-03-31. A month back there is no 31st
        // of February, so the goal is Friday the 28th, at 80: 100 / 80 - 1.
        // Rolling over into March would start at Monday the 3rd, at 90.
        let days = ["2025-02-27", "2025-02-28", "2025-03-03", "2025-03-31"];
        let stats = Made::new(100.0, &[100.0, 80.0, 90.0, 100.0]).on(&days).stats();
        near(stats.trailing_1m, 0.25);
    }

    #[test]
    fn a_run_of_one_day_has_a_concentration_and_nothing_else() {
        let mut made = Made::new(100.0, &[99.0]);
        made.weights = vec![vec![0.6, 0.4]];
        let stats = made.stats();
        assert_eq!(stats.days, 0);
        // 0.36 + 0.16.
        near(stats.concentration, 0.52);
        assert_eq!(stats.missing.len(), 26);
        assert_eq!(stats.missing["total_return"], "The run has one day, so it has no daily change yet.");
        assert_eq!(stats.missing["trailing_1d"], "The run has one day, so it has no daily change yet.");
        assert_eq!(stats.missing["trailing_1w"], "The run is shorter than one week.");
    }

    #[test]
    fn a_figure_waits_for_the_changes_it_takes() {
        // The same closes cut to 2, 3, 4 and 5 days: r = 0.1, -0.1, 0.1, 0.2.
        let closes = [100.0, 110.0, 99.0, 108.9, 130.68];
        let cut = |days: usize| Made::new(100.0, &closes[..days]).stats();

        // One change has a mean and no spread.
        let stats = cut(2);
        near(stats.mean, 0.1);
        for (found, name) in [(stats.volatility, "volatility"), (stats.sharpe, "sharpe"), (stats.sortino, "sortino"), (stats.tail_ratio, "tail_ratio")] {
            gap(found, &stats.missing, name, "This takes a run of at least 3 days and this one has 2.");
        }
        gap(stats.skewness, &stats.missing, "skewness", "This takes a run of at least 4 days and this one has 2.");
        gap(stats.kurtosis, &stats.missing, "kurtosis", "This takes a run of at least 5 days and this one has 2.");

        // Two changes, 0.1 and -0.1, with a mean of 0: variance 0.02 / 1, and for a year 5.04.
        let stats = cut(3);
        near(stats.volatility, 5.04_f64.sqrt());
        // A mean of 0 over a spread, and over a downside, is 0.
        near(stats.sharpe, 0.0);
        near(stats.sortino, 0.0);
        // Both percentiles are off the ends of two values: 0.1 over 0.1.
        near(stats.tail_ratio, 1.0);
        gap(stats.skewness, &stats.missing, "skewness", "This takes a run of at least 4 days and this one has 3.");
        gap(stats.kurtosis, &stats.missing, "kurtosis", "This takes a run of at least 5 days and this one has 3.");

        let stats = cut(4);
        assert!(stats.skewness.is_some());
        gap(stats.kurtosis, &stats.missing, "kurtosis", "This takes a run of at least 5 days and this one has 4.");

        let stats = cut(5);
        assert!(stats.skewness.is_some() && stats.kurtosis.is_some());
    }

    #[test]
    fn a_flat_run_sends_zeros_and_says_what_it_cannot_divide_by() {
        let stats = Made::new(100.0, &[100.0; 5]).stats();
        assert_eq!(stats.days, 4);
        let zeros = [
            stats.total_return,
            stats.annual_return,
            stats.mean,
            stats.median,
            stats.min,
            stats.max,
            stats.volatility,
            stats.max_drawdown,
            stats.win_rate,
            stats.turnover,
            stats.weight_turnover,
            stats.trailing_1d,
        ];
        assert_eq!(zeros, [Some(0.0); 12]);
        assert_eq!(stats.concentration, Some(1.0));
        gap(stats.sharpe, &stats.missing, "sharpe", SAME_CHANGES);
        gap(stats.sortino, &stats.missing, "sortino", NO_LOSS);
        gap(stats.calmar, &stats.missing, "calmar", NO_FALL);
        gap(stats.skewness, &stats.missing, "skewness", SAME_CHANGES);
        gap(stats.kurtosis, &stats.missing, "kurtosis", SAME_CHANGES);
        gap(stats.tail_ratio, &stats.missing, "tail_ratio", NO_TAIL);
        gap(stats.top_day_share, &stats.missing, "top_day_share", NO_GAIN);
        gap(stats.top_5pct_days_share, &stats.missing, "top_5pct_days_share", NO_GAIN);
        gap(stats.top_10pct_days_share, &stats.missing, "top_10pct_days_share", NO_GAIN);
    }

    #[test]
    fn a_run_that_only_falls() {
        // The series is 100, 90, 72, 36, so r = -0.1, -0.2, -0.5.
        let stats = Made::new(100.0, &[100.0, 90.0, 72.0, 36.0]).stats();
        near(stats.total_return, -0.64);
        near(stats.mean, -0.8 / 3.0);
        near(stats.median, -0.2);
        near(stats.min, -0.5);
        near(stats.max, -0.1);
        // No day is above 0. That is a figure of 0, not a gap.
        assert_eq!(stats.win_rate, Some(0.0));
        // The peak is the capital and the low is the last value.
        near(stats.max_drawdown, 0.64);
        // 0.36 ^ (252 / 4) is about 1e-28, too small for a float to tell the
        // rate from -1. Then -1 / 0.64.
        near(stats.annual_return, -1.0);
        near(stats.calmar, -1.5625);
        // Every day is a loss: (0.01 + 0.04 + 0.25) / 3 = 0.1.
        // -0.8 / 3 * 252 / sqrt(0.1 * 252) = -0.8 / 3 * sqrt(2520) = -13.3866.
        near(stats.sortino, -0.8 / 3.0 * 2520.0_f64.sqrt());
        // Both percentiles are off the ends of three values: -0.1 over the
        // size of -0.5. It is negative because even the best day lost.
        near(stats.tail_ratio, -0.2);
        gap(stats.top_day_share, &stats.missing, "top_day_share", NO_GAIN);
        gap(stats.top_5pct_days_share, &stats.missing, "top_5pct_days_share", NO_GAIN);
        gap(stats.top_10pct_days_share, &stats.missing, "top_10pct_days_share", NO_GAIN);
    }

    #[test]
    fn a_run_that_only_rises_has_no_drawdown_and_no_downside() {
        // The series is 100, 110, 121, 145.2, so r = 0.1, 0.1, 0.2.
        let stats = Made::new(100.0, &[100.0, 110.0, 121.0, 145.2]).stats();
        assert_eq!(stats.max_drawdown, Some(0.0));
        assert_eq!(stats.win_rate, Some(1.0));
        gap(stats.calmar, &stats.missing, "calmar", NO_FALL);
        gap(stats.sortino, &stats.missing, "sortino", NO_LOSS);
        assert!(stats.sharpe.is_some());
    }

    #[test]
    fn a_spread_that_is_only_rounding_counts_as_none() {
        // Shares of an asset whose price gains 0.01% every day. Each change
        // of the value is 0.0001, but as floats they come out a few parts in
        // 1e16 apart, and a mean over a spread of that size would be a
        // Sharpe in the millions of millions.
        let mut price = 10.0;
        let mut closes = Vec::new();
        for _ in 0..31 {
            closes.push(1234.56789 * price);
            price *= 1.0001;
        }
        let made = Made::new(closes[0], &closes);
        let steps = changes(&made.run().series());
        assert!(steps.iter().any(|step| *step != steps[0]), "the changes differ in their last digits");
        let stats = made.stats();
        near(stats.mean, 0.0001);
        assert_eq!(stats.volatility, Some(0.0));
        gap(stats.sharpe, &stats.missing, "sharpe", SAME_CHANGES);
        gap(stats.skewness, &stats.missing, "skewness", SAME_CHANGES);
        gap(stats.kurtosis, &stats.missing, "kurtosis", SAME_CHANGES);

        // As a benchmark it has nothing to measure a run against.
        let uneven: Vec<f64> = (0..31).map(|index| 100.0 + (index * index % 7) as f64).collect();
        let run = Made::new(100.0, &uneven);
        let against = benchmark(&run.run(), &run.days, &closes).expect("sound");
        gap(against.beta, &against.missing, "beta", STILL_BENCHMARK);
        gap(against.log_correlation, &against.missing, "log_correlation", STILL_SIDE);
    }

    #[test]
    fn a_figure_too_large_for_a_float_is_missing_and_not_infinite() {
        // A thousandfold in a day is a yearly rate of 1000^126, past what a float holds.
        let stats = Made::new(1.0, &[1.0, 1000.0]).stats();
        near(stats.total_return, 999.0);
        gap(stats.annual_return, &stats.missing, "annual_return", TOO_LARGE);

        // A single change past what a float holds leaves no mean and no
        // spread to read, and each figure built on them says why.
        let stats = Made::new(1e-200, &[1e-200, 1e200, 1e-200]).stats();
        for (found, name) in [(stats.mean, "mean"), (stats.volatility, "volatility"), (stats.sharpe, "sharpe"), (stats.sortino, "sortino")] {
            gap(found, &stats.missing, name, TOO_LARGE);
        }
    }

    #[test]
    fn a_zero_is_written_one_way() {
        let kept = Missing::default().keep("mean", Ok(-0.0)).expect("a value");
        assert!(kept == 0.0 && kept.is_sign_positive());
    }

    /// Every figure that is null has a reason under its name, and no other
    /// name has one.
    #[track_caller]
    fn reasons_fit_the_gaps(answer: &impl Serialize) -> Vec<String> {
        let json = serde_json::to_value(answer).expect("serializes");
        let fields = json.as_object().expect("an object");
        let reasons = fields["missing"].as_object().expect("the reasons");
        for (name, value) in fields {
            assert_eq!(value.is_null(), reasons.contains_key(name), "{name}");
        }
        assert!(reasons.keys().all(|name| fields.contains_key(name)), "{reasons:?}");
        fields.keys().cloned().collect()
    }

    #[test]
    fn every_missing_figure_has_a_reason_and_no_other_does() {
        let runs = [
            Made::new(100.0, &[99.0]),
            Made::new(100.0, &[98.0, 110.0]),
            Made::new(100.0, &[100.0; 5]),
            Made::new(100.0, &[100.0, 90.0, 72.0, 36.0]),
            Made::new(100.0, &[99.5, 110.0, 99.0, 108.9, 130.68]),
            Made::new(1.0, &[1.0, 1000.0]),
        ];
        let mut names = Vec::new();
        for made in &runs {
            names = reasons_fit_the_gaps(&made.stats());
            for prices in [vec![50.0; made.days.len()], (0..made.days.len()).map(|index| 50.0 + (index * index) as f64).collect()] {
                reasons_fit_the_gaps(&benchmark(&made.run(), &made.days, &prices).expect("sound"));
            }
        }
        // The names an answer is read by.
        let mut expected = vec![
            "days",
            "total_return",
            "annual_return",
            "mean",
            "median",
            "min",
            "max",
            "volatility",
            "sharpe",
            "sortino",
            "max_drawdown",
            "calmar",
            "win_rate",
            "skewness",
            "kurtosis",
            "tail_ratio",
            "top_day_share",
            "top_5pct_days_share",
            "top_10pct_days_share",
            "concentration",
            "turnover",
            "weight_turnover",
            "trailing_1d",
            "trailing_1w",
            "trailing_2w",
            "trailing_1m",
            "trailing_3m",
            "trailing_1y",
            "missing",
        ];
        expected.sort_unstable();
        assert_eq!(names, expected);
    }

    #[test]
    fn any_sound_run_gives_numbers_or_reasons_and_never_fails() {
        let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut compared = 0;
        for case in 0..2000 {
            // Up to 40 days with gaps between them, up to three assets, and
            // values that sit still on some days and move on others. One run
            // in ten also jumps further than a float can carry.
            let (length, assets, wild) = (1 + (next() % 40) as usize, (next() % 4) as usize, next() % 10 == 0);
            let mut made = Made::new(1000.0, &vec![1000.0; length]);
            let (mut day, mut value) = (20_000, 1000.0_f64);
            for index in 0..length {
                day += 1 + (next() % 5) as i32;
                let step = if next() % 3 == 0 { 0.0 } else { (next() % 2001) as f64 / 1000.0 - 1.0 };
                let jump: f64 = if wild && next() % 4 == 0 { 1e300 } else { 1.05 };
                value = (value * jump.powf(step)).clamp(1e-300, 1e300);
                made.days[index] = Day::from_number(day);
                made.values[index] = value;
                made.weights[index] = (0..assets).map(|_| (next() % 1001) as f64 / 3000.0).collect();
                made.bought[index] = (next() % 500) as f64;
                made.sold[index] = (next() % 500) as f64;
            }
            let stats = statistics(&made.run()).unwrap_or_else(|reason| panic!("case {case}: {reason}"));
            assert_eq!(stats.days, length - 1);
            reasons_fit_the_gaps(&stats);

            // A benchmark on some of the run's days and on some days before it.
            let days: Vec<Day> = (0..5).map(|back| Day::from_number(20_000 - 5 + back)).chain(made.days.iter().copied().filter(|_| next() % 3 != 0)).collect();
            let prices: Vec<f64> = days.iter().map(|_| 1.0 + (next() % 1000) as f64 / 10.0).collect();
            match benchmark(&made.run(), &days, &prices) {
                Ok(against) => {
                    reasons_fit_the_gaps(&against);
                    reasons_fit_the_gaps(&against.stats);
                    compared += 1;
                }
                Err(reason) => assert_eq!(reason, "the benchmark has no day in common with the run", "case {case}"),
            }
        }
        assert!(compared > 1500, "{compared}");
    }

    #[test]
    fn a_benchmark_is_compared_on_daily_changes_and_on_their_logs() {
        // The run goes 100, 200, 100, 400, 800 and the benchmark 10, 20, 20, 40, 160.
        let made = Made::new(100.0, &[100.0, 200.0, 100.0, 400.0, 800.0]);
        let against = benchmark(&made.run(), &made.days, &[10.0, 20.0, 20.0, 40.0, 160.0]).expect("sound");
        // Changes: rs = 1, -0.5, 3, 1 with a mean of 1.125, and rb = 1, 0, 1, 3 with a mean of 1.25.
        // Distances from the means: -0.125, -1.625, 1.875, -0.125 and -0.25, -1.25, -0.25, 1.75.
        // Their products: 0.03125 + 2.03125 - 0.46875 - 0.21875 = 1.375.
        // Squares of rb add up to 4.75 and of rs to 6.1875. The divisor of 3 cancels each time.
        near(against.beta, 1.375 / 4.75);
        // (1.125 - 11/38 * 1.25) * 252 = 29/38 * 252 = 192.3158.
        near(against.alpha, 29.0 / 38.0 * 252.0);
        near(against.correlation, 1.375 / (4.75_f64 * 6.1875).sqrt());
        near(against.r_squared, 1.375 * 1.375 / (4.75 * 6.1875));
        // Logs in units of ln 2: 1, -1, 2, 1 with a mean of 0.75, and 1, 0, 1, 2 with a mean of 1.
        // Distances: 0.25, -1.75, 1.25, 0.25 and 0, -1, 0, 1. Their products: 1.75 + 0.25 = 2.
        // Squares add up to 2 for the benchmark and 4.75 for the run.
        near(against.log_beta, 1.0);
        // (0.75 - 1 * 1) * ln 2 * 252.
        near(against.log_alpha, -63.0 * 2.0_f64.ln());
        near(against.log_correlation, 2.0 / 9.5_f64.sqrt());
        near(against.log_r_squared, 4.0 / 9.5);
        assert!(against.missing.is_empty());
    }

    #[test]
    fn a_benchmark_is_compared_over_the_days_both_have() {
        // The run trades Monday to Friday. Its first close is 99, on a capital of 100.
        let run_days = ["2025-01-06", "2025-01-07", "2025-01-08", "2025-01-09", "2025-01-10"];
        let made = Made::new(100.0, &[99.0, 200.0, 100.0, 400.0, 1600.0]).on(&run_days);

        // This benchmark has the Friday before the run and no Wednesday. On
        // the four days both have, the run is 100, 200, 400, 1600 and the
        // benchmark 10, 20, 40, 160: the same changes, 1, 1 and 3. The run's
        // first day counts as its capital, or the first change would be
        // 200 / 99 - 1 and the two would not move as one.
        let days: Vec<Day> = ["2025-01-03", "2025-01-06", "2025-01-07", "2025-01-09", "2025-01-10"].map(day).to_vec();
        let against = benchmark(&made.run(), &days, &[7.0, 10.0, 20.0, 40.0, 160.0]).expect("sound");
        near(against.beta, 1.0);
        near(against.alpha, 0.0);
        near(against.correlation, 1.0);
        near(against.log_beta, 1.0);
        // Its own figures start at 10, its price on the first day both have: 160 / 10 - 1.
        assert_eq!(against.stats.days, 3);
        near(against.stats.total_return, 15.0);

        // This one starts a day after the run. The run's level on the first
        // day both have is its close of 200: 200, 100, 400, 1600 against
        // 20, 10, 40, 160, which again move as one.
        let against = benchmark(&made.run(), &made.days[1..], &[20.0, 10.0, 40.0, 160.0]).expect("sound");
        near(against.beta, 1.0);
        near(against.correlation, 1.0);
        near(against.stats.total_return, 7.0);
    }

    #[test]
    fn a_benchmark_has_its_own_figures_from_its_first_price() {
        let made = Made::new(100.0, &[100.0, 101.0, 102.0, 103.0]);
        // Prices 50, 55, 44, 66: changes of 0.1, -0.2 and 0.5.
        let own = benchmark(&made.run(), &made.days, &[50.0, 55.0, 44.0, 66.0]).expect("sound").stats;
        assert_eq!(own.days, 3);
        // 66 / 50 - 1.
        near(own.total_return, 0.32);
        // (0.1 - 0.2 + 0.5) / 3.
        near(own.mean, 0.4 / 3.0);
        // From 55 to 44.
        near(own.max_drawdown, 0.2);
        // One asset, held in full and never traded.
        assert_eq!((own.concentration, own.turnover, own.weight_turnover), (Some(1.0), Some(0.0), Some(0.0)));
    }

    #[test]
    fn a_benchmark_with_too_little_to_compare_says_so() {
        let made = Made::new(100.0, &[100.0, 110.0, 99.0, 120.0]);
        let names = ["beta", "alpha", "correlation", "r_squared", "log_beta", "log_alpha", "log_correlation", "log_r_squared"];
        let reasons = |days: &[Day], prices: &[f64]| {
            let against = benchmark(&made.run(), days, prices).expect("sound");
            assert_eq!(serde_json::to_value(&against).expect("serializes").as_object().expect("an object").values().filter(|value| value.is_null()).count(), 8);
            names.map(|name| against.missing[name].clone())
        };

        // A price that never moves has no variance to divide by.
        let still = reasons(&made.days, &[50.0; 4]);
        assert_eq!(still, [STILL_BENCHMARK, STILL_BENCHMARK, STILL_SIDE, STILL_SIDE, STILL_BENCHMARK, STILL_BENCHMARK, STILL_SIDE, STILL_SIDE]);

        // Two days in common are one change, and a covariance takes two.
        let two = reasons(&made.days[..2], &[50.0, 55.0]);
        assert_eq!(two, [(); 8].map(|()| "This takes at least 3 days that the run and the benchmark both have, and there are 2."));
        let one = reasons(&made.days[3..], &[50.0]);
        assert_eq!(one, [(); 8].map(|()| "This takes at least 3 days that the run and the benchmark both have, and there is 1."));
    }

    #[test]
    fn a_run_that_is_not_sound_is_refused_with_a_reason() {
        let refused = |spoil: fn(&mut Made)| {
            let mut made = Made::new(100.0, &[100.0, 110.0, 120.0]);
            spoil(&mut made);
            statistics(&made.run()).expect_err("refused")
        };
        assert_eq!(refused(|made| made.capital = 0.0), "a capital of 0 is not above 0");
        assert_eq!(refused(|made| made.capital = f64::NAN), "a capital of NaN is not above 0");
        assert_eq!(refused(|made| made.values[1] = 0.0), "a value of 0 on 2025-01-02 is not above 0");
        assert_eq!(refused(|made| made.values[2] = f64::INFINITY), "a value of inf on 2025-01-03 is not above 0");
        assert_eq!(refused(|made| made.values.truncate(2)), "the run has 3 days but 2 values");
        assert_eq!(refused(|made| made.weights.truncate(1)), "the run has 3 days but 1 rows of weights");
        assert_eq!(refused(|made| made.bought.push(0.0)), "the run has 3 days but 4 amounts bought");
        assert_eq!(refused(|made| made.sold.clear()), "the run has 3 days but 0 amounts sold");
        assert_eq!(refused(|made| made.days.swap(1, 2)), "2025-01-02 is listed after 2025-01-03 but is not a later day");
        assert_eq!(refused(|made| made.days[1] = made.days[0]), "2025-01-01 is listed after 2025-01-01 but is not a later day");
        assert_eq!(refused(|made| made.weights[1] = vec![0.5, 0.5]), "2025-01-02 has 2 weights but the first day has 1");
        assert_eq!(refused(|made| made.weights[1] = vec![-0.5]), "a weight of -0.5 on 2025-01-02 is not 0 or more");
        assert_eq!(refused(|made| made.bought[1] = -1.0), "-1 dollars traded on 2025-01-02 is not 0 or more");
        assert_eq!(refused(|made| made.sold[2] = f64::NAN), "NaN dollars traded on 2025-01-03 is not 0 or more");
        let nothing = Run { capital: 100.0, days: &[], values: &[], weights: &[], bought: &[], sold: &[] };
        assert_eq!(statistics(&nothing).expect_err("refused"), "a run needs at least one day");
    }

    #[test]
    fn a_benchmark_that_is_not_sound_is_refused_with_a_reason() {
        let made = Made::new(100.0, &[100.0, 110.0, 120.0]);
        let refused = |days: &[Day], prices: &[f64]| benchmark(&made.run(), days, prices).expect_err("refused");
        assert_eq!(refused(&made.days, &[50.0, 55.0]), "the benchmark has 3 days but 2 prices");
        assert_eq!(refused(&made.days, &[50.0, 0.0, 55.0]), "a benchmark price of 0 on 2025-01-02 is not above 0");
        assert_eq!(refused(&[made.days[1], made.days[0]], &[50.0, 55.0]), "2025-01-01 is listed after 2025-01-02 but is not a later day");
        assert_eq!(refused(&[day("2024-12-31")], &[50.0]), "the benchmark has no day in common with the run");
        let unsound = Run { capital: -1.0, ..made.run() };
        assert_eq!(benchmark(&unsound, &made.days, &[50.0, 55.0, 60.0]).expect_err("refused"), "a capital of -1 is not above 0");
    }
}
