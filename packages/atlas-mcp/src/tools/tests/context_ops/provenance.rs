use super::*;

#[test]
fn get_impact_radius_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" });
    let resp = call("get_impact_radius", Some(&args), "/repo", &fixture.db_path)
        .expect("get_impact_radius");
    assert_provenance(&resp, "/repo", &fixture.db_path);
}

#[test]
fn get_review_context_includes_provenance() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" });
    let resp = call("get_review_context", Some(&args), "/repo", &fixture.db_path)
        .expect("get_review_context");
    assert_provenance(&resp, "/repo", &fixture.db_path);
    assert!(
        resp["structuredContent"]
            .get("ranking_evidence_legend")
            .is_some()
    );
}

#[test]
fn get_review_context_json_includes_changed_symbol_evidence() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" });
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
    assert!(
        resp["structuredContent"]
            .get("ranking_evidence_legend")
            .is_some()
    );
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
