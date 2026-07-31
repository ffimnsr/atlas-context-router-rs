use anyhow::{Context, Result};
use atlas_core::model::ContextTarget;
use atlas_core::{BudgetManager, BudgetStatus, RankingEvidence, SearchQuery};
use atlas_review::{ResolvedTarget, normalize_qn_kind_tokens, resolve_target};
use atlas_search::semantic as sem;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::time::Instant;

use crate::context::{compact_node, package_impact};
use crate::tool_result::{
    InputShapeErrorSpec, ToolErrorPayload, ToolSuccessEnvelope, input_shape_error_payload,
    normalized_tool_result_value, tool_execution_error_value,
};

use super::shared::{
    bool_arg, error_code_docs, error_message, error_suggestions, inject_budget_metadata,
    inject_deprecated_input_fields, load_budget_policy, load_embedding_config,
    mcp_query_looks_like_unstructured_description, mcp_supported_query_grammar_examples,
    open_store, parse_mcp_query_grammar, repo_aliases_by_id, resolve_kind_alias,
    resolve_repo_scope_selection, str_arg, string_array_arg, tool_result_value, u64_arg,
};

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

fn node_repo_id(node: &atlas_core::Node) -> Option<&str> {
    node.extra_json
        .as_object()
        .and_then(|extra| extra.get("repo_id"))
        .and_then(|value| value.as_str())
}

fn node_repo_json(
    node: &atlas_core::Node,
    repo_aliases: &std::collections::BTreeMap<String, String>,
) -> serde_json::Value {
    let repo_id = node_repo_id(node);
    serde_json::json!({
        "repo_id": repo_id,
        "display_alias": repo_id.and_then(|id| repo_aliases.get(id)).cloned(),
    })
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

pub(super) fn tool_list_graph_stats(
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let store = open_store(db_path)?;
    let stats = store.stats().context("stats query failed")?;
    tool_result_value(&stats, output_format)
}

pub(super) fn tool_query_graph(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let raw_text = str_arg(args, "text")?;
    let had_text_input = raw_text.is_some();
    let source_text = raw_text.map(str::to_owned).unwrap_or_default();
    let kind = str_arg(args, "kind")?.map(str::to_owned);
    let language = str_arg(args, "language")?.map(str::to_owned);
    let requested_limit = u64_arg(args, "limit").unwrap_or(20) as usize;
    let semantic = bool_arg(args, "semantic").unwrap_or(false);
    let expand = bool_arg(args, "expand").unwrap_or(false);
    let expand_hops = u64_arg(args, "expand_hops").unwrap_or(1) as u32;
    let raw_regex = str_arg(args, "regex")?;
    let had_regex_input = raw_regex.is_some();
    let regex = normalized_optional_query_regex(raw_regex);
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let fuzzy = bool_arg(args, "fuzzy").unwrap_or(false);
    let hybrid = bool_arg(args, "hybrid").unwrap_or(false);
    let include_files = bool_arg(args, "include_files").unwrap_or(false);
    let repo_scope = match resolve_repo_scope_selection("query_graph", args, repo_root) {
        Ok(scope) => scope,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };

    if let Err(payload) = validate_query_graph_inputs(
        "query_graph",
        &source_text,
        &regex,
        had_text_input,
        had_regex_input,
    ) {
        return tool_execution_error_value(output_format, &payload);
    }

    let parsed_query = parse_mcp_query_grammar(&source_text);
    let text = parsed_query.normalized_text.clone();

    let store = open_store(db_path)?;
    let embed_cfg = load_embedding_config(repo_root)?;
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let limit = budgets.resolve_limit(
        policy.query_candidates_and_seeds.candidates,
        "query_candidates_and_seeds.max_candidates",
        Some(requested_limit),
    );
    let query = SearchQuery {
        text,
        kind,
        language,
        include_files,
        limit,
        subpath,
        graph_expand: expand,
        graph_max_hops: expand_hops,
        regex_pattern: regex,
        fuzzy_match: fuzzy,
        hybrid,
        repo_ids: repo_scope
            .selection
            .as_ref()
            .map(|scope| scope.repo_ids.clone())
            .unwrap_or_default(),
        ..Default::default()
    };

    let started_at = Instant::now();
    let results =
        atlas_search::execute_query_with_embedding(&store, &query, semantic, embed_cfg.as_ref())
            .context("search failed")?;
    let elapsed_ms = started_at.elapsed().as_millis() as usize;
    budgets.record_usage(
        policy.query_candidates_and_seeds.wall_time_ms,
        "query_candidates_and_seeds.max_query_wall_time_ms",
        policy.query_candidates_and_seeds.wall_time_ms.default_limit,
        elapsed_ms,
        elapsed_ms > policy.query_candidates_and_seeds.wall_time_ms.default_limit,
    );
    let explanation = atlas_search::explain_query_with_embedding(
        Some(&store),
        true,
        &query,
        semantic,
        embed_cfg.as_ref(),
    );

    #[derive(Serialize)]
    struct CompactResult<'a> {
        score: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        ranking_evidence: Option<RankingEvidence>,
        repo: serde_json::Value,
        #[serde(flatten)]
        node: crate::context::CompactNode<'a>,
    }

    let repo_aliases = repo_aliases_by_id(repo_root);
    let compact: Vec<CompactResult<'_>> = results
        .iter()
        .map(|r| CompactResult {
            score: (r.score * 1000.0).round() / 1000.0,
            ranking_evidence: r.ranking_evidence.clone(),
            repo: node_repo_json(&r.node, &repo_aliases),
            node: compact_node(&r.node),
        })
        .collect();

    let warnings = query_graph_empty_semantic_hint(semantic, compact.is_empty())
        .into_iter()
        .collect::<Vec<_>>();
    let truncated = compact.len() == limit;
    let query_intent = parsed_query_intent_json(&parsed_query);

    let mut response = normalized_tool_result_value(
        &serde_json::json!({
            "tool": "query_graph",
            "query": {
                "text": source_text,
                "normalized_text": query.text,
                "regex": query.regex_pattern,
                "kind": query.kind,
                "language": query.language,
                "requested_limit": requested_limit,
                "applied_limit": limit,
                "semantic": semantic,
                "expand": query.graph_expand,
                "expand_hops": query.graph_max_hops,
                "subpath": query.subpath,
                "fuzzy": query.fuzzy_match,
                "hybrid": query.hybrid,
                "include_files": query.include_files,
                "repo_scope": {
                    "selection": repo_scope.selection.as_ref().map(|selection| serde_json::json!({
                        "kind": if selection.repo_ids.len() > 1 { "all" } else { "repo_id" },
                        "repo_ids": selection.repo_ids,
                        "repo_count": selection.repo_ids.len(),
                    })),
                    "deprecated_input_fields": repo_scope.deprecated_input_fields.clone(),
                },
                "query_intent": query_intent,
                "active_query_mode": explanation.active_query_mode,
            },
            "matches": compact,
            "summary": {
                "match_count": results.len(),
                "returned_count": results.len(),
                "usage_edges_included": false,
                "relationship_tools": ["symbol_neighbors", "traverse_graph", "get_context"],
                "ranking_evidence_legend": ranking_evidence_legend_json(),
            },
            "truncated": truncated,
            "warnings": warnings,
        }),
        output_format,
    )?;
    response["atlas_usage_edges_included"] = serde_json::Value::Bool(false);
    response["atlas_relationship_tools"] =
        serde_json::json!(["symbol_neighbors", "traverse_graph", "get_context"]);
    response["atlas_truncated"] = serde_json::json!(truncated);
    response["atlas_query_mode"] = serde_json::Value::String(explanation.active_query_mode);
    response["atlas_ranking_evidence_legend"] = ranking_evidence_legend_json();
    response["atlas_query_elapsed_ms"] = serde_json::json!(elapsed_ms);
    response["atlas_query_intent"] = query_intent;
    let budget = budgets.summary(
        "query_candidates_and_seeds.max_candidates",
        limit,
        results.len(),
    );
    inject_budget_metadata(&mut response, &budget);
    inject_deprecated_input_fields(&mut response, &repo_scope.deprecated_input_fields);
    Ok(response)
}

pub(super) fn tool_batch_query_graph(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let repo_scope = match resolve_repo_scope_selection("batch_query_graph", args, repo_root) {
        Ok(scope) => scope,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let parsed_items = match parse_batch_query_items(args) {
        Ok(parsed) => parsed,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };

    let store = open_store(db_path)?;
    let embed_cfg = load_embedding_config(repo_root)?;
    let policy = load_budget_policy(repo_root)?;

    #[derive(Serialize)]
    struct BatchResultNode {
        score: f64,
        #[serde(skip_serializing_if = "Option::is_none")]
        ranking_evidence: Option<RankingEvidence>,
        repo: serde_json::Value,
        name: String,
        qualified_name: String,
        kind: String,
        file_path: String,
        line_start: u32,
        language: String,
    }

    let mut normalized_items: Vec<Value> = Vec::with_capacity(parsed_items.items.len());
    let mut batch_results: Vec<Value> = Vec::with_capacity(parsed_items.items.len());
    let mut batch_budget_reports = Vec::with_capacity(parsed_items.items.len());
    let repo_aliases = repo_aliases_by_id(repo_root);

    for (idx, q) in parsed_items.items.iter().enumerate() {
        let q_args = Some(q);
        let raw_text = str_arg(q_args, "text")?;
        let had_text_input = raw_text.is_some();
        let source_text = raw_text.map(str::to_owned).unwrap_or_default();
        let kind = str_arg(q_args, "kind")?.map(str::to_owned);
        let language = str_arg(q_args, "language")?.map(str::to_owned);
        let requested_limit = u64_arg(q_args, "limit").unwrap_or(20) as usize;
        let semantic = bool_arg(q_args, "semantic").unwrap_or(false);
        let expand = bool_arg(q_args, "expand").unwrap_or(false);
        let expand_hops = u64_arg(q_args, "expand_hops").unwrap_or(1) as u32;
        let raw_regex = str_arg(q_args, "regex")?;
        let had_regex_input = raw_regex.is_some();
        let regex = normalized_optional_query_regex(raw_regex);
        let subpath = str_arg(q_args, "subpath")?.map(str::to_owned);
        let fuzzy = bool_arg(q_args, "fuzzy").unwrap_or(false);
        let hybrid = bool_arg(q_args, "hybrid").unwrap_or(false);
        let include_files = bool_arg(q_args, "include_files").unwrap_or(false);

        let mut budgets = BudgetManager::new();
        let budget_name = format!("query_candidates_and_seeds.max_candidates[{idx}]");
        let limit = budgets.resolve_limit(
            policy.query_candidates_and_seeds.candidates,
            budget_name.clone(),
            Some(requested_limit),
        );

        validate_query_graph_inputs(
            &format!("query at index {idx}"),
            &source_text,
            &regex,
            had_text_input,
            had_regex_input,
        )
        .map_err(|payload| anyhow::anyhow!(payload.message.clone()))?;
        let parsed_query = parse_mcp_query_grammar(&source_text);
        let text = parsed_query.normalized_text.clone();

        let query = SearchQuery {
            text: text.clone(),
            kind,
            language,
            include_files,
            limit,
            subpath,
            graph_expand: expand,
            graph_max_hops: expand_hops,
            regex_pattern: regex,
            fuzzy_match: fuzzy,
            hybrid,
            repo_ids: repo_scope
                .selection
                .as_ref()
                .map(|scope| scope.repo_ids.clone())
                .unwrap_or_default(),
            ..Default::default()
        };

        let started_at = Instant::now();
        let results = atlas_search::execute_query_with_embedding(
            &store,
            &query,
            semantic,
            embed_cfg.as_ref(),
        )
        .context("search failed")?;
        let elapsed_ms = started_at.elapsed().as_millis() as usize;
        budgets.record_usage(
            policy.query_candidates_and_seeds.wall_time_ms,
            format!("query_candidates_and_seeds.max_query_wall_time_ms[{idx}]"),
            policy.query_candidates_and_seeds.wall_time_ms.default_limit,
            elapsed_ms,
            elapsed_ms > policy.query_candidates_and_seeds.wall_time_ms.default_limit,
        );

        let items: Vec<BatchResultNode> = results
            .iter()
            .map(|r| BatchResultNode {
                score: (r.score * 1000.0).round() / 1000.0,
                ranking_evidence: r.ranking_evidence.clone(),
                repo: node_repo_json(&r.node, &repo_aliases),
                name: r.node.name.clone(),
                qualified_name: r.node.qualified_name.clone(),
                kind: r.node.kind.as_str().to_owned(),
                file_path: r.node.file_path.clone(),
                line_start: r.node.line_start,
                language: r.node.language.clone(),
            })
            .collect();

        let warnings = query_graph_empty_semantic_hint(semantic, items.is_empty())
            .into_iter()
            .collect::<Vec<_>>();
        let query_intent = parsed_query_intent_json(&parsed_query);
        let truncated = items.len() == limit;

        normalized_items.push(serde_json::json!({
            "query_index": idx,
            "text": source_text,
            "normalized_text": text,
            "regex": query.regex_pattern,
            "kind": query.kind,
            "language": query.language,
            "requested_limit": requested_limit,
            "applied_limit": limit,
            "semantic": semantic,
            "expand": query.graph_expand,
            "expand_hops": query.graph_max_hops,
            "subpath": query.subpath,
            "fuzzy": query.fuzzy_match,
            "hybrid": query.hybrid,
            "include_files": query.include_files,
            "query_intent": query_intent,
        }));
        batch_results.push(serde_json::json!({
            "query_index": idx,
            "matches": items,
            "summary": {
                "match_count": results.len(),
                "returned_count": results.len(),
            },
            "truncated": truncated,
            "warnings": warnings,
        }));
        batch_budget_reports.push(budgets.summary(budget_name, limit, results.len()));
    }

    let ranking_evidence_legend = ranking_evidence_legend_json();
    let mut response = normalized_tool_result_value(
        &serde_json::json!({
            "tool": "batch_query_graph",
            "items": normalized_items,
            "results": batch_results,
            "summary": {
                "query_count": parsed_items.items.len(),
                "ranking_evidence_legend": ranking_evidence_legend,
            },
            "warnings": [],
        }),
        output_format,
    )?;
    let worst_budget = batch_budget_reports
        .into_iter()
        .max_by(|left, right| {
            let left_rank = match left.budget_status {
                BudgetStatus::WithinBudget => 0,
                BudgetStatus::OverrideClamped => 1,
                BudgetStatus::PartialResult => 2,
                BudgetStatus::Blocked => 3,
            };
            let right_rank = match right.budget_status {
                BudgetStatus::WithinBudget => 0,
                BudgetStatus::OverrideClamped => 1,
                BudgetStatus::PartialResult => 2,
                BudgetStatus::Blocked => 3,
            };
            left_rank.cmp(&right_rank)
        })
        .unwrap_or_else(|| {
            atlas_core::BudgetReport::within_budget(
                "query_candidates_and_seeds.max_candidates",
                0,
                0,
            )
        });
    response["atlas_query_count"] =
        serde_json::Value::Number(serde_json::Number::from(parsed_items.items.len()));
    response["atlas_ranking_evidence_legend"] = ranking_evidence_legend_json();
    inject_budget_metadata(&mut response, &worst_budget);
    let mut deprecated_input_fields = parsed_items.deprecated_input_fields;
    deprecated_input_fields.extend(repo_scope.deprecated_input_fields);
    inject_deprecated_input_fields(&mut response, &deprecated_input_fields);
    Ok(response)
}

pub(super) fn tool_traverse_graph(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let from_qn = normalize_qn_kind_tokens(
        str_arg(args, "from_qn")?
            .ok_or_else(|| anyhow::anyhow!("missing required argument: from_qn"))?,
    );
    let max_depth = u64_arg(args, "max_depth").unwrap_or(3) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(100) as usize;

    let store = open_store(db_path)?;
    let policy = load_budget_policy(repo_root)?;
    let result = store
        .traverse_from_qnames(
            &[from_qn.as_str()],
            max_depth,
            max_nodes,
            policy.graph_traversal.edges.default_limit,
        )
        .context("traverse_from_qnames failed")?;

    let seeds = vec![from_qn.clone()];
    let packaged = package_impact(&result, &seeds);
    let mut payload = Map::new();
    let mut combined_nodes = serde_json::to_value(&packaged.changed_nodes)?
        .as_array()
        .cloned()
        .unwrap_or_default();
    combined_nodes.extend(
        serde_json::to_value(&packaged.impacted_nodes)?
            .as_array()
            .cloned()
            .unwrap_or_default(),
    );
    payload.insert("root_symbol".to_owned(), json!(from_qn));
    payload.insert("direction".to_owned(), json!("bidirectional"));
    payload.insert("depth".to_owned(), json!(max_depth));
    payload.insert("nodes".to_owned(), Value::Array(combined_nodes));
    payload.insert(
        "edges".to_owned(),
        Value::Array(
            result
                .relevant_edges
                .iter()
                .map(|edge| {
                    json!({
                        "from": edge.source_qn,
                        "to": edge.target_qn,
                        "kind": edge.kind.as_str(),
                        "direction": if edge.source_qn == from_qn {
                            "outbound"
                        } else if edge.target_qn == from_qn {
                            "inbound"
                        } else {
                            "transitive"
                        }
                    })
                })
                .collect(),
        ),
    );
    payload.insert(
        "summary".to_owned(),
        json!({
            "changed_symbol_count": packaged.changed_node_count,
            "impacted_symbol_count": packaged.impacted_node_count,
            "impacted_file_count": packaged.impacted_file_count,
            "relevant_edge_count": packaged.relevant_edge_count,
        }),
    );
    payload.insert("truncated".to_owned(), json!(packaged.truncated));
    payload.insert("impacted_files".to_owned(), json!(packaged.impacted_files));
    payload.insert("seed_budgets".to_owned(), json!(packaged.seed_budgets));
    payload.insert(
        "traversal_budget".to_owned(),
        json!(packaged.traversal_budget),
    );

    let envelope = ToolSuccessEnvelope::new("traverse_graph", Value::Object(payload))
        .with_truncation(
            packaged.truncated,
            packaged
                .truncated
                .then_some("traversal capped by node or edge limits"),
        );
    let mut response = normalized_tool_result_value(&envelope, output_format)?;
    inject_budget_metadata(&mut response, &result.budget);
    Ok(response)
}

pub(super) fn tool_symbol_neighbors(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let qname = normalize_qn_kind_tokens(
        str_arg(args, "qname")?
            .ok_or_else(|| anyhow::anyhow!("missing required argument: qname"))?,
    );
    let requested_limit = u64_arg(args, "limit").unwrap_or(10) as usize;
    let limit = budgets.resolve_limit(
        policy.review_context_extraction.nodes,
        "review_context_extraction.max_nodes",
        Some(requested_limit),
    );

    let store = open_store(db_path)?;
    let nbhd =
        sem::symbol_neighborhood(&store, &qname, limit).context("symbol_neighborhood failed")?;
    let caller_pairs = store
        .direct_callers(&qname, limit)
        .context("direct_callers failed")?;
    let callee_pairs = store
        .direct_callees(&qname, limit)
        .context("direct_callees failed")?;

    #[derive(Serialize)]
    struct CompactCallEdge<'a> {
        from: &'a str,
        to: &'a str,
        file: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<u32>,
        confidence: f32,
        #[serde(skip_serializing_if = "Option::is_none")]
        tier: Option<&'a str>,
    }

    fn compact_call_edge(edge: &atlas_core::Edge) -> CompactCallEdge<'_> {
        CompactCallEdge {
            from: &edge.source_qn,
            to: &edge.target_qn,
            file: &edge.file_path,
            line: edge.line,
            confidence: edge.confidence,
            tier: edge.confidence_tier.as_deref(),
        }
    }

    fn compact_unique_nodes_from_pairs<'a>(
        pairs: &'a [(atlas_core::Node, atlas_core::Edge)],
    ) -> Vec<crate::context::CompactNode<'a>> {
        let mut seen = std::collections::HashSet::new();
        let mut nodes = Vec::new();
        for (node, _) in pairs {
            if seen.insert(node.qualified_name.as_str()) {
                nodes.push(compact_node(node));
            }
        }
        nodes
    }

    let callers = compact_unique_nodes_from_pairs(&caller_pairs);
    let callees = compact_unique_nodes_from_pairs(&callee_pairs);
    let caller_edges: Vec<_> = caller_pairs
        .iter()
        .map(|(_, edge)| compact_call_edge(edge))
        .collect();
    let callee_edges: Vec<_> = callee_pairs
        .iter()
        .map(|(_, edge)| compact_call_edge(edge))
        .collect();
    let tests: Vec<_> = nbhd.tests.iter().map(compact_node).collect();
    let siblings: Vec<_> = nbhd.siblings.iter().map(compact_node).collect();
    let imports: Vec<_> = nbhd.import_neighbors.iter().map(compact_node).collect();
    let exists = store
        .node_by_qname(&qname)
        .map(|n| n.is_some())
        .unwrap_or(false);
    let status = if exists { "ok" } else { "node_not_found" };
    let warnings = if exists {
        Vec::new()
    } else {
        vec![error_message("node_not_found").to_owned()]
    };
    let call_sites = caller_edges
        .iter()
        .map(|edge| {
            json!({
                "direction": "incoming",
                "from": edge.from,
                "to": edge.to,
                "file": edge.file,
                "line": edge.line,
                "confidence": edge.confidence,
                "tier": edge.tier,
            })
        })
        .chain(callee_edges.iter().map(|edge| {
            json!({
                "direction": "outgoing",
                "from": edge.from,
                "to": edge.to,
                "file": edge.file,
                "line": edge.line,
                "confidence": edge.confidence,
                "tier": edge.tier,
            })
        }))
        .collect::<Vec<_>>();
    let payload = json!({
        "tool": "symbol_neighbors",
        "symbol": {
            "qname": qname,
            "found": exists,
        },
        "lookup": {
            "status": status,
            "error_code": if exists { Value::Null } else { json!("node_not_found") },
            "error_code_docs": if exists { Value::Null } else { json!(error_code_docs("node_not_found")) },
            "message": if exists { Value::Null } else { json!(error_message("node_not_found")) },
            "suggestions": if exists { json!([]) } else { json!(error_suggestions("node_not_found")) },
        },
        "callers": callers,
        "callees": callees,
        "call_sites": call_sites,
        "tests": tests,
        "siblings": siblings,
        "imports": imports,
        "summary": {
            "status": status,
            "caller_count": caller_pairs.len(),
            "callee_count": callee_pairs.len(),
            "call_site_count": caller_edges.len() + callee_edges.len(),
            "test_count": nbhd.tests.len(),
            "sibling_count": nbhd.siblings.len(),
            "import_count": nbhd.import_neighbors.len(),
        },
        "warnings": warnings,
    });

    let mut response = tool_result_value(&payload, output_format)?;

    let observed = caller_pairs.len()
        + callee_pairs.len()
        + nbhd.tests.len()
        + nbhd.siblings.len()
        + nbhd.import_neighbors.len()
        + caller_edges.len()
        + callee_edges.len();
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "review_context_extraction.max_nodes",
            limit,
            requested_limit.max(observed),
        ),
    );

    Ok(response)
}

pub(super) fn tool_cross_file_links(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let file = str_arg(args, "file")?
        .ok_or_else(|| anyhow::anyhow!("missing required argument: file"))?
        .to_owned();
    let requested_limit = u64_arg(args, "limit").unwrap_or(20) as usize;
    let limit = budgets.resolve_limit(
        policy.review_context_extraction.files,
        "review_context_extraction.max_files",
        Some(requested_limit),
    );

    let store = open_store(db_path)?;
    let links = sem::cross_file_links(&store, &file, limit).context("cross_file_links failed")?;

    let linked_files: Vec<_> = links
        .into_iter()
        .enumerate()
        .map(|(idx, l)| {
            let coupling_metric = (l.strength * 10.0).round() / 10.0;
            json!({
                "file": l.to_file,
                "via_symbols": l.via_symbols,
                "coupling_metric": coupling_metric,
                "rank": idx + 1,
            })
        })
        .collect();
    let total_strength = linked_files
        .iter()
        .filter_map(|item| item.get("coupling_metric").and_then(Value::as_f64))
        .sum::<f64>();
    let max_strength = linked_files
        .iter()
        .filter_map(|item| item.get("coupling_metric").and_then(Value::as_f64))
        .fold(0.0_f64, f64::max);
    let payload = json!({
        "tool": "cross_file_links",
        "source_file": file,
        "linked_files": linked_files,
        "coupling_metric": {
            "linked_file_count": linked_files.len(),
            "max_strength": max_strength,
            "total_strength": ((total_strength * 10.0).round() / 10.0),
        },
        "summary": {
            "status": "ok",
            "linked_file_count": linked_files.len(),
            "isolated": linked_files.is_empty(),
        },
        "warnings": [],
    });

    let mut response = tool_result_value(&payload, output_format)?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "review_context_extraction.max_files",
            limit,
            requested_limit.max(linked_files.len()),
        ),
    );
    Ok(response)
}

pub(super) fn tool_concept_clusters(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let files = string_array_arg(args, "files")?;
    if files.is_empty() {
        return Err(anyhow::anyhow!("missing required argument: files"));
    }
    let requested_limit = u64_arg(args, "limit").unwrap_or(10) as usize;
    let limit = budgets.resolve_limit(
        policy.review_context_extraction.files,
        "review_context_extraction.max_files",
        Some(requested_limit),
    );

    let store = open_store(db_path)?;
    let seed_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let clusters = sem::cluster_by_shared_symbols(&store, &seed_refs, limit)
        .context("concept_clusters failed")?;

    let truncated = clusters.len() >= limit;
    let result: Vec<_> = clusters
        .into_iter()
        .enumerate()
        .map(|(idx, c)| {
            json!({
                "files": c.files,
                "shared_symbols": c.shared_symbols,
                "density": (c.density * 1000.0).round() / 1000.0,
                "rank": idx + 1,
            })
        })
        .collect();
    let payload = json!({
        "tool": "concept_clusters",
        "seed_files": files,
        "clusters": result,
        "summary": {
            "status": "ok",
            "cluster_count": result.len(),
            "seed_file_count": seed_refs.len(),
        },
        "truncated": truncated,
        "warnings": [],
    });

    let mut response = tool_result_value(&payload, output_format)?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "review_context_extraction.max_files",
            limit,
            requested_limit.max(result.len()),
        ),
    );
    Ok(response)
}

pub(super) fn tool_explain_query(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let repo_scope = match resolve_repo_scope_selection("explain_query", args, repo_root) {
        Ok(scope) => scope,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();
    let raw_text = str_arg(args, "text")?;
    let had_text_input = raw_text.is_some();
    let source_text = raw_text.map(str::to_owned).unwrap_or_default();
    let kind = str_arg(args, "kind")?.map(str::to_owned);
    let language = str_arg(args, "language")?.map(str::to_owned);
    let requested_limit = u64_arg(args, "limit").unwrap_or(20) as usize;
    let limit = budgets.resolve_limit(
        policy.query_candidates_and_seeds.candidates,
        "query_candidates_and_seeds.max_candidates",
        Some(requested_limit),
    );
    let semantic = bool_arg(args, "semantic").unwrap_or(false);
    let raw_regex = str_arg(args, "regex")?;
    let had_regex_input = raw_regex.is_some();
    let regex = normalized_optional_query_regex(raw_regex);
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let fuzzy = bool_arg(args, "fuzzy").unwrap_or(false);
    let hybrid = bool_arg(args, "hybrid").unwrap_or(false);
    let include_files = bool_arg(args, "include_files").unwrap_or(false);

    if let Err(payload) = validate_query_graph_inputs(
        "explain_query",
        &source_text,
        &regex,
        had_text_input,
        had_regex_input,
    ) {
        return tool_execution_error_value(output_format, &payload);
    }

    let parsed_query = parse_mcp_query_grammar(&source_text);
    let text = parsed_query.normalized_text.clone();

    let db_exists = std::path::Path::new(db_path).exists();
    let embed_cfg = load_embedding_config(repo_root)?;
    let query = SearchQuery {
        text,
        kind,
        language,
        include_files,
        limit,
        subpath,
        regex_pattern: regex,
        fuzzy_match: fuzzy,
        hybrid,
        repo_ids: repo_scope
            .selection
            .as_ref()
            .map(|scope| scope.repo_ids.clone())
            .unwrap_or_default(),
        ..Default::default()
    };
    let store = if db_exists {
        atlas_store_sqlite::Store::open(db_path).ok()
    } else {
        None
    };
    let result = atlas_search::explain_query_with_embedding(
        store.as_ref(),
        db_exists,
        &query,
        semantic,
        embed_cfg.as_ref(),
    );

    let fts_token_count = result.fts_tokens.len();
    let fts_phrase = result.fts_phrase.clone();
    let matches = result.matches.clone().unwrap_or_default();
    let payload = json!({
        "input": result.input,
        "normalized_query": {
            "active_query_mode": result.active_query_mode,
            "search_path": result.search_path,
            "indexed_node_count": result.indexed_node_count,
            "db_exists": result.db_exists,
            "ranking_factors": result.ranking_factors,
            "filters_applied": result.filters_applied,
            "active_capabilities": result.active_capabilities,
        },
        "tokenization": {
            "fts_tokens": result.fts_tokens,
            "fts_phrase": fts_phrase.clone(),
        },
        "fts_plan": {
            "enabled": !query.text.trim().is_empty(),
            "phrase": fts_phrase,
            "token_count": fts_token_count,
            "limit": query.limit,
            "semantic": semantic,
            "expand": query.graph_expand,
            "include_files": query.include_files,
        },
        "regex_plan": {
            "enabled": query.regex_pattern.is_some(),
            "pattern": query.regex_pattern,
            "valid": result.regex_valid,
            "error": result.regex_error,
        },
        "warnings": result.warnings,
        "latency_ms": result.latency_ms.map(|value| value as u64),
        "result_count": result.result_count.unwrap_or(matches.len()),
        "matches": matches,
    });

    let envelope = ToolSuccessEnvelope::new("explain_query", payload);
    let mut response = normalized_tool_result_value(&envelope, output_format)?;
    response["atlas_ranking_evidence_legend"] = ranking_evidence_legend_json();
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "query_candidates_and_seeds.max_candidates",
            limit,
            requested_limit.max(limit),
        ),
    );
    inject_deprecated_input_fields(&mut response, &repo_scope.deprecated_input_fields);
    Ok(response)
}

pub(super) fn tool_resolve_symbol(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    const DEFAULT_LIMIT: usize = 10;
    let repo_scope = match resolve_repo_scope_selection("resolve_symbol", args, repo_root) {
        Ok(scope) => scope,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let repo_aliases = repo_aliases_by_id(repo_root);
    let policy = load_budget_policy(repo_root)?;
    let mut budgets = BudgetManager::new();

    let name = str_arg(args, "name")?
        .ok_or_else(|| anyhow::anyhow!("resolve_symbol requires 'name'"))?
        .to_owned();
    let kind_input = str_arg(args, "kind")?.map(str::to_owned);
    let file_filter = str_arg(args, "file")?.map(str::to_owned);
    let language = str_arg(args, "language")?.map(str::to_owned);
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

    if name.contains("::") {
        let target = ContextTarget::QualifiedName {
            qname: name.clone(),
        };
        match resolve_target(&store, &target).context("resolve_symbol qname lookup failed")? {
            ResolvedTarget::Node(node) => {
                if let Some(scope) = repo_scope.selection.as_ref()
                    && node_repo_id(&node).is_some_and(|repo_id| {
                        !scope.repo_ids.iter().any(|candidate| candidate == repo_id)
                    })
                {
                    let payload = input_shape_error_payload(
                        "resolve_symbol",
                        format!("symbol '{name}' is outside selected repo scope"),
                        "Qualified name resolved, but not inside requested repo_id/all_repos scope. Retry without repo selector or choose matching repo_id.".to_owned(),
                        InputShapeErrorSpec {
                            offending_fields: vec!["name".to_owned(), "repo_id".to_owned()],
                            normalization_performed: Vec::new(),
                            accepted_argument_families: vec!["name".to_owned(), "name + repo_id".to_owned()],
                            retry_example: Some(json!({"name": name})),
                            fail_closed_reason: Some("Atlas refused cross-repo exact resolution outside selected registry scope".to_owned()),
                            retry_guidance: Some("Pick matching repo_id or remove repo selector, then retry.".to_owned()),
                            extra_details: Some(json!({"selected_repo_ids": scope.repo_ids})),
                        },
                    );
                    return tool_execution_error_value(output_format, &payload);
                }
                #[derive(Serialize)]
                struct ResolvedMatch<'a> {
                    qualified_name: &'a str,
                    name: &'a str,
                    kind: &'a str,
                    file_path: &'a str,
                    language: &'a str,
                    line_start: u32,
                    repo: serde_json::Value,
                }
                let m = ResolvedMatch {
                    qualified_name: &node.qualified_name,
                    name: &node.name,
                    kind: node.kind.as_str(),
                    file_path: &node.file_path,
                    language: &node.language,
                    line_start: node.line_start,
                    repo: node_repo_json(&node, &repo_aliases),
                };
                let normalised = normalize_qn_kind_tokens(&name);
                let alias_note = if normalised != name {
                    Some(format!(
                        "Input '{name}' normalised to canonical QN '{normalised}'"
                    ))
                } else {
                    None
                };
                let result = serde_json::json!({
                    "tool": "resolve_symbol",
                    "query": {
                        "name": name,
                        "kind": kind_input,
                        "file": file_filter,
                        "language": language,
                    },
                    "best_match": m,
                    "ambiguity": {
                        "ambiguous": false,
                        "matches": [m],
                    },
                    "suggestions": [{
                        "hint": "Exact match resolved. Pass qualified_name to symbol_neighbors or traverse_graph for callers, callees, and relationships.",
                        "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
                    }],
                    "summary": {
                        "status": "resolved",
                        "match_count": 1,
                        "truncated": false,
                    },
                    "warnings": alias_note.into_iter().collect::<Vec<_>>(),
                });
                let mut response = tool_result_value(&result, output_format)?;
                inject_budget_metadata(
                    &mut response,
                    &budgets.summary(
                        "query_candidates_and_seeds.max_candidates",
                        limit,
                        requested_limit.max(1),
                    ),
                );
                inject_deprecated_input_fields(&mut response, &repo_scope.deprecated_input_fields);
                return Ok(response);
            }
            ResolvedTarget::Ambiguous(meta) => {
                let result = serde_json::json!({
                    "tool": "resolve_symbol",
                    "query": {
                        "name": name,
                        "kind": kind_input,
                        "file": file_filter,
                        "language": language,
                    },
                    "best_match": Value::Null,
                    "ambiguity": {
                        "ambiguous": true,
                        "matches": serde_json::Value::Array(
                            meta.candidates.iter().map(|qn| {
                                let node = store.node_by_qname(qn).ok().flatten();
                                let repo = node
                                    .as_ref()
                                    .map(|node| node_repo_json(node, &repo_aliases))
                                    .unwrap_or_else(|| serde_json::json!({"repo_id": null, "display_alias": null}));
                                let file_path = node.as_ref().map(|node| node.file_path.clone());
                                let kind = node.as_ref().map(|node| node.kind.as_str().to_owned());
                                serde_json::json!({
                                    "qualified_name": qn,
                                    "file_path": file_path,
                                    "kind": kind,
                                    "repo": repo,
                                })
                            }).collect()
                        ),
                    },
                    "suggestions": [{
                        "hint": "Multiple symbols match. Narrow with 'file', 'kind', or 'language'. Then pass the exact qualified_name to symbol_neighbors or traverse_graph.",
                        "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
                    }],
                    "summary": {
                        "status": "ambiguous",
                        "match_count": meta.candidates.len(),
                        "truncated": false,
                    },
                    "warnings": [],
                });
                let mut response = tool_result_value(&result, output_format)?;
                inject_budget_metadata(
                    &mut response,
                    &budgets.summary(
                        "query_candidates_and_seeds.max_candidates",
                        limit,
                        requested_limit.max(meta.candidates.len()),
                    ),
                );
                inject_deprecated_input_fields(&mut response, &repo_scope.deprecated_input_fields);
                return Ok(response);
            }
            ResolvedTarget::NotFound { suggestions } => {
                let payload = input_shape_error_payload(
                    "resolve_symbol",
                    format!("no symbol matched '{name}'"),
                    format!(
                        "No symbol matched '{name}'. Verify canonical QN tokens (e.g. '::fn::' not '::function::'). Candidates: {suggestions:?}. Try query_graph or resolve_symbol with a shorter name."
                    ),
                    InputShapeErrorSpec {
                        offending_fields: vec!["name".to_owned()],
                        normalization_performed: Vec::new(),
                        accepted_argument_families: vec!["name".to_owned(), "name + kind".to_owned(), "name + file".to_owned()],
                        retry_example: Some(json!({"name": "compute"})),
                        fail_closed_reason: Some("Atlas could not resolve the requested symbol name in the indexed graph".to_owned()),
                        retry_guidance: Some("Use query_graph or explain_query to find exact symbol names, then retry resolve_symbol.".to_owned()),
                        extra_details: Some(json!({"candidates": suggestions})),
                    },
                );
                return tool_execution_error_value(output_format, &payload);
            }
            ResolvedTarget::File(_) => {}
        }
    }

    let resolved_kind = kind_input.as_deref().map(resolve_kind_alias);
    let fetch_limit = budgets.resolve_limit(
        policy.query_candidates_and_seeds.candidates,
        "query_candidates_and_seeds.max_candidates",
        Some((limit * 4).max(40)),
    );
    let query = SearchQuery {
        text: name.clone(),
        kind: resolved_kind.clone(),
        language: language.clone(),
        limit: fetch_limit,
        repo_ids: repo_scope
            .selection
            .as_ref()
            .map(|scope| scope.repo_ids.clone())
            .unwrap_or_default(),
        ..Default::default()
    };
    let results = atlas_search::execute_query(&store, &query, false)
        .context("resolve_symbol search failed")?;

    let filtered: Vec<_> = if let Some(ref file_pat) = file_filter {
        results
            .into_iter()
            .filter(|r| r.node.file_path.contains(file_pat.as_str()))
            .collect()
    } else {
        results
    };

    let total_before_limit = filtered.len();
    let ranked: Vec<_> = filtered.into_iter().take(limit).collect();
    let truncated = total_before_limit > ranked.len();
    if truncated {
        budgets.record_usage(
            policy.query_candidates_and_seeds.candidates,
            "query_candidates_and_seeds.max_candidates",
            limit,
            total_before_limit,
            true,
        );
    }

    if ranked.is_empty() {
        let payload = input_shape_error_payload(
            "resolve_symbol",
            format!("no symbol matched '{name}'"),
            "No symbol matched requested name in indexed graph. Use query_graph or explain_query to find exact identifiers, then retry resolve_symbol.".to_owned(),
            InputShapeErrorSpec {
                offending_fields: vec!["name".to_owned()],
                normalization_performed: Vec::new(),
                accepted_argument_families: vec!["name".to_owned(), "name + kind".to_owned(), "name + file".to_owned()],
                retry_example: Some(json!({"name": "compute"})),
                fail_closed_reason: Some("Atlas could not resolve the requested symbol name in the indexed graph".to_owned()),
                retry_guidance: Some("Use query_graph or explain_query to discover exact symbol names, then retry resolve_symbol.".to_owned()),
                extra_details: None,
            },
        );
        return tool_execution_error_value(output_format, &payload);
    }

    let best_qn = ranked.first().map(|r| r.node.qualified_name.as_str());
    let ambiguous = ranked.len() > 1;

    #[derive(Serialize)]
    struct ResolvedMatch<'a> {
        qualified_name: &'a str,
        name: &'a str,
        kind: &'a str,
        file_path: &'a str,
        language: &'a str,
        line_start: u32,
        repo: serde_json::Value,
    }

    let matches: Vec<ResolvedMatch<'_>> = ranked
        .iter()
        .map(|r| ResolvedMatch {
            qualified_name: &r.node.qualified_name,
            name: &r.node.name,
            kind: r.node.kind.as_str(),
            file_path: &r.node.file_path,
            language: &r.node.language,
            line_start: r.node.line_start,
            repo: node_repo_json(&r.node, &repo_aliases),
        })
        .collect();

    let suggestions: Vec<serde_json::Value> = if best_qn.is_some() {
        if ambiguous {
            vec![serde_json::json!({
                "hint": "Multiple symbols match. Narrow with 'file', 'kind', or 'language'. Then pass the exact qualified_name to symbol_neighbors or traverse_graph.",
                "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
            })]
        } else {
            vec![serde_json::json!({
                "hint": "Exact match resolved. Pass qualified_name to symbol_neighbors or traverse_graph for callers, callees, and relationships.",
                "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
            })]
        }
    } else {
        vec![serde_json::json!({
            "hint": "No symbol matched. Try query_graph with a regex pattern, or use explain_query to validate the search input.",
            "next_tools": ["query_graph", "explain_query"]
        })]
    };

    let result = serde_json::json!({
        "tool": "resolve_symbol",
        "query": {
            "name": name,
            "kind": kind_input,
            "file": file_filter,
            "language": language,
        },
        "best_match": matches.first(),
        "ambiguity": {
            "ambiguous": ambiguous,
            "matches": matches,
        },
        "suggestions": suggestions,
        "summary": {
            "status": if best_qn.is_some() { if ambiguous { "ambiguous" } else { "resolved" } } else { "not_found" },
            "match_count": total_before_limit.min(limit),
            "truncated": truncated,
        },
        "warnings": [],
    });

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
