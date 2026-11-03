//! The desk's command line.
//!
//! `compounder-desk test`    tests every fund over the venue's past bars and writes `tested.json`.
//! `compounder-desk run`     runs every fund live against real books, each into its own ledger.
//! `compounder-desk verify`  checks every ledger's hashes and prints what each replays to.
//!
//! Everything is kept under `DESK_DIR`, by default `~/.local/share/compounder/desk`.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    thread::sleep,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use compounder_desk::{
    backtest::{self, History, Tested},
    fill::{orders, to_step, walk, Costs},
    fund::{catalogue, FundSpec},
    ledger::{self, replay, Event, Ledger},
    market::{Interval, View, DAY_MS, HOUR_MS},
    stats::{figures, Point},
    strategy::Strategy,
    venue::Venue,
};

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64
}

fn dir() -> PathBuf {
    std::env::var_os("DESK_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").expect("HOME is set")).join(".local/share/compounder/desk"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("test") => test(args.get(1).map(String::as_str)),
        Some("run") => run(),
        Some("verify") => verify(),
        _ => Err("usage: compounder-desk test [fund] | run | verify".into()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

/// Past bars for a coin, kept for six hours so tests run again in a moment.
fn cached_bars(venue: &Venue, coin: &str, interval: Interval) -> Result<Vec<compounder_desk::market::Bar>, String> {
    let path = dir().join("history").join(format!("{}-{}.json", coin.replace(':', "_"), interval.code()));
    if let Ok(meta) = fs::metadata(&path) {
        if meta.modified().ok().and_then(|m| m.elapsed().ok()).is_some_and(|age| age < Duration::from_secs(6 * 3600)) {
            if let Ok(bars) = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?) {
                return Ok(bars);
            }
        }
    }
    let end = now();
    let bars = venue.bars(coin, interval, end - 5000 * interval.ms(), end)?;
    write_json(&path, &bars)?;
    Ok(bars)
}

fn cached_funding(venue: &Venue, coin: &str, start: i64) -> Result<Vec<(i64, f64)>, String> {
    let path = dir().join("history").join(format!("{}-funding.json", coin.replace(':', "_")));
    if let Ok(meta) = fs::metadata(&path) {
        if meta.modified().ok().and_then(|m| m.elapsed().ok()).is_some_and(|age| age < Duration::from_secs(6 * 3600)) {
            if let Ok(rates) = serde_json::from_slice(&fs::read(&path).map_err(|e| e.to_string())?) {
                return Ok(rates);
            }
        }
    }
    let rates = venue.funding(coin, start, now())?;
    write_json(&path, &rates)?;
    Ok(rates)
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    fs::create_dir_all(path.parent().unwrap()).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, serde_json::to_vec(value).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// At most `n` points of a curve, evenly spread, keeping the last.
fn thin(curve: &[Point], n: usize) -> Vec<Point> {
    if curve.len() <= n {
        return curve.to_vec();
    }
    let step = curve.len() as f64 / n as f64;
    let mut out: Vec<Point> = (0..n).map(|i| curve[(i as f64 * step) as usize]).collect();
    out.push(*curve.last().unwrap());
    out
}

fn test(only: Option<&str>) -> Result<(), String> {
    let venue = Venue::new();
    let costs = Costs::default();
    let path = dir().join("tested.json");
    let mut all: BTreeMap<String, Tested> = fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default();
    println!(
        "{:<22} {:>10} {:>8} {:>8} {:>7} {:>8} {:>9} {:>7}",
        "fund", "from", "return", "yearly", "sharpe", "drawdown", "turnover", "fills"
    );
    for fund in catalogue() {
        if only.is_some_and(|id| id != fund.id) || !fund.strategy.testable() {
            continue;
        }
        let mut history = History::default();
        for coin in fund.coins() {
            let bars = cached_bars(&venue, &coin, fund.interval)?;
            let start = bars.first().map(|b| b.t).unwrap_or(0);
            history.funding.insert(coin.clone(), cached_funding(&venue, &coin, start)?);
            history.bars.insert(coin, bars);
        }
        let mut tested = backtest::run(&fund, &history, &costs);
        if tested.curve.len() < 2 {
            println!("{:<22} not enough history yet", fund.id);
            all.remove(&fund.id);
            continue;
        }
        let f = &tested.figures;
        println!(
            "{:<22} {:>10} {:>7.1}% {:>7.1}% {:>7.2} {:>7.1}% {:>8.0}x {:>7}",
            fund.id,
            day(f.start),
            f.total_return * 100.0,
            f.annual_return * 100.0,
            f.sharpe,
            f.max_drawdown * 100.0,
            tested.turnover,
            tested.fills
        );
        tested.curve = thin(&tested.curve, 400);
        all.insert(fund.id.clone(), tested);
    }
    write_json(&path, &all)
}

fn day(t: i64) -> String {
    let days = t.div_euclid(DAY_MS);
    // Civil date from days since 1970 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y}-{m:02}-{d:02}")
}

/// A fund running live: its spec, strategy and ledger.
struct Live {
    fund: FundSpec,
    strategy: Box<dyn Strategy>,
    ledger: Ledger,
}

impl Live {
    fn last_bar_decided(&self) -> Option<i64> {
        self.ledger.entries.iter().rev().find(|e| matches!(e.event, Event::Decision { .. })).map(|e| self.fund.interval.floor(e.t))
    }

    fn last_mark_hour(&self) -> Option<i64> {
        self.ledger.entries.iter().rev().find(|e| matches!(e.event, Event::Mark { .. })).map(|e| Interval::H1.floor(e.t))
    }

    fn opened(&self) -> i64 {
        self.ledger.entries.first().map(|e| e.t).unwrap_or(0)
    }

    /// The last funding time already paid on a coin.
    fn funded_to(&self, coin: &str) -> i64 {
        self.ledger
            .entries
            .iter()
            .rev()
            .find_map(|e| match &e.event {
                Event::Funding { coin: c, .. } if c == coin => Some(e.t),
                _ => None,
            })
            .unwrap_or_else(|| self.opened())
    }
}

fn run() -> Result<(), String> {
    let venue = Venue::new();
    let costs = Costs::default();
    let mut funds: Vec<Live> = Vec::new();
    for fund in catalogue() {
        let path = dir().join("funds").join(format!("{}.jsonl", fund.id));
        let mut ledger = Ledger::open(&path).map_err(|e| e.to_string())?;
        if ledger.entries.is_empty() {
            ledger.push(now(), Event::Open { fund: fund.clone() }).map_err(|e| e.to_string())?;
        }
        let strategy = fund.strategy.build(fund.interval);
        funds.push(Live { fund, strategy, ledger });
    }
    eprintln!("running {} funds into {}", funds.len(), dir().display());
    loop {
        if let Err(error) = tick(&venue, &costs, &mut funds) {
            eprintln!("{error}");
        }
        // Wake five seconds past each minute, once the minute's bars have closed.
        let t = now();
        let wake = (t / 60_000 + 1) * 60_000 + 5_000;
        sleep(Duration::from_millis((wake - t) as u64));
    }
}

/// Bars fetched once a minute and shared by every fund that reads them.
#[derive(Default)]
struct Bars(BTreeMap<(String, Interval), Vec<compounder_desk::market::Bar>>);

impl Bars {
    fn get(
        &mut self,
        venue: &Venue,
        coin: &str,
        interval: Interval,
        bars: usize,
        t: i64,
    ) -> Result<&[compounder_desk::market::Bar], String> {
        let key = (coin.to_string(), interval);
        let have = self.0.get(&key).map_or(0, Vec::len);
        if have < bars {
            let start = interval.floor(t) - (bars as i64 + 2) * interval.ms();
            self.0.insert(key.clone(), venue.bars(coin, interval, start, t)?);
        }
        let all = &self.0[&key];
        Ok(&all[all.len().saturating_sub(bars)..])
    }
}

/// One minute of the desk: funding paid, decisions at bar closes, fills against
/// the books, and a mark each hour. Market data is read once and shared; the
/// funds decide side by side, so a slow model holds up no other fund.
fn tick(venue: &Venue, costs: &Costs, funds: &mut [Live]) -> Result<(), String> {
    let t = now();
    let mut coins: Vec<String> = funds.iter().flat_map(|f| f.fund.coins()).collect();
    coins.sort();
    coins.dedup();
    let markets = venue.markets_for(&coins)?;
    let marks: BTreeMap<String, f64> = markets.iter().map(|(c, m)| (c.clone(), m.mark)).collect();

    // Funding paid on the hour, on what was held through it.
    for live in funds.iter_mut() {
        let state = replay(&live.ledger.entries);
        for (coin, qty) in &state.account.qty {
            let from = live.funded_to(coin) + 1;
            if t - from < HOUR_MS / 2 {
                continue;
            }
            for (when, rate) in venue.funding(coin, from, t)? {
                let px = marks.get(coin).copied().unwrap_or(0.0);
                live.ledger
                    .push(when, Event::Funding { coin: coin.clone(), rate, px, paid: qty * px * rate })
                    .map_err(|e| e.to_string())?;
            }
        }
    }

    // The funds whose bar has closed, with what each will see.
    let mut bars = Bars::default();
    let mut due: Vec<(usize, i64, BTreeMap<String, Vec<compounder_desk::market::Bar>>)> = Vec::new();
    for (index, live) in funds.iter().enumerate() {
        let bar = live.fund.interval.floor(t);
        let opened_bar = live.fund.interval.floor(live.opened());
        if !live.last_bar_decided().map_or(bar >= opened_bar, |last| bar > last) {
            continue;
        }
        let mut seen = BTreeMap::new();
        for coin in live.fund.coins() {
            match bars.get(venue, &coin, live.fund.interval, live.fund.strategy.lookback(), t) {
                Ok(b) => {
                    seen.insert(coin, b.to_vec());
                }
                Err(error) => eprintln!("{}: bars for {coin}: {error}", live.fund.id),
            }
        }
        due.push((index, bar, seen));
    }

    // Every due fund decides at once.
    let decisions: Vec<(usize, compounder_desk::strategy::Decision)> = std::thread::scope(|scope| {
        let mut handles = Vec::new();
        let mut rest: Vec<(usize, &mut Live)> = funds.iter_mut().enumerate().collect();
        for (index, bar, seen) in &due {
            let Some(at) = rest.iter().position(|(i, _)| i == index) else {
                continue;
            };
            let (index, live) = rest.swap_remove(at);
            let weights = replay(&live.ledger.entries).account.weights(&marks);
            let funding: BTreeMap<String, f64> = markets.iter().map(|(c, m)| (c.clone(), m.funding)).collect();
            let bar = *bar;
            handles.push(scope.spawn(move || {
                let view = View { now: bar, bars: seen.iter().map(|(c, b)| (c.clone(), b.as_slice())).collect(), funding, weights };
                (index, live.strategy.decide(&view))
            }));
        }
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });

    // Trades against the books, one book per coin per minute.
    let mut books = BTreeMap::new();
    for (index, decision) in decisions {
        let live = &mut funds[index];
        let state = replay(&live.ledger.entries);
        live.ledger
            .push(now(), Event::Decision { targets: decision.targets.clone(), note: decision.note.clone() })
            .map_err(|e| e.to_string())?;
        for (coin, qty) in orders(&state.account, &decision.targets, &marks, live.fund.min_trade) {
            let Some(market) = markets.get(&coin) else {
                continue;
            };
            let qty = to_step(qty, market.size_decimals);
            // The venue takes no order under ten dollars.
            if (qty * market.mark).abs() < 10.0 {
                continue;
            }
            if !books.contains_key(&coin) {
                books.insert(coin.clone(), venue.book(&coin)?);
            }
            if let Some(fill) = walk(&books[&coin], &coin, qty, costs) {
                live.ledger.push(now(), Event::Fill(fill)).map_err(|e| e.to_string())?;
            }
        }
    }

    // A mark each hour.
    let hour = Interval::H1.floor(t);
    for live in funds.iter_mut() {
        if live.last_mark_hour().is_some_and(|last| hour <= last) {
            continue;
        }
        let state = replay(&live.ledger.entries);
        let held: BTreeMap<String, f64> =
            live.fund.coins().iter().chain(state.account.qty.keys()).filter_map(|c| Some((c.clone(), *marks.get(c)?))).collect();
        let equity = state.account.equity(&held);
        live.ledger.push(now(), Event::Mark { marks: held, equity }).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn verify() -> Result<(), String> {
    let funds = dir().join("funds");
    let mut paths: Vec<PathBuf> = fs::read_dir(&funds).map_err(|e| e.to_string())?.filter_map(|e| Some(e.ok()?.path())).collect();
    paths.sort();
    for path in paths {
        let entries = ledger::read(&path).map_err(|e| e.to_string())?;
        let ok = ledger::verify(&entries);
        let state = replay(&entries);
        let f = figures(&state.curve);
        println!(
            "{:<28} {:>6} entries {:>4} fills  hashes {}  equity {:.2}  return {:+.2}%",
            path.file_stem().unwrap().to_string_lossy(),
            entries.len(),
            state.fills,
            match ok {
                Ok(()) => "hold".to_string(),
                Err(at) => format!("break at {at}"),
            },
            state.curve.last().map(|p| p.equity).unwrap_or(0.0),
            f.total_return * 100.0
        );
    }
    Ok(())
}
