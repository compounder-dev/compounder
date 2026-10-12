//! The funds the app offers. A starter is one a person can begin with: it
//! holds tokens in set shares and has the fixed document the money side runs.
//! The rest of the catalogue follows rules and is tested on listed shares.

use std::{collections::BTreeMap, sync::OnceLock};

use compounder_engine::{
    rules::{Node, RuleStrategy, Weighting},
    strategy::Strategy,
};
use serde::Deserialize;
use serde_json::Value;

/// One fund: its words and the rule document it follows.
pub struct Entry {
    pub id: String,
    pub name: String,
    pub summary: String,
    pub accent: String,
    /// The fixed document the money side runs and a browser can check. Only a starter has one.
    pub fixed: Option<Strategy>,
    pub rules: RuleStrategy,
}

impl Entry {
    pub fn is_starter(&self) -> bool {
        self.fixed.is_some()
    }

    /// Whether it holds named assets in shares that no price changes. Anything else follows rules.
    pub fn is_basket(&self) -> bool {
        match &self.rules.root {
            Node::Asset(_) => true,
            Node::Weight { weighting: Weighting::Equal | Weighting::Specified { .. }, children, .. } => children.iter().all(|child| matches!(child, Node::Asset(_))),
            _ => false,
        }
    }
}

#[derive(Deserialize)]
struct StarterFile {
    starters: Vec<Starter>,
}

#[derive(Deserialize)]
struct Starter {
    id: String,
    name: String,
    summary: String,
    accent: String,
    strategy: Strategy,
}

#[derive(Deserialize)]
struct StrategyFile {
    strategies: Vec<Written>,
}

#[derive(Deserialize)]
struct Written {
    id: String,
    name: String,
    summary: String,
    accent: String,
    rules: Value,
}

/// Every fund, the starters first. The files are part of the build, so one that
/// cannot be read is a fault in the build and stops the server the first time it is asked.
pub fn entries() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| {
        let starters: StarterFile = serde_json::from_str(include_str!("../data/starters.json")).expect("data/starters.json is valid");
        let written: StrategyFile = serde_json::from_str(include_str!("../data/strategies.json")).expect("data/strategies.json is valid");
        let mut entries: Vec<Entry> = starters
            .starters
            .into_iter()
            .map(|starter| {
                let rules = RuleStrategy::from_fixed(&starter.strategy).unwrap_or_else(|issues| panic!("the starter {} is not a valid strategy: {issues:?}", starter.id));
                Entry { id: starter.id, name: starter.name, summary: starter.summary, accent: starter.accent, fixed: Some(starter.strategy), rules }
            })
            .collect();
        entries.extend(written.strategies.into_iter().map(|each| {
            let rules = RuleStrategy::from_document(&each.rules).unwrap_or_else(|issues| panic!("the strategy {} is not a valid rule document: {issues:?}", each.id));
            Entry { id: each.id, name: each.name, summary: each.summary, accent: each.accent, fixed: None, rules }
        }));
        for (place, entry) in entries.iter().enumerate() {
            assert!(entries[..place].iter().all(|earlier| earlier.id != entry.id), "two funds are called {}", entry.id);
        }
        entries
    })
}

pub fn find(id: &str) -> Option<&'static Entry> {
    entries().iter().find(|entry| entry.id == id)
}

#[derive(Deserialize)]
struct ListedFile {
    listed: Vec<Listed>,
}

#[derive(Deserialize)]
struct Listed {
    symbol: String,
    name: String,
}

/// What a person calls a listed share or fund, where the app has a name for it.
pub fn listed_name(symbol: &str) -> Option<&'static str> {
    static NAMES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    let names = NAMES.get_or_init(|| {
        let file: ListedFile = serde_json::from_str(include_str!("../data/listed.json")).expect("data/listed.json is valid");
        file.listed.into_iter().map(|listed| (listed.symbol, listed.name)).collect()
    });
    names.get(symbol).map(String::as_str)
}
