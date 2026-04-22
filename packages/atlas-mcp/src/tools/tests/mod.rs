use super::{call, tool_list};
use crate::output::OutputFormat;
use atlas_core::EdgeKind;
use atlas_core::kinds::NodeKind;
use atlas_core::model::{Edge, Node, NodeId};
use atlas_store_sqlite::Store;
use tempfile::TempDir;

pub(super) use super::shared::{DEFAULT_OUTPUT_DESCRIPTION, tool_result_value};

mod analysis;
mod context_ops;
mod graph;
mod health;
mod registry;

pub(super) struct McpFixture {
    pub(super) _dir: TempDir,
    pub(super) db_path: String,
}

pub(super) fn make_node(kind: NodeKind, name: &str, qualified_name: &str, file_path: &str) -> Node {
    Node {
        id: NodeId::UNSET,
        kind,
        name: name.to_owned(),
        qualified_name: qualified_name.to_owned(),
        file_path: file_path.to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: Some("pub".to_owned()),
        is_test: kind == NodeKind::Test,
        file_hash: format!("hash:{file_path}"),
        extra_json: serde_json::json!({}),
    }
}

pub(super) fn make_edge(kind: EdgeKind, source_qn: &str, target_qn: &str, file_path: &str) -> Edge {
    Edge {
        id: 0,
        kind,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: file_path.to_owned(),
        line: Some(1),
        confidence: 1.0,
        confidence_tier: Some("high".to_owned()),
        extra_json: serde_json::json!({}),
    }
}

pub(super) fn setup_mcp_fixture() -> McpFixture {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();

    let mut store = Store::open(&db_path).expect("open store");

    let compute = make_node(
        NodeKind::Function,
        "compute",
        "src/service.rs::fn::compute",
        "src/service.rs",
    );
    store
        .replace_file_graph(
            "src/service.rs",
            "hash:src/service.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&compute),
            &[],
        )
        .expect("replace service graph");

    let handle = make_node(
        NodeKind::Function,
        "handle_request",
        "src/api.rs::fn::handle_request",
        "src/api.rs",
    );
    let handle_calls_compute = make_edge(
        EdgeKind::Calls,
        "src/api.rs::fn::handle_request",
        "src/service.rs::fn::compute",
        "src/api.rs",
    );
    store
        .replace_file_graph(
            "src/api.rs",
            "hash:src/api.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&handle),
            &[handle_calls_compute],
        )
        .expect("replace api graph");

    let compute_test = make_node(
        NodeKind::Test,
        "compute_test",
        "tests/service_test.rs::fn::compute_test",
        "tests/service_test.rs",
    );
    let test_targets_compute = make_edge(
        EdgeKind::Tests,
        "tests/service_test.rs::fn::compute_test",
        "src/service.rs::fn::compute",
        "tests/service_test.rs",
    );
    store
        .replace_file_graph(
            "tests/service_test.rs",
            "hash:tests/service_test.rs",
            Some("rust"),
            Some(5),
            std::slice::from_ref(&compute_test),
            &[test_targets_compute],
        )
        .expect("replace test graph");

    McpFixture { _dir: dir, db_path }
}

pub(super) fn unwrap_tool_text(resp: serde_json::Value) -> String {
    resp.get("content")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
        .and_then(|c0| c0.get("text"))
        .and_then(|t| t.as_str())
        .expect("tool response content[0].text")
        .to_owned()
}

pub(super) fn unwrap_tool_format(resp: &serde_json::Value) -> &str {
    resp.get("atlas_output_format")
        .and_then(|value| value.as_str())
        .expect("tool response atlas_output_format")
}

pub(super) fn assert_provenance(resp: &serde_json::Value, expected_repo: &str, expected_db: &str) {
    let prov = resp
        .get("atlas_provenance")
        .expect("atlas_provenance must be present");
    assert_eq!(
        prov.get("repo_root").and_then(|v| v.as_str()),
        Some(expected_repo),
        "provenance.repo_root mismatch"
    );
    assert_eq!(
        prov.get("db_path").and_then(|v| v.as_str()),
        Some(expected_db),
        "provenance.db_path mismatch"
    );
    assert!(
        prov.get("indexed_file_count")
            .and_then(|v| v.as_i64())
            .is_some(),
        "provenance.indexed_file_count must be an integer"
    );
    assert!(
        prov.get("last_indexed_at").is_some(),
        "provenance.last_indexed_at key must be present"
    );
}
