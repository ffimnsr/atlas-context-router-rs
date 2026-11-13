//! `resolve_symbol` tool: deterministic exact → name → prefix resolution with
//! explicit opt-in fuzzy fallback.
//!
//! Kept in its own module because `graph.rs` is past the 1K LOC threshold.

use anyhow::{Context, Result};
use atlas_core::Node;
use atlas_search::{SymbolResolveRequest, resolve_symbol_nodes};
use camino::Utf8Path;
use serde::Serialize;
use serde_json::{Value, json};

use crate::tool_result::{
    InputShapeErrorSpec, input_shape_error_payload, tool_execution_error_value, tool_result_value,
};

use super::shared::{
    bool_arg, inject_budget_metadata, inject_deprecated_input_fields, load_budget_policy,
    node_repo_id, node_repo_json, open_store, repo_aliases_by_id, resolve_repo_scope_selection,
    str_arg, u64_arg,
};

const DEFAULT_LIMIT: usize = 10;

pub(super) fn tool_resolve_symbol(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let repo_scope = match resolve_repo_scope_selection("resolve_symbol", args, repo_root) {
        Ok(scope) => scope,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let repo_aliases = repo_aliases_by_id(repo_root);
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = atlas_core::BudgetManager::new();

    let name = str_arg(args, "name")?
        .ok_or_else(|| anyhow::anyhow!("resolve_symbol requires 'name'"))?
        .to_owned();
    let kind_input = str_arg(args, "kind")?.map(str::to_owned);
    let file_filter = str_arg(args, "file")?.map(str::to_owned);
    // Best-effort normalization: root-prefixed and absolute-under-root forms
    // collapse to canonical repo-relative identity; unmatched inputs stay as
    // typed so the contains-filter can still match literally.
    let file_filter = file_filter.map(|raw| {
        atlas_repo::normalize_repo_file_path(Utf8Path::new(repo_root), &raw)
            .map(|resolved| resolved.canonical)
            .unwrap_or(raw)
    });
    let language = str_arg(args, "language")?.map(str::to_owned);
    let fuzzy = bool_arg(args, "fuzzy").unwrap_or(false);
    let requested_limit = u64_arg(args, "limit").unwrap_or(DEFAULT_LIMIT as u64) as usize;
    let limit = budgets.resolve_limit(
        policy.query_candidates_and_seeds.candidates,
        "query_candidates_and_seeds.max_candidates",
        Some(requested_limit),
    );

    if name.trim().is_empty() {
        anyhow::bail!("resolve_symbol requires non-empty 'name'");
    }

    let store = open_store(db_path)?;
    let resolution = resolve_symbol_nodes(
        &store,
        &SymbolResolveRequest {
            name: &name,
            kind: kind_input.as_deref(),
            language: language.as_deref(),
            file: file_filter.as_deref(),
            limit,
            fuzzy,
        },
    )
    .context("resolve_symbol lookup failed")?;

    let selected_repo_ids = repo_scope
        .selection
        .as_ref()
        .map(|scope| scope.repo_ids.clone())
        .unwrap_or_default();
    let mut matches: Vec<Node> = resolution.matches;
    if !selected_repo_ids.is_empty() {
        let in_scope: Vec<Node> = matches
            .iter()
            .filter(|node| {
                node_repo_id(node).is_none_or(|repo_id| {
                    selected_repo_ids
                        .iter()
                        .any(|candidate| candidate == repo_id)
                })
            })
            .cloned()
            .collect();
        if in_scope.is_empty() && !matches.is_empty() {
            let payload = input_shape_error_payload(
                "resolve_symbol",
                format!("symbol '{name}' is outside selected repo scope"),
                "Symbol resolved, but not inside requested repo_id/all_repos scope. Retry without repo selector or choose matching repo_id.".to_owned(),
                InputShapeErrorSpec {
                    offending_fields: vec!["name".to_owned(), "repo_id".to_owned()],
                    normalization_performed: Vec::new(),
                    accepted_argument_families: vec!["name".to_owned(), "name + repo_id".to_owned()],
                    retry_example: Some(json!({"name": name})),
                    fail_closed_reason: Some("Atlas refused cross-repo exact resolution outside selected registry scope".to_owned()),
                    retry_guidance: Some("Pick matching repo_id or remove repo selector, then retry.".to_owned()),
                    extra_details: Some(json!({"selected_repo_ids": selected_repo_ids})),
                },
            );
            return tool_execution_error_value(output_format, &payload);
        }
        matches = in_scope;
    }

    let status = match matches.len() {
        0 => "not_found",
        1 => "resolved",
        _ => "ambiguous",
    };
    let ambiguous = matches.len() > 1;
    let truncated = resolution.truncated;

    #[derive(Serialize)]
    struct ResolvedMatch<'a> {
        qualified_name: &'a str,
        name: &'a str,
        kind: &'a str,
        file_path: &'a str,
        language: &'a str,
        line_start: u32,
        line_end: u32,
        repo: serde_json::Value,
    }

    let rendered: Vec<ResolvedMatch<'_>> = matches
        .iter()
        .map(|node| ResolvedMatch {
            qualified_name: &node.qualified_name,
            name: &node.name,
            kind: node.kind.as_str(),
            file_path: &node.file_path,
            language: &node.language,
            line_start: node.line_start,
            line_end: node.line_end,
            repo: node_repo_json(node, &repo_aliases),
        })
        .collect();

    let suggestions: Vec<Value> = match status {
        "resolved" => vec![json!({
            "hint": "Exact match resolved. Pass qualified_name to symbol_neighbors or traverse_graph for callers, callees, and relationships.",
            "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
        })],
        "ambiguous" => vec![json!({
            "hint": "Multiple symbols match. Narrow with 'file', 'kind', or 'language'. Then pass the exact qualified_name to symbol_neighbors or traverse_graph.",
            "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
        })],
        _ => vec![json!({
            "hint": "No symbol matched. Try query_graph with a regex pattern, or retry with fuzzy=true for edit-distance recovery.",
            "next_tools": ["query_graph", "explain_query"]
        })],
    };
    let warnings: Vec<String> = if status == "not_found" {
        vec![format!(
            "No symbol matched '{name}' after exact qualified name, exact name, and prefix lookup."
        )]
    } else {
        Vec::new()
    };

    let mut result = json!({
        "tool": "resolve_symbol",
        "query": {
            "name": name,
            "kind": kind_input,
            "file": file_filter,
            "language": language,
            "fuzzy": fuzzy,
        },
        "best_match": rendered.first(),
        "ambiguity": {
            "ambiguous": ambiguous,
            "matches": rendered,
        },
        "resolution": {
            "stage": resolution.stage.map(|stage| stage.as_str()),
            "fuzzy_enabled": fuzzy,
        },
        "suggestions": suggestions,
        "summary": {
            "status": status,
            "match_count": matches.len(),
            "truncated": truncated,
        },
        "warnings": warnings,
    });
    if status == "not_found" {
        result["best_match"] = Value::Null;
    }

    let mut response = tool_result_value(&result, output_format)?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "query_candidates_and_seeds.max_candidates",
            limit,
            requested_limit.max(matches.len()),
        ),
    );
    inject_deprecated_input_fields(&mut response, &repo_scope.deprecated_input_fields);
    Ok(response)
}
