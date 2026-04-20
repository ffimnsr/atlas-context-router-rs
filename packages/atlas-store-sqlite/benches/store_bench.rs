use criterion::{BatchSize, BenchmarkId, Criterion, criterion_group, criterion_main};

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile, SearchQuery};
use atlas_store_sqlite::Store;

// ---------------------------------------------------------------------------
// Fixture helpers
// ---------------------------------------------------------------------------

fn make_store() -> Store {
    Store::open(":memory:").expect("open in-memory store")
}

fn make_nodes(file: &str, count: usize) -> Vec<Node> {
    (0..count)
        .map(|i| Node {
            id: NodeId(0),
            kind: if i % 3 == 0 {
                NodeKind::Function
            } else if i % 3 == 1 {
                NodeKind::Struct
            } else {
                NodeKind::Method
            },
            name: format!("symbol_{i}"),
            qualified_name: format!("{file}::fn::symbol_{i}"),
            file_path: file.to_owned(),
            line_start: (i as u32) * 10 + 1,
            line_end: (i as u32) * 10 + 5,
            language: "rust".to_owned(),
            parent_name: None,
            params: Some(format!("(arg_{i}: u32)")),
            return_type: Some("bool".to_owned()),
            modifiers: None,
            is_test: i % 7 == 0,
            file_hash: "abc123".to_owned(),
            extra_json: serde_json::Value::Null,
        })
        .collect()
}

fn make_edges(file: &str, nodes: &[Node]) -> Vec<Edge> {
    nodes
        .windows(2)
        .map(|w| Edge {
            id: 0,
            kind: EdgeKind::Calls,
            source_qn: w[0].qualified_name.clone(),
            target_qn: w[1].qualified_name.clone(),
            file_path: file.to_owned(),
            line: Some(w[0].line_start),
            confidence: 1.0,
            confidence_tier: None,
            extra_json: serde_json::Value::Null,
        })
        .collect()
}

fn make_parsed_file(path: &str, node_count: usize) -> ParsedFile {
    let nodes = make_nodes(path, node_count);
    let edges = make_edges(path, &nodes);
    ParsedFile {
        path: path.to_owned(),
        language: Some("rust".to_owned()),
        hash: "abc123".to_owned(),
        size: None,
        nodes,
        edges,
    }
}

/// Populate a store with `file_count` files of `nodes_per_file` nodes each,
/// returning the list of file paths for later use.
fn seed_store(store: &mut Store, file_count: usize, nodes_per_file: usize) -> Vec<String> {
    let files: Vec<ParsedFile> = (0..file_count)
        .map(|i| make_parsed_file(&format!("src/module_{i}.rs"), nodes_per_file))
        .collect();
    let paths: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
    store
        .replace_files_transactional(&files)
        .expect("seed store");
    paths
}

// ---------------------------------------------------------------------------
// 15.1  Build/write performance
// ---------------------------------------------------------------------------

fn bench_replace_single_file(c: &mut Criterion) {
    let file = make_parsed_file("src/lib.rs", 50);
    c.bench_function("store/replace_single_file_50_nodes", |b| {
        b.iter_batched(
            make_store,
            |mut s| {
                s.replace_files_transactional(std::slice::from_ref(&file))
                    .expect("replace");
            },
            BatchSize::SmallInput,
        );
    });
}

fn bench_replace_batch(c: &mut Criterion) {
    let mut group = c.benchmark_group("store/replace_batch");
    for size in [10usize, 50, 100] {
        let files: Vec<ParsedFile> = (0..size)
            .map(|i| make_parsed_file(&format!("src/mod_{i}.rs"), 20))
            .collect();
        group.bench_with_input(BenchmarkId::from_parameter(size), &files, |b, fs| {
            b.iter_batched(
                make_store,
                |mut s| {
                    s.replace_files_transactional(fs).expect("replace");
                },
                BatchSize::SmallInput,
            );
        });
    }
    group.finish();
}

fn bench_db_write_throughput(c: &mut Criterion) {
    // 100 files × 30 nodes = 3000 nodes; measures raw write nodes/sec baseline.
    let files: Vec<ParsedFile> = (0..100)
        .map(|i| make_parsed_file(&format!("src/file_{i}.rs"), 30))
        .collect();
    c.bench_function("store/write_3000_nodes_100_files", |b| {
        b.iter_batched(
            make_store,
            |mut s| {
                s.replace_files_transactional(&files).expect("write");
            },
            BatchSize::SmallInput,
        );
    });
}

// ---------------------------------------------------------------------------
// 15.2  Query performance
// ---------------------------------------------------------------------------

fn bench_fts_query(c: &mut Criterion) {
    let mut store = make_store();
    seed_store(&mut store, 50, 20); // 1000 nodes

    c.bench_function("store/fts_search_1000_nodes", |b| {
        let query = SearchQuery {
            text: "symbol_1".to_owned(),
            limit: 20,
            ..Default::default()
        };
        b.iter(|| store.search(&query).expect("search"));
    });
}

fn bench_impact_radius(c: &mut Criterion) {
    let mut store = make_store();
    let paths = seed_store(&mut store, 30, 15); // 450 nodes in a chain

    c.bench_function("store/impact_radius_450_nodes", |b| {
        let seed: Vec<&str> = paths[..3].iter().map(String::as_str).collect();
        b.iter(|| store.impact_radius(&seed, 5, 200).expect("impact"));
    });
}

fn bench_find_dependents(c: &mut Criterion) {
    let mut store = make_store();
    let paths = seed_store(&mut store, 30, 15);

    c.bench_function("store/find_dependents_30_files", |b| {
        let seed: Vec<&str> = paths[..5].iter().map(String::as_str).collect();
        b.iter(|| store.find_dependents(&seed).expect("dependents"));
    });
}

// ---------------------------------------------------------------------------

criterion_group!(
    benches,
    bench_replace_single_file,
    bench_replace_batch,
    bench_db_write_throughput,
    bench_fts_query,
    bench_impact_radius,
    bench_find_dependents,
);
criterion_main!(benches);
