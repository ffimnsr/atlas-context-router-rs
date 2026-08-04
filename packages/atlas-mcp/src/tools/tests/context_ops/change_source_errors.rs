use super::*;

#[test]
fn change_source_invalid_combinations_return_structured_errors() {
    let fixture = setup_mcp_fixture();

    let impact_err = call(
        "get_impact_radius",
        Some(&serde_json::json!({
            "files": ["src/service.rs"],
            "staged": true,
            "output_format": "json"
        })),
        "/repo",
        &fixture.db_path,
    )
    .expect("impact must reject ambiguous change source as tool error result");
    let impact_details = &impact_err["structuredContent"]["details"];
    assert_eq!(impact_err["isError"], serde_json::json!(true));
    assert_eq!(
        impact_err["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert_eq!(
        impact_err["structuredContent"]["message"],
        serde_json::json!("legacy change_source fields are no longer supported")
    );
    assert_eq!(
        impact_details["offending_fields"],
        serde_json::json!(["files", "staged"])
    );
    assert_eq!(
        impact_details["present_mode_families"],
        serde_json::json!([])
    );
    assert_eq!(
        impact_details["accepted_argument_families"],
        serde_json::json!([
            "change_source.kind=files",
            "change_source.kind=base",
            "change_source.kind=staged",
            "change_source.kind=working_tree"
        ])
    );
    assert_eq!(
        impact_details["retry_example"],
        serde_json::json!({"change_source": {"kind": "files", "files": ["src/service.rs"]}})
    );
    assert_eq!(
        impact_details["fail_closed_reason"],
        serde_json::json!("Atlas refused to guess between conflicting change-source selectors")
    );

    let review_err = call(
        "get_review_context",
        Some(&serde_json::json!({
            "base": "HEAD",
            "working_tree": true,
            "output_format": "json"
        })),
        "/repo",
        &fixture.db_path,
    )
    .expect("review must reject ambiguous change source as tool error result");
    let review_details = &review_err["structuredContent"]["details"];
    assert_eq!(review_err["isError"], serde_json::json!(true));
    assert_eq!(
        review_err["structuredContent"]["code"],
        serde_json::json!("invalid_input")
    );
    assert_eq!(
        review_details["offending_fields"],
        serde_json::json!(["base", "working_tree"])
    );
    assert_eq!(
        review_details["present_mode_families"],
        serde_json::json!([])
    );

    assert_eq!(
        review_details["retry_example"],
        serde_json::json!({"change_source": {"kind": "files", "files": ["src/service.rs"]}})
    );
}
