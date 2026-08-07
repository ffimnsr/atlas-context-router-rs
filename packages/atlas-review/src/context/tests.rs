use super::*;
use atlas_contentstore::SourceMeta;
use atlas_core::{
    BudgetLimitRule, BudgetPolicy, BudgetStatus, EdgeKind, NodeId, NodeKind,
    model::{
        ContextIntent, ContextRequest, ContextTarget, Edge, Node, ParsedFile, SavedContextSource,
        SelectionReason, TruncationMeta,
    },
};
use atlas_store_sqlite::Store;

fn open_store() -> Store {
    let mut s = Store::open(":memory:").unwrap();
    s.migrate().unwrap();
    s
}

fn make_node(qname: &str, name: &str, file: &str, kind: NodeKind, parent: Option<&str>) -> Node {
    Node {
        id: NodeId::UNSET,
        kind,
        name: name.to_string(),
        qualified_name: qname.to_string(),
        file_path: file.to_string(),
        line_start: 1,
        line_end: 10,
        language: "rust".to_string(),
        parent_name: parent.map(String::from),
        params: None,
        return_type: None,
        modifiers: Some("pub".to_string()),
        is_test: false,
        file_hash: "abc".to_string(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

fn with_repo(mut node: Node, repo_id: &str) -> Node {
    node.extra_json = serde_json::json!({"repo_id": repo_id});
    node
}

fn make_edge(src: &str, tgt: &str, kind: EdgeKind, file: &str) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: src.to_string(),
        target_qn: tgt.to_string(),
        file_path: file.to_string(),
        line: None,
        confidence: 1.0,
        confidence_tier: None,
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

fn seed_graph(store: &mut Store) {
    let nodes = [
        make_node(
            "src/a.rs::fn_a",
            "fn_a",
            "src/a.rs",
            NodeKind::Function,
            None,
        ),
        make_node(
            "src/a.rs::fn_a_helper",
            "fn_a_helper",
            "src/a.rs",
            NodeKind::Function,
            Some("mod_a"),
        ),
        make_node(
            "src/b.rs::fn_b",
            "fn_b",
            "src/b.rs",
            NodeKind::Function,
            None,
        ),
        make_node(
            "src/b.rs::fn_c",
            "fn_c",
            "src/b.rs",
            NodeKind::Function,
            None,
        ),
        make_node(
            "tests/test_a.rs::test_fn_a",
            "test_fn_a",
            "tests/test_a.rs",
            NodeKind::Test,
            None,
        ),
    ];
    let edges = [
        make_edge(
            "src/a.rs::fn_a",
            "src/b.rs::fn_b",
            EdgeKind::Calls,
            "src/a.rs",
        ),
        make_edge(
            "src/b.rs::fn_b",
            "src/b.rs::fn_c",
            EdgeKind::Calls,
            "src/b.rs",
        ),
        make_edge(
            "tests/test_a.rs::test_fn_a",
            "src/a.rs::fn_a",
            EdgeKind::Tests,
            "tests/test_a.rs",
        ),
    ];
    let files: Vec<ParsedFile> = vec![
        ParsedFile {
            path: "src/a.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h1".to_string(),
            size: None,
            nodes: nodes[0..2].to_vec(),
            edges: edges[0..1].to_vec(),
        },
        ParsedFile {
            path: "src/b.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h2".to_string(),
            size: None,
            nodes: nodes[2..4].to_vec(),
            edges: edges[1..2].to_vec(),
        },
        ParsedFile {
            path: "tests/test_a.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h3".to_string(),
            size: None,
            nodes: nodes[4..5].to_vec(),
            edges: edges[2..3].to_vec(),
        },
    ];
    store.replace_batch(&files).unwrap();
}

fn saved_source_meta(id: &str) -> SourceMeta {
    SourceMeta {
        id: id.to_owned(),
        session_id: Some("sess-1".into()),
        agent_id: None,
        source_type: "review_context".into(),
        label: format!("artifact-{id}"),
        repo_root: Some("/repo".into()),
        repo_roots: vec!["/repo".into()],
        repo_id: None,
        repo_ids: vec![],
        identity_kind: "artifact_label".into(),
        identity_value: format!("artifact-{id}"),
    }
}

fn symbol_request(qname: &str) -> ContextRequest {
    ContextRequest {
        intent: ContextIntent::Symbol,
        target: ContextTarget::QualifiedName {
            qname: qname.to_string(),
        },
        include_tests: false,
        include_imports: false,
        include_neighbors: false,
        ..ContextRequest::default()
    }
}

mod budget;
mod content;
mod evidence;
mod resolve;
mod review;
mod spans_tests;
mod symbol_context;
mod token_accounting;
mod tokenizer_budget;
