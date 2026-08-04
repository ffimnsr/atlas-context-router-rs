use super::*;

#[test]
fn get_context_missing_args_returns_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let store = Store::open(&db_path).expect("open store");
    // Mark graph as built so the readiness check passes and the tool itself
    // handles the missing-args validation (rather than being blocked early).
    store
        .finish_build(
            "/ignored",
            atlas_store_sqlite::BuildFinishStats {
                state: atlas_store_sqlite::GraphBuildState::Built,
                files_discovered: 0,
                files_processed: 0,
                files_accepted: 0,
                files_skipped_by_byte_budget: 0,
                files_failed: 0,
                bytes_accepted: 0,
                bytes_skipped: 0,
                nodes_written: 0,
                edges_written: 0,
                budget_stop_reason: None,
            },
        )
        .expect("finish_build");

    let result = call(
        "get_context",
        Some(&serde_json::json!({})),
        "/ignored",
        &db_path,
    )
    .expect("empty get_context args must return tool error result");
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
}

#[test]
fn get_context_query_returns_packaged_result() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();

    let mut store = Store::open(&db_path).expect("open store");
    let node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "compute".to_owned(),
        qualified_name: "src/math.rs::fn::compute".to_owned(),
        file_path: "src/math.rs".to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("(x: i32) -> i32".to_owned()),
        return_type: Some("i32".to_owned()),
        modifiers: Some("pub".to_owned()),
        is_test: false,
        file_hash: "h1".to_owned(),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    };
    store
        .replace_file_graph("src/math.rs", "h1", Some("rust"), Some(5), &[node], &[])
        .expect("replace_file_graph");

    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
    let text = unwrap_tool_text(resp.clone());
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert!(v.get("intent").is_some(), "result must have intent");
    assert!(v.get("node_count").is_some(), "result must have node_count");
    assert!(
        v.get("nodes").and_then(|n| n.as_array()).is_some(),
        "nodes must be array"
    );
    assert!(
        v.get("truncated").is_some(),
        "result must have truncated flag"
    );
    assert!(
        v["nodes"]
            .as_array()
            .and_then(|nodes| nodes.first())
            .and_then(|node| node.get("context_ranking_evidence"))
            .is_some(),
        "packaged context node must include context ranking evidence"
    );
    assert!(
        resp["structuredContent"]
            .get("ranking_evidence_legend")
            .is_some()
    );
    assert_eq!(v["target"]["kind"], serde_json::json!("query"));
    assert_eq!(v["target"]["query"], serde_json::json!("compute"));
    assert!(resp["_meta"].get("deprecated_input_fields").is_none());
}

#[test]
fn get_context_accepts_target_object_and_reports_normalized_target() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();

    let mut store = Store::open(&db_path).expect("open store");
    let node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "compute".to_owned(),
        qualified_name: "src/math.rs::fn::compute".to_owned(),
        file_path: "src/math.rs".to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("(x: i32) -> i32".to_owned()),
        return_type: Some("i32".to_owned()),
        modifiers: Some("pub".to_owned()),
        is_test: false,
        file_hash: "h1".to_owned(),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    };
    store
        .replace_file_graph("src/math.rs", "h1", Some("rust"), Some(5), &[node], &[])
        .expect("replace_file_graph");

    let args = serde_json::json!({
        "target": { "kind": "query", "query": "compute" },
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
    let v: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(resp.clone())).expect("parse json");

    assert_eq!(v["target"]["kind"], serde_json::json!("query"));
    assert_eq!(v["target"]["query"], serde_json::json!("compute"));
    assert!(resp["_meta"].get("deprecated_input_fields").is_none());
}

#[test]
fn get_context_accepts_supported_query_intent_phrases() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();

    let mut store = Store::open(&db_path).expect("open store");
    let node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "compute".to_owned(),
        qualified_name: "src/math.rs::fn::compute".to_owned(),
        file_path: "src/math.rs".to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("(x: i32) -> i32".to_owned()),
        return_type: Some("i32".to_owned()),
        modifiers: Some("pub".to_owned()),
        is_test: false,
        file_hash: "h1".to_owned(),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    };
    store
        .replace_file_graph("src/math.rs", "h1", Some("rust"), Some(5), &[node], &[])
        .expect("replace_file_graph");

    let resp = call(
        "get_context",
        Some(&serde_json::json!({
            "target": { "kind": "query", "query": "who calls compute" },
            "output_format": "json"
        })),
        "/ignored",
        &db_path,
    )
    .expect("get_context who calls");
    let v: serde_json::Value = serde_json::from_str(&unwrap_tool_text(resp)).expect("parse json");
    assert_eq!(v["target"]["kind"], serde_json::json!("query"));
    assert_eq!(v["target"]["query"], serde_json::json!("who calls compute"));
}

#[test]
fn get_context_rejects_natural_language_only_query_descriptions() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let store = Store::open(&db_path).expect("open store");
    store
        .finish_build(
            "/ignored",
            atlas_store_sqlite::BuildFinishStats {
                state: atlas_store_sqlite::GraphBuildState::Built,
                files_discovered: 0,
                files_processed: 0,
                files_accepted: 0,
                files_skipped_by_byte_budget: 0,
                files_failed: 0,
                bytes_accepted: 0,
                bytes_skipped: 0,
                nodes_written: 0,
                edges_written: 0,
                budget_stop_reason: None,
            },
        )
        .expect("finish_build");

    let result = call(
        "get_context",
        Some(&serde_json::json!({
            "target": { "kind": "query", "query": "please show me authentication flow" },
            "output_format": "json"
        })),
        "/ignored",
        &db_path,
    )
    .expect("invalid get_context query must return tool result");
    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["message"],
        serde_json::json!(
            "target.query must be exact identifier, qualified name, or supported intent phrase"
        )
    );
}

#[test]
fn get_context_rejects_legacy_target_fields() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let store = Store::open(&db_path).expect("open store");
    store
        .finish_build(
            "/ignored",
            atlas_store_sqlite::BuildFinishStats {
                state: atlas_store_sqlite::GraphBuildState::Built,
                files_discovered: 0,
                files_processed: 0,
                files_accepted: 0,
                files_skipped_by_byte_budget: 0,
                files_failed: 0,
                bytes_accepted: 0,
                bytes_skipped: 0,
                nodes_written: 0,
                edges_written: 0,
                budget_stop_reason: None,
            },
        )
        .expect("finish_build");

    let result = call(
        "get_context",
        Some(&serde_json::json!({
            "query": "compute",
            "output_format": "json"
        })),
        "/ignored",
        &db_path,
    )
    .expect("tool result");

    assert_eq!(result["isError"], serde_json::json!(true));
    assert_eq!(
        result["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert_eq!(
        result["structuredContent"]["message"],
        serde_json::json!("legacy get_context target fields are no longer supported")
    );
}

#[test]
fn get_context_files_returns_review_intent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let store = Store::open(&db_path).expect("open store");
    // A built (empty) graph is sufficient to pass readiness; the test only
    // checks that the `files` argument sets intent=review.
    store
        .finish_build(
            "/ignored",
            atlas_store_sqlite::BuildFinishStats {
                state: atlas_store_sqlite::GraphBuildState::Built,
                files_discovered: 0,
                files_processed: 0,
                files_accepted: 0,
                files_skipped_by_byte_budget: 0,
                files_failed: 0,
                bytes_accepted: 0,
                bytes_skipped: 0,
                nodes_written: 0,
                edges_written: 0,
                budget_stop_reason: None,
            },
        )
        .expect("finish_build");

    let args = serde_json::json!({ "target": { "kind": "files", "files": ["src/main.rs"] }, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
    let text = unwrap_tool_text(resp.clone());
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        v.get("intent").and_then(|i| i.as_str()),
        Some("review"),
        "files arg must produce review intent"
    );
}

#[test]
fn get_context_not_found_returns_empty_nodes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let store = Store::open(&db_path).expect("open store");
    // A built (empty) graph is sufficient to pass readiness; the test only
    // checks that an unknown query returns 0 nodes.
    store
        .finish_build(
            "/ignored",
            atlas_store_sqlite::BuildFinishStats {
                state: atlas_store_sqlite::GraphBuildState::Built,
                files_discovered: 0,
                files_processed: 0,
                files_accepted: 0,
                files_skipped_by_byte_budget: 0,
                files_failed: 0,
                bytes_accepted: 0,
                bytes_skipped: 0,
                nodes_written: 0,
                edges_written: 0,
                budget_stop_reason: None,
            },
        )
        .expect("finish_build");

    let args = serde_json::json!({ "target": { "kind": "query", "query": "nonexistent_xyz_unknown_symbol" }, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let node_count = v.get("node_count").and_then(|n| n.as_u64()).unwrap_or(99);
    assert_eq!(node_count, 0, "not-found query must return 0 nodes");
}

#[test]
fn get_context_defaults_to_json_output() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();

    let mut store = Store::open(&db_path).expect("open store");
    let node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "compute".to_owned(),
        qualified_name: "src/math.rs::fn::compute".to_owned(),
        file_path: "src/math.rs".to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("(x: i32) -> i32".to_owned()),
        return_type: Some("i32".to_owned()),
        modifiers: Some("pub".to_owned()),
        is_test: false,
        file_hash: "h1".to_owned(),
        extra_json: serde_json::json!({}),
        repo_provenance: None,
    };
    store
        .replace_file_graph("src/math.rs", "h1", Some("rust"), Some(5), &[node], &[])
        .expect("replace_file_graph");

    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" } });
    let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
    let text = unwrap_tool_text(resp.clone());

    assert_eq!(unwrap_tool_format(&resp), "json");
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(value["intent"], serde_json::json!("symbol"));
}

#[test]
fn explicit_json_argument_is_ignored_and_stays_json() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let text = unwrap_tool_text(resp.clone());

    assert_eq!(unwrap_tool_format(&resp), "json");
    assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
}

#[test]
fn get_context_ignores_stale_toon_output_argument() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "output_format": "toon" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let text = unwrap_tool_text(resp.clone());
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(unwrap_tool_format(&resp), "json");
    assert_eq!(value["intent"], serde_json::json!("symbol"));
    assert!(value["nodes"].as_array().is_some_and(|nodes| {
        nodes
            .iter()
            .any(|node| node["qn"] == serde_json::json!("src/service.rs::fn::compute"))
    }));
}

#[test]
fn query_graph_legacy_repo_scope_is_rejected() {
    let fixture = setup_mcp_fixture();
    let resp = call(
        "query_graph",
        Some(&serde_json::json!({
            "text": "compute",
            "all_repos": true,
            "output_format": "json"
        })),
        "/ignored",
        &fixture.db_path,
    )
    .expect("call");

    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["message"],
        serde_json::json!("legacy repo scope fields are no longer supported")
    );
}
