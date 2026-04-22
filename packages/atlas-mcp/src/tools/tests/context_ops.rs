use super::*;

#[test]
fn get_context_missing_args_returns_error() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let _ = Store::open(&db_path).expect("open store");

    let result = call(
        "get_context",
        Some(&serde_json::json!({})),
        "/ignored",
        &db_path,
    );
    assert!(
        result.is_err(),
        "empty get_context args must return an error"
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
    };
    store
        .replace_file_graph("src/math.rs", "h1", Some("rust"), Some(5), &[node], &[])
        .expect("replace_file_graph");

    let args = serde_json::json!({ "query": "compute", "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
    let text = unwrap_tool_text(resp);
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
}

#[test]
fn get_context_files_returns_review_intent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let _ = Store::open(&db_path).expect("open store");

    let args = serde_json::json!({ "files": ["src/main.rs"], "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
    let text = unwrap_tool_text(resp);
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
    let _ = Store::open(&db_path).expect("open store");

    let args =
        serde_json::json!({ "query": "nonexistent_xyz_unknown_symbol", "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    let node_count = v.get("node_count").and_then(|n| n.as_u64()).unwrap_or(99);
    assert_eq!(node_count, 0, "not-found query must return 0 nodes");
}

#[test]
fn get_context_defaults_to_toon_output_format() {
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
    };
    store
        .replace_file_graph("src/math.rs", "h1", Some("rust"), Some(5), &[node], &[])
        .expect("replace_file_graph");

    let args = serde_json::json!({ "query": "compute" });
    let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
    let text = unwrap_tool_text(resp.clone());

    assert_eq!(unwrap_tool_format(&resp), "toon");
    assert!(text.contains("intent: symbol"));
}

#[test]
fn explicit_json_override_beats_toon_default() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "query": "compute", "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let text = unwrap_tool_text(resp.clone());

    assert_eq!(unwrap_tool_format(&resp), "json");
    assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
}

#[test]
fn get_context_supports_toon_output_format() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "query": "compute", "output_format": "toon" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let text = unwrap_tool_text(resp.clone());

    assert_eq!(unwrap_tool_format(&resp), "toon");
    assert!(text.contains("intent: symbol"));
    assert!(text.contains("src/service.rs::fn::compute"));
    assert!(!text.contains("\"intent\""));
}

#[test]
fn explain_change_reports_change_kind_counts() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();

    let mut store = Store::open(&db_path).expect("open store");
    let node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "foo".to_owned(),
        qualified_name: "src/a.rs::fn::foo".to_owned(),
        file_path: "src/a.rs".to_owned(),
        line_start: 1,
        line_end: 3,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("x: i32".to_owned()),
        return_type: Some("i32".to_owned()),
        modifiers: Some("pub".to_owned()),
        is_test: false,
        file_hash: "h1".to_owned(),
        extra_json: serde_json::json!({}),
    };
    store
        .replace_file_graph("src/a.rs", "h1", Some("rust"), Some(10), &[node], &[])
        .expect("replace_file_graph");

    let args = serde_json::json!({
        "files": ["src/a.rs"],
        "max_depth": 5,
        "max_nodes": 200,
        "output_format": "json",
    });
    let resp = call("explain_change", Some(&args), "/ignored", &db_path).expect("call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        v.get("changed_file_count").and_then(|n| n.as_u64()),
        Some(1)
    );
    assert_eq!(
        v.get("changed_symbol_count").and_then(|n| n.as_u64()),
        Some(1)
    );
    assert_eq!(
        v.pointer("/changed_by_kind/signature_change")
            .and_then(|n| n.as_u64()),
        Some(1)
    );
    assert_eq!(
        v.pointer("/changed_symbols/0/change_kind")
            .and_then(|s| s.as_str()),
        Some("signature_change")
    );
    assert_eq!(
        v.pointer("/changed_symbols/0/qn").and_then(|s| s.as_str()),
        Some("src/a.rs::fn::foo")
    );
}

#[test]
fn mcp_agent_facing_flows_pass_usability_acceptance_gate() {
    let fixture = setup_mcp_fixture();

    let query_args = serde_json::json!({ "text": "compute" });
    let query_resp = call(
        "query_graph",
        Some(&query_args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("query_graph call");
    let query_text = unwrap_tool_text(query_resp.clone());
    let query_format = unwrap_tool_format(&query_resp);
    assert!(
        query_format == "toon" || query_format == "json",
        "expected toon or json, got {query_format}"
    );
    assert!(
        !query_text.is_empty(),
        "query_graph must return ranked results"
    );
    assert!(query_text.contains("src/service.rs::fn::compute"));
    assert_eq!(query_resp["atlas_result_kind"], "symbol_search");
    assert_eq!(query_resp["atlas_usage_edges_included"], false);
    assert!(
        query_resp["atlas_relationship_tools"]
            .as_array()
            .expect("relationship tools array")
            .iter()
            .any(|tool| tool.as_str() == Some("symbol_neighbors"))
    );

    let impact_args = serde_json::json!({ "files": ["src/service.rs"] });
    let impact_resp = call(
        "get_impact_radius",
        Some(&impact_args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("get_impact_radius call");
    let impact_text = unwrap_tool_text(impact_resp.clone());
    assert_eq!(unwrap_tool_format(&impact_resp), "toon");
    assert!(impact_resp.get("atlas_fallback_reason").is_none());
    assert!(impact_text.contains("changed_file_count: 1"));
    assert!(impact_text.contains("src/api.rs::fn::handle_request"));
    assert!(impact_text.contains("tests/service_test.rs::fn::compute_test"));

    let review_args = serde_json::json!({ "files": ["src/service.rs"] });
    let review_resp = call(
        "get_review_context",
        Some(&review_args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("get_review_context call");
    let review_text = unwrap_tool_text(review_resp.clone());
    assert_eq!(unwrap_tool_format(&review_resp), "toon");
    assert!(review_resp.get("atlas_fallback_reason").is_none());
    assert!(review_text.contains("intent: review"));
    assert!(review_text.contains("file_count:"));
    assert!(review_text.contains("src/service.rs"));
    assert!(review_text.contains("src/api.rs"));

    let context_args = serde_json::json!({ "query": "compute" });
    let context_resp = call(
        "get_context",
        Some(&context_args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("get_context call");
    let context_text = unwrap_tool_text(context_resp.clone());
    assert_eq!(unwrap_tool_format(&context_resp), "toon");
    assert!(context_resp.get("atlas_fallback_reason").is_none());
    assert!(context_text.contains("intent: symbol"));
    assert!(context_text.contains("src/service.rs::fn::compute"));
    assert!(context_text.contains("src/api.rs::fn::handle_request"));
}

#[test]
fn get_impact_radius_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "files": ["src/service.rs"] });
    let resp = call("get_impact_radius", Some(&args), "/repo", &fixture.db_path)
        .expect("get_impact_radius");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn get_review_context_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "files": ["src/service.rs"] });
    let resp = call("get_review_context", Some(&args), "/repo", &fixture.db_path)
        .expect("get_review_context");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn get_context_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "query": "compute", "output_format": "json" });
    let resp = call("get_context", Some(&args), "/repo", &fixture.db_path).expect("get_context");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}
