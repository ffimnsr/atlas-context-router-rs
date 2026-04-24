//! Retrieval quality and token-efficiency benchmarks (Patch R6).
//!
//! Measures recall@k, MRR, exact hit rate, context noise, and token
//! efficiency for the four retrieval modes over a synthetic labeled dataset.
//! Also benchmarks quality under fixed small/medium context budgets.
//!
//! Run with:
//!   cargo bench -p atlas-search --bench retrieval_eval_bench

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind};
use atlas_search::eval::{BudgetClass, RetrievalCase, RetrievalMode, evaluate};
use atlas_store_sqlite::Store;
use criterion::{Criterion, criterion_group, criterion_main};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_store() -> Store {
    let mut s = Store::open(":memory:").expect("open in-memory store");
    s.migrate().expect("migrate in-memory store");
    s
}

fn make_fn_node(file_path: &str, qn: &str, name: &str, is_pub: bool) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: name.to_owned(),
        qualified_name: qn.to_owned(),
        file_path: file_path.to_owned(),
        line_start: 1,
        line_end: 10,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: Some("Result<()>".to_owned()),
        modifiers: if is_pub { Some("pub".to_owned()) } else { None },
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

/// Build a labeled corpus with `module_count` modules.
///
/// Each module has three functions (`handle_request_N`, `service_process_N`,
/// `repo_fetch_N`) wired as a call chain.  The evaluation case for each module
/// queries the handler name and expects both the handler and the service as
/// relevant targets.
fn build_corpus(module_count: usize) -> (Store, Vec<RetrievalCase>) {
    let mut store = make_store();
    let mut cases = Vec::with_capacity(module_count);

    for m in 0..module_count {
        let file = format!("src/module_{m}.rs");
        let handler_qn = format!("{file}::fn::handle_request_{m}");
        let service_qn = format!("{file}::fn::service_process_{m}");
        let repo_qn = format!("{file}::fn::repo_fetch_{m}");

        let nodes = vec![
            make_fn_node(&file, &handler_qn, &format!("handle_request_{m}"), true),
            make_fn_node(&file, &service_qn, &format!("service_process_{m}"), false),
            make_fn_node(&file, &repo_qn, &format!("repo_fetch_{m}"), false),
        ];
        let edges = vec![
            make_call_edge(&file, &handler_qn, &service_qn),
            make_call_edge(&file, &service_qn, &repo_qn),
        ];

        store
            .replace_file_graph(
                &file,
                &format!("hash-{m}"),
                Some("rust"),
                None,
                &nodes,
                &edges,
            )
            .expect("seed graph");

        cases.push(RetrievalCase {
            query: format!("handle_request_{m}"),
            expected_targets: vec![handler_qn, service_qn],
        });
    }

    (store, cases)
}

// ---------------------------------------------------------------------------
// Benchmarks: retrieval mode comparison
// ---------------------------------------------------------------------------

fn bench_retrieval_modes(c: &mut Criterion) {
    let (store, cases) = build_corpus(50);
    let mut group = c.benchmark_group("retrieval_eval/modes");

    for mode in [
        RetrievalMode::LexicalOnly,
        RetrievalMode::GraphOnly,
        RetrievalMode::Hybrid,
        RetrievalMode::HybridGraphExpand,
    ] {
        group.bench_function(mode.label(), |b| {
            b.iter(|| evaluate(&store, &cases, mode, 10, 5, None).expect("evaluate"))
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Benchmarks: fixed-budget evaluation
// ---------------------------------------------------------------------------

fn bench_fixed_budget(c: &mut Criterion) {
    let (store, cases) = build_corpus(50);
    let mut group = c.benchmark_group("retrieval_eval/budget");

    for (budget_label, budget) in [
        ("small", BudgetClass::Small),
        ("medium", BudgetClass::Medium),
    ] {
        for mode in [
            RetrievalMode::LexicalOnly,
            RetrievalMode::GraphOnly,
            RetrievalMode::Hybrid,
            RetrievalMode::HybridGraphExpand,
        ] {
            let label = format!("{}/{budget_label}", mode.label());
            group.bench_function(&label, |b| {
                b.iter(|| evaluate(&store, &cases, mode, 10, 5, Some(budget)).expect("evaluate"))
            });
        }
    }

    group.finish();
}

criterion_group!(benches, bench_retrieval_modes, bench_fixed_budget);
criterion_main!(benches);
