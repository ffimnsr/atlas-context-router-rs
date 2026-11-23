use anyhow::{Context, Result};
use atlas_core::{BudgetManager, BudgetStatus, RankingEvidence, SearchQuery};
use atlas_review::normalize_qn_kind_tokens;
use atlas_search::semantic as sem;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::time::Instant;

use crate::context::{compact_node, package_impact};
use crate::tool_result::{
    InputShapeErrorSpec, ToolErrorPayload, ToolSuccessEnvelope, input_shape_error_payload,
    normalized_tool_result_value, tool_execution_error_value,
};

use super::analysis_payloads;
use super::config_defaults::hybrid_enabled_default;
use super::shared::{
    bool_arg, error_code_docs, error_message, error_suggestions, inject_budget_metadata,
    inject_deprecated_input_fields, load_budget_policy, load_embedding_config,
    mcp_query_looks_like_unstructured_description, mcp_supported_query_grammar_examples,
    node_repo_json, open_store, parse_mcp_query_grammar, repo_aliases_by_id,
    resolve_repo_scope_selection, str_arg, string_array_arg, tool_result_value, u64_arg,
};
use super::traversal_wire;

fn ranking_evidence_legend_json() -> serde_json::Value {
    atlas_core::ranking_evidence_legend()
}

fn context_intent_str(intent: atlas_core::model::ContextIntent) -> &'static str {
    match intent {
        atlas_core::model::ContextIntent::Symbol => "symbol",
        atlas_core::model::ContextIntent::File => "file",
        atlas_core::model::ContextIntent::Review => "review",
        atlas_core::model::ContextIntent::Impact => "impact",
        atlas_core::model::ContextIntent::ImpactAnalysis => "impact_analysis",
        atlas_core::model::ContextIntent::UsageLookup => "usage_lookup",
        atlas_core::model::ContextIntent::RefactorSafety => "refactor_safety",
        atlas_core::model::ContextIntent::DeadCodeCheck => "dead_code_check",
        atlas_core::model::ContextIntent::RenamePreview => "rename_preview",
        atlas_core::model::ContextIntent::DependencyRemoval => "dependency_removal",
    }
}

fn normalized_optional_query_regex(raw: Option<&str>) -> Option<String> {
    raw.and_then(|pattern| {
        let trimmed = pattern.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

fn query_graph_empty_semantic_hint(semantic: bool, items_empty: bool) -> Option<String> {
    if !semantic || !items_empty {
        return None;
    }
    Some(
        "FTS found no symbol names matching the query text. \
         FTS searches indexed identifiers, not natural language phrases. \
         Try: (1) a short exact symbol name like 'BalancesTab'; \
         (2) the regex param for pattern matching (e.g. regex='Balance'); \
         (3) get_context with a file path; \
         (4) list_graph_stats to confirm the graph has been built."
            .to_owned(),
    )
}

fn parsed_query_intent_json(parsed_query: &crate::tools::shared::McpQueryGrammar) -> Value {
    serde_json::json!({
        "kind": parsed_query.kind,
        "source_text": parsed_query.source_text,
        "normalized_text": parsed_query.normalized_text,
        "intent": context_intent_str(parsed_query.parsed_request.intent),
        "accepted": parsed_query.accepted,
    })
}

fn validate_query_graph_inputs(
    tool_name: &str,
    text: &str,
    regex: &Option<String>,
    had_text_input: bool,
    had_regex_input: bool,
) -> std::result::Result<(), Box<ToolErrorPayload>> {
    if text.trim().is_empty() && regex.is_none() {
        let mut normalization_performed = Vec::new();
        if had_text_input && text.trim().is_empty() {
            normalization_performed.push("trimmed whitespace-only text to empty".to_owned());
        }
        if had_regex_input {
            normalization_performed.push("normalized empty regex to missing".to_owned());
        }
        return Err(Box::new(input_shape_error_payload(
            tool_name,
            format!("{tool_name} needs non-empty 'text', non-empty 'regex', or both"),
            "Provide a non-empty text query, a non-empty regex pattern, or both. Atlas refused to guess because both searchable inputs were empty after normalization.",
            InputShapeErrorSpec {
                offending_fields: vec!["text".to_owned(), "regex".to_owned()],
                normalization_performed,
                accepted_argument_families: vec![
                    "text".to_owned(),
                    "regex".to_owned(),
                    "text + regex".to_owned(),
                ],
                retry_example: Some(serde_json::json!({ "text": "compute" })),
                fail_closed_reason: Some(
                    "Atlas refused to guess because both searchable inputs were empty after normalization"
                        .to_owned(),
                ),
                retry_guidance: Some("Provide one accepted query shape and retry.".to_owned()),
                extra_details: Some(serde_json::json!({
                    "alternate_retry_example": { "regex": "compute|handle_request" }
                })),
            },
        )));
    }

    if mcp_query_looks_like_unstructured_description(text) {
        return Err(Box::new(input_shape_error_payload(
            tool_name,
            format!(
                "{tool_name} text must be exact identifier, qualified name, or supported intent phrase"
            ),
            "Natural-language-only descriptions are not accepted here. Use a plain symbol identifier, exact qualified name, or one of the supported intent phrases with a concrete identifier.",
            InputShapeErrorSpec {
                offending_fields: vec!["text".to_owned()],
                normalization_performed: Vec::new(),
                accepted_argument_families: mcp_supported_query_grammar_examples()
                    .iter()
                    .map(|example| example.to_string())
                    .collect(),
                retry_example: Some(serde_json::json!({ "text": "who calls handle_request" })),
                fail_closed_reason: Some(
                    "Atlas refused to guess a concrete symbol from a natural-language-only description"
                        .to_owned(),
                ),
                retry_guidance: Some(
                    "Retry with exact identifier, exact qualified name, or supported intent phrase containing a concrete identifier."
                        .to_owned(),
                ),
                extra_details: Some(serde_json::json!({
                    "supported_query_grammar": mcp_supported_query_grammar_examples(),
                })),
            },
        )));
    }

    if let Some(pat) = regex {
        regex::Regex::new(pat)
            .map_err(|e| Box::new(input_shape_error_payload(
                tool_name,
                format!("invalid regex pattern: {e}"),
                format!("invalid regex pattern: {e}"),
                InputShapeErrorSpec {
                    offending_fields: vec!["regex".to_owned()],
                    normalization_performed: Vec::new(),
                    accepted_argument_families: vec!["regex".to_owned(), "text + regex".to_owned()],
                    retry_example: Some(serde_json::json!({ "regex": "compute|handle_request" })),
                    fail_closed_reason: None,
                    retry_guidance: Some(
                        "Fix regex syntax, or switch to literal text search if regex is not required, then retry."
                            .to_owned(),
                    ),
                    extra_details: None,
                },
            )))?;
    }

    Ok(())
}

#[derive(Debug)]
struct ParsedBatchQueryItems {
    items: Vec<Value>,
    deprecated_input_fields: Vec<String>,
}

fn batch_query_graph_input_error(
    message: impl Into<String>,
    detail: impl Into<String>,
    offending_fields: Vec<&str>,
    retry_example: Value,
) -> Box<ToolErrorPayload> {
    Box::new(input_shape_error_payload(
        "batch_query_graph",
        message,
        detail,
        InputShapeErrorSpec {
            offending_fields: offending_fields.into_iter().map(str::to_owned).collect(),
            normalization_performed: Vec::new(),
            accepted_argument_families: vec!["items".to_owned()],
            retry_example: Some(retry_example),
            fail_closed_reason: Some(
                "Atlas refused to guess between conflicting batch_query_graph input families"
                    .to_owned(),
            ),
            retry_guidance: Some(
                "Send items=[...] with 1-20 query_graph-shaped objects, then retry.".to_owned(),
            ),
            extra_details: None,
        },
    ))
}

fn parse_batch_query_items(
    args: Option<&Value>,
) -> std::result::Result<ParsedBatchQueryItems, Box<ToolErrorPayload>> {
    const MAX_QUERIES: usize = 20;

    if args.is_some_and(|value| value.get("queries").is_some() || value.get("text").is_some()) {
        return Err(batch_query_graph_input_error(
            "legacy batch_query_graph fields are no longer supported",
            "Use top-level items=[...] and remove legacy queries/text fields.",
            vec!["queries", "text"],
            serde_json::json!({ "items": [{ "text": "compute" }] }),
        ));
    }

    let items = args
        .and_then(|a| a.get("items"))
        .and_then(Value::as_array)
        .ok_or_else(|| {
            batch_query_graph_input_error(
                "items is required",
                "Provide items=[...] with 1-20 query_graph-shaped objects.",
                vec!["items"],
                serde_json::json!({ "items": [{ "text": "compute" }] }),
            )
        })?;
    if items.is_empty() {
        return Err(batch_query_graph_input_error(
            "batch_query_graph items must be non-empty",
            "Provide at least one query object in items.",
            vec!["items"],
            serde_json::json!({ "items": [{ "text": "compute" }] }),
        ));
    }
    if items.len() > MAX_QUERIES {
        return Err(batch_query_graph_input_error(
            format!("batch_query_graph exceeds maximum of {MAX_QUERIES} items"),
            format!(
                "Provide at most {MAX_QUERIES} query objects in items, or split the request into smaller batches."
            ),
            vec!["items"],
            serde_json::json!({ "items": [{ "text": "compute" }] }),
        ));
    }
    Ok(ParsedBatchQueryItems {
        items: items.to_vec(),
        deprecated_input_fields: Vec::new(),
    })
}

mod batch;
mod explain;
mod links;
mod navigation;
mod query;

pub(super) use self::batch::tool_batch_query_graph;
pub(super) use self::explain::tool_explain_query;
pub(crate) use self::links::tool_concept_clusters;
pub(crate) use self::links::tool_cross_file_links;
pub(crate) use self::navigation::tool_symbol_neighbors;
pub(super) use self::navigation::tool_traverse_graph;
pub(super) use self::query::tool_query_graph;
