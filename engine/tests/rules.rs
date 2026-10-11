//! The example strategies in `tests/rules`. Each folder holds a block-tree
//! document, `input.json`, and the tree it must import to, `expected.json`,
//! which is the canonical text laid out one item to a line.
//!
//! The expected files were not made by this importer. They were worked out
//! separately from the same documents, so a mistake here cannot hide in them.

use compounder_engine::rules::{import_json, IssueCode, MAX_NODES};
use serde_json::Value;
use std::fs;
use std::path::Path;

/// The examples that hold something no strategy may run, and why.
const REFUSED: [(&str, IssueCode); 2] = [("06-market-cap-weight", IssueCode::UnsupportedBlock), ("35-empty-placeholder", IssueCode::IncompleteStrategy)];

struct Example {
    name: String,
    input: String,
    expected: Option<String>,
}

fn examples() -> Vec<Example> {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rules");
    let mut examples: Vec<Example> = fs::read_dir(&folder)
        .expect("the examples folder is there")
        .map(|entry| entry.expect("a folder entry").path())
        .map(|path| Example {
            name: path.file_name().expect("a folder name").to_string_lossy().into_owned(),
            input: fs::read_to_string(path.join("input.json")).expect("every example has an input"),
            expected: fs::read_to_string(path.join("expected.json")).ok(),
        })
        .collect();
    examples.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(examples.len(), 35, "an example went missing");
    examples
}

/// Take out the white space between tokens, which is all that stands
/// between an expected file and the canonical text.
fn compact(laid_out: &str) -> String {
    let (mut text, mut in_text, mut escaped) = (String::new(), false, false);
    for character in laid_out.chars() {
        if in_text {
            in_text = escaped || character != '"';
            escaped = !escaped && character == '\\';
        } else if character == '"' {
            in_text = true;
        } else if character.is_whitespace() {
            continue;
        }
        text.push(character);
    }
    text
}

/// The count as the block-tree format defines it: every block except the
/// `if` wrapper around its two branches.
fn counted(block: &Value) -> usize {
    let below: usize = block["children"].as_array().map_or(0, |children| children.iter().map(counted).sum());
    usize::from(block["step"] != "if") + below
}

#[test]
fn every_example_imports_to_exactly_its_expected_tree() {
    for example in examples() {
        let Some(expected) = &example.expected else { continue };
        let strategy = import_json(&example.input).unwrap_or_else(|issues| panic!("{}: {issues:#?}", example.name));
        assert_eq!(strategy.canonical_json(), compact(expected), "{}", example.name);
    }
}

#[test]
fn an_example_that_cannot_run_is_refused_for_its_one_reason() {
    for example in examples().into_iter().filter(|example| example.expected.is_none()) {
        let (_, code) = REFUSED.iter().find(|(name, _)| *name == example.name).unwrap_or_else(|| panic!("{} has no expected tree", example.name));
        let issues = import_json(&example.input).expect_err("refused");
        assert_eq!(issues.iter().map(|issue| issue.code).collect::<Vec<_>>(), vec![*code], "{}", example.name);
    }
    assert_eq!(examples().iter().filter(|example| example.expected.is_none()).count(), REFUSED.len());
}

#[test]
fn every_example_counts_as_many_nodes_as_its_document_does() {
    for example in examples().into_iter().filter(|example| example.expected.is_some()) {
        let document: Value = serde_json::from_str(&example.input).expect("json");
        let strategy = import_json(&example.input).expect("imports");
        assert_eq!(strategy.node_count(), counted(&document), "{}", example.name);
    }
    let kitchen_sink = examples().into_iter().find(|example| example.name == "34-kitchen-sink").expect("there");
    assert_eq!(import_json(&kitchen_sink.input).expect("imports").node_count(), 21);
}

#[test]
fn every_example_that_imports_is_valid() {
    for example in examples().into_iter().filter(|example| example.expected.is_some()) {
        let issues = import_json(&example.input).expect("imports").validate(MAX_NODES);
        assert!(issues.is_empty(), "{}: {issues:#?}", example.name);
    }
}

/// The digest of the first example's canonical text, worked out outside
/// this code. It is written in two halves so no line holds a 64-digit value.
#[test]
fn the_hash_is_the_sha256_of_the_canonical_text() {
    let first = examples().into_iter().find(|example| example.name == "01-equal-weight").expect("there");
    let hash = import_json(&first.input).expect("imports").hash();
    assert_eq!(hash, concat!("f4f411b1db4b6a76440964113979e5c2", "bd2f8d5142550aa0083ef121d028d0e4"));
}
