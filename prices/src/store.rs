//! The price store: a directory with one plain file of raw bars per ticker.
//!
//! `SPY.csv` reads:
//!
//! ```text
//! day,close,dividend,split
//! 2025-11-14,1112.17,,
//! 2025-11-17,110.29,,10
//! ```
//!
//! One line a day, oldest first. The dividend and the split are blank on the
//! days that have none, which is nearly all of them. A person can read the
//! file, and fix it by hand if it comes to that.
//!
//! A file is replaced whole and in one step: the new text goes to a scratch
//! file beside it, which then takes the real name. A reader sees the old
//! history or the new one, never half of either, and a writer that dies
//! leaves the old one in place.
//!
//! `_index.csv` lists every ticker with its first day, its last day and how
//! many days it has, so the span of each is known without opening its file.
//! The index only repeats what the files say, and a ticker's file is the
//! truth. So the index is believed only while it lists exactly the tickers
//! that have a file and no file has been written since. Otherwise, as after a
//! file is changed by hand or a writer dies between a file and the index, the
//! spans are read from the files, and the next write puts the index right.
//!
//! One process writes to a store at a time. Any number may read, and reading
//! never writes.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    io::{ErrorKind, Write as _},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::SystemTime,
};

use crate::{
    adjust::{adjust, validate, Bar, Series},
    day::Day,
};

const HEADER: &str = "day,close,dividend,split";
const INDEX_FILE: &str = "_index.csv";
const INDEX_HEADER: &str = "ticker,first_day,last_day,days";
const LONGEST_TICKER: usize = 12;

/// What the index says about one ticker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub first_day: Day,
    pub last_day: Day,
    pub days: usize,
}

pub struct Store {
    dir: PathBuf,
    index: BTreeMap<String, Span>,
}

/// Whether `ticker` is one: capital letters and digits, with a slash, a dot
/// or a dash allowed inside it, as in `BRK/B`. Anything else is refused, so a
/// ticker can never name a path outside the store or reshape a request.
pub fn check_ticker(ticker: &str) -> Result<(), String> {
    let bytes = ticker.as_bytes();
    let plain = |byte: &u8| byte.is_ascii_uppercase() || byte.is_ascii_digit();
    let inside = bytes.iter().all(|byte| plain(byte) || matches!(byte, b'/' | b'.' | b'-'));
    if bytes.is_empty() || bytes.len() > LONGEST_TICKER || !inside || !plain(&bytes[0]) || !plain(&bytes[bytes.len() - 1]) {
        return Err(format!("{ticker:?} is not a ticker"));
    }
    Ok(())
}

/// The file a ticker's bars are kept in.
fn file_name(ticker: &str) -> Result<String, String> {
    check_ticker(ticker)?;
    Ok(format!("{}.csv", ticker.replace('/', "_")))
}

/// The ticker a file in the store holds, when it is a ticker's file.
fn ticker_of(file: &str) -> Option<String> {
    let ticker = file.strip_suffix(".csv")?.replace('_', "/");
    file_name(&ticker).is_ok().then_some(ticker)
}

fn text_of(bars: &[Bar]) -> String {
    let mut text = String::with_capacity(HEADER.len() + 1 + bars.len() * 24);
    text.push_str(HEADER);
    text.push('\n');
    for bar in bars {
        // Writing to a string cannot fail.
        let _ = write!(text, "{},{},", bar.day, bar.close);
        if bar.dividend != 0.0 {
            let _ = write!(text, "{}", bar.dividend);
        }
        text.push(',');
        if bar.split != 1.0 {
            let _ = write!(text, "{}", bar.split);
        }
        text.push('\n');
    }
    text
}

fn bars_of(text: &str) -> Result<Vec<Bar>, String> {
    let mut lines = text.lines();
    if lines.next() != Some(HEADER) {
        return Err(format!("the first line is not `{HEADER}`"));
    }
    let mut bars = Vec::with_capacity(text.len() / 20);
    for (index, line) in lines.enumerate() {
        let wrong = |what: &str| format!("line {}: {what} in `{line}`", index + 2);
        let mut fields = line.split(',');
        let (Some(day), Some(close), Some(dividend), Some(split), None) = (fields.next(), fields.next(), fields.next(), fields.next(), fields.next()) else {
            return Err(wrong("there are not four fields"));
        };
        let number = |field: &str, absent: f64, what: &str| if field.is_empty() { Ok(absent) } else { field.parse::<f64>().map_err(|_| wrong(what)) };
        bars.push(Bar {
            day: day.parse().map_err(|_| wrong("the day is not a date"))?,
            close: close.parse().map_err(|_| wrong("the close is not a number"))?,
            dividend: number(dividend, 0.0, "the dividend is not a number")?,
            split: number(split, 1.0, "the split is not a number")?,
        });
    }
    validate(&bars)?;
    Ok(bars)
}

/// Put `text` at `path` in one step. It is written to a scratch file in the
/// same directory and flushed to disk, and the scratch file then takes the
/// real name, which replaces the old file whole.
fn replace(path: &Path, text: &str) -> Result<(), String> {
    // Two writes in one process never share a scratch file.
    static WRITES: AtomicU64 = AtomicU64::new(0);
    let scratch = path.with_extension(format!("{}-{}.part", std::process::id(), WRITES.fetch_add(1, Ordering::Relaxed)));
    let written = fs::File::create(&scratch)
        .and_then(|mut file| file.write_all(text.as_bytes()).and_then(|()| file.sync_all()))
        .and_then(|()| fs::rename(&scratch, path));
    written.map_err(|error| {
        let _ = fs::remove_file(&scratch);
        format!("cannot write {}: {error}", path.display())
    })
}

impl Store {
    /// Open the store in `dir`, making the directory if it is not there.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Store, String> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(|error| format!("cannot make {}: {error}", dir.display()))?;
        let mut store = Store { dir, index: BTreeMap::new() };
        let path = store.dir.join(INDEX_FILE);
        let listed = fs::read_to_string(&path).ok().and_then(|text| index_of(&text));
        let written = fs::metadata(&path).and_then(|file| file.modified()).ok();
        let files = store.files()?;
        store.index = match (listed, written) {
            (Some(index), Some(written)) if index.keys().eq(files.iter().map(|(ticker, _)| ticker)) && files.iter().all(|(_, changed)| *changed <= written) => index,
            _ => store.spans()?,
        };
        Ok(store)
    }

    /// Every ticker's span, read from its file.
    fn spans(&self) -> Result<BTreeMap<String, Span>, String> {
        let mut spans = BTreeMap::new();
        for (ticker, _) in self.files()? {
            let span = span_of(&self.load(&ticker)?);
            spans.insert(ticker, span);
        }
        Ok(spans)
    }

    /// The tickers that have a file, in order, and when each file was last written.
    fn files(&self) -> Result<Vec<(String, SystemTime)>, String> {
        let unreadable = |error: std::io::Error| format!("cannot list {}: {error}", self.dir.display());
        let mut files = Vec::new();
        for entry in fs::read_dir(&self.dir).map_err(unreadable)? {
            let entry = entry.map_err(unreadable)?;
            let Some(ticker) = entry.file_name().to_str().and_then(ticker_of) else { continue };
            files.push((ticker, entry.metadata().and_then(|file| file.modified()).map_err(unreadable)?));
        }
        files.sort();
        Ok(files)
    }

    /// Every ticker in the store, in order.
    pub fn tickers(&self) -> impl Iterator<Item = &str> {
        self.index.keys().map(String::as_str)
    }

    /// The days a ticker covers, when the store has it.
    pub fn span(&self, ticker: &str) -> Option<Span> {
        self.index.get(ticker).copied()
    }

    /// A ticker's raw bars, oldest first, or nothing when the store has no such ticker.
    pub fn find(&self, ticker: &str) -> Result<Option<Vec<Bar>>, String> {
        let path = self.dir.join(file_name(ticker)?);
        match fs::read_to_string(&path) {
            Ok(text) => bars_of(&text).map(Some).map_err(|reason| format!("{}: {reason}", path.display())),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(format!("cannot read {}: {error}", path.display())),
        }
    }

    /// A ticker's raw bars, oldest first.
    pub fn load(&self, ticker: &str) -> Result<Vec<Bar>, String> {
        self.find(ticker)?.ok_or_else(|| format!("the store has no {ticker}"))
    }

    /// A ticker's adjusted series, worked out from its stored bars.
    pub fn series(&self, ticker: &str) -> Result<Series, String> {
        adjust(&self.load(ticker)?)
    }

    /// Store a ticker's whole history in place of whatever was there.
    pub fn save(&mut self, ticker: &str, bars: &[Bar]) -> Result<(), String> {
        let name = file_name(ticker)?;
        validate(bars).map_err(|reason| format!("{ticker}: {reason}"))?;
        replace(&self.dir.join(name), &text_of(bars))?;
        self.index.insert(ticker.to_string(), span_of(bars));
        self.write_index()
    }

    /// Write the index afresh from the files, for the writer of a store whose
    /// files were changed by something else.
    pub fn reindex(&mut self) -> Result<(), String> {
        self.index = self.spans()?;
        self.write_index()
    }

    fn write_index(&self) -> Result<(), String> {
        let mut text = format!("{INDEX_HEADER}\n");
        for (ticker, span) in &self.index {
            let _ = writeln!(text, "{ticker},{},{},{}", span.first_day, span.last_day, span.days);
        }
        replace(&self.dir.join(INDEX_FILE), &text)
    }
}

/// `bars` has passed `validate`, so it is not empty.
fn span_of(bars: &[Bar]) -> Span {
    Span { first_day: bars[0].day, last_day: bars[bars.len() - 1].day, days: bars.len() }
}

/// What an index file says, or nothing when it is not an index.
fn index_of(text: &str) -> Option<BTreeMap<String, Span>> {
    let mut lines = text.lines();
    if lines.next() != Some(INDEX_HEADER) {
        return None;
    }
    let mut index = BTreeMap::new();
    for line in lines {
        let fields: Vec<&str> = line.split(',').collect();
        let [ticker, first, last, days] = fields[..] else { return None };
        check_ticker(ticker).ok()?;
        index.insert(ticker.to_string(), Span { first_day: first.parse().ok()?, last_day: last.parse().ok()?, days: days.parse().ok()? });
    }
    Some(index)
}

/// A store in a directory of its own, removed again when the test ends.
#[cfg(test)]
pub(crate) mod scratch {
    use super::*;

    pub struct Scratch {
        pub dir: PathBuf,
    }

    impl Scratch {
        pub fn new(name: &str) -> Scratch {
            let dir = std::env::temp_dir().join(format!("compounder-prices-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            Scratch { dir }
        }

        pub fn store(&self) -> Store {
            Store::open(&self.dir).unwrap()
        }

        /// The names of the files in the directory, in order.
        pub fn files(&self) -> Vec<String> {
            let mut names: Vec<String> = fs::read_dir(&self.dir).unwrap().map(|entry| entry.unwrap().file_name().into_string().unwrap()).collect();
            names.sort();
            names
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::scratch::Scratch;
    use super::*;
    use crate::adjust::fixtures::*;
    use crate::calendar::Calendar;
    use std::io::Read as _;

    fn sample() -> Vec<Bar> {
        vec![
            Bar::new(day("2025-11-13"), 1_154.23),
            Bar { day: day("2025-11-14"), close: 1_112.17, dividend: 0.4625, split: 1.0 },
            Bar { day: day("2025-11-17"), close: 110.29, dividend: 0.0, split: 10.0 },
            Bar { day: day("2025-11-18"), close: 114.0, dividend: 0.05, split: 0.2 },
        ]
    }

    #[test]
    fn a_history_comes_back_exactly_as_it_was_stored() {
        let scratch = Scratch::new("round-trip");
        let mut store = scratch.store();
        for (ticker, text) in [("KO", DIVIDEND_PAYER), ("NFLX", SPLIT_STOCK), ("TLT", BOND_FUND), ("SPY", INDEX_FUND), ("SZK", THIN_FUND)] {
            let bars = bars(text);
            store.save(ticker, &bars).unwrap();
            // The same numbers to the last bit, from this store and from one opened afresh.
            assert_eq!(store.load(ticker).unwrap(), bars);
            assert_eq!(scratch.store().load(ticker).unwrap(), bars);
            assert_eq!(store.series(ticker).unwrap(), adjust(&bars).unwrap());
        }
        assert_eq!(store.tickers().collect::<Vec<_>>(), ["KO", "NFLX", "SPY", "SZK", "TLT"]);
    }

    #[test]
    fn the_file_is_plain_text_a_person_can_read() {
        let scratch = Scratch::new("plain-text");
        scratch.store().save("NFLX", &sample()).unwrap();
        let text = fs::read_to_string(scratch.dir.join("NFLX.csv")).unwrap();
        assert_eq!(text, "day,close,dividend,split\n2025-11-13,1154.23,,\n2025-11-14,1112.17,0.4625,\n2025-11-17,110.29,,10\n2025-11-18,114,0.05,0.2\n");
        // And one written by hand is read.
        fs::write(scratch.dir.join("ABC.csv"), "day,close,dividend,split\n2026-01-05,10.5,,\n2026-01-06,10.75,0.1,\n").unwrap();
        let by_hand = scratch.store().load("ABC").unwrap();
        assert_eq!(by_hand, [Bar::new(day("2026-01-05"), 10.5), Bar { day: day("2026-01-06"), close: 10.75, dividend: 0.1, split: 1.0 }]);
    }

    /// Mark a file as written `seconds` after now, as a later change would be.
    fn touch(path: &Path, seconds: u64) {
        let later = SystemTime::now() + std::time::Duration::from_secs(seconds);
        fs::File::options().write(true).open(path).unwrap().set_modified(later).unwrap();
    }

    #[test]
    fn the_index_lists_each_ticker_and_its_span() {
        let scratch = Scratch::new("index");
        let mut store = scratch.store();
        assert_eq!(store.tickers().count(), 0);
        assert!(scratch.files().is_empty(), "an empty store writes nothing");
        store.save("SPY", &bars(INDEX_FUND)).unwrap();
        store.save("CRCL", &bars(NEW_LISTING)).unwrap();
        let listing = Span { first_day: day("2025-06-05"), last_day: day("2026-10-09"), days: 339 };
        assert_eq!(store.span("CRCL"), Some(listing));
        assert_eq!(store.span("KO"), None);
        let text = fs::read_to_string(scratch.dir.join("_index.csv")).unwrap();
        assert_eq!(text, "ticker,first_day,last_day,days\nCRCL,2025-06-05,2026-10-09,339\nSPY,2012-01-03,2026-10-09,3714\n");

        // A store opened later reads the index and opens no ticker's file:
        // one that cannot be read does not stop it, as long as it is no newer than the index.
        let path = scratch.dir.join("CRCL.csv");
        let written = fs::metadata(&path).unwrap().modified().unwrap();
        fs::write(&path, "not a history").unwrap();
        fs::File::options().write(true).open(&path).unwrap().set_modified(written).unwrap();
        let reopened = scratch.store();
        assert_eq!(reopened.span("CRCL"), Some(listing));
        assert!(reopened.load("CRCL").unwrap_err().contains("the first line is not"));
    }

    #[test]
    fn files_changed_behind_the_index_are_seen_when_the_store_is_opened() {
        let scratch = Scratch::new("reindex");
        let mut store = scratch.store();
        store.save("SPY", &bars(INDEX_FUND)).unwrap();
        store.save("BRK/B", &sample()).unwrap();
        store.save("KO", &sample()).unwrap();
        // One file gone, one new, and something that is no ticker's file.
        fs::remove_file(scratch.dir.join("SPY.csv")).unwrap();
        fs::write(scratch.dir.join("ABC.csv"), "day,close,dividend,split\n2026-01-05,10.5,,\n").unwrap();
        fs::write(scratch.dir.join("notes.txt"), "not a ticker").unwrap();
        assert_eq!(scratch.store().tickers().collect::<Vec<_>>(), ["ABC", "BRK/B", "KO"]);

        // One file changed after the index was written: by hand, or by a
        // writer that died before it reached the index.
        fs::write(scratch.dir.join("BRK_B.csv"), text_of(&sample()[..2])).unwrap();
        touch(&scratch.dir.join("BRK_B.csv"), 5);
        let mut reopened = scratch.store();
        assert_eq!(reopened.span("BRK/B").unwrap().days, 2);
        assert_eq!(reopened.span("KO").unwrap().days, 4);

        // Opening wrote nothing: the index on disk is as the first store left it.
        let on_disk = || fs::read_to_string(scratch.dir.join("_index.csv")).unwrap();
        assert!(on_disk().contains("SPY,") && on_disk().contains("BRK/B,2025-11-13,2025-11-18,4\n"), "{}", on_disk());
        // The next write puts it right.
        reopened.save("KO", &sample()[..3]).unwrap();
        assert_eq!(on_disk(), "ticker,first_day,last_day,days\nABC,2026-01-05,2026-01-05,1\nBRK/B,2025-11-13,2025-11-14,2\nKO,2025-11-13,2025-11-17,3\n");

        // A store that stays open is told to look again.
        fs::remove_file(scratch.dir.join("KO.csv")).unwrap();
        store.reindex().unwrap();
        assert_eq!(store.tickers().collect::<Vec<_>>(), ["ABC", "BRK/B"]);
        assert_eq!(on_disk(), "ticker,first_day,last_day,days\nABC,2026-01-05,2026-01-05,1\nBRK/B,2025-11-13,2025-11-14,2\n");
    }

    #[test]
    fn a_missing_or_damaged_index_is_not_believed() {
        let scratch = Scratch::new("no-index");
        scratch.store().save("SPY", &sample()).unwrap();
        for damage in ["", "ticker,first_day,last_day,days\nSPY,2025-11-13\n", "ticker,first_day,last_day,days\nSPY,yesterday,today,4\n"] {
            fs::write(scratch.dir.join("_index.csv"), damage).unwrap();
            assert_eq!(scratch.store().span("SPY").unwrap().days, 4);
        }
        fs::remove_file(scratch.dir.join("_index.csv")).unwrap();
        let mut store = scratch.store();
        assert_eq!(store.tickers().collect::<Vec<_>>(), ["SPY"]);
        assert_eq!(scratch.files(), ["SPY.csv"]);
        store.reindex().unwrap();
        assert_eq!(fs::read_to_string(scratch.dir.join("_index.csv")).unwrap(), "ticker,first_day,last_day,days\nSPY,2025-11-13,2025-11-18,4\n");
    }

    #[test]
    fn a_ticker_names_a_file_inside_the_store_and_nothing_else() {
        assert_eq!(file_name("SPY").unwrap(), "SPY.csv");
        assert_eq!(file_name("BRK/B").unwrap(), "BRK_B.csv");
        assert_eq!(file_name("BF.B").unwrap(), "BF.B.csv");
        assert_eq!(ticker_of("BRK_B.csv").as_deref(), Some("BRK/B"));
        assert_eq!(ticker_of("_index.csv"), None);
        assert_eq!(ticker_of("SPY.1234-0.part"), None);
        let scratch = Scratch::new("names");
        let mut store = scratch.store();
        for wrong in ["", "spy", "../SPY", "/etc/passwd", "SPY/", ".SPY", "A B", "_index", "ABCDEFGHIJKLM", "SPY\n"] {
            assert!(store.save(wrong, &sample()).unwrap_err().contains("is not a ticker"), "{wrong:?}");
            assert!(store.load(wrong).unwrap_err().contains("is not a ticker"), "{wrong:?}");
        }
        assert!(scratch.files().is_empty());
    }

    #[test]
    fn a_damaged_file_is_refused_with_the_line_that_is_wrong() {
        let scratch = Scratch::new("damaged");
        let store = scratch.store();
        let load = |text: &str| {
            fs::write(scratch.dir.join("ABC.csv"), text).unwrap();
            store.load("ABC").unwrap_err()
        };
        assert!(load("2026-01-05,10.5,,\n").contains("the first line is not"));
        assert!(load("day,close,dividend,split\n2026-01-05,10.5,,\n2026-01-06,10.5\n").contains("line 3: there are not four fields"));
        assert!(load("day,close,dividend,split\n2026-01-05,ten,,\n").contains("line 2: the close is not a number"));
        assert!(load("day,close,dividend,split\n2026-13-05,10,,\n").contains("line 2: the day is not a date"));
        assert!(load("day,close,dividend,split\n2026-01-05,10,x,\n").contains("the dividend is not a number"));
        assert!(load("day,close,dividend,split\n2026-01-05,10,,x\n").contains("the split is not a number"));
        assert!(load("day,close,dividend,split\n2026-01-06,10,,\n2026-01-05,10,,\n").contains("out of order"));
        assert!(load("day,close,dividend,split\n").contains("no days"));
        // The file is named, so the person who has to mend it knows where to look.
        assert!(load("day,close\n").starts_with(&scratch.dir.join("ABC.csv").display().to_string()));
    }

    #[test]
    fn a_history_that_makes_no_sense_is_not_stored() {
        let scratch = Scratch::new("refused");
        let mut store = scratch.store();
        store.save("ABC", &sample()).unwrap();
        let mut wrong = sample();
        wrong[2].close = -1.0;
        assert!(store.save("ABC", &wrong).unwrap_err().contains("ABC: the close on 2025-11-17"));
        assert!(store.save("ABC", &[]).unwrap_err().contains("no days"));
        assert_eq!(store.load("ABC").unwrap(), sample());
    }

    #[test]
    fn a_file_is_replaced_whole_and_no_scratch_file_is_left() {
        let scratch = Scratch::new("atomic");
        let mut store = scratch.store();
        store.save("SPY", &bars(INDEX_FUND)).unwrap();
        let path = scratch.dir.join("SPY.csv");
        let before = fs::read_to_string(&path).unwrap();

        // A reader that opened the file before the write still reads the whole
        // old history afterwards: the new one took the name, the old bytes
        // were never written over.
        let mut reader = fs::File::open(&path).unwrap();
        let mut first_bytes = [0u8; 64];
        reader.read_exact(&mut first_bytes).unwrap();
        store.save("SPY", &sample()).unwrap();
        let mut rest = String::new();
        reader.read_to_string(&mut rest).unwrap();
        assert_eq!(format!("{}{rest}", std::str::from_utf8(&first_bytes).unwrap()), before);

        // The new file is the new history alone, with nothing of the longer old one after it.
        assert_eq!(fs::read_to_string(&path).unwrap(), text_of(&sample()));
        assert_eq!(scratch.files(), ["SPY.csv", "_index.csv"]);
    }

    #[test]
    fn a_write_that_fails_leaves_the_old_history_in_place() {
        let scratch = Scratch::new("failed-write");
        let mut store = scratch.store();
        store.save("SPY", &sample()).unwrap();
        // A directory sits where the file should go, so the scratch file cannot take its name.
        fs::create_dir(scratch.dir.join("KO.csv")).unwrap();
        assert!(store.save("KO", &sample()).unwrap_err().contains("cannot write"));
        assert_eq!(scratch.files(), ["KO.csv", "SPY.csv", "_index.csv"], "the scratch file was not cleared away");
        assert_eq!(store.load("SPY").unwrap(), sample());
        assert_eq!(store.tickers().collect::<Vec<_>>(), ["SPY"]);
    }

    #[test]
    fn a_scratch_file_left_by_a_dead_writer_is_ignored() {
        let scratch = Scratch::new("stale-scratch");
        let mut store = scratch.store();
        store.save("SPY", &sample()).unwrap();
        fs::write(scratch.dir.join("SPY.999-0.part"), "day,close,dividend,split\n2026-01-05,1").unwrap();
        assert_eq!(store.load("SPY").unwrap(), sample());
        assert_eq!(scratch.store().tickers().collect::<Vec<_>>(), ["SPY"]);
    }

    /// Run by hand in a release build:
    /// `cargo test --release -p compounder-prices four_hundred -- --ignored --nocapture`.
    /// 400 tickers, each with every trading day since 1990, is more than a
    /// backtest will ask for at once.
    #[test]
    #[ignore = "a timing, for a release build"]
    fn four_hundred_long_histories_load_in_well_under_a_second() {
        let scratch = Scratch::new("timing");
        let mut store = scratch.store();
        // The dividend payer's fourteen years, joined end to end until they reach from 1990 to today.
        let pattern = bars(DIVIDEND_PAYER);
        let history: Vec<Bar> = (0..9_261).map(|index| Bar { day: Day::from_number(7_306 + index as i32), ..pattern[index % pattern.len()] }).collect();
        let tickers: Vec<String> = (0..400).map(|number| format!("T{number}")).collect();
        for ticker in &tickers {
            store.save(ticker, &history).unwrap();
        }
        let bytes: u64 = tickers.iter().map(|ticker| fs::metadata(scratch.dir.join(format!("{ticker}.csv"))).unwrap().len()).sum();

        let started = std::time::Instant::now();
        let reopened = scratch.store();
        let loaded: Vec<Series> = tickers.iter().map(|ticker| reopened.series(ticker).unwrap()).collect();
        let took = started.elapsed();
        let days: usize = loaded.iter().map(Series::len).sum();
        println!("{} tickers, {days} days, {} MB on disk: opened, read and adjusted in {took:?}", loaded.len(), bytes / 1_000_000);
        assert_eq!(days, 400 * 9_261);
        assert!(took.as_millis() < 500, "{took:?}");

        // Laid on a calendar, and every price of every ticker then read once by position and once by day.
        let started = std::time::Instant::now();
        let calendar = Calendar::new(loaded[0].days().iter().copied()).unwrap();
        let aligned: Vec<_> = loaded.iter().map(|series| calendar.align(series)).collect();
        let laid_out = started.elapsed();
        let started = std::time::Instant::now();
        let by_position: f64 = aligned.iter().map(|ticker| (0..calendar.len()).filter_map(|position| ticker.at(position)).sum::<f64>()).sum();
        let read_by_position = started.elapsed();
        let started = std::time::Instant::now();
        let by_day: f64 = loaded.iter().map(|series| calendar.days().iter().filter_map(|day| series.on(*day)).sum::<f64>()).sum();
        let read_by_day = started.elapsed();
        assert_eq!(by_position, by_day);
        let each = |took: std::time::Duration| took.as_nanos() as f64 / days as f64;
        println!("laid on the calendar in {laid_out:?}. One price read by position in {:.1} ns, by day in {:.1} ns", each(read_by_position), each(read_by_day));
    }
}
