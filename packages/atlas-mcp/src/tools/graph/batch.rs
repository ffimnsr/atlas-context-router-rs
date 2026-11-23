use super::*;

pub(crate) fn tool_batch_query_graph(
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
    let hybrid_default = hybrid_enabled_default(repo_root);

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
        let hybrid = bool_arg(q_args, "hybrid").unwrap_or(hybrid_default);
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
            "text": source_text.clone(),
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
            "query": source_text.clone(),
            "text": source_text,
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
