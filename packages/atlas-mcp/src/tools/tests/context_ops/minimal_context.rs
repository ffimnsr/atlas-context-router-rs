use super::*;

#[test]
fn get_minimal_context_rejects_conflicting_change_source_modes() {
    let fixture = setup_mcp_fixture();
    let resp = call(
        "get_minimal_context",
        Some(&serde_json::json!({
            "base": "HEAD",
            "staged": true,
            "output_format": "json"
        })),
        "/repo",
        &fixture.db_path,
    )
    .expect("minimal context conflict should return tool error result");

    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert_eq!(
        resp["structuredContent"]["details"]["accepted_argument_families"],
        serde_json::json!([
            "change_source.kind=base",
            "change_source.kind=staged",
            "change_source.kind=working_tree"
        ])
    );
}

// ---------------------------------------------------------------------------
// MCP12 — Context detail controls
// ---------------------------------------------------------------------------
