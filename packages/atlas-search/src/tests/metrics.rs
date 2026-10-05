use super::*;
use atlas_core::{Node, NodeId, NodeKind, SearchQuery};

#[test]
fn execute_query_records_query_metrics_by_mode() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let mut store = Store::open(&db_path).expect("open store");

    let function = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "compute".to_string(),
        qualified_name: "src/service.rs::fn::compute".to_string(),
        file_path: "src/service.rs".to_string(),
        line_start: 1,
        line_end: 3,
        language: "rust".to_string(),
        parent_name: None,
        params: Some("()".to_string()),
        return_type: None,
        modifiers: Some("pub".to_string()),
        is_test: false,
        file_hash: "h1".to_string(),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    };
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "src/service.rs",
            "h1",
            Some("rust"),
            Some(3),
            &[function],
            &[],
        )
        .expect("replace function graph");

    let before = atlas_metrics::snapshot();
    let query = SearchQuery {
        text: "compute".to_string(),
        limit: 10,
        ..SearchQuery::default()
    };
    let results = execute_query(&store, &query, false).expect("execute query");
    assert!(!results.is_empty(), "expected query results");
    let after = atlas_metrics::snapshot();

    let calls_before = before.query_calls.get("fts5").copied().unwrap_or(0);
    let calls_after = after.query_calls.get("fts5").copied().unwrap_or(0);
    assert!(
        calls_after > calls_before,
        "execute_query must record a mode-labelled query call"
    );

    let latency_before = before
        .query_duration_ms
        .get("fts5")
        .map(|histogram| histogram.count)
        .unwrap_or(0);
    let latency_after = after
        .query_duration_ms
        .get("fts5")
        .map(|histogram| histogram.count)
        .unwrap_or(0);
    assert!(
        latency_after > latency_before,
        "execute_query must record mode-labelled query latency"
    );
}
