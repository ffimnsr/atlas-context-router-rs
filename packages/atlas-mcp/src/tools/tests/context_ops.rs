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
    assert!(resp.get("atlas_context_ranking_evidence_legend").is_some());
}

#[test]
fn get_context_files_returns_review_intent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let _ = Store::open(&db_path).expect("open store");

    let args = serde_json::json!({ "files": ["src/main.rs"], "output_format": "json" });
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
fn postprocess_graph_returns_noop_when_graph_missing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    git_run(root, &["init", "--quiet"]);
    git_run(root, &["config", "user.email", "atlas-tests@example.com"]);
    git_run(root, &["config", "user.name", "Atlas Tests"]);
    write_repo_file(root, "src/lib.rs", "pub fn helper() {}\n");
    git_run(root, &["add", "-A"]);
    git_run(root, &["commit", "--quiet", "-m", "initial"]);

    let db_path = root.join("atlas.db").to_string_lossy().to_string();
    let _ = Store::open(&db_path).expect("open store");
    let repo_root = root.to_string_lossy().to_string();
    let args = serde_json::json!({ "output_format": "json" });
    let response = call("postprocess_graph", Some(&args), &repo_root, &db_path)
        .expect("postprocess_graph call");
    let text = unwrap_tool_text(response);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(value["ok"], serde_json::json!(true));
    assert_eq!(value["noop"], serde_json::json!(true));
    assert_eq!(value["graph_built"], serde_json::json!(false));
}

#[test]
fn postprocess_graph_surfaces_unknown_stage_error_code() {
    let fixture = setup_git_mcp_fixture();
    let args = serde_json::json!({ "stage": "not_real", "output_format": "json" });
    let response = call(
        "postprocess_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("postprocess_graph call");
    let text = unwrap_tool_text(response);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(value["ok"], serde_json::json!(false));
    assert_eq!(value["error_code"], serde_json::json!("unknown_stage"));
}

#[test]
fn postprocess_graph_supports_single_stage_changed_only() {
    let fixture = setup_git_mcp_fixture();
    let long_file = "pub fn compute() -> i32 {\n".to_string()
        + &"    let value = 1;\n".repeat(45)
        + "    value\n}\n";
    write_repo_file(fixture._dir.path(), "src/service.rs", &long_file);

    let args = serde_json::json!({
        "changed_only": true,
        "stage": "large_function_summaries",
        "output_format": "json"
    });
    let response = call(
        "postprocess_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("postprocess_graph call");
    let text = unwrap_tool_text(response);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(value["requested_mode"], serde_json::json!("changed_only"));
    assert_eq!(
        value["stage_filter"],
        serde_json::json!("large_function_summaries")
    );
    assert_eq!(value["stages"].as_array().map(Vec::len), Some(1));
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
    let args = serde_json::json!({ "files": ["src/service.rs"], "output_format": "json" });
    let resp = call("get_review_context", Some(&args), "/repo", &fixture.db_path)
        .expect("get_review_context");
    assert_provenance(&resp, "/repo", &fixture.db_path);
    assert!(resp.get("atlas_context_ranking_evidence_legend").is_some());
}

#[test]
fn get_review_context_json_includes_changed_symbol_evidence() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "files": ["src/service.rs"], "output_format": "json" });
    let resp = call("get_review_context", Some(&args), "/repo", &fixture.db_path)
        .expect("get_review_context");
    let text = unwrap_tool_text(resp.clone());
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    let direct_target = value["nodes"]
        .as_array()
        .expect("nodes")
        .iter()
        .find(|node| node["reason"] == "direct_target")
        .expect("direct target node");
    assert_eq!(
        direct_target["context_ranking_evidence"]["changed_symbol"].as_bool(),
        Some(true)
    );
    assert!(resp.get("atlas_context_ranking_evidence_legend").is_some());
}

#[test]
fn get_context_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "query": "compute", "output_format": "json" });
    let resp = call("get_context", Some(&args), "/repo", &fixture.db_path).expect("get_context");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn get_context_changed_code_file_emits_freshness_warning() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 42 }\n",
    );
    let args = serde_json::json!({ "query": "compute", "output_format": "json" });

    let resp = call(
        "get_context",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_context");

    assert_eq!(
        resp.pointer("/atlas_freshness/stale")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        resp.pointer("/atlas_freshness/stale_result_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
}

#[test]
fn get_review_context_changed_code_file_emits_freshness_warning() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 77 }\n",
    );
    let args = serde_json::json!({ "working_tree": true, "output_format": "json" });

    let resp = call(
        "get_review_context",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_review_context");

    assert_eq!(
        resp.pointer("/atlas_freshness/stale")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        resp.pointer("/atlas_freshness/stale_result_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
}

#[test]
fn get_impact_radius_changed_code_file_emits_freshness_warning() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 88 }\n",
    );
    let args = serde_json::json!({ "working_tree": true, "output_format": "json" });

    let resp = call(
        "get_impact_radius",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_impact_radius");

    assert_eq!(
        resp.pointer("/atlas_freshness/stale")
            .and_then(|value| value.as_bool()),
        Some(true)
    );
    assert_eq!(
        resp.pointer("/atlas_freshness/stale_result_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
}

#[test]
fn get_impact_radius_accepts_explicit_files_and_reports_change_source_metadata() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "files": ["src/service.rs"],
        "output_format": "json"
    });

    let resp = call("get_impact_radius", Some(&args), "/repo", &fixture.db_path)
        .expect("get_impact_radius");
    let text = unwrap_tool_text(resp.clone());
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        value.get("changed_file_count").and_then(|n| n.as_u64()),
        Some(1)
    );
    assert_eq!(
        resp.pointer("/atlas_change_source/mode")
            .and_then(|value| value.as_str()),
        Some("explicit_files")
    );
    assert_eq!(
        resp.pointer("/atlas_change_source/resolved_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
}

#[test]
fn get_review_context_accepts_explicit_files_and_reports_change_source_metadata() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "files": ["src/service.rs"],
        "output_format": "json"
    });

    let resp = call("get_review_context", Some(&args), "/repo", &fixture.db_path)
        .expect("get_review_context");
    let text = unwrap_tool_text(resp.clone());
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        value.get("intent").and_then(|intent| intent.as_str()),
        Some("review")
    );
    assert_eq!(
        resp.pointer("/atlas_change_source/mode")
            .and_then(|value| value.as_str()),
        Some("explicit_files")
    );
    assert_eq!(
        resp.pointer("/atlas_change_source/resolved_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
}

#[test]
fn get_impact_radius_resolves_base_diff_files() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 2 }\n",
    );
    let args = serde_json::json!({ "base": "HEAD", "output_format": "json" });

    let resp = call(
        "get_impact_radius",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_impact_radius");
    let text = unwrap_tool_text(resp.clone());
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        value.get("changed_file_count").and_then(|n| n.as_u64()),
        Some(1)
    );
    assert_eq!(
        resp.pointer("/atlas_change_source/mode")
            .and_then(|value| value.as_str()),
        Some("base_ref")
    );
    assert_eq!(
        resp.pointer("/atlas_change_source/resolved_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
}

#[test]
fn get_review_context_resolves_staged_diff_files() {
    let fixture = setup_git_mcp_fixture();
    let repo_root = std::path::Path::new(&fixture.repo_root);
    write_repo_file(
        repo_root,
        "src/service.rs",
        "pub fn compute() -> i32 { 3 }\n",
    );
    git_run(repo_root, &["add", "src/service.rs"]);
    let args = serde_json::json!({ "staged": true, "output_format": "json" });

    let resp = call(
        "get_review_context",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_review_context");
    let text = unwrap_tool_text(resp.clone());
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        value.get("intent").and_then(|intent| intent.as_str()),
        Some("review")
    );
    assert_eq!(
        resp.pointer("/atlas_change_source/mode")
            .and_then(|value| value.as_str()),
        Some("staged")
    );
    assert_eq!(
        resp.pointer("/atlas_change_source/resolved_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
}

#[test]
fn get_review_context_resolves_working_tree_diff_files() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 4 }\n",
    );
    let args = serde_json::json!({ "working_tree": true, "output_format": "json" });

    let resp = call(
        "get_review_context",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_review_context");

    assert_eq!(
        resp.pointer("/atlas_change_source/mode")
            .and_then(|value| value.as_str()),
        Some("working_tree")
    );
    assert_eq!(
        resp.pointer("/atlas_change_source/resolved_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
}

#[test]
fn get_impact_radius_empty_diff_returns_empty_result() {
    let fixture = setup_git_mcp_fixture();
    let args = serde_json::json!({ "working_tree": true, "output_format": "json" });

    let resp = call(
        "get_impact_radius",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_impact_radius");
    let text = unwrap_tool_text(resp.clone());
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        value.get("changed_file_count").and_then(|n| n.as_u64()),
        Some(0)
    );
    assert_eq!(
        value.get("impacted_file_count").and_then(|n| n.as_u64()),
        Some(0)
    );
    assert_eq!(
        resp.pointer("/atlas_change_source/mode")
            .and_then(|value| value.as_str()),
        Some("working_tree")
    );
    assert_eq!(
        resp.pointer("/atlas_change_source/resolved_files")
            .and_then(|value| value.as_array())
            .map(|items| items.len()),
        Some(0)
    );
}

#[test]
fn change_source_invalid_combinations_return_clear_errors() {
    let fixture = setup_mcp_fixture();

    let impact_err = call(
        "get_impact_radius",
        Some(&serde_json::json!({
            "files": ["src/service.rs"],
            "staged": true
        })),
        "/repo",
        &fixture.db_path,
    )
    .expect_err("impact must reject ambiguous change source");
    assert!(impact_err.to_string().contains(
        "ambiguous change source: provide either files or one of base/staged/working_tree"
    ));

    let review_err = call(
        "get_review_context",
        Some(&serde_json::json!({
            "base": "HEAD",
            "working_tree": true
        })),
        "/repo",
        &fixture.db_path,
    )
    .expect_err("review must reject ambiguous change source");
    assert!(
        review_err
            .to_string()
            .contains("ambiguous change source: base and working_tree cannot be combined")
    );
}

// ---------------------------------------------------------------------------
// MCP12 — Context detail controls
// ---------------------------------------------------------------------------

#[test]
fn get_context_response_includes_detail_controls_metadata() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "query": "compute", "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let controls = resp
        .get("atlas_detail_controls")
        .expect("atlas_detail_controls");
    assert!(
        controls.get("code_spans").is_some(),
        "code_spans key present"
    );
    assert!(controls.get("tests").is_some(), "tests key present");
    assert!(controls.get("imports").is_some(), "imports key present");
    assert!(controls.get("neighbors").is_some(), "neighbors key present");
    assert!(controls.get("semantic").is_some(), "semantic key present");
    assert!(
        controls.get("omitted_sections").is_some(),
        "omitted_sections key present"
    );
}

#[test]
fn get_context_default_omits_tests_code_spans_neighbors() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "query": "compute", "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let controls = &resp["atlas_detail_controls"];
    let omitted = controls["omitted_sections"].as_array().expect("array");
    let omitted_strs: Vec<&str> = omitted.iter().flat_map(|v| v.as_str()).collect();
    assert!(omitted_strs.contains(&"tests"), "tests omitted by default");
    assert!(
        omitted_strs.contains(&"code_spans"),
        "code_spans omitted by default"
    );
    assert!(
        omitted_strs.contains(&"neighbors"),
        "neighbors omitted by default"
    );
}

#[test]
fn get_context_tests_toggle_enables_test_nodes() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "query": "compute",
        "tests": true,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let controls = &resp["atlas_detail_controls"];
    assert_eq!(controls["tests"].as_bool(), Some(true));
    // omitted_sections must NOT contain "tests" when enabled
    let omitted = controls["omitted_sections"].as_array().expect("array");
    let omitted_strs: Vec<&str> = omitted.iter().flat_map(|v| v.as_str()).collect();
    assert!(
        !omitted_strs.contains(&"tests"),
        "tests not omitted when enabled"
    );
    // test node must appear in result
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    let nodes = v["nodes"].as_array().expect("nodes array");
    let has_test = nodes.iter().any(|n| {
        n.get("qn")
            .and_then(|q| q.as_str())
            .map(|q| q.contains("compute_test"))
            .unwrap_or(false)
    });
    assert!(has_test, "test node must appear when tests=true");
}

#[test]
fn get_context_tests_false_excludes_test_nodes() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "query": "compute",
        "tests": false,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    let nodes = v["nodes"].as_array().expect("nodes array");
    let has_test = nodes.iter().any(|n| {
        n.get("qn")
            .and_then(|q| q.as_str())
            .map(|q| q.contains("compute_test"))
            .unwrap_or(false)
    });
    assert!(!has_test, "test node must not appear when tests=false");
}

#[test]
fn get_context_code_spans_toggle_controls_line_ranges() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let mut store = Store::open(&db_path).expect("open store");
    let node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "render".to_owned(),
        qualified_name: "src/ui.rs::fn::render".to_owned(),
        file_path: "src/ui.rs".to_owned(),
        line_start: 10,
        line_end: 20,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: Some("pub".to_owned()),
        is_test: false,
        file_hash: "h1".to_owned(),
        extra_json: serde_json::json!({}),
    };
    store
        .replace_file_graph("src/ui.rs", "h1", Some("rust"), Some(20), &[node], &[])
        .expect("replace_file_graph");

    let with_spans =
        serde_json::json!({ "file": "src/ui.rs", "code_spans": true, "output_format": "json" });
    let resp_with = call("get_context", Some(&with_spans), "/ignored", &db_path).expect("call");
    assert_eq!(
        resp_with["atlas_detail_controls"]["code_spans"].as_bool(),
        Some(true)
    );

    let without_spans =
        serde_json::json!({ "file": "src/ui.rs", "code_spans": false, "output_format": "json" });
    let resp_without =
        call("get_context", Some(&without_spans), "/ignored", &db_path).expect("call");
    assert_eq!(
        resp_without["atlas_detail_controls"]["code_spans"].as_bool(),
        Some(false)
    );
    let omitted = resp_without["atlas_detail_controls"]["omitted_sections"]
        .as_array()
        .expect("array");
    let omitted_strs: Vec<&str> = omitted.iter().flat_map(|v| v.as_str()).collect();
    assert!(
        omitted_strs.contains(&"code_spans"),
        "code_spans in omitted_sections when disabled"
    );
}

#[test]
fn get_context_imports_false_reflected_in_controls() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "query": "compute", "imports": false, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    assert_eq!(
        resp["atlas_detail_controls"]["imports"].as_bool(),
        Some(false)
    );
}

#[test]
fn get_context_neighbors_toggle_reflected_in_controls() {
    let fixture = setup_mcp_fixture();
    let args =
        serde_json::json!({ "query": "compute", "neighbors": true, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    assert_eq!(
        resp["atlas_detail_controls"]["neighbors"].as_bool(),
        Some(true)
    );
    let omitted = resp["atlas_detail_controls"]["omitted_sections"]
        .as_array()
        .expect("array");
    let omitted_strs: Vec<&str> = omitted.iter().flat_map(|v| v.as_str()).collect();
    assert!(
        !omitted_strs.contains(&"neighbors"),
        "neighbors not omitted when enabled"
    );
}

#[test]
fn get_context_max_files_reflected_in_controls() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "query": "compute", "max_files": 3, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    assert_eq!(
        resp["atlas_detail_controls"]["max_files"].as_u64(),
        Some(3),
        "max_files echoed in controls"
    );
}

#[test]
fn get_context_combined_toggles_applied_correctly() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "query": "compute",
        "tests": true,
        "code_spans": true,
        "neighbors": true,
        "imports": false,
        "max_files": 5,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    let controls = &resp["atlas_detail_controls"];
    assert_eq!(controls["tests"].as_bool(), Some(true));
    assert_eq!(controls["code_spans"].as_bool(), Some(true));
    assert_eq!(controls["neighbors"].as_bool(), Some(true));
    assert_eq!(controls["imports"].as_bool(), Some(false));
    assert_eq!(controls["max_files"].as_u64(), Some(5));
    // none of the enabled sections should appear in omitted_sections
    let omitted = controls["omitted_sections"].as_array().expect("array");
    let omitted_strs: Vec<&str> = omitted.iter().flat_map(|v| v.as_str()).collect();
    assert!(!omitted_strs.contains(&"tests"));
    assert!(!omitted_strs.contains(&"code_spans"));
    assert!(!omitted_strs.contains(&"neighbors"));
}

#[test]
fn get_context_semantic_flag_reflected_in_controls() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "query": "compute", "semantic": true, "output_format": "json" });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    assert_eq!(
        resp["atlas_detail_controls"]["semantic"].as_bool(),
        Some(true),
        "semantic flag echoed in controls"
    );
}

#[test]
fn get_context_files_with_max_files_cap_respected() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "files": ["src/service.rs", "src/api.rs"],
        "max_files": 1,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), "/ignored", &fixture.db_path).expect("call");
    assert_eq!(
        resp["atlas_detail_controls"]["max_files"].as_u64(),
        Some(1),
        "max_files echoed in controls for files-mode request"
    );
}

#[test]
fn get_context_json_and_toon_output_both_include_controls() {
    let fixture = setup_mcp_fixture();

    let json_args =
        serde_json::json!({ "query": "compute", "tests": true, "output_format": "json" });
    let json_resp = call(
        "get_context",
        Some(&json_args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("call json");
    assert!(
        json_resp.get("atlas_detail_controls").is_some(),
        "json: controls present"
    );

    let toon_args = serde_json::json!({ "query": "compute", "tests": true });
    let toon_resp = call(
        "get_context",
        Some(&toon_args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("call toon");
    assert!(
        toon_resp.get("atlas_detail_controls").is_some(),
        "toon: controls present"
    );
}

#[test]
fn get_context_applies_mcp_response_byte_cap() {
    let fixture = setup_mcp_fixture();
    let atlas_dir = fixture._dir.path().join(".atlas");
    std::fs::create_dir_all(&atlas_dir).expect("create atlas dir");
    std::fs::write(
        atlas_dir.join("config.toml"),
        "[mcp]\nworker_threads = 2\ntool_timeout_ms = 300000\nmax_mcp_response_bytes = 2500\n",
    )
    .expect("write config");

    let repo_root = fixture._dir.path().to_string_lossy().to_string();
    let args = serde_json::json!({
        "query": "compute",
        "tests": true,
        "neighbors": true,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), &repo_root, &fixture.db_path).expect("call");

    let text = unwrap_tool_text(resp.clone());
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse tool body");
    assert_eq!(value["truncated"].as_bool(), Some(true));
    assert!(
        value["nodes"]
            .as_array()
            .is_some_and(|nodes| !nodes.is_empty()),
        "response cap must still retain some context nodes"
    );
    assert!(
        value["payload_truncation"]["omitted_byte_count"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "response cap must report omitted bytes"
    );
    assert_eq!(resp["budget_status"], "partial_result");
    assert_eq!(
        resp["budget_name"],
        "mcp_cli_payload_serialization.max_mcp_response_bytes"
    );
    assert_eq!(resp["safe_to_answer"], true);
}

#[test]
fn get_context_fails_closed_when_mcp_response_budget_is_impossible() {
    let fixture = setup_mcp_fixture();
    let atlas_dir = fixture._dir.path().join(".atlas");
    std::fs::create_dir_all(&atlas_dir).expect("create atlas dir");
    std::fs::write(
        atlas_dir.join("config.toml"),
        "[mcp]\nworker_threads = 2\ntool_timeout_ms = 300000\nmax_mcp_response_bytes = 64\n",
    )
    .expect("write config");

    let repo_root = fixture._dir.path().to_string_lossy().to_string();
    let args = serde_json::json!({
        "query": "compute",
        "output_format": "json"
    });
    let err = call("get_context", Some(&args), &repo_root, &fixture.db_path)
        .expect_err("response budget under impossible cap must fail closed");
    assert!(
        err.to_string().contains("max_mcp_response_bytes"),
        "unexpected error: {err:#}"
    );
}
