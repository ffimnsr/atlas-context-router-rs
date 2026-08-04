use super::*;

#[test]
fn change_source_tools_resolve_files_for_canonical_inputs() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        fixture._dir.path(),
        "src/service.rs",
        "pub fn compute() -> i32 { 2 }\n",
    );

    let detect = call(
        "detect_changes",
        Some(&serde_json::json!({
            "change_source": { "kind": "working_tree" },
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("detect");
    let detect_payload: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(detect)).expect("detect json");
    assert!(
        detect_payload["files"]
            .as_array()
            .is_some_and(|files| !files.is_empty())
    );

    let minimal = call(
        "get_minimal_context",
        Some(&serde_json::json!({
            "change_source": { "kind": "working_tree" },
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("minimal");
    let minimal_payload: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(minimal)).expect("minimal json");
    assert_eq!(
        minimal_payload["summary"]["changed_file_count"],
        serde_json::json!(1)
    );
}

#[test]
fn review_impact_and_explain_change_accept_canonical_change_source() {
    let fixture = setup_git_mcp_fixture();

    let review = call(
        "get_review_context",
        Some(&serde_json::json!({
            "change_source": { "kind": "files", "files": ["src/service.rs"] },
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("review");
    let review_payload: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(review)).expect("review json");
    assert_eq!(
        review_payload["change_source"]["kind"],
        serde_json::json!("files")
    );

    let impact = call(
        "get_impact_radius",
        Some(&serde_json::json!({
            "change_source": { "kind": "files", "files": ["src/service.rs"] },
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("impact");
    let impact_payload: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(impact)).expect("impact json");
    assert_eq!(
        impact_payload["seed_files"],
        serde_json::json!(["src/service.rs"])
    );

    let explain = call(
        "explain_change",
        Some(&serde_json::json!({
            "change_source": { "kind": "files", "files": ["src/service.rs"] },
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("explain");
    let explain_payload: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(explain)).expect("explain json");
    assert_eq!(
        explain_payload["change_source"]["kind"],
        serde_json::json!("files")
    );
}

#[test]
fn review_and_impact_context_report_cross_repo_hops() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db").to_string_lossy().to_string();
    let mut store = Store::open(&db_path).expect("open store");

    let root_node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "call_dep".to_owned(),
        qualified_name: "src/app.rs::fn::call_dep".to_owned(),
        file_path: "src/app.rs".to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: Some("pub".to_owned()),
        is_test: false,
        file_hash: "h-app".to_owned(),
        extra_json: serde_json::json!({"repo_id": "repo_root"}),
        repo_provenance: None,
    };
    let dep_node = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "dep_helper".to_owned(),
        qualified_name: "repo::repo_dep::src/lib.rs::fn::dep_helper".to_owned(),
        file_path: "src/lib.rs".to_owned(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_owned(),
        parent_name: None,
        params: Some("()".to_owned()),
        return_type: None,
        modifiers: Some("pub".to_owned()),
        is_test: false,
        file_hash: "h-dep".to_owned(),
        extra_json: serde_json::json!({"repo_id": "repo_dep"}),
        repo_provenance: None,
    };
    let cross_edge = Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: "src/app.rs::fn::call_dep".to_owned(),
        target_qn: "repo::repo_dep::src/lib.rs::fn::dep_helper".to_owned(),
        file_path: "src/app.rs".to_owned(),
        line: Some(1),
        confidence: 1.0,
        confidence_tier: Some("high".to_owned()),
        extra_json: serde_json::json!({"repo_id": "repo_root"}),
        repo_provenance: None,
    };
    store
        .replace_file_graph(
            "src/app.rs",
            "h-app",
            Some("rust"),
            Some(5),
            &[root_node],
            &[cross_edge],
        )
        .expect("replace app graph");
    store
        .replace_file_graph(
            "src/lib.rs",
            "h-dep",
            Some("rust"),
            Some(5),
            &[dep_node],
            &[],
        )
        .expect("replace dep graph");

    let review = call(
        "get_review_context",
        Some(&serde_json::json!({ "change_source": { "kind": "files", "files": ["src/app.rs"] }, "output_format": "json" })),
        "/ignored",
        &db_path,
    )
    .expect("review context call");
    let review_payload: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(review)).expect("parse review json");
    assert_eq!(
        review_payload["risk_summary"]["cross_repo_boundary"],
        serde_json::json!(true)
    );
    assert_eq!(
        review_payload["boundary_summary"]["cross_repo"],
        serde_json::json!(true)
    );
    assert!(
        review_payload["boundary_summary"]["cross_repo_count"]
            .as_u64()
            .unwrap_or_default()
            >= 1
    );

    let impact = call(
        "get_impact_radius",
        Some(&serde_json::json!({ "change_source": { "kind": "files", "files": ["src/app.rs"] }, "output_format": "json" })),
        "/ignored",
        &db_path,
    )
    .expect("impact call");
    let impact_payload: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(impact)).expect("parse impact json");
    assert_eq!(
        impact_payload["summary"]["cross_repo_boundary"],
        serde_json::json!(true)
    );
    assert_eq!(
        impact_payload["boundary_summary"]["cross_repo"],
        serde_json::json!(true)
    );
    assert!(
        impact_payload["boundary_summary"]["cross_repo_count"]
            .as_u64()
            .unwrap_or_default()
            >= 1
    );
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
        repo_provenance: None,
    };
    store
        .replace_file_graph("src/a.rs", "h1", Some("rust"), Some(10), &[node], &[])
        .expect("replace_file_graph");

    let args = serde_json::json!({
        "change_source": { "kind": "files", "files": ["src/a.rs"] },
        "max_depth": 5,
        "max_nodes": 200,
        "output_format": "json",
    });
    let resp = call("explain_change", Some(&args), "/ignored", &db_path).expect("call");
    let text = unwrap_tool_text(resp);
    let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        v.pointer("/summary/changed_file_count")
            .and_then(|n| n.as_u64()),
        Some(1)
    );
    assert_eq!(
        v.pointer("/summary/changed_symbol_count")
            .and_then(|n| n.as_u64()),
        Some(1)
    );
    assert_eq!(
        v.pointer("/change_kinds/signature_change")
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
    let query_value: serde_json::Value = serde_json::from_str(&query_text).expect("query json");
    assert_eq!(unwrap_tool_format(&query_resp), "json");
    assert!(
        query_value["matches"]
            .as_array()
            .is_some_and(|matches| !matches.is_empty()),
        "query_graph must return ranked results"
    );
    assert!(query_value["matches"].as_array().is_some_and(|matches| {
        matches
            .iter()
            .any(|item| item["qn"] == serde_json::json!("src/service.rs::fn::compute"))
    }));
    assert_eq!(query_resp["atlas_usage_edges_included"], false);
    assert!(
        query_resp["atlas_relationship_tools"]
            .as_array()
            .expect("relationship tools array")
            .iter()
            .any(|tool| tool.as_str() == Some("symbol_neighbors"))
    );

    let impact_args =
        serde_json::json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] } });
    let impact_resp = call(
        "get_impact_radius",
        Some(&impact_args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("get_impact_radius call");
    let impact_text = unwrap_tool_text(impact_resp.clone());
    let impact_value: serde_json::Value = serde_json::from_str(&impact_text).expect("impact json");
    assert_eq!(unwrap_tool_format(&impact_resp), "json");
    assert!(fallback_reason(&impact_resp).is_none());
    assert_eq!(
        impact_value["summary"]["changed_file_count"],
        serde_json::json!(1)
    );
    assert!(
        impact_value["impacted_symbols"]
            .as_array()
            .is_some_and(|symbols| symbols
                .iter()
                .any(|item| item["qn"] == serde_json::json!("src/api.rs::fn::handle_request")))
    );
    assert!(
        impact_value["relevant_edges"]
            .as_array()
            .is_some_and(|edges| {
                edges.iter().any(|item| {
                    item["from"] == serde_json::json!("tests/service_test.rs::fn::compute_test")
                        || item["to"]
                            == serde_json::json!("tests/service_test.rs::fn::compute_test")
                })
            })
    );

    let review_args =
        serde_json::json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] } });
    let review_resp = call(
        "get_review_context",
        Some(&review_args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("get_review_context call");
    let review_text = unwrap_tool_text(review_resp.clone());
    let review_value: serde_json::Value = serde_json::from_str(&review_text).expect("review json");
    assert_eq!(unwrap_tool_format(&review_resp), "json");
    assert!(fallback_reason(&review_resp).is_none());
    assert_eq!(review_value["intent"], serde_json::json!("review"));
    assert!(review_value["file_count"].as_u64().is_some());
    assert!(review_value["files"].as_array().is_some_and(|files| {
        files
            .iter()
            .any(|item| item["path"] == serde_json::json!("src/service.rs"))
    }));
    assert!(review_value["files"].as_array().is_some_and(|files| {
        files
            .iter()
            .any(|item| item["path"] == serde_json::json!("src/api.rs"))
    }));

    let context_args = serde_json::json!({ "target": { "kind": "query", "query": "compute" } });
    let context_resp = call(
        "get_context",
        Some(&context_args),
        "/ignored",
        &fixture.db_path,
    )
    .expect("get_context call");
    let context_text = unwrap_tool_text(context_resp.clone());
    let context_value: serde_json::Value =
        serde_json::from_str(&context_text).expect("context json");
    assert_eq!(unwrap_tool_format(&context_resp), "json");
    assert!(fallback_reason(&context_resp).is_none());
    assert_eq!(context_value["intent"], serde_json::json!("symbol"));
    assert!(context_value["nodes"].as_array().is_some_and(|nodes| {
        nodes
            .iter()
            .any(|item| item["qn"] == serde_json::json!("src/service.rs::fn::compute"))
    }));
    assert!(context_value["nodes"].as_array().is_some_and(|nodes| {
        nodes
            .iter()
            .any(|item| item["qn"] == serde_json::json!("src/api.rs::fn::handle_request"))
    }));
}
