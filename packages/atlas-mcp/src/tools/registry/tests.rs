//! Registry invariants: contracts, typed schemas, descriptor shape.

use super::{
    ALLOWED_TEXT_ONLY_TOOLS, TYPED_ANALYSIS_SCHEMA_TOOLS, TYPED_CONTEXT_REVIEW_SCHEMA_TOOLS,
    TYPED_DISCOVERY_SCHEMA_TOOLS, TYPED_GRAPH_SCHEMA_TOOLS, TYPED_HEALTH_SCHEMA_TOOLS,
    TYPED_SESSION_MEMORY_SCHEMA_TOOLS, ToolResultContract, raw_tool_input_schema_by_name,
    tool_descriptors, tool_input_schema_by_name, tool_list, tool_list_markdown,
    tool_result_contract,
};
use crate::descriptors::JSON_SCHEMA_2020_12_URI;
use jsonschema::{Draft, JSONSchema};
use serde_json::json;
use std::collections::BTreeSet;

fn compile_schema(schema: &serde_json::Value) {
    JSONSchema::options()
        .with_draft(Draft::Draft202012)
        .compile(schema)
        .expect("valid 2020-12 schema");
}

fn required_field_names(schema: &serde_json::Value) -> BTreeSet<String> {
    schema
        .get("required")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(ToOwned::to_owned)
        .collect()
}

fn resolve_local_schema<'a>(
    root: &'a serde_json::Value,
    schema: &'a serde_json::Value,
) -> &'a serde_json::Value {
    let mut current = schema;
    loop {
        if let Some(reference) = current.get("$ref").and_then(serde_json::Value::as_str) {
            let pointer = reference
                .strip_prefix('#')
                .expect("local schema ref must start with #");
            current = root
                .pointer(pointer)
                .unwrap_or_else(|| panic!("missing local schema ref target: {reference}"));
            continue;
        }

        let mut advanced = false;
        for keyword in ["anyOf", "oneOf", "allOf"] {
            let Some(options) = current.get(keyword).and_then(serde_json::Value::as_array) else {
                continue;
            };
            if let Some(candidate) = options
                .iter()
                .find(|candidate| candidate.get("type") != Some(&json!("null")))
            {
                current = candidate;
                advanced = true;
                break;
            }
        }
        if !advanced {
            break;
        }
    }
    current
}

fn schema_node_at<'a>(root: &'a serde_json::Value, pointer: &str) -> &'a serde_json::Value {
    let mut current = root;
    for segment in pointer.trim_start_matches('/').split('/') {
        current = resolve_local_schema(root, current);
        current = current
            .get(segment)
            .unwrap_or_else(|| panic!("missing schema path /{segment} in {pointer}"));
    }
    resolve_local_schema(root, current)
}

fn schema_enum_values(root: &serde_json::Value, schema: &serde_json::Value) -> Option<Vec<String>> {
    let schema = resolve_local_schema(root, schema);
    if let Some(values) = schema.get("enum").and_then(serde_json::Value::as_array) {
        return Some(
            values
                .iter()
                .map(|value| {
                    value
                        .as_str()
                        .unwrap_or_else(|| panic!("enum value must be string: {value}"))
                        .to_owned()
                })
                .collect(),
        );
    }

    for keyword in ["anyOf", "oneOf", "allOf"] {
        if let Some(values) = schema.get(keyword).and_then(serde_json::Value::as_array) {
            for candidate in values {
                if candidate.get("type") == Some(&json!("null")) {
                    continue;
                }
                if let Some(enum_values) = schema_enum_values(root, candidate) {
                    return Some(enum_values);
                }
            }
        }
    }

    None
}

fn assert_schema_enum_values(root: &serde_json::Value, pointer: &str, expected: &[&str]) {
    let actual = schema_enum_values(root, schema_node_at(root, pointer))
        .unwrap_or_else(|| panic!("missing enum at schema path {pointer}"));
    let expected = expected
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    assert_eq!(actual, expected, "enum mismatch at schema path {pointer}");
}

#[test]
fn every_tool_name_title_and_annotations_are_present() {
    for tool in tool_descriptors() {
        assert!(
            !tool.title.as_deref().unwrap_or_default().trim().is_empty(),
            "missing title for {}",
            tool.name
        );
        let read_only = tool
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.read_only_hint)
            .unwrap_or(false);
        let destructive = tool
            .annotations
            .as_ref()
            .and_then(|annotations| annotations.destructive_hint)
            .unwrap_or(false);
        if !read_only {
            assert!(
                !read_only,
                "state-changing tool marked read-only: {}",
                tool.name
            );
        }
        if destructive {
            assert!(
                !read_only,
                "destructive tool must be state-changing: {}",
                tool.name
            );
        }
    }
}

#[test]
fn tool_registry_schemas_validate_as_2020_12() {
    for tool in tool_descriptors() {
        let input_schema = serde_json::Value::Object((*tool.input_schema).clone());
        assert_eq!(input_schema["$schema"], json!(JSON_SCHEMA_2020_12_URI));
        compile_schema(&input_schema);
        if let Some(output_schema) = tool.output_schema.as_ref() {
            let output_schema = serde_json::Value::Object((**output_schema).clone());
            assert_eq!(output_schema["$schema"], json!(JSON_SCHEMA_2020_12_URI));
            compile_schema(&output_schema);
        }
    }
}

#[test]
fn every_tool_has_inventory_contract_and_matching_schema_policy() {
    for tool in tool_descriptors() {
        match tool_result_contract(&tool.name) {
            ToolResultContract::StableObject => {
                assert!(
                    tool.output_schema.is_some(),
                    "{} must advertise outputSchema",
                    tool.name
                );
            }
            ToolResultContract::TextOnly => {
                assert!(
                    tool.output_schema.is_none(),
                    "{} must omit outputSchema",
                    tool.name
                );
            }
        }
        assert_eq!(
            tool.meta
                .as_ref()
                .and_then(|meta| meta.get("atlas:resultContract"))
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            json!(tool_result_contract(&tool.name).label())
        );
    }
}

#[test]
fn schema_builder_output_matches_registry_entries() {
    for tool in tool_descriptors() {
        let built = tool_input_schema_by_name(&tool.name).expect("schema by name");
        assert_eq!(
            built,
            serde_json::Value::Object((*tool.input_schema).clone()),
            "input schema mismatch for {}",
            tool.name
        );
    }
}

#[test]
fn tool_list_serializes_typed_descriptors() {
    let value = tool_list();
    let tools = value["tools"].as_array().expect("tools array");
    assert!(tools.iter().all(|tool| tool.get("title").is_some()));
    assert!(tools.iter().all(|tool| tool.get("annotations").is_some()));
    assert!(tools.iter().all(|tool| tool.get("icons").is_none()));
    assert!(tools.iter().all(|tool| tool.get("outputSchema").is_some()));
    assert!(
        tools
            .iter()
            .all(|tool| tool.pointer("/_meta/atlas:resultContract").is_some())
    );
}

#[test]
fn tool_list_markdown_documents_result_contract_inventory() {
    let markdown = tool_list_markdown();
    assert!(markdown.contains("generated from rmcp-backed `atlas_mcp::tool_list()` descriptors serialized from `rmcp::model::Tool`"));
    assert!(markdown.contains(
        "| Tool | Title | Input schema | Result contract | Output schema | Description |"
    ));
    assert!(markdown.contains("`stable-object`"));
    assert!(markdown.contains("`text-only`"));
    assert!(markdown.contains("structuredContent` is source of truth"));
    assert!(
        !markdown
            .contains("| `query_graph` | Query Graph | object; required fields: 0 | `text-only` |")
    );
    assert!(!markdown.contains(
        "| `batch_query_graph` | Batch Query Graph | object; required fields: 1 | `text-only` |"
    ));
    assert!(!markdown.contains("mixed-needs-redesign"));
    assert!(
            markdown.contains("`broker_status` | Broker Status | object; required fields: 0 | `stable-object` | exact structuredContent schema")
        );
}

#[test]
fn no_current_tool_uses_mixed_needs_redesign_contract() {
    for tool in tool_descriptors() {
        assert!(
            matches!(
                tool_result_contract(&tool.name),
                ToolResultContract::StableObject | ToolResultContract::TextOnly
            ),
            "{} must classify as stable-object or text-only",
            tool.name
        );
    }
}

#[test]
fn text_only_contracts_require_explicit_allowlist_entry() {
    let text_only = tool_descriptors()
        .into_iter()
        .filter(|tool| {
            matches!(
                tool_result_contract(&tool.name),
                ToolResultContract::TextOnly
            )
        })
        .map(|tool| tool.name)
        .collect::<Vec<_>>();
    assert_eq!(text_only, ALLOWED_TEXT_ONLY_TOOLS);
}

#[test]
fn typed_health_schemas_preserve_required_fields_and_property_descriptions() {
    for name in TYPED_HEALTH_SCHEMA_TOOLS {
        let schema = raw_tool_input_schema_by_name(name).expect("typed health tool schema");
        let required = schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        assert!(
            required.is_empty(),
            "{name} should not require input fields"
        );

        let properties = schema["properties"].as_object().expect("properties object");
        assert!(
            properties
                .get("output_format")
                .and_then(|property| property.get("description"))
                .and_then(serde_json::Value::as_str)
                .is_some_and(|description| !description.trim().is_empty()),
            "{name} output_format description must stay non-empty"
        );

        if matches!(*name, "db_check" | "debug_graph") {
            assert!(
                properties
                    .get("limit")
                    .and_then(|property| property.get("description"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|description| !description.trim().is_empty()),
                "{name} limit description must stay non-empty"
            );
        }
    }
}

#[test]
fn typed_health_schemas_keep_enum_values_stable() {
    for name in TYPED_HEALTH_SCHEMA_TOOLS {
        let schema = raw_tool_input_schema_by_name(name).expect("typed health tool schema");
        let properties = schema["properties"].as_object().expect("properties object");
        for property_name in ["output_format", "limit"] {
            let Some(property) = properties.get(property_name) else {
                continue;
            };
            assert!(
                property.get("enum").is_none(),
                "{name}.{property_name} unexpectedly gained enum values"
            );
        }
    }
}

#[test]
fn typed_discovery_schemas_preserve_required_fields_and_descriptions() {
    for name in TYPED_DISCOVERY_SCHEMA_TOOLS {
        let schema = raw_tool_input_schema_by_name(name).expect("typed discovery tool schema");
        let required = required_field_names(&schema);
        let expected = match *name {
            "search_files" => ["pattern"].into_iter().map(ToOwned::to_owned).collect(),
            "search_content" => ["query"].into_iter().map(ToOwned::to_owned).collect(),
            "read_file_excerpt" | "get_docs_section" => ["file", "selector"]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
            "read_file_around_match" => ["file", "query"]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
            "search_templates" | "search_text_assets" => BTreeSet::new(),
            _ => unreachable!("unexpected discovery schema tool"),
        };
        assert_eq!(
            required, expected,
            "{name} required fields changed unexpectedly"
        );

        let properties = schema["properties"].as_object().expect("properties object");
        assert!(
            properties.values().all(|property| property
                .get("description")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|description| !description.trim().is_empty())),
            "{name} top-level property descriptions must stay non-empty"
        );
    }
}

#[test]
fn typed_discovery_schemas_keep_enum_values_stable() {
    let excerpt = raw_tool_input_schema_by_name("read_file_excerpt").expect("excerpt schema");
    assert_schema_enum_values(
        &excerpt,
        "/properties/selector/properties/kind",
        &["range", "ranges", "context"],
    );

    let docs = raw_tool_input_schema_by_name("get_docs_section").expect("docs schema");
    assert_schema_enum_values(
        &docs,
        "/properties/selector/properties/kind",
        &["heading", "line"],
    );

    let templates = raw_tool_input_schema_by_name("search_templates").expect("templates schema");
    assert_schema_enum_values(
        &templates,
        "/properties/kind",
        &[
            "html",
            "jinja",
            "handlebars",
            "tera",
            "mako",
            "mustache",
            "twig",
            "liquid",
            "erb",
            "haml",
            "pug",
        ],
    );

    let text_assets =
        raw_tool_input_schema_by_name("search_text_assets").expect("text assets schema");
    assert_schema_enum_values(
        &text_assets,
        "/properties/kind",
        &["sql", "config", "env", "prompt"],
    );
}

#[test]
fn typed_graph_schemas_preserve_required_fields_and_descriptions() {
    for name in TYPED_GRAPH_SCHEMA_TOOLS {
        let schema = raw_tool_input_schema_by_name(name).expect("typed graph tool schema");
        let required = required_field_names(&schema);
        let expected = match *name {
            "query_graph" | "explain_query" => BTreeSet::new(),
            "batch_query_graph" => ["items"].into_iter().map(ToOwned::to_owned).collect(),
            "resolve_symbol" => ["name"].into_iter().map(ToOwned::to_owned).collect(),
            "symbol_neighbors" => ["qname"].into_iter().map(ToOwned::to_owned).collect(),
            "traverse_graph" => ["from_qn"].into_iter().map(ToOwned::to_owned).collect(),
            "cross_file_links" => ["file"].into_iter().map(ToOwned::to_owned).collect(),
            "concept_clusters" => ["files"].into_iter().map(ToOwned::to_owned).collect(),
            _ => unreachable!("unexpected graph schema tool"),
        };
        assert_eq!(
            required, expected,
            "{name} required fields changed unexpectedly"
        );

        let properties = schema["properties"].as_object().expect("properties object");
        assert!(
            properties.values().all(|property| property
                .get("description")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|description| !description.trim().is_empty())),
            "{name} top-level property descriptions must stay non-empty"
        );
    }
}

#[test]
fn typed_graph_schemas_keep_repo_scope_and_batch_limits_stable() {
    for name in [
        "query_graph",
        "batch_query_graph",
        "resolve_symbol",
        "explain_query",
    ] {
        let schema = raw_tool_input_schema_by_name(name).expect("typed graph tool schema");
        assert_schema_enum_values(
            &schema,
            "/properties/repo_scope/properties/kind",
            &["current", "repo_id", "all"],
        );
    }

    let batch = raw_tool_input_schema_by_name("batch_query_graph").expect("batch schema");
    assert_eq!(
        schema_node_at(&batch, "/properties/items").get("maxItems"),
        Some(&json!(20)),
        "batch_query_graph.items maxItems changed unexpectedly"
    );
}

#[test]
fn typed_context_review_schemas_preserve_required_fields_and_descriptions() {
    for name in TYPED_CONTEXT_REVIEW_SCHEMA_TOOLS {
        let schema = raw_tool_input_schema_by_name(name).expect("typed context/review tool schema");
        let required = required_field_names(&schema);
        let expected = match *name {
            "detect_changes"
            | "get_review_context"
            | "get_minimal_context"
            | "get_impact_radius"
            | "explain_change" => ["change_source"]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
            "get_context" => ["target"].into_iter().map(ToOwned::to_owned).collect(),
            "build_or_update_graph" | "postprocess_graph" => BTreeSet::new(),
            _ => unreachable!("unexpected context/review schema tool"),
        };
        assert_eq!(
            required, expected,
            "{name} required fields changed unexpectedly"
        );

        let properties = schema["properties"].as_object().expect("properties object");
        assert!(
            properties.values().all(|property| property
                .get("description")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|description| !description.trim().is_empty())),
            "{name} top-level property descriptions must stay non-empty"
        );
    }
}

#[test]
fn typed_context_review_schemas_keep_enum_values_stable() {
    for name in ["get_impact_radius", "get_review_context", "explain_change"] {
        let schema = raw_tool_input_schema_by_name(name).expect("context/review schema");
        assert_schema_enum_values(
            &schema,
            "/properties/change_source/properties/kind",
            &["files", "base", "staged", "working_tree"],
        );
    }

    for name in ["detect_changes", "get_minimal_context"] {
        let schema = raw_tool_input_schema_by_name(name).expect("context/review schema");
        assert_schema_enum_values(
            &schema,
            "/properties/change_source/properties/kind",
            &["base", "staged", "working_tree"],
        );
    }

    let build = raw_tool_input_schema_by_name("build_or_update_graph").expect("build schema");
    assert_schema_enum_values(
        &build,
        "/properties/operation/properties/kind",
        &["build", "update"],
    );
    assert_schema_enum_values(
        &build,
        "/properties/operation/properties/change_source/properties/kind",
        &["files", "base", "staged", "working_tree"],
    );

    let postprocess =
        raw_tool_input_schema_by_name("postprocess_graph").expect("postprocess schema");
    assert_schema_enum_values(
        &postprocess,
        "/properties/stage",
        &[
            "flows",
            "communities",
            "architecture_metrics",
            "query_hints",
            "large_function_summaries",
        ],
    );

    let context = raw_tool_input_schema_by_name("get_context").expect("get_context schema");
    assert_schema_enum_values(
        &context,
        "/properties/target/properties/kind",
        &["query", "file", "files"],
    );
    assert_schema_enum_values(
        &context,
        "/properties/intent",
        &[
            "symbol",
            "file",
            "review",
            "impact",
            "usage_lookup",
            "refactor_safety",
            "dead_code_check",
            "rename_preview",
            "dependency_removal",
        ],
    );
}

#[test]
fn typed_analysis_schemas_preserve_required_fields_and_descriptions() {
    for name in TYPED_ANALYSIS_SCHEMA_TOOLS {
        let schema = raw_tool_input_schema_by_name(name).expect("typed analysis tool schema");
        let required = required_field_names(&schema);
        let expected = match *name {
            "analyze_safety" | "analyze_dependency" | "find_similar_functions" => {
                ["symbol"].into_iter().map(ToOwned::to_owned).collect()
            }
            "analyze_remove" => ["symbols"].into_iter().map(ToOwned::to_owned).collect(),
            "analyze_dead_code"
            | "find_large_functions"
            | "find_complex_functions"
            | "find_duplicates"
            | "infer_modules"
            | "label_components" => BTreeSet::new(),
            _ => unreachable!("unexpected analysis schema tool"),
        };
        assert_eq!(
            required, expected,
            "{name} required fields changed unexpectedly"
        );

        let properties = schema["properties"].as_object().expect("properties object");
        assert!(
            properties.values().all(|property| property
                .get("description")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|description| !description.trim().is_empty())),
            "{name} top-level property descriptions must stay non-empty"
        );
    }
}

#[test]
fn typed_analysis_schemas_keep_enum_values_stable() {
    let large =
        raw_tool_input_schema_by_name("find_large_functions").expect("large functions schema");
    assert_schema_enum_values(
        &large,
        "/properties/mode",
        &["large", "complex", "large-or-complex"],
    );

    let dead_code = raw_tool_input_schema_by_name("analyze_dead_code").expect("dead code schema");
    assert_schema_enum_values(
        &dead_code,
        "/properties/exclude_kind/items",
        &[
            "function",
            "method",
            "struct",
            "enum",
            "trait",
            "interface",
            "class",
            "constant",
            "variable",
        ],
    );
}

#[test]
fn typed_session_memory_schemas_preserve_required_fields_and_descriptions() {
    for name in TYPED_SESSION_MEMORY_SCHEMA_TOOLS {
        let schema = raw_tool_input_schema_by_name(name).expect("typed session/memory tool schema");
        let required = required_field_names(&schema);
        let expected = match *name {
            "search_saved_context"
            | "search_decisions"
            | "cross_session_search"
            | "memory_recall" => ["query"].into_iter().map(ToOwned::to_owned).collect(),
            "read_saved_context" => ["source_id"].into_iter().map(ToOwned::to_owned).collect(),
            "save_context_artifact" => ["content", "label"]
                .into_iter()
                .map(ToOwned::to_owned)
                .collect(),
            "memory_store" => ["text"].into_iter().map(ToOwned::to_owned).collect(),
            "record_session_event" => ["event"].into_iter().map(ToOwned::to_owned).collect(),
            "get_session_status"
            | "compact_session"
            | "resume_session"
            | "purge_saved_context"
            | "get_global_memory"
            | "wake_up" => BTreeSet::new(),
            _ => unreachable!("unexpected session/memory schema tool"),
        };
        assert_eq!(
            required, expected,
            "{name} required fields changed unexpectedly"
        );

        let properties = schema["properties"].as_object().expect("properties object");
        assert!(
            properties.values().all(|property| property
                .get("description")
                .and_then(serde_json::Value::as_str)
                .is_some_and(|description| !description.trim().is_empty())),
            "{name} top-level property descriptions must stay non-empty"
        );
    }
}

#[test]
fn typed_session_memory_schemas_keep_enum_values_stable() {
    for name in [
        "search_saved_context",
        "read_saved_context",
        "save_context_artifact",
        "cross_session_search",
    ] {
        let schema = raw_tool_input_schema_by_name(name).expect("session/memory schema");
        assert_schema_enum_values(
            &schema,
            "/properties/repo_scope/properties/kind",
            &["current", "repo_id", "all"],
        );
    }

    for name in ["record_session_event", "wake_up"] {
        let schema = raw_tool_input_schema_by_name(name).expect("session/memory schema");
        assert_schema_enum_values(
            &schema,
            "/properties/repo_scope/properties/kind",
            &["current", "repo_id"],
        );
    }

    let memory_store = raw_tool_input_schema_by_name("memory_store").expect("memory_store schema");
    assert_schema_enum_values(
        &memory_store,
        "/properties/importance",
        &["critical", "high", "normal", "low"],
    );
    assert_schema_enum_values(
        &memory_store,
        "/properties/scope",
        &["project", "session", "frontend", "global"],
    );

    let memory_recall =
        raw_tool_input_schema_by_name("memory_recall").expect("memory_recall schema");
    assert_schema_enum_values(
        &memory_recall,
        "/properties/importance",
        &["critical", "high", "normal", "low"],
    );
    assert_schema_enum_values(
        &memory_recall,
        "/properties/scope",
        &["project", "session", "frontend", "global"],
    );
}

#[test]
fn all_tool_descriptions_and_output_schema_contracts_remain_present() {
    for tool in tool_descriptors() {
        assert!(
            tool.title
                .as_deref()
                .is_some_and(|title| !title.trim().is_empty()),
            "{} missing title",
            tool.name
        );
        assert!(
            tool.description
                .as_deref()
                .is_some_and(|description| !description.trim().is_empty()),
            "{} missing description",
            tool.name
        );
        assert!(
            !tool.input_schema.is_empty(),
            "{} must keep input schema",
            tool.name
        );
        if matches!(
            tool_result_contract(&tool.name),
            ToolResultContract::StableObject
        ) {
            assert!(
                tool.output_schema.is_some(),
                "{} must keep output schema",
                tool.name
            );
        }
    }
}
