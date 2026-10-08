//! Parse, print and edit benchmarks over a hand written fixture and the looker repo.
//!
//! Run with `cargo bench`. The looker repo is found or cloned as in the tests (see
//! tests/support/looker_repo.rs); without it only the fixture benchmarks run.

#[path = "../tests/support/looker_repo.rs"]
mod looker_repo;

use std::fs;
use std::hint::black_box;
use std::path::Path;

use looker_cst::{Document, parse};
use criterion::{BatchSize, Criterion, Throughput, criterion_group, criterion_main};

const DIMENSION: &str = "dimension: benchmark_marker {\n  type: number\n  sql: ${TABLE}.marker ;;\n}";

fn fixture() -> String {
    fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/edge_cases.view.lkml"))
        .unwrap()
}

fn bench_fixture(c: &mut Criterion) {
    let source = fixture();
    let mut group = c.benchmark_group("fixture");
    group.throughput(Throughput::Bytes(source.len() as u64));
    group.bench_function("parse", |b| b.iter(|| parse(black_box(&source)).unwrap()));
    let doc = parse(&source).unwrap();
    group.bench_function("print", |b| b.iter(|| black_box(&doc).to_string()));
    group.finish();
}

fn bench_looker_repo(c: &mut Criterion) {
    let sources: Vec<String> = looker_repo::looker_files()
        .iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect();
    if sources.is_empty() {
        return;
    }
    let total_bytes: usize = sources.iter().map(String::len).sum();

    let mut group = c.benchmark_group("looker_repo");
    group.throughput(Throughput::Bytes(total_bytes as u64));
    group.bench_function("parse", |b| {
        b.iter(|| {
            for source in &sources {
                black_box(parse(source).unwrap());
            }
        })
    });
    let docs: Vec<Document> = sources.iter().map(|s| parse(s).unwrap()).collect();
    group.bench_function("print", |b| {
        b.iter(|| {
            for doc in &docs {
                black_box(doc.to_string());
            }
        })
    });
    group.bench_function("round_trip", |b| {
        b.iter(|| {
            for source in &sources {
                black_box(parse(source).unwrap().to_string());
            }
        })
    });
    group.finish();

    let largest = sources.iter().max_by_key(|s| s.len()).unwrap();
    let mut group = c.benchmark_group("largest_file");
    group.throughput(Throughput::Bytes(largest.len() as u64));
    group.bench_function("parse", |b| b.iter(|| parse(black_box(largest)).unwrap()));
    group.bench_function("add_dimension_and_print", |b| {
        b.iter_batched(
            || parse(largest).unwrap(),
            |mut doc| {
                add_dimension_to_views(&mut doc);
                doc.to_string()
            },
            BatchSize::SmallInput,
        )
    });
    group.finish();
}

/// Appends DIMENSION to every top level view, as a typical scripted edit would.
fn add_dimension_to_views(doc: &mut Document) {
    for view in doc.body.find_all("view", None) {
        let indent = view.read().leading.rsplit('\n').next().unwrap_or("").to_string();
        let mut guard = view.write();
        if let Some(body) = guard.body_mut() {
            body.insert_source(usize::MAX, DIMENSION, &indent, false).unwrap();
        }
    }
}

criterion_group!(benches, bench_fixture, bench_looker_repo);
criterion_main!(benches);
