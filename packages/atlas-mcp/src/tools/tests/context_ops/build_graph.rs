use super::*;

#[test]
fn build_graph_accepts_empty_object() {
    let fixture = setup_git_mcp_fixture();
    let resp = call(
        "build_graph",
        Some(&serde_json::json!({ "output_format": "json" })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("build_graph");

    assert_ne!(resp.get("isError"), Some(&serde_json::json!(true)));
    assert_eq!(
        resp.pointer("/structuredContent/mode")
            .and_then(|value| value.as_str()),
        Some("build")
    );
}

#[test]
fn build_graph_rejects_old_operation_shape() {
    let fixture = setup_git_mcp_fixture();
    let resp = call(
        "build_graph",
        Some(&serde_json::json!({ "operation": { "kind": "build" } })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("build_graph invalid input result");

    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["message"],
        serde_json::json!("build_graph does not accept operation or change_source")
    );
}

#[test]
fn update_graph_accepts_working_tree() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 9 }\n",
    );
    let resp = call(
        "update_graph",
        Some(&serde_json::json!({
            "change_source": { "kind": "working_tree" },
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("update_graph working tree");

    assert_eq!(
        resp.pointer("/structuredContent/source/target_kind")
            .and_then(|value| value.as_str()),
        Some("working_tree")
    );
}

#[test]
fn update_graph_accepts_staged_base_and_files() {
    let fixture = setup_git_mcp_fixture();
    let repo_root = std::path::Path::new(&fixture.repo_root);
    write_repo_file(
        repo_root,
        "src/service.rs",
        "pub fn compute() -> i32 { 10 }\n",
    );
    git_run(repo_root, &["add", "src/service.rs"]);

    for source in [
        serde_json::json!({ "kind": "staged" }),
        serde_json::json!({ "kind": "base", "base": "HEAD" }),
        serde_json::json!({ "kind": "files", "files": ["src/service.rs"] }),
    ] {
        let resp = call(
            "update_graph",
            Some(&serde_json::json!({ "change_source": source, "output_format": "json" })),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("update_graph source");
        assert_ne!(resp.get("isError"), Some(&serde_json::json!(true)));
    }
}

#[test]
fn update_graph_rejects_missing_or_null_change_source() {
    let fixture = setup_git_mcp_fixture();
    for args in [
        serde_json::json!({}),
        serde_json::json!({ "change_source": null }),
    ] {
        let resp = call(
            "update_graph",
            Some(&args),
            &fixture.repo_root,
            &fixture.db_path,
        )
        .expect("update_graph invalid input result");
        assert_eq!(resp["isError"], serde_json::json!(true));
        assert_eq!(
            resp["structuredContent"]["details"]["retry_example"],
            serde_json::json!({ "change_source": { "kind": "working_tree" } })
        );
    }
}
