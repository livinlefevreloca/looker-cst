//! Reads, modifies and rewrites every .lkml file in the looker repo.
//!
//! See support/looker_repo.rs for how the checkout is found or cloned. When it is unavailable
//! these tests pass without checking anything, unless REQUIRE_LOOKER_REPO is set.

mod support;

use std::fs;
use std::path::Path;

use support::looker_repo::looker_files;

use albert_looker_cst::edit::{block_value, infer_scalar, new_pair};
use albert_looker_cst::{Body, Document, PairRef, Value, parse};

const MARKER_KEY: &str = "lkml_cst_marker";
/// Parses every file, running check on each and reporting all failures together.
fn for_each_file(check: impl Fn(&Path, &str) -> Result<(), String>) {
    let failures: Vec<String> = looker_files()
        .iter()
        .filter_map(|path| {
            let source = fs::read_to_string(path).unwrap();
            check(path, &source)
                .err()
                .map(|e| format!("{}: {e}", path.display()))
        })
        .collect();
    assert!(
        failures.is_empty(),
        "{} files failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

fn parse_file(source: &str) -> Result<Document, String> {
    parse(source).map_err(|e| e.to_string())
}

/// Every pair in the document, depth first.
fn all_pairs(body: &Body, out: &mut Vec<PairRef>) {
    for item in &body.items {
        out.push(item.clone());
        if let Some(body) = item.read().body() {
            all_pairs(body, out);
        }
    }
}

fn pairs_of(doc: &Document) -> Vec<PairRef> {
    let mut out = Vec::new();
    all_pairs(&doc.body, &mut out);
    out
}

/// A shape of the tree that ignores formatting: keys, names and scalar values.
fn outline(body: &Body, depth: usize, out: &mut Vec<String>) {
    for item in &body.items {
        let pair = item.read();
        let value = match &pair.value {
            Value::Block(block) => format!("{{{}}}", block.name.as_deref().unwrap_or("")),
            Value::List(list) => format!("[{}]", list.items.len()),
            _ => pair.text().unwrap(),
        };
        out.push(format!("{}{}: {}", "  ".repeat(depth), pair.key, value));
        if let Some(body) = pair.body() {
            outline(body, depth + 1, out);
        }
    }
}

fn outline_of(doc: &Document) -> Vec<String> {
    let mut out = Vec::new();
    outline(&doc.body, 0, &mut out);
    out
}

#[test]
fn every_file_round_trips() {
    for_each_file(|_, source| {
        let doc = parse_file(source)?;
        if doc.to_string() != source {
            return Err("printed output differs from source".into());
        }
        Ok(())
    });
}

#[test]
fn rewriting_values_with_themselves_changes_nothing() {
    for_each_file(|_, source| {
        let doc = parse_file(source)?;
        for pair in pairs_of(&doc) {
            let mut pair = pair.write();
            if let Some(text) = pair.text() {
                pair.set_text(&text);
            } else if let Value::List(list) = &pair.value {
                // Lists are only rebuilt from their elements when no comments would be lost.
                if !list.to_string().contains('#') {
                    let list = list.clone();
                    pair.set_value(Value::List(list));
                }
            }
        }
        if doc.to_string() != source {
            return Err("identity edits changed the output".into());
        }
        Ok(())
    });
}

#[test]
fn edited_values_are_written_and_read_back() {
    for_each_file(|_, source| {
        let doc = parse_file(source)?;
        let mut expected = Vec::new();
        for (i, pair) in pairs_of(&doc).iter().enumerate() {
            let mut pair = pair.write();
            let new_text = match &pair.value {
                Value::Literal(text) => format!("{text}_{i}"),
                Value::String(_) => format!("edited \"{i}\" \\ value"),
                Value::Expr(expr) => format!("{} /* {i} */", expr.text),
                _ => continue,
            };
            pair.set_text(&new_text);
            expected.push(new_text);
        }
        let reparsed = parse_file(&doc.to_string())?;
        let actual: Vec<String> = pairs_of(&reparsed)
            .iter()
            .filter_map(|p| p.read().text())
            .collect();
        if actual != expected {
            return Err("edited values did not survive a reparse".into());
        }
        Ok(())
    });
}

#[test]
fn adding_and_removing_pairs_in_every_block() {
    for_each_file(|_, source| {
        let mut doc = parse_file(source)?;
        let original_outline = outline_of(&doc);
        let blocks: Vec<PairRef> = pairs_of(&doc)
            .into_iter()
            .filter(|p| p.read().body().is_some())
            .collect();

        let mut added = Vec::new();
        for block in &blocks {
            let indent = block.read().leading.rsplit('\n').next().unwrap_or("").to_string();
            let mut guard = block.write();
            let body = guard.body_mut().unwrap();
            added.push(body.insert(
                0,
                new_pair(MARKER_KEY, infer_scalar(MARKER_KEY, "first")),
                &indent,
                false,
            ));
            added.push(body.insert(
                usize::MAX,
                new_pair(MARKER_KEY, block_value(Some("last"))),
                &indent,
                false,
            ));
        }
        added.push(doc_insert(&mut doc, 0));
        added.push(doc_insert(&mut doc, usize::MAX));

        let edited = doc.to_string();
        let reparsed = parse_file(&edited).map_err(|e| format!("edited file does not parse: {e}"))?;
        if reparsed.to_string() != edited {
            return Err("edited file does not round trip".into());
        }
        let markers = pairs_of(&reparsed)
            .iter()
            .filter(|p| p.read().key == MARKER_KEY)
            .count();
        if markers != added.len() {
            return Err(format!("expected {} markers, found {markers}", added.len()));
        }
        let mut without_markers = outline_of(&reparsed);
        without_markers.retain(|line| !line.trim_start().starts_with(MARKER_KEY));
        if without_markers != original_outline {
            return Err("adding pairs changed the rest of the tree".into());
        }

        for pair in &added {
            let removed = doc_remove(&mut doc, pair);
            if !removed {
                return Err("added pair was not found for removal".into());
            }
        }
        if doc.to_string() != source {
            return Err("removing the added pairs did not restore the source".into());
        }
        Ok(())
    });
}

fn doc_insert(doc: &mut Document, index: usize) -> PairRef {
    doc.body.insert(
        index,
        new_pair(MARKER_KEY, infer_scalar(MARKER_KEY, "root")),
        "",
        true,
    )
}

fn doc_remove(doc: &mut Document, pair: &PairRef) -> bool {
    if doc.body.remove_pair(pair, true) {
        return true;
    }
    pairs_of(doc).iter().any(|block| {
        block
            .write()
            .body_mut()
            .is_some_and(|body| body.remove_pair(pair, false))
    })
}
