//! Play money. Each account starts with credits. It trades a market up or down
//! with a stake and a multiplier, or backs a fund. Every open, close and bust is
//! priced here from the venue's live prices; the browser only shows them.
//!
//! A trade's size is stake times multiplier. Its gain is that size times the
//! move from the entry price, in its direction. It busts when the loss reaches
//! the stake, which is a move of one over the multiplier against it.

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, Mutex, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Json, Router,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{auth::User, live};

/// Credits a new account starts with.
pub const START_CREDITS: f64 = 10_000.0;
/// The fee on opening a trade, as a share of its size.
pub const FEE: f64 = 0.0009;

const INFO_URL: &str = "https://api.hyperliquid.xyz/info";

/// What can be traded: the venue's coin, the name people know, its kind, and its largest multiplier.
const MARKETS: &[(&str, &str, &str, f64)] = &[
    ("xyz:GOLD", "Gold", "commodities", 100.0),
    ("xyz:SILVER", "Silver", "commodities", 100.0),
    ("xyz:CL", "Oil (WTI)", "commodities", 100.0),
    ("xyz:BRENTOIL", "Oil (Brent)", "commodities", 100.0),
    ("xyz:SP500", "S&P 500", "indices", 200.0),
    ("xyz:XYZ100", "Nasdaq 100", "indices", 200.0),
    ("xyz:NVDA", "Nvidia", "stocks", 100.0),
    ("xyz:TSLA", "Tesla", "stocks", 100.0),
    ("xyz:AAPL", "Apple", "stocks", 100.0),
    ("xyz:MSFT", "Microsoft", "stocks", 100.0),
    ("xyz:GOOGL", "Alphabet", "stocks", 100.0),
    ("xyz:AMZN", "Amazon", "stocks", 100.0),
    ("xyz:META", "Meta", "stocks", 100.0),
    ("xyz:AMD", "AMD", "stocks", 100.0),
    ("xyz:MU", "Micron", "stocks", 100.0),
    ("xyz:PLTR", "Palantir", "stocks", 100.0),
    ("xyz:COIN", "Coinbase", "stocks", 100.0),
    ("xyz:MSTR", "Strategy", "stocks", 100.0),
    ("xyz:HOOD", "Robinhood", "stocks", 100.0),
    ("xyz:SPCX", "SpaceX", "stocks", 50.0),
    ("BTC", "Bitcoin", "crypto", 200.0),
    ("ETH", "Ethereum", "crypto", 200.0),
    ("SOL", "Solana", "crypto", 100.0),
    ("HYPE", "Hyperliquid", "crypto", 100.0),
];

fn market(id: &str) -> Option<&'static (&'static str, &'static str, &'static str, f64)> {
    MARKETS.iter().find(|m| m.0 == id)
}

/// The symbol people see: the coin without its venue prefix.
fn symbol(id: &str) -> &str {
    id.rsplit(':').next().unwrap_or(id)
}

fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as i64
}

/// Live prices: mids every two seconds, and yesterday's price twice a minute.
#[derive(Default)]
pub struct Prices {
    mid: RwLock<HashMap<String, f64>>,
    prev_day: RwLock<HashMap<String, f64>>,
    updated: RwLock<i64>,
}

impl Prices {
    pub fn get(&self, id: &str) -> Option<f64> {
        self.mid.read().unwrap().get(id).copied().filter(|p| *p > 0.0)
    }
}

async fn ask(http: &reqwest::Client, body: Value) -> Result<Value, String> {
    let response = http.post(INFO_URL).json(&body).send().await.map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(response.status().to_string());
    }
    response.json().await.map_err(|e| e.to_string())
}

fn num(v: &Value) -> f64 {
    v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_f64()).unwrap_or(0.0)
}

async fn refresh_mids(http: &reqwest::Client, prices: &Prices) -> Result<(), String> {
    let mut mids = HashMap::new();
    for body in [json!({ "type": "allMids" }), json!({ "type": "allMids", "dex": "xyz" })] {
        let answer = ask(http, body).await?;
        for (coin, px) in answer.as_object().ok_or("no mids")? {
            if market(coin).is_some() {
                mids.insert(coin.clone(), num(px));
            }
        }
    }
    prices.mid.write().unwrap().extend(mids);
    *prices.updated.write().unwrap() = now_ms();
    Ok(())
}

async fn refresh_prev_day(http: &reqwest::Client, prices: &Prices) -> Result<(), String> {
    let mut prev = HashMap::new();
    for body in [json!({ "type": "metaAndAssetCtxs" }), json!({ "type": "metaAndAssetCtxs", "dex": "xyz" })] {
        let answer = ask(http, body).await?;
        let universe = answer[0]["universe"].as_array().ok_or("no universe")?;
        let contexts = answer[1].as_array().ok_or("no contexts")?;
        for (meta, ctx) in universe.iter().zip(contexts) {
            let coin = meta["name"].as_str().unwrap_or_default();
            if market(coin).is_some() {
                prev.insert(coin.to_string(), num(&ctx["prevDayPx"]));
            }
        }
    }
    *prices.prev_day.write().unwrap() = prev;
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Close {
    pub price: f64,
    pub at: i64,
    pub pnl: f64,
    /// "closed", "bust", "take_profit" or "stop_loss".
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trade {
    pub id: String,
    pub market: String,
    /// 1 for up, -1 for down.
    pub side: f64,
    pub stake: f64,
    pub multiplier: f64,
    pub entry: f64,
    pub fee: f64,
    pub opened: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub take_profit: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_loss: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close: Option<Close>,
}

impl Trade {
    /// The gain at a price. Never a loss beyond the stake.
    pub fn pnl(&self, price: f64) -> f64 {
        (self.stake * self.multiplier * self.side * (price / self.entry - 1.0)).max(-self.stake)
    }

    /// The price at which the loss reaches the stake.
    pub fn bust_price(&self) -> f64 {
        self.entry * (1.0 - self.side / self.multiplier)
    }

    /// Why this trade ends at a price, if it does.
    fn ends_at(&self, price: f64) -> Option<(&'static str, f64)> {
        let past = |level: f64| if self.side > 0.0 { price <= level } else { price >= level };
        let beyond = |level: f64| if self.side > 0.0 { price >= level } else { price <= level };
        if past(self.bust_price()) {
            return Some(("bust", self.bust_price()));
        }
        if let Some(level) = self.stop_loss.filter(|l| past(*l)) {
            return Some(("stop_loss", level));
        }
        if let Some(level) = self.take_profit.filter(|l| beyond(*l)) {
            return Some(("take_profit", level));
        }
        None
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Account {
    /// The fund's name. Empty until it is started.
    #[serde(default)]
    pub name: String,
    /// What the fund started with, which its change is measured from.
    #[serde(default = "start_credits")]
    pub start: f64,
    #[serde(default)]
    pub temperament: String,
    /// The fund's value over time, every five minutes: `[t, value]`.
    #[serde(default)]
    pub curve: Vec<(i64, f64)>,
    pub balance: f64,
    pub trades: Vec<Trade>,
    /// Fund id to units held. A unit is worth the fund's equity over its starting capital.
    #[serde(default)]
    pub backed: BTreeMap<String, Backing>,
    pub created: i64,
}

fn start_credits() -> f64 {
    START_CREDITS
}

/// The managers each temperament hires, and the share of the fund each runs. The
/// tenth left over stays as cash for the owner's own calls.
const TEMPERAMENTS: &[(&str, &[(&str, f64)])] = &[
    ("steady", &[("all-weather", 0.6), ("trend-compounder", 0.3)]),
    ("balanced", &[("all-weather", 0.35), ("magnificent", 0.25), ("open-drive", 0.3)]),
    ("bold", &[("open-drive", 0.45), ("oil-breakout", 0.2), ("magnificent", 0.25)]),
];

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Backing {
    pub units: f64,
    /// Credits put in, less credits taken out.
    pub cost: f64,
}

#[derive(Default, Serialize, Deserialize)]
struct Book {
    accounts: BTreeMap<String, Account>,
    next: u64,
}

#[derive(Clone)]
pub struct Play {
    file: Option<PathBuf>,
    book: Arc<Mutex<Book>>,
    pub prices: Arc<Prices>,
}

impl Play {
    /// Accounts are kept in `PLAY_DIR`, or only in memory without it.
    pub fn from_env() -> Play {
        let file = std::env::var_os("PLAY_DIR").map(|dir| PathBuf::from(dir).join("accounts.json"));
        let book = file.as_ref().and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Play { file, book: Arc::new(Mutex::new(book)), prices: Arc::new(Prices::default()) }
    }

    fn save(&self, book: &Book) {
        let Some(file) = &self.file else { return };
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = file.with_extension("tmp");
        if std::fs::write(&tmp, serde_json::to_vec(book).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(tmp, file);
        }
    }

    /// Prices kept fresh, and trades closed when they bust or reach their levels.
    pub fn start(&self) {
        let play = self.clone();
        tokio::spawn(async move {
            let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().expect("a client builds");
            let mut tick = 0u64;
            loop {
                if tick.is_multiple_of(15) {
                    if let Err(error) = refresh_prev_day(&http, &play.prices).await {
                        eprintln!("prices: {error}");
                    }
                }
                match refresh_mids(&http, &play.prices).await {
                    Ok(()) => play.settle(),
                    Err(error) => eprintln!("prices: {error}"),
                }
                if tick.is_multiple_of(150) {
                    if let Ok(funds) = tokio::task::spawn_blocking(fund_prices).await {
                        play.record(&funds);
                    }
                }
                tick += 1;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        });
    }

    /// Notes every fund's value now, for its chart.
    fn record(&self, funds: &BTreeMap<String, (f64, live::Card)>) {
        let mut book = self.book.lock().unwrap();
        let at = now_ms();
        for account in book.accounts.values_mut().filter(|a| !a.name.is_empty()) {
            let value = account_view(self, account, funds)["equity"].as_f64().unwrap_or(account.balance);
            account.curve.push((at, value));
            if account.curve.len() > 4000 {
                account.curve.drain(..account.curve.len() - 4000);
            }
        }
        self.save(&book);
    }

    /// Closes every open trade that has busted or reached a level.
    fn settle(&self) {
        let mut book = self.book.lock().unwrap();
        let mut changed = false;
        let at = now_ms();
        for account in book.accounts.values_mut() {
            let mut paid = 0.0;
            for trade in account.trades.iter_mut().filter(|t| t.close.is_none()) {
                let Some(price) = self.prices.get(&trade.market) else { continue };
                if let Some((reason, level)) = trade.ends_at(price) {
                    let pnl = trade.pnl(level);
                    paid += trade.stake + pnl;
                    trade.close = Some(Close { price: level, at, pnl, reason: reason.to_string() });
                    changed = true;
                }
            }
            account.balance += paid;
        }
        if changed {
            self.save(&book);
        }
    }
}

fn who(user: Option<Extension<User>>) -> String {
    user.map(|Extension(User(email))| email).unwrap_or_else(|| "local".to_string())
}

fn refuse(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": { "code": "refused", "message": message } }))).into_response()
}

/// Each fund's unit price now: its equity over its starting capital, and its card.
fn fund_prices() -> BTreeMap<String, (f64, live::Card)> {
    live::board(&live::dir()).into_iter().map(|card| ((card.id.clone()), (card.equity / card.capital, card))).collect()
}

async fn markets(State(play): State<Play>) -> Json<Value> {
    let prev = play.prices.prev_day.read().unwrap().clone();
    let list: Vec<Value> = MARKETS
        .iter()
        .map(|(id, name, kind, max)| {
            let price = play.prices.get(id);
            let change = match (price, prev.get(*id)) {
                (Some(p), Some(&q)) if q > 0.0 => Some(p / q - 1.0),
                _ => None,
            };
            json!({ "id": id, "symbol": symbol(id), "name": name, "kind": kind, "price": price, "change_day": change, "max_multiplier": max })
        })
        .collect();
    let funds = tokio::task::spawn_blocking(|| live::board(&live::dir())).await.unwrap_or_default();
    Json(json!({ "markets": list, "funds": funds, "fee": FEE }))
}

fn account_view(play: &Play, account: &Account, funds: &BTreeMap<String, (f64, live::Card)>) -> Value {
    let open: Vec<Value> = account
        .trades
        .iter()
        .filter(|t| t.close.is_none())
        .rev()
        .map(|t| {
            let price = play.prices.get(&t.market);
            json!({ "trade": t, "symbol": symbol(&t.market), "price": price, "pnl": price.map(|p| t.pnl(p)), "bust": t.bust_price() })
        })
        .collect();
    let history: Vec<Value> = account
        .trades
        .iter()
        .filter(|t| t.close.is_some())
        .rev()
        .take(50)
        .map(|t| json!({ "trade": t, "symbol": symbol(&t.market) }))
        .collect();
    let backed: Vec<Value> = account
        .backed
        .iter()
        .filter(|(_, b)| b.units > 0.0)
        .map(|(id, b)| {
            let (unit, name) = funds.get(id).map(|(u, c)| (*u, c.name.clone())).unwrap_or((1.0, id.clone()));
            json!({ "fund": id, "name": name, "units": b.units, "cost": b.cost, "value": b.units * unit })
        })
        .collect();
    let in_trades: f64 = open.iter().map(|o| o["trade"]["stake"].as_f64().unwrap_or(0.0) + o["pnl"].as_f64().unwrap_or(0.0)).sum();
    let in_funds: f64 = backed.iter().map(|b| b["value"].as_f64().unwrap_or(0.0)).sum();
    let equity = account.balance + in_trades + in_funds;
    let mut curve = account.curve.clone();
    curve.push((now_ms(), equity));
    json!({
        "fund": account.name,
        "temperament": account.temperament,
        "curve": curve,
        "balance": account.balance,
        "equity": equity,
        "start": account.start,
        "open": open,
        "history": history,
        "backed": backed,
    })
}

fn with_account<T>(play: &Play, email: &str, f: impl FnOnce(&mut Account, &mut u64) -> T) -> T {
    let mut book = play.book.lock().unwrap();
    let Book { accounts, next } = &mut *book;
    let account = accounts.entry(email.to_string()).or_insert_with(|| Account { balance: 0.0, start: 0.0, created: now_ms(), ..Account::default() });
    let out = f(account, next);
    play.save(&book);
    out
}

async fn me(State(play): State<Play>, user: Option<Extension<User>>) -> Json<Value> {
    let email = who(user);
    let funds = tokio::task::spawn_blocking(fund_prices).await.unwrap_or_default();
    let view = with_account(&play, &email, |account, _| account_view(&play, account, &funds));
    let mut view = view;
    view["name"] = json!(email.split('@').next().unwrap_or("you"));
    Json(view)
}

#[derive(Deserialize)]
struct Open {
    market: String,
    /// "up" or "down".
    side: String,
    stake: f64,
    multiplier: f64,
    take_profit: Option<f64>,
    stop_loss: Option<f64>,
}

async fn open_trade(State(play): State<Play>, user: Option<Extension<User>>, Json(order): Json<Open>) -> Response {
    let Some(&(id, _, _, max)) = market(&order.market) else { return refuse(StatusCode::NOT_FOUND, "no such market") };
    let side = match order.side.as_str() {
        "up" => 1.0,
        "down" => -1.0,
        _ => return refuse(StatusCode::UNPROCESSABLE_ENTITY, "side is up or down"),
    };
    if !(order.stake.is_finite() && order.stake >= 1.0) {
        return refuse(StatusCode::UNPROCESSABLE_ENTITY, "stake at least 1 credit");
    }
    if !(order.multiplier.is_finite() && order.multiplier >= 1.0 && order.multiplier <= max) {
        return refuse(StatusCode::UNPROCESSABLE_ENTITY, &format!("multiplier from 1 to {max}"));
    }
    let Some(entry) = play.prices.get(id) else { return refuse(StatusCode::SERVICE_UNAVAILABLE, "no live price yet") };
    let fee = order.stake * order.multiplier * FEE;
    let email = who(user);
    let funds = tokio::task::spawn_blocking(fund_prices).await.unwrap_or_default();
    with_account(&play, &email, |account, next| {
        if account.balance < order.stake + fee {
            return refuse(StatusCode::UNPROCESSABLE_ENTITY, "not enough credits");
        }
        *next += 1;
        let trade = Trade {
            id: format!("t{next}"),
            market: id.to_string(),
            side,
            stake: order.stake,
            multiplier: order.multiplier,
            entry,
            fee,
            opened: now_ms(),
            take_profit: order.take_profit.filter(|v| v.is_finite() && *v > 0.0),
            stop_loss: order.stop_loss.filter(|v| v.is_finite() && *v > 0.0),
            close: None,
        };
        account.balance -= order.stake + fee;
        account.trades.push(trade);
        Json(account_view(&play, account, &funds)).into_response()
    })
}

async fn close_trade(State(play): State<Play>, user: Option<Extension<User>>, Path(trade_id): Path<String>) -> Response {
    let email = who(user);
    let funds = tokio::task::spawn_blocking(fund_prices).await.unwrap_or_default();
    with_account(&play, &email, |account, _| {
        let Some(trade) = account.trades.iter_mut().find(|t| t.id == trade_id && t.close.is_none()) else {
            return refuse(StatusCode::NOT_FOUND, "no such open trade");
        };
        let Some(price) = play.prices.get(&trade.market) else { return refuse(StatusCode::SERVICE_UNAVAILABLE, "no live price yet") };
        let pnl = trade.pnl(price);
        trade.close = Some(Close { price, at: now_ms(), pnl, reason: "closed".into() });
        account.balance += trade.stake + pnl;
        Json(account_view(&play, account, &funds)).into_response()
    })
}

#[derive(Deserialize)]
struct Back {
    fund: String,
    /// Credits to put in. Left out or zero takes everything out.
    #[serde(default)]
    amount: f64,
}

async fn back_fund(State(play): State<Play>, user: Option<Extension<User>>, Json(order): Json<Back>) -> Response {
    let email = who(user);
    let funds = tokio::task::spawn_blocking(fund_prices).await.unwrap_or_default();
    let Some((unit, _)) = funds.get(&order.fund) else { return refuse(StatusCode::NOT_FOUND, "no such fund") };
    let unit = *unit;
    if unit <= 0.0 {
        return refuse(StatusCode::UNPROCESSABLE_ENTITY, "this fund has no value left");
    }
    with_account(&play, &email, |account, _| {
        if order.amount > 0.0 {
            if account.balance < order.amount {
                return refuse(StatusCode::UNPROCESSABLE_ENTITY, "not enough credits");
            }
            account.balance -= order.amount;
            let backing = account.backed.entry(order.fund.clone()).or_default();
            backing.units += order.amount / unit;
            backing.cost += order.amount;
        } else if let Some(backing) = account.backed.remove(&order.fund) {
            account.balance += backing.units * unit;
        }
        Json(account_view(&play, account, &funds)).into_response()
    })
}

/// Everyone ranked by what their credits are worth now.
async fn leaderboard(State(play): State<Play>) -> Json<Value> {
    let funds = tokio::task::spawn_blocking(fund_prices).await.unwrap_or_default();
    let book = play.book.lock().unwrap();
    let mut rows: Vec<Value> = book
        .accounts
        .iter()
        .map(|(email, account)| {
            let view = account_view(&play, account, &funds);
            json!({ "name": email.split('@').next().unwrap_or("player"), "equity": view["equity"], "trades": account.trades.len() })
        })
        .collect();
    rows.sort_by(|a, b| b["equity"].as_f64().unwrap_or(0.0).total_cmp(&a["equity"].as_f64().unwrap_or(0.0)));
    Json(json!({ "players": rows, "start": START_CREDITS }))
}

#[derive(Deserialize)]
struct Start {
    name: String,
    amount: f64,
    temperament: String,
}

/// Starts the fund afresh: the amount, and the managers its temperament hires.
async fn start_fund(State(play): State<Play>, user: Option<Extension<User>>, Json(order): Json<Start>) -> Response {
    let Some((_, mix)) = TEMPERAMENTS.iter().find(|(name, _)| *name == order.temperament) else {
        return refuse(StatusCode::UNPROCESSABLE_ENTITY, "temperament is steady, balanced or bold");
    };
    if !(order.amount.is_finite() && order.amount >= 10.0) {
        return refuse(StatusCode::UNPROCESSABLE_ENTITY, "start with at least $10");
    }
    let email = who(user);
    let funds = tokio::task::spawn_blocking(fund_prices).await.unwrap_or_default();
    let name = if order.name.trim().is_empty() { "My Fund".to_string() } else { order.name.trim().chars().take(40).collect() };
    with_account(&play, &email, |account, _| {
        let at = now_ms();
        *account = Account { name, start: order.amount, temperament: order.temperament.clone(), curve: vec![(at, order.amount)], balance: order.amount, created: at, ..Account::default() };
        for (fund, share) in mix.iter() {
            let Some((unit, _)) = funds.get(*fund) else { continue };
            let amount = order.amount * share;
            account.balance -= amount;
            let backing = account.backed.entry(fund.to_string()).or_default();
            backing.units += amount / unit;
            backing.cost += amount;
        }
        Json(account_view(&play, account, &funds)).into_response()
    })
}

#[derive(Deserialize)]
struct Add {
    amount: f64,
}

/// Adds practice money to the fund. Real money arrives by deposit instead.
async fn add_money(State(play): State<Play>, user: Option<Extension<User>>, Json(order): Json<Add>) -> Response {
    if !(order.amount.is_finite() && order.amount > 0.0) {
        return refuse(StatusCode::UNPROCESSABLE_ENTITY, "add more than nothing");
    }
    let email = who(user);
    let funds = tokio::task::spawn_blocking(fund_prices).await.unwrap_or_default();
    with_account(&play, &email, |account, _| {
        account.balance += order.amount;
        account.start += order.amount;
        Json(account_view(&play, account, &funds)).into_response()
    })
}

pub fn routes(play: Play) -> Router {
    Router::new()
        .route("/fund/add", post(add_money))
        .route("/fund/start", post(start_fund))
        .route("/markets", get(markets))
        .route("/me", get(me))
        .route("/trades", post(open_trade))
        .route("/trades/{id}/close", post(close_trade))
        .route("/back", post(back_fund))
        .route("/leaderboard", get(leaderboard))
        .with_state(play)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trade(side: f64, multiplier: f64) -> Trade {
        Trade {
            id: "t1".into(),
            market: "BTC".into(),
            side,
            stake: 100.0,
            multiplier,
            entry: 1000.0,
            fee: 0.0,
            opened: 0,
            take_profit: None,
            stop_loss: None,
            close: None,
        }
    }

    #[test]
    fn a_trade_gains_its_size_times_the_move_and_busts_at_one_over_the_multiplier() {
        let up = trade(1.0, 10.0);
        assert!((up.pnl(1010.0) - 10.0).abs() < 1e-9);
        assert!((up.bust_price() - 900.0).abs() < 1e-9);
        assert_eq!(up.pnl(800.0), -100.0);
        assert_eq!(up.ends_at(899.0).map(|e| e.0), Some("bust"));
        let down = trade(-1.0, 50.0);
        assert!((down.pnl(990.0) - 50.0).abs() < 1e-9);
        assert!((down.bust_price() - 1020.0).abs() < 1e-9);
        assert_eq!(down.ends_at(1000.0), None);
    }

    #[test]
    fn levels_close_a_trade_on_the_right_side() {
        let mut up = trade(1.0, 2.0);
        up.take_profit = Some(1100.0);
        up.stop_loss = Some(950.0);
        assert_eq!(up.ends_at(1100.0).map(|e| e.0), Some("take_profit"));
        assert_eq!(up.ends_at(949.0).map(|e| e.0), Some("stop_loss"));
        assert_eq!(up.ends_at(1000.0), None);
    }
}
