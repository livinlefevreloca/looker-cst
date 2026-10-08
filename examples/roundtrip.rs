//! Parses every .lkml file under a directory and checks that printing reproduces it.
//! Usage: cargo run --example roundtrip -- ../looker

use std::path::Path;

fn walk(dir: &Path, files: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n != ".git") {
                walk(&path, files);
            }
        } else if path.extension().is_some_and(|e| e == "lkml") {
            files.push(path);
        }
    }
}

fn main() {
    let root = std::env::args().nth(1).unwrap_or_else(|| "../looker".to_string());
    let mut files = Vec::new();
    walk(Path::new(&root), &mut files);
    let mut failures = 0;
    for path in &files {
        let source = std::fs::read_to_string(path).unwrap();
        match looker_cst::parse(&source) {
            Ok(doc) if doc.to_string() == source => {}
            Ok(_) => {
                failures += 1;
                println!("MISMATCH {}", path.display());
            }
            Err(e) => {
                failures += 1;
                println!("ERROR {}: {e}", path.display());
            }
        }
    }
    println!("{} files, {} failures", files.len(), failures);
}
