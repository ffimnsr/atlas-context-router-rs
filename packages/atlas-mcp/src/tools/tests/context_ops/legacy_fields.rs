use super::*;

#[test]
fn detect_changes_accepts_canonical_change_source_field() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        std::path::Path::new(&fixture.repo_root),
        "src/service.rs",
        "pub fn compute() -> i32 { 9 }\n",
    );
    let args = serde_json::json!({
        "change_source": { "kind": "working_tree" },
        "output_format": "json"
    });

    let resp = call(
        "detect_changes",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("detect_changes with canonical change_source");

    assert_eq!(
        resp.pointer("/structuredContent/change_source/kind")
            .and_then(|value| value.as_str()),
        Some("working_tree")
    );
}

#[test]
fn detect_changes_rejects_legacy_mode_and_fields_with_examples() {
    let fixture = setup_git_mcp_fixture();
    let args = serde_json::json!({
        "mode": "base",
        "base": "HEAD",
        "working_tree": true,
        "output_format": "json"
    });

    let resp = call(
        "detect_changes",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("detect_changes conflict should return tool error result");
    let details = &resp["structuredContent"]["details"];

    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert_eq!(
        resp["structuredContent"]["message"],
        serde_json::json!("legacy change_source fields are no longer supported")
    );
    assert_eq!(
        details["retry_example"],
        serde_json::json!({"change_source": {"kind": "base", "base": "origin/main"}})
    );
}
