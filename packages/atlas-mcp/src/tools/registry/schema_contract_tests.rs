//! MCP wire-format contract tests for `tools/list` descriptors.

use super::tool_list;
use crate::descriptors::JSON_SCHEMA_2020_12_URI;
use jsonschema::{Draft, JSONSchema};
use std::collections::BTreeSet;

#[test]
fn tools_list_serializes_only_mcp_supported_descriptor_fields() {
    let tools = tool_list()["tools"]
        .as_array()
        .expect("tools array")
        .clone();
    let allowed = super::ALLOWED_TOOL_DESCRIPTOR_FIELDS
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();

    for tool in tools {
        let keys = tool
            .as_object()
            .expect("tool descriptor object")
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        assert!(
            keys.is_subset(&allowed),
            "descriptor keys not allowed: {:?}",
            keys.difference(&allowed).copied().collect::<Vec<_>>()
        );
    }
}

#[test]
fn tools_list_emitted_schemas_compile_under_json_schema_2020_12() {
    for tool in tool_list()["tools"].as_array().expect("tools array") {
        let input_schema = tool.get("inputSchema").expect("input schema");
        assert_eq!(
            input_schema["$schema"],
            serde_json::json!(JSON_SCHEMA_2020_12_URI)
        );
        JSONSchema::options()
            .with_draft(Draft::Draft202012)
            .compile(input_schema)
            .expect("input schema compiles");

        if let Some(output_schema) = tool.get("outputSchema") {
            assert_eq!(
                output_schema["$schema"],
                serde_json::json!(JSON_SCHEMA_2020_12_URI)
            );
            JSONSchema::options()
                .with_draft(Draft::Draft202012)
                .compile(output_schema)
                .expect("output schema compiles");
        }
    }
}

#[test]
fn r2_output_schemas_expose_nested_defs_for_normalized_payloads() {
    let registry = tool_list();
    let tools = registry["tools"].as_array().expect("tools array");

    let by_name = |name: &str| {
        tools
            .iter()
            .find(|tool| tool.get("name") == Some(&serde_json::json!(name)))
            .expect("tool present")
    };

    let impact = by_name("get_impact_radius");
    assert_eq!(
        impact["outputSchema"]["properties"]["changed_symbols"]["items"]["$ref"],
        serde_json::json!("#/$defs/compact_node")
    );
    assert!(impact["outputSchema"].get("$defs").is_some());

    let review = by_name("get_review_context");
    assert_eq!(
        review["outputSchema"]["properties"]["risk_summary"]["$ref"],
        serde_json::json!("#/$defs/review_risk_summary")
    );
    assert!(review["outputSchema"].get("$defs").is_some());

    let context = by_name("get_context");
    assert_eq!(
        context["outputSchema"]["properties"]["ranked_symbols"]["items"]["$ref"],
        serde_json::json!("#/$defs/ranked_symbol_summary")
    );
    assert_eq!(
        context["outputSchema"]["properties"]["ambiguity"]["$ref"],
        serde_json::json!("#/$defs/ambiguity")
    );
}
