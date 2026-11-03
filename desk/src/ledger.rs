//! A fund's ledger: every decision, fill, funding payment and mark, in order,
//! each entry holding the hash of the one before. Change or drop any entry and
//! every hash after it breaks. The account and the equity curve are what the
//! ledger replays to, so the track record is the ledger and nothing else.

use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    account::{Account, Fill},
    fund::FundSpec,
    stats::Point,
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// The fund starts, with the exact spec it runs.
    Open {
        fund: FundSpec,
    },
    Decision {
        targets: BTreeMap<String, f64>,
        note: String,
    },
    Fill(Fill),
    Funding {
        coin: String,
        rate: f64,
        px: f64,
        paid: f64,
    },
    /// The fund's marks and the equity they give.
    Mark {
        marks: BTreeMap<String, f64>,
        equity: f64,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry {
    pub seq: u64,
    pub t: i64,
    pub event: Event,
    pub prev: String,
    pub hash: String,
}

fn genesis() -> String {
    "0".repeat(64)
}

fn digest(seq: u64, t: i64, event: &Event, prev: &str) -> String {
    let body = serde_json::to_string(&(seq, t, event, prev)).expect("events serialise");
    hex::encode(Sha256::digest(body.as_bytes()))
}

/// Checks every hash. Returns the first entry that does not hold.
pub fn verify(entries: &[Entry]) -> Result<(), u64> {
    let mut prev = genesis();
    for (i, entry) in entries.iter().enumerate() {
        if entry.seq != i as u64 || entry.prev != prev || entry.hash != digest(entry.seq, entry.t, &entry.event, &entry.prev) {
            return Err(i as u64);
        }
        prev = entry.hash.clone();
    }
    Ok(())
}

/// What a ledger replays to.
#[derive(Clone, Debug, Default)]
pub struct Replay {
    pub fund: Option<FundSpec>,
    pub account: Account,
    pub marks: BTreeMap<String, f64>,
    pub curve: Vec<Point>,
    pub last_decision: Option<(i64, String, BTreeMap<String, f64>)>,
    pub fills: usize,
}

pub fn replay(entries: &[Entry]) -> Replay {
    let mut out = Replay::default();
    for entry in entries {
        match &entry.event {
            Event::Open { fund } => {
                out.account = Account::new(fund.capital);
                out.fund = Some(fund.clone());
                out.curve.push(Point { t: entry.t, equity: fund.capital });
            }
            Event::Decision { targets, note } => out.last_decision = Some((entry.t, note.clone(), targets.clone())),
            Event::Fill(fill) => {
                out.account.apply_fill(fill);
                out.marks.insert(fill.coin.clone(), fill.px);
                out.fills += 1;
            }
            Event::Funding { coin, rate, px, .. } => {
                out.account.apply_funding(coin, *rate, *px);
            }
            Event::Mark { marks, .. } => {
                out.marks.extend(marks.clone());
                out.curve.push(Point { t: entry.t, equity: out.account.equity(&out.marks) });
            }
        }
    }
    out
}

/// A ledger on disk, one entry per line.
pub struct Ledger {
    path: PathBuf,
    pub entries: Vec<Entry>,
}

impl Ledger {
    pub fn open(path: &Path) -> std::io::Result<Self> {
        let entries = if path.exists() { read(path)? } else { Vec::new() };
        if let Err(at) = verify(&entries) {
            return Err(std::io::Error::other(format!("{} breaks at entry {at}", path.display())));
        }
        Ok(Self { path: path.to_path_buf(), entries })
    }

    pub fn push(&mut self, t: i64, event: Event) -> std::io::Result<&Entry> {
        let seq = self.entries.len() as u64;
        let prev = self.entries.last().map(|e| e.hash.clone()).unwrap_or_else(genesis);
        let hash = digest(seq, t, &event, &prev);
        let entry = Entry { seq, t, event, prev, hash };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(file, "{}", serde_json::to_string(&entry)?)?;
        file.sync_data()?;
        self.entries.push(entry);
        Ok(self.entries.last().unwrap())
    }
}

pub fn read(path: &Path) -> std::io::Result<Vec<Entry>> {
    BufReader::new(File::open(path)?)
        .lines()
        .filter(|line| line.as_ref().map(|l| !l.trim().is_empty()).unwrap_or(true))
        .map(|line| serde_json::from_str(&line?).map_err(std::io::Error::other))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fund::catalogue, market::HOUR_MS};

    #[test]
    fn a_ledger_replays_to_its_account_and_any_edit_breaks_it() {
        let dir = std::env::temp_dir().join(format!("desk-ledger-{}", std::process::id()));
        let path = dir.join("ledger.jsonl");
        let _ = fs::remove_file(&path);
        let fund = catalogue().remove(0);
        let mut ledger = Ledger::open(&path).unwrap();
        ledger.push(0, Event::Open { fund: fund.clone() }).unwrap();
        ledger.push(1, Event::Fill(Fill { coin: "X".into(), qty: 1.0, px: 100.0, fee: 0.1 })).unwrap();
        // Prices that print with many digits must read back to the same hash.
        ledger.push(2, Event::Fill(Fill { coin: "Y".into(), qty: 1.0 / 3.0, px: 0.1 + 0.2, fee: 0.0 })).unwrap();
        ledger.push(3, Event::Fill(Fill { coin: "Y".into(), qty: -1.0 / 3.0, px: 0.1 + 0.2, fee: 0.0 })).unwrap();
        ledger.push(HOUR_MS, Event::Funding { coin: "X".into(), rate: 0.001, px: 100.0, paid: 0.1 }).unwrap();
        ledger.push(HOUR_MS, Event::Mark { marks: BTreeMap::from([("X".into(), 110.0)]), equity: 0.0 }).unwrap();

        let again = Ledger::open(&path).unwrap();
        let replay = replay(&again.entries);
        assert!((replay.curve.last().unwrap().equity - (fund.capital + 10.0 - 0.2)).abs() < 1e-9);

        let mut tampered = again.entries.clone();
        if let Event::Fill(fill) = &mut tampered[1].event {
            fill.px = 90.0;
        }
        assert_eq!(verify(&tampered), Err(1));
        let _ = fs::remove_dir_all(dir);
    }
}
