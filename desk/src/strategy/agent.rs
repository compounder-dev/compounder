//! A fund run by a model. Each bar it is shown each market's price, recent
//! moves, volatility, funding and what the fund holds, and answers with target
//! shares and a line on why. The model is reached through any chat completions
//! endpoint: `DESK_MODEL_URL`, with the key in the file `DESK_MODEL_KEY_FILE`.

use std::{collections::BTreeMap, time::Duration};

use serde_json::{json, Value};

use super::{short, Decision, Strategy};
use crate::market::{returns, stdev, Interval, View, YEAR_MS};

const DEFAULT_URL: &str = "https://api.allocate.network/v1/chat/completions";

pub struct Agent {
    coins: Vec<String>,
    model: String,
    mandate: String,
    max_gross: f64,
    interval: Interval,
    http: reqwest::blocking::Client,
}

impl Agent {
    pub fn new(coins: Vec<String>, model: String, mandate: String, max_gross: f64, interval: Interval) -> Self {
        let http = reqwest::blocking::Client::builder().timeout(Duration::from_secs(120)).build().expect("a client builds");
        Self { coins, model, mandate, max_gross, interval, http }
    }

    fn brief(&self, view: &View) -> String {
        let per_hour = (crate::market::HOUR_MS / self.interval.ms()).max(1) as usize;
        let bars_in = |hours: usize| (hours * per_hour).max(1);
        let mut lines = vec!["market | price | 1h | 24h | 7d | vol (yearly) | funding (yearly) | held".to_string()];
        for coin in &self.coins {
            let closes = view.closes(coin);
            let Some(&last) = closes.last() else { continue };
            let change = |n: usize| {
                closes.len().checked_sub(n + 1).map(|i| format!("{:+.2}%", (last / closes[i] - 1.0) * 100.0)).unwrap_or("-".into())
            };
            let vol = stdev(&returns(&closes)) * self.interval.per_year().sqrt();
            let funding = view.funding.get(coin).map(|r| format!("{:+.1}%", r * YEAR_MS / crate::market::HOUR_MS as f64 * 100.0));
            lines.push(format!(
                "{} | {last} | {} | {} | {} | {:.0}% | {} | {:+.2}",
                short(coin),
                if per_hour == 1 { change(1) } else { change(bars_in(1)) },
                change(bars_in(24)),
                change(bars_in(24 * 7)),
                vol * 100.0,
                funding.unwrap_or("-".into()),
                view.weights.get(coin).copied().unwrap_or(0.0),
            ));
        }
        lines.join("\n")
    }

    fn ask(&self, view: &View) -> Result<Decision, String> {
        let url = std::env::var("DESK_MODEL_URL").unwrap_or_else(|_| DEFAULT_URL.into());
        let key_file = std::env::var("DESK_MODEL_KEY_FILE").map_err(|_| "DESK_MODEL_KEY_FILE is not set")?;
        let key = std::fs::read_to_string(&key_file).map_err(|e| format!("reading the key file: {e}"))?;
        let key = key.trim();
        let auth = match key.split_once(':') {
            Some((name, value)) if name.eq_ignore_ascii_case("authorization") => value.trim().to_string(),
            _ => format!("Bearer {key}"),
        };
        let markets: Vec<&str> = self.coins.iter().map(|c| short(c)).collect();
        let system = format!(
            "You run a fund that trades perpetual futures around the clock. Your mandate: {}\n\
             Each turn you see the markets and what the fund holds, as signed shares of equity \
             (0.5 is a long half the fund's size, -0.5 a short). Answer with JSON only: \
             {{\"targets\": {{market: share}}, \"note\": \"one plain sentence on why\"}}. \
             Markets: {}. Shares must add up, in absolute value, to at most {}. \
             Every trade pays about 0.1% of what it moves, so change only what you mean to.",
            self.mandate,
            markets.join(", "),
            self.max_gross
        );
        let body = json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": self.brief(view) },
            ],
            "response_format": { "type": "json_object" },
        });
        let response = self.http.post(&url).header("authorization", auth).json(&body).send().map_err(|e| e.to_string())?;
        if !response.status().is_success() {
            return Err(format!("the model answered {}", response.status()));
        }
        let answer: Value = response.json().map_err(|e| e.to_string())?;
        let text = answer["choices"][0]["message"]["content"].as_str().ok_or("no answer")?;
        let start = text.find('{').ok_or("no JSON in the answer")?;
        let end = text.rfind('}').ok_or("no JSON in the answer")?;
        let reply: Value = serde_json::from_str(&text[start..=end]).map_err(|e| e.to_string())?;
        let mut targets = BTreeMap::new();
        for coin in &self.coins {
            let share = reply["targets"][short(coin)].as_f64().or_else(|| reply["targets"][coin.as_str()].as_f64()).unwrap_or(0.0);
            if share.is_finite() && share != 0.0 {
                targets.insert(coin.clone(), share);
            }
        }
        let gross: f64 = targets.values().map(|w| w.abs()).sum();
        if gross > self.max_gross {
            targets.values_mut().for_each(|w| *w *= self.max_gross / gross);
        }
        let note = reply["note"].as_str().unwrap_or("").trim().to_string();
        Ok(Decision { targets, note })
    }
}

impl Strategy for Agent {
    fn decide(&mut self, view: &View) -> Decision {
        if std::env::var_os("DESK_MODEL_KEY_FILE").is_none() {
            return Decision { targets: view.weights.clone(), note: "Paused: the model is switched off.".into() };
        }
        match self.ask(view) {
            Ok(decision) => decision,
            Err(error) => Decision { targets: view.weights.clone(), note: format!("Held: the model could not be reached ({error}).") },
        }
    }
}
