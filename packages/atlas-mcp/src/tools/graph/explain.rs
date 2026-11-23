use super::*;

pub(crate) fn tool_explain_query(
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
    // Best-effort normalization: root-prefixed and absolute-under-root forms
    // collapse to canonical repo-relative prefixes; unmatched inputs stay as
    // typed so the prefix filter can still match literally.
    let subpath = subpath.map(|raw| {
        atlas_repo::normalize_repo_file_path(camino::Utf8Path::new(repo_root), &raw)
            .map(|resolved| resolved.canonical)
            .unwrap_or(raw)
    });
    let fuzzy = bool_arg(args, "fuzzy").unwrap_or(false);
    let hybrid = bool_arg(args, "hybrid").unwrap_or_else(|| hybrid_enabled_default(repo_root));
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
    let query_intent = parsed_query_intent_json(&parsed_query);
    let intent_target_symbol =
        if matches!(parsed_query.kind, "who_calls" | "what_breaks" | "tests_for") {
            json!(parsed_query.normalized_text)
        } else {
            Value::Null
        };

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
        "query_intent": query_intent,
        "intent_target_symbol": intent_target_symbol,
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
