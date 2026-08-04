//! MCP tool registry: descriptors, typed input/output schemas.
//!
//! Split across `registry/` submodules to keep each file under 1000 lines:
//! - `contract` – result-contract classification + generated markdown
//! - `annotations` – per-tool annotations, categories, test consts
//! - `schemas` – typed argument-schema structs (schemars)
//! - `inventory_*` – base `tools/list` entry JSON per tool family
//! - `typed_input_*` – typed input-schema builders per tool family
//! - `output_schemas/*` – output-schema builders per tool family
//! - `tests`, `schema_contract_tests` – registry invariants

use crate::descriptors::{
    ToolDescriptor, descriptor_meta, ensure_schema_2020_12, human_title, validate_descriptor_name,
    validate_mcp_schema,
};
use crate::spec;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use annotations::*;
pub use contract::tool_list_markdown;
pub(crate) use contract::{ToolResultContract, tool_result_contract};
use output_schemas::*;
pub(crate) use schemas::*;

mod annotations;
mod contract;
mod inventory_analysis;
mod inventory_content;
mod inventory_context;
mod inventory_discovery;
mod inventory_graph;
mod inventory_health;
mod inventory_memory;
mod output_schemas;
mod schemas;
mod typed_input_analysis;
mod typed_input_content;
mod typed_input_context;
mod typed_input_graph;
mod typed_input_health;
mod typed_input_memory;

#[cfg(test)]
mod schema_contract_tests;
#[cfg(test)]
mod tests;

/// Return the MCP `tools/list` response body.
fn strip_output_format_properties(value: &mut Value) {
    match value {
        Value::Object(object) => {
            if let Some(properties) = object.get_mut("properties").and_then(Value::as_object_mut) {
                properties.remove("output_format");
            }
            for child in object.values_mut() {
                strip_output_format_properties(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_output_format_properties(item);
            }
        }
        _ => {}
    }
}

/// Base tool inventory JSON, assembled from per-family entry lists. Descriptor
/// order is normalized afterwards (`tool_descriptors` sorts by name), so the
/// order of these lists is cosmetic.
fn base_tool_list_json() -> Value {
    let mut tools = Vec::new();
    tools.extend(inventory_health::tools());
    tools.extend(inventory_discovery::tools());
    tools.extend(inventory_graph::tools());
    tools.extend(inventory_context::tools());
    tools.extend(inventory_analysis::tools());
    tools.extend(inventory_content::tools());
    tools.extend(inventory_memory::tools());
    let mut value = serde_json::json!({ "tools": tools });
    strip_output_format_properties(&mut value);
    value
}

#[derive(Deserialize)]
struct ToolDescriptorSeed {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

pub(crate) fn tool_descriptors() -> Vec<ToolDescriptor> {
    let tools_value = base_tool_list_json()["tools"].clone();
    let seeds: Vec<ToolDescriptorSeed> =
        serde_json::from_value(tools_value).expect("base tool registry json must be valid");
    let mut descriptors = seeds
        .into_iter()
        .map(build_tool_descriptor)
        .collect::<Vec<_>>();
    descriptors.sort_by(|left, right| left.name.cmp(&right.name));
    descriptors
}

pub(crate) fn tool_descriptor_by_name(name: &str) -> Option<ToolDescriptor> {
    tool_descriptors()
        .into_iter()
        .find(|tool| tool.name == name)
}

#[cfg(test)]
pub fn tool_input_schema_by_name(name: &str) -> Option<Value> {
    tool_descriptors()
        .into_iter()
        .find(|tool| tool.name == name)
        .map(|tool| Value::Object((*tool.input_schema).clone()))
}

#[cfg(test)]
pub(crate) fn raw_tool_input_schema_by_name(name: &str) -> Option<Value> {
    let tools_value = base_tool_list_json()["tools"].clone();
    let seeds: Vec<ToolDescriptorSeed> =
        serde_json::from_value(tools_value).expect("base tool registry json must be valid");
    seeds
        .into_iter()
        .find(|tool| tool.name == name)
        .map(|seed| {
            ensure_schema_2020_12(typed_input_schema_for(&seed.name).unwrap_or(seed.input_schema))
        })
}

const TOOLS_LIST_CACHE_TTL_MS: u64 = 300_000;
const TOOLS_LIST_CACHE_SCOPE: &str = spec::CACHE_SCOPE_PUBLIC;

pub fn tool_list() -> Value {
    let mut result = serde_json::json!({
        "tools": tool_descriptors(),
    });
    spec::annotate_cacheable_result(&mut result, TOOLS_LIST_CACHE_TTL_MS, TOOLS_LIST_CACHE_SCOPE);
    result
}

fn build_tool_descriptor(seed: ToolDescriptorSeed) -> ToolDescriptor {
    validate_descriptor_name(&seed.name).expect("tool name must satisfy MCP guidance");
    let category = tool_category(&seed.name);
    let contract = tool_result_contract(&seed.name);
    let mut meta = descriptor_meta("tool", category);
    meta["atlas:resultContract"] = serde_json::json!(contract.label());
    meta["atlas:resultContractGuidance"] = serde_json::json!(contract.guidance());
    let mut input_schema =
        ensure_schema_2020_12(typed_input_schema_for(&seed.name).unwrap_or(seed.input_schema));
    strip_output_format_properties(&mut input_schema);
    validate_mcp_schema(&input_schema)
        .unwrap_or_else(|error| panic!("{} input schema invalid for MCP: {error}", seed.name));
    let output_schema = tool_output_schema_for(&seed.name);
    if let Some(schema) = output_schema.as_ref() {
        validate_mcp_schema(schema)
            .unwrap_or_else(|error| panic!("{} output schema invalid for MCP: {error}", seed.name));
    }
    let tool_name = seed.name;
    let tool_description = seed.description;
    let mut descriptor = ToolDescriptor::new_with_raw(
        tool_name.clone(),
        Some(std::borrow::Cow::Owned(tool_description)),
        crate::rmcp_types::schema_object_from_value(input_schema)
            .expect("tool input schema must be object"),
    )
    .with_title(human_title(&tool_name));
    descriptor.annotations = Some(tool_annotations(tool_name.as_str()));
    if let Some(schema) = output_schema {
        descriptor = descriptor.with_raw_output_schema(
            crate::rmcp_types::schema_object_from_value(schema)
                .expect("tool output schema must be object"),
        );
    }
    if let Some(meta) = crate::rmcp_types::meta_object_from_value(meta)
        .expect("tool descriptor meta must be object")
    {
        descriptor = descriptor.with_meta(meta);
    }
    descriptor
}

/// Typed input schema for `name`, dispatched per tool family.
fn typed_input_schema_for(name: &str) -> Option<Value> {
    typed_input_health::typed_input_schema_for(name)
        .or_else(|| typed_input_graph::typed_input_schema_for(name))
        .or_else(|| typed_input_context::typed_input_schema_for(name))
        .or_else(|| typed_input_analysis::typed_input_schema_for(name))
        .or_else(|| typed_input_content::typed_input_schema_for(name))
        .or_else(|| typed_input_memory::typed_input_schema_for(name))
}

fn typed_schema_with_descriptions<T: JsonSchema>(descriptions: &[(&str, &str)]) -> Value {
    let mut schema = serde_json::to_value(schemars::schema_for!(T)).expect("typed schema value");
    for &(path, description) in descriptions {
        annotate_schema_description(&mut schema, path, description);
    }
    schema
}

fn annotate_schema_description(schema: &mut Value, path: &str, description: &str) {
    let mut current = schema;
    for segment in path.split('/') {
        match current {
            Value::Object(object) => {
                let Some(next) = object.get_mut(segment) else {
                    return;
                };
                current = next;
            }
            Value::Array(items) if segment == "items" => {
                if items.is_empty() {
                    return;
                }
                current = &mut items[0];
            }
            _ => return,
        }
    }
    if let Value::Object(object) = current {
        object.insert(
            "description".to_owned(),
            Value::String(description.to_owned()),
        );
    }
}

/// Output schema for `name`, dispatched per tool family.
fn tool_output_schema_for(name: &str) -> Option<Value> {
    match name {
        "list_graph_stats" => Some(list_graph_stats_output_schema()),
        "repo_registry" => Some(repo_registry_output_schema()),
        "tool_list" => Some(tool_list_output_schema()),
        "tool_search" => Some(tool_search_output_schema()),
        "tool_help" => Some(man_output_schema()),
        "broker_status" => Some(broker_status_output_schema()),
        "build_or_update_graph" => Some(build_or_update_graph_output_schema()),
        "postprocess_graph" => Some(postprocess_graph_output_schema()),
        "status" => Some(status_output_schema()),
        "doctor" => Some(doctor_output_schema()),
        "db_check" => Some(db_check_output_schema()),
        "debug_graph" => Some(debug_graph_output_schema()),
        "query_graph" => Some(query_graph_output_schema()),
        "batch_query_graph" => Some(batch_query_graph_output_schema()),
        "explain_query" => Some(explain_query_output_schema()),
        "analyze_architecture" => Some(insight_report_output_schema()),
        "analyze_metrics" => Some(insight_report_output_schema()),
        "assess_risk" => Some(insight_report_output_schema()),
        "analyze_patterns" => Some(insight_report_output_schema()),
        "find_large_functions" => Some(large_function_report_output_schema()),
        "find_complex_functions" => Some(large_function_report_output_schema()),
        "find_similar_functions" => Some(similar_function_report_output_schema()),
        "find_duplicates" => Some(duplicate_report_output_schema()),
        "infer_modules" => Some(inferred_module_report_output_schema()),
        "label_components" => Some(component_label_report_output_schema()),
        "detect_changes" => Some(detect_changes_output_schema()),
        "get_impact_radius" => Some(get_impact_radius_output_schema()),
        "get_review_context" => Some(get_review_context_output_schema()),
        "get_minimal_context" => Some(get_minimal_context_output_schema()),
        "explain_change" => Some(explain_change_output_schema()),
        "traverse_graph" => Some(traverse_graph_output_schema()),
        "get_context" => Some(get_context_output_schema()),
        "get_session_status" => Some(get_session_status_output_schema()),
        "compact_session" => Some(compact_session_output_schema()),
        "resume_session" => Some(resume_session_output_schema()),
        "record_session_event" => Some(record_session_event_output_schema()),
        "wake_up" => Some(wake_up_output_schema()),
        "search_saved_context" => Some(search_saved_context_output_schema()),
        "search_decisions" => Some(search_decisions_output_schema()),
        "read_saved_context" => Some(read_saved_context_output_schema()),
        "save_context_artifact" => Some(save_context_artifact_output_schema()),
        "get_context_stats" => Some(get_context_stats_output_schema()),
        "purge_saved_context" => Some(purge_saved_context_output_schema()),
        "cross_session_search" => Some(cross_session_search_output_schema()),
        "get_global_memory" => Some(get_global_memory_output_schema()),
        "memory_store" => Some(memory_store_output_schema()),
        "memory_recall" => Some(memory_recall_output_schema()),
        "symbol_neighbors" => Some(symbol_neighbors_output_schema()),
        "cross_file_links" => Some(cross_file_links_output_schema()),
        "concept_clusters" => Some(concept_clusters_output_schema()),
        "analyze_safety" => Some(analyze_safety_output_schema()),
        "analyze_remove" => Some(analyze_remove_output_schema()),
        "analyze_dead_code" => Some(analyze_dead_code_output_schema()),
        "analyze_dependency" => Some(analyze_dependency_output_schema()),
        "resolve_symbol" => Some(resolve_symbol_output_schema()),
        "search_files" => Some(search_files_output_schema()),
        "search_content" => Some(search_content_output_schema()),
        "read_file_excerpt" => Some(read_file_excerpt_output_schema()),
        "get_docs_section" => Some(get_docs_section_output_schema()),
        "read_file_around_match" => Some(read_file_around_match_output_schema()),
        "search_templates" => Some(search_templates_output_schema()),
        "search_text_assets" => Some(search_text_assets_output_schema()),
        "man" => Some(man_output_schema()),
        _ => None,
    }
}
