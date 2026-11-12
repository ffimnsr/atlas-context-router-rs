use atlas_core::{
    Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile, RepoProvenance,
    model::{ContextIntent, ContextRequest, ContextTarget},
};
use atlas_review::ContextEngine;
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
    kind: NodeKind,
    line_start: u32,
    parent_name: Option<&str>,
) -> Node {
    Node {
        id: NodeId::UNSET,
        kind,
        name,
        qualified_name,
        file_path: file_path.to_owned(),
        line_start,
        line_end: line_start + 4,
        language: "rust".to_owned(),
        parent_name: parent_name.map(str::to_owned),
        params: Some("()".to_owned()),
        return_type: Some("()".to_owned()),
        modifiers: Some("pub".to_owned()),
        is_test: kind == NodeKind::Test,
        file_hash: "bench-hash".to_owned(),
        extra_json: serde_json::Value::Null,
        repo_provenance: Some(RepoProvenance::new("repo_test")),
    }
}

fn make_edge(file_path: &str, source_qn: String, target_qn: String, kind: EdgeKind) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn,
        target_qn,
        file_path: file_path.to_owned(),
        line: Some(1),
        confidence: 1.0,
        confidence_tier: Some("high".to_owned()),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

fn seed_context_graph(store: &mut Store, module_count: usize) -> String {
    let mut parsed_files = Vec::with_capacity(module_count * 2);
    for module_idx in 0..module_count {
        let source_file = format!("src/module_{module_idx}.rs");
        let module_scope = format!("module_{module_idx}");
        let entry_qn = format!("{source_file}::fn::entry_{module_idx}");
        let helper_qn = format!("{source_file}::fn::helper_{module_idx}");
        let sibling_qn = format!("{source_file}::fn::sibling_{module_idx}");

        let nodes = vec![
            make_node(
                &source_file,
                entry_qn.clone(),
                format!("entry_{module_idx}"),
                NodeKind::Function,
                1,
                Some(&module_scope),
            ),
            make_node(
                &source_file,
                helper_qn.clone(),
                format!("helper_{module_idx}"),
                NodeKind::Function,
                8,
                Some(&module_scope),
            ),
            make_node(
                &source_file,
                sibling_qn.clone(),
                format!("sibling_{module_idx}"),
                NodeKind::Function,
                16,
                Some(&module_scope),
            ),
        ];

        let mut edges = vec![make_edge(
            &source_file,
            entry_qn.clone(),
            helper_qn.clone(),
            EdgeKind::Calls,
        )];
        if module_idx + 1 < module_count {
            let next_qn = format!(
                "src/module_{}.rs::fn::entry_{}",
                module_idx + 1,
                module_idx + 1
            );
            edges.push(make_edge(&source_file, helper_qn, next_qn, EdgeKind::Calls));
        }

        parsed_files.push(ParsedFile {
            path: source_file.clone(),
            language: Some("rust".to_owned()),
            hash: format!("source-hash-{module_idx}"),
            size: None,
            nodes,
            edges,
        });

        let test_file = format!("tests/module_{module_idx}_test.rs");
        let test_qn = format!("{test_file}::test::entry_{module_idx}_works");
        parsed_files.push(ParsedFile {
            path: test_file.clone(),
            language: Some("rust".to_owned()),
            hash: format!("test-hash-{module_idx}"),
            size: None,
            nodes: vec![make_node(
                &test_file,
                test_qn.clone(),
                format!("entry_{module_idx}_works"),
                NodeKind::Test,
                1,
                None,
            )],
            edges: vec![make_edge(&test_file, test_qn, entry_qn, EdgeKind::Tests)],
        });
    }

    store
        .replace_files_transactional_for_repo("repo_test", &parsed_files)
        .expect("seed context graph");

    let seed_index = module_count / 2;
    format!("src/module_{seed_index}.rs::fn::entry_{seed_index}")
}

fn bench_context_retrieval_latency(c: &mut Criterion) {
    let mut group = c.benchmark_group("review/context_retrieval_latency");
    for module_count in [64usize, 256] {
        let mut store = make_store();
        let seed_qname = seed_context_graph(&mut store, module_count);
        let engine = ContextEngine::new(&store);
        let request = benchmark_request(&seed_qname);

        group.bench_with_input(
            BenchmarkId::from_parameter(module_count),
            &request,
            |b, request| {
                b.iter(|| {
                    black_box(engine.build(black_box(request)).expect("build context"));
                });
            },
        );
    }
    group.finish();
}

fn benchmark_request(seed_qname: &str) -> ContextRequest {
    ContextRequest {
        intent: ContextIntent::Symbol,
        target: ContextTarget::QualifiedName {
            qname: seed_qname.to_owned(),
        },
        max_nodes: Some(96),
        max_edges: Some(128),
        max_files: Some(48),
        depth: Some(2),
        include_tests: true,
        include_imports: false,
        include_neighbors: true,
        include_code_spans: true,
        include_callers: true,
        include_callees: true,
        include_telemetry: true,
        ..ContextRequest::default()
    }
}

/// Fixed telemetry corpus: symbol, file, review, change-set, oversized context,
/// and provenance-sensitive requests. Run with:
/// `cargo bench -p atlas-review --bench context_bench -- --noplot`
fn telemetry_corpus(store: &Store, seed_qname: &str) -> Vec<serde_json::Value> {
    let engine = ContextEngine::new(store);
    let seed_file = seed_qname.split("::").next().unwrap_or("src/lib.rs");
    let changed_files = vec![seed_file.to_owned()];
    let requests = vec![
        ("symbol", benchmark_request(seed_qname)),
        (
            "file",
            ContextRequest {
                intent: ContextIntent::File,
                target: ContextTarget::FilePath {
                    path: seed_file.to_owned(),
                },
                include_telemetry: true,
                include_code_spans: true,
                ..ContextRequest::default()
            },
        ),
        (
            "review",
            ContextRequest {
                intent: ContextIntent::Review,
                target: ContextTarget::ChangedFiles {
                    paths: changed_files.clone(),
                },
                include_telemetry: true,
                include_tests: true,
                ..ContextRequest::default()
            },
        ),
        (
            "change_set",
            ContextRequest {
                intent: ContextIntent::ImpactAnalysis,
                target: ContextTarget::ChangedSymbols {
                    qnames: vec![seed_qname.to_owned()],
                },
                include_telemetry: true,
                ..ContextRequest::default()
            },
        ),
        (
            "oversized_context",
            ContextRequest {
                token_budget: Some(32),
                max_nodes: Some(512),
                max_edges: Some(512),
                include_telemetry: true,
                ..benchmark_request(seed_qname)
            },
        ),
        (
            "provenance_sensitive",
            ContextRequest {
                intent: ContextIntent::Review,
                target: ContextTarget::ChangedFiles {
                    paths: changed_files,
                },
                max_nodes: Some(256),
                include_telemetry: true,
                include_code_spans: true,
                ..ContextRequest::default()
            },
        ),
    ];

    requests
        .into_iter()
        .map(|(name, request)| {
            let default_result = engine
                .build(&request)
                .expect("build default telemetry case");
            let mut evidence_request = request.clone();
            evidence_request.include_ranking_evidence = true;
            let evidence_result = engine
                .build(&evidence_request)
                .expect("build ranking-evidence telemetry case");
            let default_tokens = default_result
                .telemetry
                .as_ref()
                .map(|telemetry| telemetry.emitted_tokens)
                .unwrap_or(0);
            let evidence_tokens = evidence_result
                .telemetry
                .as_ref()
                .map(|telemetry| telemetry.emitted_tokens)
                .unwrap_or(0);
            let token_reduction_ratio = if evidence_tokens == 0 {
                0.0
            } else {
                1.0 - (default_tokens as f64 / evidence_tokens as f64)
            };
            let required_evidence_retained = default_result.nodes.iter().all(|node| {
                !node.node.qualified_name.is_empty()
                    && !node.node.file_path.is_empty()
                    && node.node.line_start <= node.node.line_end
                    && !node.selection_reason.as_str().is_empty()
            }) && default_result
                .files
                .iter()
                .all(|file| !file.path.is_empty() && !file.selection_reason.as_str().is_empty());
            serde_json::json!({
                "case": name,
                "default": default_result.telemetry,
                "with_ranking_evidence": evidence_result.telemetry,
                "token_reduction_ratio": token_reduction_ratio,
                "required_evidence_retained": required_evidence_retained,
            })
        })
        .collect()
}

fn bench_telemetry_corpus(c: &mut Criterion) {
    let mut store = make_store();
    let seed_qname = seed_context_graph(&mut store, 64);
    let corpus = telemetry_corpus(&store, &seed_qname);
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "corpus": corpus,
        }))
        .expect("serialize telemetry corpus")
    );

    let engine = ContextEngine::new(&store);
    let requests: Vec<ContextRequest> = vec![
        benchmark_request(&seed_qname),
        ContextRequest {
            intent: ContextIntent::File,
            target: ContextTarget::FilePath {
                path: seed_qname
                    .split("::")
                    .next()
                    .unwrap_or("src/lib.rs")
                    .to_owned(),
            },
            include_telemetry: true,
            ..ContextRequest::default()
        },
        ContextRequest {
            intent: ContextIntent::Review,
            target: ContextTarget::ChangedFiles {
                paths: vec![
                    seed_qname
                        .split("::")
                        .next()
                        .unwrap_or("src/lib.rs")
                        .to_owned(),
                ],
            },
            include_telemetry: true,
            ..ContextRequest::default()
        },
    ];
    let mut group = c.benchmark_group("review/context_telemetry_corpus");
    group.bench_function("fixed_corpus", |b| {
        b.iter(|| {
            for request in &requests {
                black_box(
                    engine
                        .build(black_box(request))
                        .expect("build telemetry case"),
                );
            }
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_context_retrieval_latency,
    bench_telemetry_corpus
);
criterion_main!(benches);
