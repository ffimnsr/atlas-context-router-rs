use super::*;

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
    let repo_root =
        atlas_repo::canonical_filesystem_path(camino::Utf8Path::from_path(root).unwrap())
            .expect("canonical repo root")
            .to_string();
    let args = serde_json::json!({ "output_format": "json" });
    let response = call("postprocess_graph", Some(&args), &repo_root, &db_path)
        .expect("postprocess_graph call");
    let text = unwrap_tool_text(response);
    let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");
    assert_eq!(value["summary"]["ok"], serde_json::json!(true));
    assert_eq!(value["summary"]["noop"], serde_json::json!(true));
    assert_eq!(value["summary"]["graph_built"], serde_json::json!(false));
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
    assert_eq!(value["summary"]["ok"], serde_json::json!(false));
    assert_eq!(
        value["summary"]["error_code"],
        serde_json::json!("unknown_stage")
    );
    assert_error_code_doc_link(
        value["summary"]["error_code_docs"]
            .as_str()
            .expect("error_code_docs"),
        "unknown_stage",
    );
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
    assert_eq!(value["mode"], serde_json::json!("changed_only"));
    assert_eq!(
        value["scope"]["stage_filter"],
        serde_json::json!("large_function_summaries")
    );
    assert_eq!(value["executed_stages"].as_array().map(Vec::len), Some(1));
}
