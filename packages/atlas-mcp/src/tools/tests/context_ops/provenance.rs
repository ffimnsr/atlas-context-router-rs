use super::*;

#[test]
fn get_impact_radius_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" });
    let resp = call(
        "get_impact_radius",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_impact_radius");
    assert_provenance(&resp, &fixture.repo_root, &fixture.db_path);
}

#[test]
fn get_review_context_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" });
    let resp = call(
        "get_review_context",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_review_context");
    assert_provenance(&resp, &fixture.repo_root, &fixture.db_path);
    assert!(
        resp["structuredContent"]
            .get("ranking_evidence_legend")
            .is_none()
    );

    let args = serde_json::json!({
        "change_source": { "kind": "files", "files": ["src/service.rs"] },
        "include_ranking_evidence": true,
        "output_format": "json"
    });
    let resp = call(
        "get_review_context",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_review_context");
    assert!(
        resp["structuredContent"]
            .get("ranking_evidence_legend")
            .is_some()
    );
}

#[test]
fn get_review_context_json_includes_changed_symbol_evidence() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "change_source": { "kind": "files", "files": ["src/service.rs"] },
        "include_ranking_evidence": true,
        "output_format": "json"
    });
    let resp = call(
        "get_review_context",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
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
    assert!(
        resp["structuredContent"]
            .get("ranking_evidence_legend")
            .is_some()
    );
}

#[test]
fn get_review_context_reports_neighbors_and_consistent_traversal_counts() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "change_source": { "kind": "files", "files": ["src/service.rs"] },
        "max_nodes": 20,
        "output_format": "json"
    });
    let resp = call(
        "get_review_context",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("get_review_context");
    let text = unwrap_tool_text(resp);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert!(
        value["changed_symbols"]
            .as_array()
            .is_some_and(|symbols| !symbols.is_empty()),
        "changed_symbols must not be empty: {value}"
    );
    assert!(
        value["impacted_neighbors"]
            .as_array()
            .is_some_and(|neighbors| !neighbors.is_empty()),
        "impacted_neighbors must be populated: {value}"
    );
    let nodes = value["nodes"].as_array().map_or(0, Vec::len);
    let edges = value["edges"].as_array().map_or(0, Vec::len);
    assert_eq!(value["traversal_budget"]["emitted_node_count"], nodes);
    assert_eq!(value["traversal_budget"]["emitted_edge_count"], edges);
    assert_eq!(
        value["critical_edges"].as_array().map_or(0, Vec::len),
        edges
    );
}

#[test]
fn explain_change_docs_only_delta_reports_no_coverage_gaps() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "README.md",
        "# Overview\nchanged docs\n## Install\nstep\n",
    );
    let args = serde_json::json!({
        "change_source": { "kind": "working_tree" },
        "output_format": "json"
    });
    let resp = call(
        "explain_change",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("explain_change");
    let text = unwrap_tool_text(resp);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(value["risk_level"], "low", "payload: {value}");
    assert!(
        value["coverage_gaps"]
            .as_array()
            .is_some_and(|gaps| gaps.is_empty()),
        "docs-only delta must not report coverage gaps: {value}"
    );
    assert_eq!(value["test_impact"]["uncovered_symbol_count"], 0);
}

#[test]
fn get_context_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "output_format": "json" });
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
    let args = serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "output_format": "json" });

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
    let args =
        serde_json::json!({ "change_source": { "kind": "working_tree" }, "output_format": "json" });

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
    let args =
        serde_json::json!({ "change_source": { "kind": "working_tree" }, "output_format": "json" });

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
