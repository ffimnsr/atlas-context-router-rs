use super::*;

#[test]
fn build_or_update_graph_accepts_operation_build() {
    let fixture = setup_git_mcp_fixture();
    let args = serde_json::json!({
        "operation": { "kind": "build" },
        "output_format": "json"
    });

    let resp = call(
        "build_or_update_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("build_or_update_graph build op");

    assert_ne!(resp.get("isError"), Some(&serde_json::json!(true)));
    assert_eq!(
        resp.pointer("/structuredContent/mode")
            .and_then(|value| value.as_str()),
        Some("build")
    );
    assert!(resp["_meta"].get("deprecated_input_fields").is_none());
}

#[test]
fn build_or_update_graph_accepts_operation_update_working_tree() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 9 }\n",
    );
    let args = serde_json::json!({
        "operation": {
            "kind": "update",
            "change_source": { "kind": "working_tree" }
        },
        "output_format": "json"
    });

    let resp = call(
        "build_or_update_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("build_or_update_graph working tree op");

    assert_eq!(
        resp.pointer("/structuredContent/source/target_kind")
            .and_then(|value| value.as_str()),
        Some("working_tree")
    );
}

#[test]
fn build_or_update_graph_accepts_operation_update_staged() {
    let fixture = setup_git_mcp_fixture();
    let repo_root = std::path::Path::new(&fixture.repo_root);
    write_repo_file(
        repo_root,
        "src/service.rs",
        "pub fn compute() -> i32 { 10 }\n",
    );
    git_run(repo_root, &["add", "src/service.rs"]);
    let args = serde_json::json!({
        "operation": {
            "kind": "update",
            "change_source": { "kind": "staged" }
        },
        "output_format": "json"
    });

    let resp = call(
        "build_or_update_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("build_or_update_graph staged op");

    assert_eq!(
        resp.pointer("/structuredContent/source/target_kind")
            .and_then(|value| value.as_str()),
        Some("staged")
    );
}

#[test]
fn build_or_update_graph_accepts_operation_update_base() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 11 }\n",
    );
    let args = serde_json::json!({
        "operation": {
            "kind": "update",
            "change_source": { "kind": "base", "base": "HEAD" }
        },
        "output_format": "json"
    });

    let resp = call(
        "build_or_update_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("build_or_update_graph base op");

    assert_eq!(
        resp.pointer("/structuredContent/source/target_kind")
            .and_then(|value| value.as_str()),
        Some("base")
    );
    assert_eq!(
        resp.pointer("/structuredContent/source/base_ref")
            .and_then(|value| value.as_str()),
        Some("HEAD")
    );
}

#[test]
fn build_or_update_graph_accepts_operation_update_files() {
    let fixture = setup_git_mcp_fixture();
    let args = serde_json::json!({
        "operation": {
            "kind": "update",
            "change_source": { "kind": "files", "files": ["src/service.rs"] }
        },
        "output_format": "json"
    });

    let resp = call(
        "build_or_update_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("build_or_update_graph files op");

    assert_eq!(
        resp.pointer("/structuredContent/source/target_kind")
            .and_then(|value| value.as_str()),
        Some("files")
    );
}

#[test]
fn build_or_update_graph_rejects_legacy_mode_update() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 12 }\n",
    );
    let args = serde_json::json!({
        "mode": "update",
        "output_format": "json"
    });

    let resp = call(
        "build_or_update_graph",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("build_or_update_graph legacy update");

    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["message"],
        serde_json::json!("legacy build_or_update_graph fields are no longer supported")
    );
}

#[test]
fn build_or_update_graph_rejects_build_operation_with_change_source() {
    let fixture = setup_git_mcp_fixture();
    let resp = call(
        "build_or_update_graph",
        Some(&serde_json::json!({
            "operation": {
                "kind": "build",
                "change_source": { "kind": "working_tree" }
            },
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("build conflict result");

    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["message"],
        serde_json::json!("operation.kind='build' cannot include change_source")
    );
}

#[test]
fn build_or_update_graph_rejects_update_operation_without_change_source() {
    let fixture = setup_git_mcp_fixture();
    let resp = call(
        "build_or_update_graph",
        Some(&serde_json::json!({
            "operation": { "kind": "update" },
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("missing change_source result");

    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["message"],
        serde_json::json!("operation.kind='update' requires operation.change_source")
    );
}

#[test]
fn build_or_update_graph_rejects_mixed_operation_and_legacy_fields() {
    let fixture = setup_git_mcp_fixture();
    let resp = call(
        "build_or_update_graph",
        Some(&serde_json::json!({
            "operation": { "kind": "build" },
            "mode": "build",
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("mixed operation result");

    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["message"],
        serde_json::json!("conflicting build operation selectors")
    );
}
