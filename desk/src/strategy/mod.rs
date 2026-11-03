//! Strategies: each turns what it sees into target shares of equity per market.
//! A share of 1.0 is a long as large as the fund; -0.5 a short half its size.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::market::{ema, new_york, returns, stdev, Interval, View};

#[cfg(feature = "venue")]
pub mod agent;

/// What a strategy decided, and why in a line.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Decision {
    pub targets: BTreeMap<String, f64>,
    pub note: String,
}

pub trait Strategy: Send {
    fn decide(&mut self, view: &View) -> Decision;
}

/// A strategy as a document: what a fund is defined by and what is stored with it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Spec {
    /// Fixed shares, put back each bar they drift.
    Basket { weights: BTreeMap<String, f64> },
    /// Long when the fast average is above the slow one, short (or flat) below,
    /// each market sized to the same share of the fund's target volatility.
    Trend { coins: Vec<String>, fast: usize, slow: usize, target_vol: f64, max_weight: f64, long_only: bool },
    /// Enter on a close beyond the highest high or lowest low of `enter` bars,
    /// leave on a close beyond the opposite extreme of `exit` bars.
    Breakout { coins: Vec<String>, enter: usize, exit: usize, target_vol: f64, max_weight: f64, long_only: bool },
    /// Every `hold` bars, rank by return over `lookback` bars: long the best
    /// `top`, and short the worst `top` when `short` is set. Equal shares adding
    /// to `gross`. A negative lookback return rank reverses: `reverse` buys losers.
    Rank { coins: Vec<String>, lookback: usize, hold: usize, top: usize, short: bool, reverse: bool, gross: f64 },
    /// On each New York weekday at 10:00, take the side of each stock's move since
    /// the last 16:00 close, among stocks whose 10:00 to 16:00 move over the last
    /// `days` sessions varied by more than `min_vol`. Equal shares adding to
    /// `gross`; out at 16:00. Runs on hourly bars.
    OpenDrive { coins: Vec<String>, min_vol: f64, days: usize, gross: f64 },
    /// A model reads the markets and sets the targets itself, within `max_gross`.
    Agent { coins: Vec<String>, model: String, mandate: String, max_gross: f64 },
}

impl Spec {
    pub fn coins(&self) -> Vec<String> {
        match self {
            Spec::Basket { weights } => weights.keys().cloned().collect(),
            Spec::Trend { coins, .. }
            | Spec::Breakout { coins, .. }
            | Spec::Rank { coins, .. }
            | Spec::OpenDrive { coins, .. }
            | Spec::Agent { coins, .. } => coins.clone(),
        }
    }

    /// How many closed bars a decision reads.
    pub fn lookback(&self) -> usize {
        match self {
            Spec::Basket { .. } => 1,
            Spec::Trend { slow, .. } => slow * 2,
            Spec::Breakout { enter, exit, .. } => (*enter).max(*exit) + 1 + VOL_BARS,
            Spec::Rank { lookback, .. } => lookback + 1,
            // Enough calendar days to hold `days` sessions and the one before.
            Spec::OpenDrive { days, .. } => (days * 7 / 5 + 6) * 24,
            Spec::Agent { .. } => 24 * 8,
        }
    }

    /// Whether a test needs every market to have bars before it starts.
    pub fn needs_all(&self) -> bool {
        !matches!(self, Spec::Rank { .. } | Spec::OpenDrive { .. })
    }

    /// Whether a test can run it without the network.
    pub fn testable(&self) -> bool {
        !matches!(self, Spec::Agent { .. })
    }

    /// The strategy itself, for a fund trading on bars of `interval`.
    pub fn build(&self, interval: Interval) -> Box<dyn Strategy> {
        match self.clone() {
            Spec::Basket { weights } => Box::new(Basket { weights }),
            Spec::Trend { coins, fast, slow, target_vol, max_weight, long_only } => {
                Box::new(Trend { coins, fast, slow, sizing: Sizing { target_vol, max_weight, interval }, long_only })
            }
            Spec::Breakout { coins, enter, exit, target_vol, max_weight, long_only } => Box::new(Breakout {
                coins,
                enter,
                exit,
                sizing: Sizing { target_vol, max_weight, interval },
                long_only,
                side: BTreeMap::new(),
            }),
            Spec::Rank { coins, lookback, hold, top, short, reverse, gross } => {
                Box::new(Rank { coins, lookback, hold, top, short, reverse, gross, since: None, held: BTreeMap::new() })
            }
            Spec::OpenDrive { coins, min_vol, days, gross } => Box::new(OpenDrive { coins, min_vol, days, gross, held: BTreeMap::new() }),
            #[cfg(feature = "venue")]
            Spec::Agent { coins, model, mandate, max_gross } => Box::new(agent::Agent::new(coins, model, mandate, max_gross, interval)),
            #[cfg(not(feature = "venue"))]
            Spec::Agent { .. } => Box::new(Basket { weights: BTreeMap::new() }),
        }
    }
}

/// Bars of returns read to size a position by its volatility.
const VOL_BARS: usize = 20;

/// Sizes each market so its expected volatility is an equal share of the fund's target.
struct Sizing {
    target_vol: f64,
    max_weight: f64,
    interval: Interval,
}

impl Sizing {
    fn weight(&self, closes: &[f64], markets: usize) -> f64 {
        let tail = &closes[closes.len().saturating_sub(VOL_BARS + 1)..];
        let vol = stdev(&returns(tail)) * self.interval.per_year().sqrt();
        if vol <= 0.0 || markets == 0 {
            return 0.0;
        }
        (self.target_vol / vol / markets as f64).min(self.max_weight)
    }
}

struct Basket {
    weights: BTreeMap<String, f64>,
}

impl Strategy for Basket {
    fn decide(&mut self, view: &View) -> Decision {
        let targets = self.weights.iter().filter(|(coin, _)| view.last(coin).is_some()).map(|(c, w)| (c.clone(), *w)).collect();
        Decision { targets, note: "Back to its set shares.".into() }
    }
}

struct Trend {
    coins: Vec<String>,
    fast: usize,
    slow: usize,
    sizing: Sizing,
    long_only: bool,
}

impl Strategy for Trend {
    fn decide(&mut self, view: &View) -> Decision {
        let mut targets = BTreeMap::new();
        let mut words = Vec::new();
        for coin in &self.coins {
            let closes = view.closes(coin);
            if closes.len() < self.slow {
                continue;
            }
            let (Some(fast), Some(slow)) = (ema(&closes, self.fast), ema(&closes, self.slow)) else {
                continue;
            };
            let side = if fast > slow {
                1.0
            } else if self.long_only {
                0.0
            } else {
                -1.0
            };
            let weight = side * self.sizing.weight(&closes, self.coins.len());
            words.push(format!(
                "{} {}",
                short(coin),
                if side > 0.0 {
                    "up"
                } else if side < 0.0 {
                    "down"
                } else {
                    "flat"
                }
            ));
            targets.insert(coin.clone(), weight);
        }
        Decision { targets, note: format!("Trend: {}.", words.join(", ")) }
    }
}

struct Breakout {
    coins: Vec<String>,
    enter: usize,
    exit: usize,
    sizing: Sizing,
    long_only: bool,
    /// The side each market is on: 1 long, -1 short.
    side: BTreeMap<String, f64>,
}

impl Strategy for Breakout {
    fn decide(&mut self, view: &View) -> Decision {
        let mut targets = BTreeMap::new();
        let mut words = Vec::new();
        for coin in &self.coins {
            let Some(bars) = view.bars.get(coin) else {
                continue;
            };
            if bars.len() < self.enter.max(self.exit) + 1 {
                continue;
            }
            let (last, before) = bars.split_last().unwrap();
            let high = |n: usize| before[before.len() - n..].iter().map(|b| b.h).fold(f64::MIN, f64::max);
            let low = |n: usize| before[before.len() - n..].iter().map(|b| b.l).fold(f64::MAX, f64::min);
            let side = self.side.get(coin).copied().unwrap_or(0.0);
            let side = if last.c > high(self.enter) {
                1.0
            } else if last.c < low(self.enter) && !self.long_only {
                -1.0
            } else if (side > 0.0 && last.c < low(self.exit)) || (side < 0.0 && last.c > high(self.exit)) {
                0.0
            } else {
                side
            };
            self.side.insert(coin.clone(), side);
            if side != 0.0 {
                words.push(format!("{} {}", short(coin), if side > 0.0 { "long" } else { "short" }));
            }
            let closes: Vec<f64> = bars.iter().map(|b| b.c).collect();
            targets.insert(coin.clone(), side * self.sizing.weight(&closes, self.coins.len()));
        }
        let note = if words.is_empty() { "Breakout: waiting for a break.".into() } else { format!("Breakout: {}.", words.join(", ")) };
        Decision { targets, note }
    }
}

struct Rank {
    coins: Vec<String>,
    lookback: usize,
    hold: usize,
    top: usize,
    short: bool,
    reverse: bool,
    gross: f64,
    /// Bars since the last rebalance.
    since: Option<usize>,
    held: BTreeMap<String, f64>,
}

impl Strategy for Rank {
    fn decide(&mut self, view: &View) -> Decision {
        if let Some(since) = self.since {
            if since + 1 < self.hold {
                self.since = Some(since + 1);
                return Decision { targets: self.held.clone(), note: "Holding until the next ranking.".into() };
            }
        }
        let mut scored: Vec<(String, f64)> = self
            .coins
            .iter()
            .filter_map(|coin| {
                let closes = view.closes(coin);
                if closes.len() <= self.lookback {
                    return None;
                }
                let then = closes[closes.len() - 1 - self.lookback];
                Some((coin.clone(), closes.last()? / then - 1.0))
            })
            .collect();
        if scored.len() < self.top * if self.short { 2 } else { 1 } {
            return Decision { targets: BTreeMap::new(), note: "Too few markets with history to rank.".into() };
        }
        scored.sort_by(|a, b| b.1.total_cmp(&a.1));
        if self.reverse {
            scored.reverse();
        }
        let legs = if self.short { 2 * self.top } else { self.top } as f64;
        let each = self.gross / legs;
        let mut targets: BTreeMap<String, f64> = scored.iter().take(self.top).map(|(c, _)| (c.clone(), each)).collect();
        if self.short {
            for (coin, _) in scored.iter().rev().take(self.top) {
                targets.insert(coin.clone(), -each);
            }
        }
        let longs: Vec<&str> = scored.iter().take(self.top).map(|(c, _)| short(c)).collect();
        self.since = Some(0);
        self.held = targets.clone();
        Decision { targets, note: format!("Ranked {} markets; long {}.", scored.len(), longs.join(", ")) }
    }
}

struct OpenDrive {
    coins: Vec<String>,
    min_vol: f64,
    days: usize,
    gross: f64,
    held: BTreeMap<String, f64>,
}

impl OpenDrive {
    /// The side to take on one stock at 10:00, if it qualifies.
    fn side(&self, bars: &[crate::market::Bar], today: i64) -> Option<f64> {
        let last = bars.last()?;
        let (day, hour, _) = new_york(last.t);
        if day != today || hour != 9 {
            return None;
        }
        // Each past session's 10:00 open and 15:00 bar close, by New York day.
        let mut sessions: BTreeMap<i64, (Option<f64>, Option<f64>)> = BTreeMap::new();
        for bar in bars {
            let (day, hour, weekday) = new_york(bar.t);
            if day >= today || weekday >= 5 {
                continue;
            }
            let session = sessions.entry(day).or_default();
            match hour {
                10 => session.0 = Some(bar.o),
                15 => session.1 = Some(bar.c),
                _ => {}
            }
        }
        let previous_close = sessions.values().rev().find_map(|s| s.1)?;
        let moves: Vec<f64> = sessions.values().filter_map(|s| Some(s.1? / s.0? - 1.0)).collect();
        if moves.len() < self.days {
            return None;
        }
        let vol = stdev(&moves[moves.len() - self.days..]);
        let overnight = last.c / previous_close - 1.0;
        (vol > self.min_vol && overnight != 0.0).then(|| overnight.signum())
    }
}

impl Strategy for OpenDrive {
    fn decide(&mut self, view: &View) -> Decision {
        let (today, hour, weekday) = new_york(view.now);
        if weekday >= 5 || !(10..16).contains(&hour) {
            self.held.clear();
            return Decision { targets: BTreeMap::new(), note: "Out until 10:00 in New York.".into() };
        }
        if hour > 10 {
            return Decision { targets: self.held.clone(), note: "Holding the morning's sides until 16:00.".into() };
        }
        let sides: Vec<(String, f64)> =
            self.coins.iter().filter_map(|coin| Some((coin.clone(), self.side(view.bars.get(coin)?, today)?))).collect();
        let each = if sides.is_empty() { 0.0 } else { self.gross / sides.len() as f64 };
        self.held = sides.iter().map(|(coin, side)| (coin.clone(), side * each)).collect();
        let up: Vec<&str> = sides.iter().filter(|(_, s)| *s > 0.0).map(|(c, _)| short(c)).collect();
        let down: Vec<&str> = sides.iter().filter(|(_, s)| *s < 0.0).map(|(c, _)| short(c)).collect();
        let note = if sides.is_empty() {
            "No stock moved enough overnight to trade today.".into()
        } else {
            format!("Up overnight, long: {}. Down overnight, short: {}.", list(&up), list(&down))
        };
        Decision { targets: self.held.clone(), note }
    }
}

fn list(names: &[&str]) -> String {
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}

/// A market's name without its venue prefix.
pub fn short(coin: &str) -> &str {
    coin.rsplit(':').next().unwrap_or(coin)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market::{Bar, HOUR_MS};

    fn bars(closes: &[f64]) -> Vec<Bar> {
        closes.iter().enumerate().map(|(i, &c)| Bar { t: i as i64 * HOUR_MS, o: c, h: c, l: c, c, v: 1.0 }).collect()
    }

    fn view<'a>(series: &'a [(&str, Vec<Bar>)]) -> View<'a> {
        View {
            now: 0,
            bars: series.iter().map(|(c, b)| (c.to_string(), b.as_slice())).collect(),
            funding: BTreeMap::new(),
            weights: BTreeMap::new(),
        }
    }

    #[test]
    fn trend_goes_long_a_rising_market_and_short_a_falling_one() {
        let up: Vec<f64> = (0..100).map(|i| 100.0 * (1.0 + 0.001 * i as f64 + 0.002 * ((i % 3) as f64))).collect();
        let down: Vec<f64> = up.iter().rev().cloned().collect();
        let series = [("UP", bars(&up)), ("DOWN", bars(&down))];
        let spec =
            Spec::Trend { coins: vec!["UP".into(), "DOWN".into()], fast: 5, slow: 20, target_vol: 0.2, max_weight: 2.0, long_only: false };
        let decision = spec.build(Interval::H1).decide(&view(&series));
        assert!(decision.targets["UP"] > 0.0);
        assert!(decision.targets["DOWN"] < 0.0);
    }

    #[test]
    fn rank_holds_between_rankings() {
        let series = [("A", bars(&[1.0, 2.0, 3.0])), ("B", bars(&[3.0, 2.0, 1.0]))];
        let spec =
            Spec::Rank { coins: vec!["A".into(), "B".into()], lookback: 2, hold: 2, top: 1, short: true, reverse: false, gross: 1.0 };
        let mut strategy = spec.build(Interval::D1);
        let first = strategy.decide(&view(&series));
        assert_eq!(first.targets, BTreeMap::from([("A".into(), 0.5), ("B".into(), -0.5)]));
        let flipped = [("A", bars(&[3.0, 2.0, 1.0])), ("B", bars(&[1.0, 2.0, 3.0]))];
        assert_eq!(strategy.decide(&view(&flipped)).targets, first.targets);
        assert_eq!(strategy.decide(&view(&flipped)).targets["B"], 0.5);
    }
}
