//! The venue's public market data: markets with their marks and funding, bars,
//! order books and funding history. No key is needed to read any of it.

use std::{collections::BTreeMap, thread::sleep, time::Duration};

use serde_json::{json, Value};

use crate::market::{Bar, Book, Interval};

pub const INFO_URL: &str = "https://api.hyperliquid.xyz/info";

/// A market as the venue lists it now.
#[derive(Clone, Debug)]
pub struct Market {
    pub coin: String,
    pub size_decimals: u32,
    pub max_leverage: u32,
    pub mark: f64,
    pub oracle: f64,
    /// The rate for the current hour.
    pub funding: f64,
    pub day_volume: f64,
    pub open_interest: f64,
}

pub struct Venue {
    http: reqwest::blocking::Client,
    url: String,
}

fn num(value: &Value) -> f64 {
    match value {
        Value::String(s) => s.parse().unwrap_or(0.0),
        Value::Number(n) => n.as_f64().unwrap_or(0.0),
        _ => 0.0,
    }
}

impl Venue {
    pub fn new() -> Self {
        let http = reqwest::blocking::Client::builder().timeout(Duration::from_secs(20)).build().expect("a client builds");
        Self { http, url: INFO_URL.to_string() }
    }

    fn post(&self, body: Value) -> Result<Value, String> {
        let mut wait = Duration::from_secs(1);
        let mut last = String::new();
        for _ in 0..8 {
            match self.http.post(&self.url).json(&body).send() {
                Ok(response) if response.status().as_u16() == 429 || response.status().is_server_error() => {
                    last = response.status().to_string();
                }
                Ok(response) if !response.status().is_success() => {
                    return Err(format!("{}: {}", response.status(), response.text().unwrap_or_default()))
                }
                Ok(response) => return response.json().map_err(|e| e.to_string()),
                Err(error) if error.is_timeout() || error.is_connect() => last = error.to_string(),
                Err(error) => return Err(error.to_string()),
            }
            sleep(wait);
            wait = (wait * 2).min(Duration::from_secs(30));
        }
        Err(format!("the venue did not answer ({last}) to {body}"))
    }

    /// Every market of a dex: `None` for the venue's own, or a deployed dex by name.
    pub fn markets(&self, dex: Option<&str>) -> Result<Vec<Market>, String> {
        let body = match dex {
            Some(dex) => json!({ "type": "metaAndAssetCtxs", "dex": dex }),
            None => json!({ "type": "metaAndAssetCtxs" }),
        };
        let answer = self.post(body)?;
        let universe = answer[0]["universe"].as_array().ok_or("no universe")?;
        let contexts = answer[1].as_array().ok_or("no contexts")?;
        Ok(universe
            .iter()
            .zip(contexts)
            .filter(|(meta, _)| !meta["isDelisted"].as_bool().unwrap_or(false))
            .map(|(meta, ctx)| Market {
                coin: meta["name"].as_str().unwrap_or_default().to_string(),
                size_decimals: meta["szDecimals"].as_u64().unwrap_or(0) as u32,
                max_leverage: meta["maxLeverage"].as_u64().unwrap_or(1) as u32,
                mark: num(&ctx["markPx"]),
                oracle: num(&ctx["oraclePx"]),
                funding: num(&ctx["funding"]),
                day_volume: num(&ctx["dayNtlVlm"]),
                open_interest: num(&ctx["openInterest"]),
            })
            .collect())
    }

    /// The markets a list of coins lives on, read from each coin's dex.
    pub fn markets_for(&self, coins: &[String]) -> Result<BTreeMap<String, Market>, String> {
        let mut dexes: Vec<Option<&str>> = coins.iter().map(|c| c.split_once(':').map(|(dex, _)| dex)).collect();
        dexes.sort();
        dexes.dedup();
        let mut out = BTreeMap::new();
        for dex in dexes {
            for market in self.markets(dex)? {
                out.insert(market.coin.clone(), market);
            }
        }
        out.retain(|coin, _| coins.contains(coin));
        Ok(out)
    }

    /// Bars from `start` to `end`, paged. Only bars that have closed by `end` are kept.
    pub fn bars(&self, coin: &str, interval: Interval, start: i64, end: i64) -> Result<Vec<Bar>, String> {
        let mut out: Vec<Bar> = Vec::new();
        let mut from = start;
        loop {
            let answer = self.post(json!({
                "type": "candleSnapshot",
                "req": { "coin": coin, "interval": interval.code(), "startTime": from, "endTime": end }
            }))?;
            let page: Vec<Bar> = answer
                .as_array()
                .ok_or("no bars")?
                .iter()
                .map(|b| Bar {
                    t: b["t"].as_i64().unwrap_or(0),
                    o: num(&b["o"]),
                    h: num(&b["h"]),
                    l: num(&b["l"]),
                    c: num(&b["c"]),
                    v: num(&b["v"]),
                })
                .collect();
            let Some(last) = page.last().map(|b| b.t) else {
                break;
            };
            let more = page.len() >= 500 && last > from;
            let after = out.last().map_or(i64::MIN, |prev| prev.t);
            out.extend(page.into_iter().filter(|b| b.t > after));
            if !more {
                break;
            }
            from = last + interval.ms();
            if from >= end {
                break;
            }
        }
        out.retain(|b| b.t + interval.ms() <= end);
        Ok(out)
    }

    /// An account's margin state on the venue's own markets (`dex` empty) or on a named dex.
    pub fn clearinghouse(&self, user: &str, dex: &str) -> Result<Value, String> {
        self.post(json!({ "type": "clearinghouseState", "user": user, "dex": dex }))
    }

    /// The number an order names each market by: its place in its dex's list, offset
    /// by the dex's own place for a deployed dex.
    pub fn assets(&self, coins: &[String]) -> Result<BTreeMap<String, u32>, String> {
        let dexes = self.post(json!({ "type": "perpDexs" }))?;
        let mut out = BTreeMap::new();
        let mut wanted: Vec<Option<&str>> = coins.iter().map(|c| c.split_once(':').map(|(d, _)| d)).collect();
        wanted.sort();
        wanted.dedup();
        for dex in wanted {
            let base = match dex {
                None => 0,
                Some(name) => {
                    let place = dexes.as_array().and_then(|list| list.iter().position(|d| d["name"].as_str() == Some(name))).ok_or("no such dex")?;
                    100_000 + place as u32 * 10_000
                }
            };
            let meta = match dex {
                None => self.post(json!({ "type": "meta" }))?,
                Some(name) => self.post(json!({ "type": "meta", "dex": name }))?,
            };
            for (i, m) in meta["universe"].as_array().into_iter().flatten().enumerate() {
                let coin = m["name"].as_str().unwrap_or_default().to_string();
                if coins.contains(&coin) {
                    out.insert(coin, base + i as u32);
                }
            }
        }
        Ok(out)
    }

    pub fn book(&self, coin: &str) -> Result<Book, String> {
        let answer = self.post(json!({ "type": "l2Book", "coin": coin }))?;
        let side = |i: usize| -> Vec<(f64, f64)> {
            answer["levels"][i].as_array().map(|l| l.iter().map(|x| (num(&x["px"]), num(&x["sz"]))).collect()).unwrap_or_default()
        };
        Ok(Book { bids: side(0), asks: side(1) })
    }

    /// Hourly funding rates from `start`, paged.
    pub fn funding(&self, coin: &str, start: i64, end: i64) -> Result<Vec<(i64, f64)>, String> {
        let mut out: Vec<(i64, f64)> = Vec::new();
        let mut from = start;
        loop {
            let answer = self.post(json!({ "type": "fundingHistory", "coin": coin, "startTime": from, "endTime": end }))?;
            let page: Vec<(i64, f64)> =
                answer.as_array().ok_or("no funding")?.iter().map(|r| (r["time"].as_i64().unwrap_or(0), num(&r["fundingRate"]))).collect();
            let Some(&(last, _)) = page.last() else { break };
            let after = out.last().map_or(i64::MIN, |prev| prev.0);
            out.extend(page.into_iter().filter(|r| r.0 > after));
            if last <= from || last >= end {
                break;
            }
            from = last + 1;
        }
        Ok(out)
    }
}

impl Default for Venue {
    fn default() -> Self {
        Self::new()
    }
}
