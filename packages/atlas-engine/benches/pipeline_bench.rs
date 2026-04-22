//! Pipeline throughput benchmark: parse workers vs SQLite writer bottleneck.
//!
//! Measures the parse-only phase, the write-only phase, and the combined
//! parse+write pipeline at varying batch sizes.  Supports item 15.1 in
//! ISSUES.md: "benchmark parser workers vs writer bottleneck" and "tune
//! batch sizes".
//!
//! Key signals to look for in results:
//! - If `parse_only` is much faster than `write_only`, DB writes dominate; a
//!   smaller batch size won't help but a larger one may hurt.
//! - If `parse_only` is slower, parse is the bottleneck; increasing rayon
//!   workers or splitting to more cores helps.
//! - `full_pipeline` latency should track `max(parse_only, write_only)` per
//!   batch; if it's much higher, I/O contention is present.

use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

use atlas_core::model::ParsedFile;
use atlas_parser::ParserRegistry;
use atlas_store_sqlite::Store;
use rayon::prelude::*;

// ---------------------------------------------------------------------------
// Synthetic source fixtures: realistic but compact.
// ---------------------------------------------------------------------------

const RUST_SRC: &str = r#"
use std::collections::HashMap;

pub struct Registry {
    entries: HashMap<String, Vec<u8>>,
    version: u32,
}

impl Registry {
    pub fn new() -> Self {
        Self { entries: HashMap::new(), version: 0 }
    }

    pub fn insert(&mut self, key: String, value: Vec<u8>) {
        self.entries.insert(key, value);
        self.version += 1;
    }

    pub fn get(&self, key: &str) -> Option<&Vec<u8>> {
        self.entries.get(key)
    }

    pub fn remove(&mut self, key: &str) -> Option<Vec<u8>> {
        self.entries.remove(key)
    }
}

pub trait Store: Send + Sync {
    fn put(&mut self, key: &str, value: &[u8]);
    fn fetch(&self, key: &str) -> Option<Vec<u8>>;
}

pub fn process(items: &[(&str, Vec<u8>)]) -> usize {
    items.iter().filter(|(k, _)| !k.is_empty()).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_insert() {
        let mut r = Registry::new();
        r.insert("k".into(), vec![1]);
        assert!(r.get("k").is_some());
    }
}
"#;

/// Prepare (path, source_bytes, hash) tuples for `file_count` synthetic files.
fn make_inputs(file_count: usize) -> Vec<(String, Vec<u8>, String)> {
    (0..file_count)
        .map(|i| {
            let path = format!("src/module_{i}.rs");
            let hash = format!("hash-{i:08x}");
            (path, RUST_SRC.as_bytes().to_vec(), hash)
        })
        .collect()
}

fn make_store() -> Store {
    let mut s = Store::open(":memory:").expect("in-memory store");
    s.migrate().expect("migrate");
    s
}

// ---------------------------------------------------------------------------
// Benchmark: parse-only phase (no DB write).
// ---------------------------------------------------------------------------

fn bench_parse_only(c: &mut Criterion) {
    let registry = ParserRegistry::with_defaults();
    let mut group = c.benchmark_group("pipeline/parse_only");

    for file_count in [64usize, 128, 256] {
        let inputs = make_inputs(file_count);
        group.bench_with_input(
            BenchmarkId::from_parameter(file_count),
            &inputs,
            |b, inputs| {
                b.iter(|| {
                    let _parsed: Vec<ParsedFile> = inputs
                        .par_iter()
                        .filter_map(|(path, src, hash)| {
                            registry
                                .parse(path, hash, src, None)
                                .map(|(pf, _)| black_box(pf))
                        })
                        .collect();
                });
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: write-only phase (pre-parsed, just measures SQLite throughput).
// ---------------------------------------------------------------------------

fn bench_write_only(c: &mut Criterion) {
    let registry = ParserRegistry::with_defaults();
    let mut group = c.benchmark_group("pipeline/write_only");

    for file_count in [64usize, 128, 256] {
        let inputs = make_inputs(file_count);
        // Pre-parse once; the bench measures only the write path.
        let parsed: Vec<ParsedFile> = inputs
            .par_iter()
            .filter_map(|(path, src, hash)| registry.parse(path, hash, src, None).map(|(pf, _)| pf))
            .collect();

        group.bench_with_input(
            BenchmarkId::from_parameter(file_count),
            &parsed,
            |b, parsed| {
                b.iter(|| {
                    let mut store = make_store();
                    let (n, e) = store
                        .replace_files_transactional(black_box(parsed))
                        .expect("write");
                    black_box((n, e));
                });
            },
        );
    }
    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmark: full pipeline at different batch sizes.
//
// Replicates the engine build loop:
//   for chunk in files.chunks(batch_size) {
//       parse in parallel → write batch in one transaction
//   }
// ---------------------------------------------------------------------------

fn bench_full_pipeline(c: &mut Criterion) {
    let registry = ParserRegistry::with_defaults();
    // Fixed file count; vary batch size to find the knee.
    const FILE_COUNT: usize = 256;
    let inputs = make_inputs(FILE_COUNT);

    let mut group = c.benchmark_group("pipeline/full_pipeline");
    for batch_size in [16usize, 32, 64, 128, 256] {
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &batch_size| {
                b.iter(|| {
                    let mut store = make_store();
                    let mut total_nodes = 0usize;
                    let mut total_edges = 0usize;

                    for chunk in inputs.chunks(batch_size) {
                        let parsed: Vec<ParsedFile> = chunk
                            .par_iter()
                            .filter_map(|(path, src, hash)| {
                                registry.parse(path, hash, src, None).map(|(pf, _)| pf)
                            })
                            .collect();

                        if !parsed.is_empty() {
                            let (n, e) = store
                                .replace_files_transactional(black_box(&parsed))
                                .expect("write");
                            total_nodes += n;
                            total_edges += e;
                        }
                    }
                    black_box((total_nodes, total_edges));
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_parse_only,
    bench_write_only,
    bench_full_pipeline
);
criterion_main!(benches);
