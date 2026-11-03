//! What an equity curve came to.

use serde::{Deserialize, Serialize};

use crate::market::{stdev, YEAR_MS};

/// Equity at a moment.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
pub struct Point {
    pub t: i64,
    pub equity: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Figures {
    pub start: i64,
    pub end: i64,
    pub total_return: f64,
    /// Return per year, compounded. Only meaningful over months.
    pub annual_return: f64,
    pub annual_vol: f64,
    pub sharpe: f64,
    pub max_drawdown: f64,
}

/// Figures for a curve sampled at even steps.
pub fn figures(curve: &[Point]) -> Figures {
    let (Some(first), Some(last)) = (curve.first(), curve.last()) else {
        return Figures::default();
    };
    if curve.len() < 2 || first.equity <= 0.0 {
        return Figures { start: first.t, end: last.t, ..Figures::default() };
    }
    let rets: Vec<f64> = curve.windows(2).map(|w| if w[0].equity > 0.0 { w[1].equity / w[0].equity - 1.0 } else { 0.0 }).collect();
    let step = (last.t - first.t) as f64 / (curve.len() - 1) as f64;
    let per_year = if step > 0.0 { YEAR_MS / step } else { 0.0 };
    let mean = rets.iter().sum::<f64>() / rets.len() as f64;
    let vol = stdev(&rets);
    let years = (last.t - first.t) as f64 / YEAR_MS;
    let total = last.equity / first.equity - 1.0;
    let mut peak = f64::MIN;
    let mut drawdown: f64 = 0.0;
    for point in curve {
        peak = peak.max(point.equity);
        drawdown = drawdown.max(1.0 - point.equity / peak);
    }
    Figures {
        start: first.t,
        end: last.t,
        total_return: total,
        annual_return: if years > 0.0 && total > -1.0 { (1.0 + total).powf(1.0 / years) - 1.0 } else { 0.0 },
        annual_vol: vol * per_year.sqrt(),
        sharpe: if vol > 0.0 { mean / vol * per_year.sqrt() } else { 0.0 },
        max_drawdown: drawdown,
    }
}
