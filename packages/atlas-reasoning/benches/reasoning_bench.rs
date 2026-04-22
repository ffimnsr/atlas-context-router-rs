use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use atlas_reasoning::ReasoningEngine;
use atlas_store_sqlite::Store;
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

fn make_store() -> Store {
    let mut store = Store::open(":memory:").expect("open in-memory store");
    store.migrate().expect("migrate in-memory store");
    store
}

fn make_node(
    file_path: &str,
    qualified_name: String,
    name: String,
    modifiers: Option<&str>,
    is_test: bool,
) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: if is_test {
            NodeKind::Test
        } else {
            NodeKind::Function
        },
        name,
        qualified_name,
        file_path: file_path.to_owned(),
        line_start: 1,
        line_end: 6,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: Some("()".to_owned()),
        modifiers: modifiers.map(str::to_owned),
        is_test,
        file_hash: "bench-hash".to_owned(),
        extra_json: serde_json::Value::Null,
    }
}

fn make_edge(file_path: &str, source_qn: String, target_qn: String, kind: EdgeKind) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn,
        target_qn,
        file_path: file_path.to_owned(),
        line: Some(2),
        confidence: 1.0,
        confidence_tier: Some("high".to_owned()),
        extra_json: serde_json::Value::Null,
    }
}

fn seed_reasoning_graph(store: &mut Store, module_count: usize) -> String {
    let mut parsed_files = Vec::with_capacity(module_count * 2);
    for module_idx in 0..module_count {
        let source_file = format!("src/module_{module_idx}.rs");
        let api_qn = format!("{source_file}::fn::api_{module_idx}");
        let worker_qn = format!("{source_file}::fn::worker_{module_idx}");
        let unused_qn = format!("{source_file}::fn::unused_{module_idx}");

        let nodes = vec![
            make_node(
                &source_file,
                api_qn.clone(),
                format!("api_{module_idx}"),
                Some("pub"),
                false,
            ),
            make_node(
                &source_file,
                worker_qn.clone(),
                format!("worker_{module_idx}"),
                None,
                false,
            ),
            make_node(
                &source_file,
                unused_qn,
                format!("unused_{module_idx}"),
                None,
                false,
            ),
        ];

        let mut edges = vec![make_edge(
            &source_file,
            api_qn.clone(),
            worker_qn.clone(),
            EdgeKind::Calls,
        )];
        if module_idx + 1 < module_count {
            let next_api_qn = format!(
                "src/module_{}.rs::fn::api_{}",
                module_idx + 1,
                module_idx + 1
            );
            edges.push(make_edge(
                &source_file,
                worker_qn.clone(),
                next_api_qn,
                EdgeKind::Calls,
            ));
        }

        parsed_files.push(ParsedFile {
            path: source_file.clone(),
            language: Some("rust".to_owned()),
            hash: format!("reason-hash-{module_idx}"),
            size: None,
            nodes,
            edges,
        });

        let test_file = format!("tests/module_{module_idx}_test.rs");
        let test_qn = format!("{test_file}::test::api_{module_idx}_works");
        parsed_files.push(ParsedFile {
            path: test_file.clone(),
            language: Some("rust".to_owned()),
            hash: format!("reason-test-hash-{module_idx}"),
            size: None,
            nodes: vec![make_node(
                &test_file,
                test_qn.clone(),
                format!("api_{module_idx}_works"),
                None,
                true,
            )],
            edges: vec![make_edge(&test_file, test_qn, api_qn, EdgeKind::Tests)],
        });
    }

    store
        .replace_files_transactional(&parsed_files)
        .expect("seed reasoning graph");

    let seed_index = module_count / 2;
    format!("src/module_{seed_index}.rs::fn::api_{seed_index}")
}

fn bench_impact_analysis_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("reasoning/impact_analysis_latency");
    for module_count in [128usize, 512] {
        let mut store = make_store();
        let seed_qname = seed_reasoning_graph(&mut store, module_count);
        let engine = ReasoningEngine::new(&store);
        let seed = [seed_qname.as_str()];

        group.bench_with_input(
            BenchmarkId::from_parameter(module_count),
            &seed,
            |b, seed| {
                b.iter(|| {
                    black_box(
                        engine
                            .analyze_removal(black_box(seed), Some(4), Some(256))
                            .expect("analyze removal"),
                    );
                });
            },
        );
    }
    group.finish();
}

fn bench_dead_code_scan_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("reasoning/dead_code_scan_latency");
    for module_count in [128usize, 512] {
        let mut store = make_store();
        seed_reasoning_graph(&mut store, module_count);
        let engine = ReasoningEngine::new(&store);

        group.bench_with_input(
            BenchmarkId::from_parameter(module_count),
            &module_count,
            |b, _| {
                b.iter(|| {
                    black_box(
                        engine
                            .detect_dead_code(&[], Some("src/"), Some(module_count * 2), &[])
                            .expect("detect dead code"),
                    );
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_impact_analysis_latency,
    bench_dead_code_scan_latency
);
criterion_main!(benches);
