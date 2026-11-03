//! Funds running live on paper, read from the desk's folder: each fund's
//! ledger replayed to its standing, and its test beside it. Nothing here is
//! kept: every answer is read from the ledgers as they are now.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use compounder_desk::{
    backtest::Tested,
    fund::{FundSpec, Speed},
    ledger::{self, replay, Event},
    stats::{figures, Point},
    strategy::{short, Spec},
};
use serde::Serialize;

/// The desk's folder: `DESK_DIR`, or where the desk keeps it by default.
pub fn dir() -> PathBuf {
    std::env::var_os("DESK_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".local/share/compounder/desk"))
}

#[derive(Serialize)]
pub struct Holding {
    pub market: String,
    pub weight: f64,
}

#[derive(Serialize)]
pub struct TestSummary {
    pub start: i64,
    pub end: i64,
    pub total_return: f64,
    pub annual_return: f64,
    pub sharpe: f64,
    pub max_drawdown: f64,
}

/// A fund as the board shows it.
#[derive(Serialize)]
pub struct Card {
    pub id: String,
    pub name: String,
    pub speed: Speed,
    pub thesis: String,
    pub rules: String,
    /// The model that runs it, for a fund a model runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub markets: Vec<String>,
    pub capital: f64,
    pub opened: i64,
    pub updated: i64,
    pub equity: f64,
    /// Since it opened, as a fraction.
    pub change: f64,
    /// Over the last day, as a fraction. Null before it has run a day.
    pub change_day: Option<f64>,
    pub worst_drop: f64,
    pub fills: usize,
    pub fees: f64,
    pub funding: f64,
    /// Equity over time: `[t, equity]`, at most about 120 points.
    pub curve: Vec<(i64, f64)>,
    pub note: String,
    pub decided: Option<i64>,
    pub holdings: Vec<Holding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub test: Option<TestSummary>,
    /// Whether every hash in the ledger holds.
    pub verified: bool,
    pub entries: usize,
    /// The hash of the last entry: the whole history in one line.
    pub head: String,
}

#[derive(Serialize)]
pub struct Decision {
    pub t: i64,
    pub note: String,
    pub targets: BTreeMap<String, f64>,
}

#[derive(Serialize)]
pub struct Trade {
    pub t: i64,
    pub market: String,
    pub qty: f64,
    pub px: f64,
    pub fee: f64,
}

/// A fund's page: its card, what it decided and traded lately, and its whole test.
#[derive(Serialize)]
pub struct Detail {
    #[serde(flatten)]
    pub card: Card,
    pub decisions: Vec<Decision>,
    pub trades: Vec<Trade>,
    pub test_curve: Vec<(i64, f64)>,
}

fn thin(curve: &[Point], n: usize) -> Vec<(i64, f64)> {
    let step = (curve.len() as f64 / n as f64).max(1.0);
    let mut out: Vec<(i64, f64)> = (0..curve.len().min(n)).map(|i| curve[(i as f64 * step) as usize]).map(|p| (p.t, p.equity)).collect();
    if let Some(last) = curve.last() {
        if out.last().map(|p| p.0) != Some(last.t) {
            out.push((last.t, last.equity));
        }
    }
    out
}

fn tests(dir: &Path) -> BTreeMap<String, Tested> {
    fs::read(dir.join("tested.json")).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

fn summary(tested: &Tested) -> TestSummary {
    let f = &tested.figures;
    TestSummary {
        start: f.start,
        end: f.end,
        total_return: f.total_return,
        annual_return: f.annual_return,
        sharpe: f.sharpe,
        max_drawdown: f.max_drawdown,
    }
}

fn card(fund: &FundSpec, entries: &[ledger::Entry], tested: Option<&Tested>) -> Card {
    let state = replay(entries);
    let curve = &state.curve;
    let equity = curve.last().map(|p| p.equity).unwrap_or(fund.capital);
    let updated = entries.last().map(|e| e.t).unwrap_or(0);
    let day_ago = updated - 24 * 3_600_000;
    let change_day = curve.iter().rev().find(|p| p.t <= day_ago).filter(|p| p.equity > 0.0).map(|p| equity / p.equity - 1.0);
    let mut holdings: Vec<Holding> = state
        .account
        .weights(&state.marks)
        .into_iter()
        .map(|(coin, weight)| Holding { market: short(&coin).to_string(), weight })
        .collect();
    holdings.sort_by(|a, b| b.weight.abs().total_cmp(&a.weight.abs()));
    let (decided, note) = match &state.last_decision {
        Some((t, note, _)) => (Some(*t), note.clone()),
        None => (None, String::new()),
    };
    Card {
        id: fund.id.clone(),
        name: fund.name.clone(),
        speed: fund.speed,
        thesis: fund.thesis.clone(),
        rules: fund.rules.clone(),
        model: match &fund.strategy {
            Spec::Agent { model, .. } => Some(model.clone()),
            _ => None,
        },
        markets: fund.coins().iter().map(|c| short(c).to_string()).collect(),
        capital: fund.capital,
        opened: entries.first().map(|e| e.t).unwrap_or(0),
        updated,
        equity,
        change: equity / fund.capital - 1.0,
        change_day,
        worst_drop: figures(curve).max_drawdown,
        fills: state.fills,
        fees: state.account.fees,
        funding: state.account.funding,
        curve: thin(curve, 120),
        note,
        decided,
        holdings,
        test: tested.filter(|t| t.curve.len() > 1).map(summary),
        verified: ledger::verify(entries).is_ok(),
        entries: entries.len(),
        head: entries.last().map(|e| e.hash.clone()).unwrap_or_default(),
    }
}

/// Every fund's ledger, read and replayed. A fund whose ledger cannot be read is left out.
fn ledgers(dir: &Path) -> Vec<(FundSpec, Vec<ledger::Entry>)> {
    let Ok(listing) = fs::read_dir(dir.join("funds")) else {
        return Vec::new();
    };
    let mut out: Vec<(FundSpec, Vec<ledger::Entry>)> = listing
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "jsonl"))
        .filter_map(|e| {
            let entries = ledger::read(&e.path()).ok()?;
            let fund = match &entries.first()?.event {
                Event::Open { fund } => fund.clone(),
                _ => return None,
            };
            Some((fund, entries))
        })
        .collect();
    out.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    out
}

/// The board: every fund, best since it opened first.
pub fn board(dir: &Path) -> Vec<Card> {
    let tested = tests(dir);
    let mut cards: Vec<Card> = ledgers(dir).iter().map(|(fund, entries)| card(fund, entries, tested.get(&fund.id))).collect();
    cards.sort_by(|a, b| b.change.total_cmp(&a.change));
    cards
}

pub fn detail(dir: &Path, id: &str) -> Option<Detail> {
    let path = dir.join("funds").join(format!("{id}.jsonl"));
    if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let entries = ledger::read(&path).ok()?;
    let fund = match &entries.first()?.event {
        Event::Open { fund } => fund.clone(),
        _ => return None,
    };
    let tested = tests(dir);
    let mut decisions = Vec::new();
    let mut trades = Vec::new();
    for entry in entries.iter().rev() {
        match &entry.event {
            Event::Decision { targets, note } if decisions.len() < 30 => decisions.push(Decision {
                t: entry.t,
                note: note.clone(),
                targets: targets.iter().map(|(c, w)| (short(c).to_string(), *w)).collect(),
            }),
            Event::Fill(fill) if trades.len() < 30 => {
                trades.push(Trade { t: entry.t, market: short(&fill.coin).to_string(), qty: fill.qty, px: fill.px, fee: fill.fee })
            }
            _ => {}
        }
    }
    let test_curve = tested.get(id).map(|t| thin(&t.curve, 200)).unwrap_or_default();
    Some(Detail { card: card(&fund, &entries, tested.get(id)), decisions, trades, test_curve })
}
