use super::*;

#[test]
fn get_impact_radius_accepts_explicit_files_and_reports_change_source_metadata() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "change_source": { "kind": "files", "files": ["src/service.rs"] },
        "output_format": "json"
    });

    let resp = call("get_impact_radius", Some(&args), "/repo", &fixture.db_path)
        .expect("get_impact_radius");
    let text = unwrap_tool_text(resp.clone());
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

    assert_eq!(
        value
            .pointer("/summary/changed_file_count")
            .and_then(|n| n.as_u64()),
        Some(1)
    );
    assert_eq!(
        resp.pointer("/structuredContent/change_source/kind")
            .and_then(|value| value.as_str()),
        Some("files")
    );
    assert_eq!(
        resp.pointer("/structuredContent/change_source/resolved_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
}

#[test]
fn get_review_context_accepts_explicit_files_and_reports_change_source_metadata() {
    let fixture = setup_mcp_fixture();
    let args = serde_json::json!({
        "change_source": { "kind": "files", "files": ["src/service.rs"] },
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
        resp.pointer("/structuredContent/change_source/kind")
            .and_then(|value| value.as_str()),
        Some("files")
    );
    assert_eq!(
        resp.pointer("/structuredContent/change_source/resolved_files/0")
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
    let args = serde_json::json!({ "change_source": { "kind": "base", "base": "HEAD" }, "output_format": "json" });

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
        value
            .pointer("/summary/changed_file_count")
            .and_then(|n| n.as_u64()),
        Some(1)
    );
    assert_eq!(
        resp.pointer("/structuredContent/change_source/kind")
            .and_then(|value| value.as_str()),
        Some("base")
    );
    assert_eq!(
        resp.pointer("/structuredContent/change_source/resolved_files/0")
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
    let args =
        serde_json::json!({ "change_source": { "kind": "staged" }, "output_format": "json" });

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
        resp.pointer("/structuredContent/change_source/kind")
            .and_then(|value| value.as_str()),
        Some("staged")
    );
    assert_eq!(
        resp.pointer("/structuredContent/change_source/resolved_files/0")
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
        resp.pointer("/structuredContent/change_source/kind")
            .and_then(|value| value.as_str()),
        Some("working_tree")
    );
    assert_eq!(
        resp.pointer("/structuredContent/change_source/resolved_files/0")
            .and_then(|value| value.as_str()),
        Some("src/service.rs")
    );
}

#[test]
fn get_impact_radius_empty_diff_returns_empty_result() {
    let fixture = setup_git_mcp_fixture();
    let args =
        serde_json::json!({ "change_source": { "kind": "working_tree" }, "output_format": "json" });

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
        value
            .pointer("/summary/changed_file_count")
            .and_then(|n| n.as_u64()),
        Some(0)
    );
    assert_eq!(
        value
            .pointer("/summary/impacted_file_count")
            .and_then(|n| n.as_u64()),
        Some(0)
    );
    assert_eq!(
        resp.pointer("/structuredContent/change_source/kind")
            .and_then(|value| value.as_str()),
        Some("working_tree")
    );
    assert_eq!(
        resp.pointer("/structuredContent/change_source/resolved_files")
            .and_then(|value| value.as_array())
            .map(|items| items.len()),
        Some(0)
    );
}
