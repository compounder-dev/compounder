//! Real money: one owner's account on the venue, traded by an agent key toward
//! what the managers it has hired want, each over its share of the account.
//!
//! Each minute the targets of every hired manager are added up per market, sized
//! to the account's value as the venue reports it, and compared with the
//! positions the venue holds. A market moves only when the change is at least the
//! venue's smallest order, unless the target closes it out. Every order and the
//! venue's answer go into the account's own ledger.
//!
//! It runs only when `REAL_OWNER`, `REAL_AGENT_KEY` and `REAL_MIX` are set, and
//! stands still while a file named `paused` sits in its folder. `REAL_URL` picks
//! the network; the account's positions, marks and market numbers are all read
//! from that same network, so the test network works end to end.

use std::{collections::BTreeMap, path::PathBuf};

use serde_json::Value;

use crate::{
    account::Fill,
    exchange::{price, Exchange, MAINNET},
    ledger::{Event, Ledger},
    strategy::short,
    venue::{Market, Venue},
};

/// The venue takes no order worth less than this many dollars.
pub const MIN_ORDER: f64 = 10.0;
/// How far past the mark an order may fill, so it fills now.
const REACH: f64 = 0.01;

pub struct Real {
    pub owner: String,
    exchange: Exchange,
    /// The network the account lives on, read for its positions and marks.
    pub venue: Venue,
    /// Manager id and the share of the account it runs.
    pub mix: Vec<(String, f64)>,
    ledger: Ledger,
    dir: PathBuf,
}

/// What the venue holds for the owner: value and signed size per market.
pub struct Holdings {
    pub value: f64,
    pub size: BTreeMap<String, f64>,
}

fn num(v: &Value) -> f64 {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_f64()).unwrap_or(0.0)
}

impl Real {
    pub fn from_env(dir: PathBuf) -> Result<Option<Real>, String> {
        let (Ok(owner), Ok(key), Ok(mix)) = (std::env::var("REAL_OWNER"), std::env::var("REAL_AGENT_KEY"), std::env::var("REAL_MIX")) else {
            return Ok(None);
        };
        let url = std::env::var("REAL_URL").unwrap_or_else(|_| MAINNET.to_string());
        let exchange = Exchange::new(&key, &url)?;
        let mix = mix
            .split(',')
            .filter_map(|part| {
                let (id, share) = part.split_once(':')?;
                Some((id.trim().to_string(), share.trim().parse::<f64>().ok()?))
            })
            .collect();
        let ledger = Ledger::open(&dir.join("ledger.jsonl")).map_err(|e| e.to_string())?;
        Ok(Some(Real { owner: owner.to_lowercase(), exchange, venue: Venue::at(&format!("{url}/info")), mix, ledger, dir }))
    }

    pub fn network(&self) -> &'static str {
        if self.exchange.mainnet() { "mainnet" } else { "testnet" }
    }

    pub fn agent(&self) -> String {
        format!("{}", self.exchange.agent())
    }

    pub fn paused(&self) -> bool {
        self.dir.join("paused").exists()
    }

    /// The owner's value and positions, across the venue's own markets and each named dex.
    pub fn holdings(&self, dexes: &[&str]) -> Result<Holdings, String> {
        let mut out = Holdings { value: 0.0, size: BTreeMap::new() };
        for dex in std::iter::once("").chain(dexes.iter().copied()) {
            let state = self.venue.clearinghouse(&self.owner, dex)?;
            out.value += num(&state["marginSummary"]["accountValue"]);
            for p in state["assetPositions"].as_array().into_iter().flatten() {
                let coin = p["position"]["coin"].as_str().unwrap_or_default().to_string();
                out.size.insert(coin, num(&p["position"]["szi"]));
            }
        }
        Ok(out)
    }

    /// The targets the hired managers add up to, as shares of the account.
    pub fn targets(&self, managers: &BTreeMap<String, BTreeMap<String, f64>>) -> BTreeMap<String, f64> {
        let mut out = BTreeMap::new();
        for (id, share) in &self.mix {
            for (coin, weight) in managers.get(id).into_iter().flatten() {
                *out.entry(coin.clone()).or_insert(0.0) += share * weight;
            }
        }
        out
    }

    /// One minute of the real account: compare, trade what moved enough, record it.
    pub fn tick(
        &mut self,
        now: i64,
        managers: &BTreeMap<String, BTreeMap<String, f64>>,
        markets: &BTreeMap<String, Market>,
        assets: &BTreeMap<String, u32>,
    ) -> Result<(), String> {
        if self.paused() {
            return Ok(());
        }
        let targets = self.targets(managers);
        let mut dexes: Vec<&str> = targets.keys().filter_map(|c| c.split_once(':').map(|(d, _)| d)).collect();
        dexes.sort();
        dexes.dedup();
        let held = self.holdings(&dexes)?;
        if held.value <= 0.0 {
            return Ok(());
        }
        let mut coins: Vec<&String> = targets.keys().chain(held.size.keys()).collect();
        coins.sort();
        coins.dedup();
        let mut wrote_decision = false;
        for coin in coins {
            let (Some(market), Some(&asset)) = (markets.get(coin), assets.get(coin)) else { continue };
            let want = targets.get(coin).copied().unwrap_or(0.0) * held.value / market.mark;
            let have = held.size.get(coin).copied().unwrap_or(0.0);
            let closing = want == 0.0 && have != 0.0;
            let delta = if closing { -have } else { want - have };
            if !closing && (delta * market.mark).abs() < MIN_ORDER {
                continue;
            }
            let size = crate::fill::to_step(delta.abs(), market.size_decimals);
            if size == 0.0 {
                continue;
            }
            if !wrote_decision {
                let note = format!("Account worth ${:.2}; moving toward the managers' targets.", held.value);
                self.ledger.push(now, Event::Decision { targets: targets.clone(), note }).map_err(|e| e.to_string())?;
                wrote_decision = true;
            }
            let buy = delta > 0.0;
            let limit = price(market.mark * if buy { 1.0 + REACH } else { 1.0 - REACH }, market.size_decimals);
            let answer = self.exchange.order(asset, buy, limit, size, closing)?;
            let filled = &answer["response"]["data"]["statuses"][0]["filled"];
            if filled.is_object() {
                let qty = num(&filled["totalSz"]) * if buy { 1.0 } else { -1.0 };
                let px = num(&filled["avgPx"]);
                self.ledger.push(now, Event::Fill(Fill { coin: coin.clone(), qty, px, fee: 0.0 })).map_err(|e| e.to_string())?;
            } else {
                eprintln!("real {}: {}", short(coin), answer);
            }
        }
        if wrote_decision {
            let marks = held.size.keys().filter_map(|c| Some((c.clone(), markets.get(c)?.mark))).collect();
            self.ledger.push(now, Event::Mark { marks, equity: held.value }).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn targets_add_up_each_manager_over_its_share() {
        let mut managers = BTreeMap::new();
        managers.insert("a".to_string(), BTreeMap::from([("GOLD".to_string(), 0.5), ("SP".to_string(), 0.5)]));
        managers.insert("b".to_string(), BTreeMap::from([("GOLD".to_string(), 1.0)]));
        let mix = vec![("a".to_string(), 0.6), ("b".to_string(), 0.3)];
        let real = Real {
            owner: String::new(),
            exchange: Exchange::new(concat!("0x01234567890123456789012345678901", "23456789012345678901234567890123"), crate::exchange::TESTNET).unwrap(),
            venue: Venue::new(),
            mix,
            ledger: Ledger::open(&std::env::temp_dir().join(format!("real-{}.jsonl", std::process::id()))).unwrap(),
            dir: std::env::temp_dir(),
        };
        let t = real.targets(&managers);
        assert!((t["GOLD"] - 0.6).abs() < 1e-9);
        assert!((t["SP"] - 0.3).abs() < 1e-9);
    }
}
