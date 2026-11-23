use super::*;

pub(crate) fn tool_query_graph(
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
    // Best-effort normalization: root-prefixed and absolute-under-root forms
    // collapse to canonical repo-relative prefixes; unmatched inputs stay as
    // typed so the prefix filter can still match literally.
    let subpath = subpath.map(|raw| {
        atlas_repo::normalize_repo_file_path(camino::Utf8Path::new(repo_root), &raw)
            .map(|resolved| resolved.canonical)
            .unwrap_or(raw)
    });
    let exact_dir = str_arg(args, "exact_dir")?.map(str::to_owned).map(|raw| {
        atlas_repo::normalize_repo_file_path(camino::Utf8Path::new(repo_root), &raw)
            .map(|resolved| resolved.canonical)
            .unwrap_or(raw)
    });
    let exclude_subpaths: Vec<String> = args
        .and_then(|value| value.get("exclude_subpaths"))
        .and_then(|value| value.as_array())
        .map(|values| {
            values
                .iter()
                .filter_map(|value| value.as_str())
                .map(str::to_owned)
                .map(|raw| {
                    atlas_repo::normalize_repo_file_path(camino::Utf8Path::new(repo_root), &raw)
                        .map(|resolved| resolved.canonical)
                        .unwrap_or(raw)
                })
                .collect()
        })
        .unwrap_or_default();
    let fuzzy = bool_arg(args, "fuzzy").unwrap_or(false);
    let hybrid = bool_arg(args, "hybrid").unwrap_or_else(|| hybrid_enabled_default(repo_root));
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
        exact_dir,
        exclude_subpaths,
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
    let mut results =
        atlas_search::execute_query_with_embedding(&store, &query, semantic, embed_cfg.as_ref())
            .context("search failed")?;
    // Supported intent phrases answer from graph edges. Fall back to the lexical
    // results when the target symbol cannot be resolved.
    let mut usage_edges_included = false;
    if let Some(execution) =
        atlas_search::execute_intent_query(&store, parsed_query.kind, &query.text, limit)
            .context("intent execution failed")?
        && !execution.results.is_empty()
    {
        results = execution.results;
        usage_edges_included = execution.usage_edges_included;
    }
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
                "usage_edges_included": usage_edges_included,
                "relationship_tools": ["symbol_neighbors", "traverse_graph", "get_context"],
                "ranking_evidence_legend": ranking_evidence_legend_json(),
            },
            "truncated": truncated,
            "warnings": warnings,
        }),
        output_format,
    )?;
    response["atlas_usage_edges_included"] = serde_json::Value::Bool(usage_edges_included);
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
