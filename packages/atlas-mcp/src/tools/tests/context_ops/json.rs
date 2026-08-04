use super::*;

fn assert_json_tool_text_deterministic(
    fixture: &GitMcpFixture,
    name: &str,
    args: serde_json::Value,
) {
    determinism_support::assert_text_deterministic(&format!("MCP {name}"), || {
        let response = call(name, Some(&args), &fixture.repo_root, &fixture.db_path)
            .unwrap_or_else(|error| panic!("{name} call failed: {error}"));
        assert_eq!(unwrap_tool_format(&response), "json", "{name} call");
        unwrap_tool_text(response)
    });
}

#[test]
fn json_tool_outputs_are_byte_identical_for_same_repo_and_commit() {
    let fixture = setup_git_mcp_fixture();

    assert_json_tool_text_deterministic(
        &fixture,
        "query_graph",
        serde_json::json!({ "text": "compute", "output_format": "json" }),
    );
    assert_json_tool_text_deterministic(
        &fixture,
        "get_context",
        serde_json::json!({ "target": { "kind": "query", "query": "compute" }, "output_format": "json" }),
    );
    assert_json_tool_text_deterministic(
        &fixture,
        "get_impact_radius",
        serde_json::json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" }),
    );
    assert_json_tool_text_deterministic(
        &fixture,
        "get_review_context",
        serde_json::json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" }),
    );
    assert_json_tool_text_deterministic(
        &fixture,
        "explain_change",
        serde_json::json!({ "change_source": { "kind": "files", "files": ["src/service.rs"] }, "output_format": "json" }),
    );
}
