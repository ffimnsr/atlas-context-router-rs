use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, SearchQuery};
use atlas_search::execute_query;
use atlas_store_sqlite::Store;
use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};

fn make_store() -> Store {
    let mut store = Store::open(":memory:").expect("open in-memory store");
    store.migrate().expect("migrate in-memory store");
    store
}

fn make_node(file_path: &str, qn: &str, name: &str, is_public: bool) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: name.to_owned(),
        qualified_name: qn.to_owned(),
        file_path: file_path.to_owned(),
        line_start: 1,
        line_end: 12,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("(request: Request)".to_owned()),
        return_type: Some("Response".to_owned()),
        modifiers: if is_public {
            Some("pub".to_owned())
        } else {
            None
        },
        is_test: false,
        file_hash: "bench-hash".to_owned(),
        extra_json: serde_json::Value::Null,
    }
}

fn make_call_edge(file_path: &str, src: &str, tgt: &str) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: src.to_owned(),
        target_qn: tgt.to_owned(),
        file_path: file_path.to_owned(),
        line: Some(2),
        confidence: 1.0,
        confidence_tier: Some("high".to_owned()),
        extra_json: serde_json::Value::Null,
    }
}

fn seed_store(module_count: usize) -> Store {
    let mut store = make_store();

    for module_idx in 0..module_count {
        let file_path = format!("src/module_{module_idx}.rs");
        let handler_qn = format!("{file_path}::fn::handle_request_{module_idx}");
        let service_qn = format!("{file_path}::fn::service_process_{module_idx}");
        let repo_qn = format!("{file_path}::fn::repo_fetch_{module_idx}");

        let nodes = vec![
            make_node(
                &file_path,
                &handler_qn,
                &format!("handle_request_{module_idx}"),
                true,
            ),
            make_node(
                &file_path,
                &service_qn,
                &format!("service_process_{module_idx}"),
                false,
            ),
            make_node(
                &file_path,
                &repo_qn,
                &format!("repo_fetch_{module_idx}"),
                false,
            ),
        ];
        let edges = vec![
            make_call_edge(&file_path, &handler_qn, &service_qn),
            make_call_edge(&file_path, &service_qn, &repo_qn),
        ];

        store
            .replace_file_graph(
                &file_path,
                &format!("hash-{module_idx}"),
                Some("rust"),
                None,
                &nodes,
                &edges,
            )
            .expect("seed query bench graph");
    }

    store
}

fn bench_query_modes(c: &mut Criterion) {
    let store = seed_store(120);
    let mut group = c.benchmark_group("search/query_modes");

    let modes = [
        (
            "fts",
            SearchQuery {
                text: "handle_request_42".to_owned(),
                limit: 20,
                ..Default::default()
            },
            false,
        ),
        (
            "regex",
            SearchQuery {
                regex_pattern: Some("^handle_request_(4[0-9]|5[0-9])$".to_owned()),
                limit: 20,
                ..Default::default()
            },
            false,
        ),
        (
            "fuzzy",
            SearchQuery {
                text: "handel_request_42".to_owned(),
                fuzzy_match: true,
                limit: 20,
                ..Default::default()
            },
            false,
        ),
        (
            "graph_expand",
            SearchQuery {
                text: "handle_request_42".to_owned(),
                graph_expand: true,
                graph_max_hops: 2,
                limit: 20,
                ..Default::default()
            },
            true,
        ),
    ];

    for (mode, query, semantic) in modes {
        group.bench_with_input(BenchmarkId::from_parameter(mode), &query, |b, query| {
            b.iter(|| {
                black_box(
                    execute_query(&store, black_box(query), semantic).expect("execute query"),
                );
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_query_modes);
criterion_main!(benches);
