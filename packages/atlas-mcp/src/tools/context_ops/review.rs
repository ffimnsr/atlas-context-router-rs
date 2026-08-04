use super::*;

pub(crate) fn tool_get_review_context(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let request = match validate_change_source_request("get_review_context", args, true) {
        Ok(request) => request,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let deprecated_change_source_fields = request.deprecated_input_fields.clone();
    let resolved = resolve_change_source(request, repo_root)?;
    let max_depth = u64_arg(args, "max_depth").unwrap_or(3) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;
    let token_budget = u64_arg(args, "token_budget").map(|n| n as usize);

    let store = open_store(db_path)?;
    let policy = load_budget_policy(repo_root)?;
    let engine = ContextEngine::new(&store).with_budget_policy(policy);
    let request = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles {
            paths: resolved.files.clone(),
        },
        max_nodes: Some(max_nodes),
        depth: Some(max_depth),
        token_budget,
        ..ContextRequest::default()
    };
    let result = engine.build(&request).context("context engine failed")?;
    let file_refs: Vec<&str> = resolved.files.iter().map(String::as_str).collect();
    let review_impact = store
        .impact_radius(
            &file_refs,
            max_depth,
            max_nodes,
            policy.graph_traversal.edges.default_limit,
        )
        .context("review impact query failed")?;
    let advanced = atlas_impact::analyze(review_impact);
    let include_context_ranking_evidence = output_format == crate::output::OutputFormat::Json;
    let packaged = package_context_result(&result, include_context_ranking_evidence);
    let mut packaged_value = serde_json::to_value(&packaged)?;
    let response_budget_limit = policy
        .mcp_cli_payload_serialization
        .mcp_response_bytes
        .default_limit;
    let response_budget_limit = response_budget_limit.saturating_sub(400);
    let stage_budget = if let Some(response_budget) =
        enforce_mcp_response_budget(&mut packaged_value, output_format, response_budget_limit)?
    {
        result.budget.clone().merge(response_budget)
    } else {
        result.budget.clone()
    };
    let changed_symbols = packaged_value
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|node| node.get("reason").and_then(Value::as_str) == Some("direct_target"))
        .collect::<Vec<_>>();
    let neighbors = packaged_value
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|node| node.get("reason").and_then(Value::as_str) != Some("direct_target"))
        .collect::<Vec<_>>();
    let critical_edges = packaged_value
        .get("edges")
        .cloned()
        .unwrap_or_else(|| json!([]));
    let artifacts = packaged_value
        .get("saved_context_sources")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|asset| {
            let mut object = as_object_map(asset);
            object.insert("artifact_kind".to_owned(), json!("saved_context"));
            Value::Object(object)
        })
        .collect::<Vec<_>>();
    let repo_aliases = repo_aliases_by_id(repo_root);
    let mut normalized_payload = as_object_map(packaged_value.clone());
    normalized_payload.remove("saved_context_sources");
    normalized_payload.insert(
        "changed_repos".to_owned(),
        changed_repo_summary_json(
            &result
                .nodes
                .iter()
                .filter(|node| node.selection_reason.as_str() == "direct_target")
                .map(|node| node.node.clone())
                .collect::<Vec<_>>(),
            &repo_aliases,
        ),
    );
    normalized_payload.insert("changed_files".to_owned(), json!(resolved.files.clone()));
    normalized_payload.insert("changed_symbols".to_owned(), Value::Array(changed_symbols));
    normalized_payload.insert("neighbors".to_owned(), Value::Array(neighbors));
    normalized_payload.insert("critical_edges".to_owned(), critical_edges);
    normalized_payload.insert("artifacts".to_owned(), Value::Array(artifacts));
    normalized_payload.insert(
        "boundary_summary".to_owned(),
        boundary_summary_json(&advanced),
    );
    normalized_payload.insert(
        "risk_summary".to_owned(),
        json!({
            "intent": normalized_payload.get("intent").cloned().unwrap_or(Value::Null),
            "node_count": normalized_payload.get("node_count").cloned().unwrap_or(Value::Null),
            "edge_count": normalized_payload.get("edge_count").cloned().unwrap_or(Value::Null),
            "file_count": normalized_payload.get("file_count").cloned().unwrap_or(Value::Null),
            "truncated": normalized_payload.get("truncated").cloned().unwrap_or(Value::Bool(false)),
            "nodes_dropped": normalized_payload.get("nodes_dropped").cloned().unwrap_or(Value::Null),
            "edges_dropped": normalized_payload.get("edges_dropped").cloned().unwrap_or(Value::Null),
            "files_dropped": normalized_payload.get("files_dropped").cloned().unwrap_or(Value::Null),
            "ambiguity_present": normalized_payload
                .get("ambiguity_candidates")
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty()),
            "cross_repo_boundary": advanced.boundary_violations.iter().any(|violation| violation.kind == atlas_core::BoundaryKind::CrossRepo),
        }),
    );
    normalized_payload.insert("change_source".to_owned(), change_source_json(&resolved));
    if include_context_ranking_evidence {
        normalized_payload.insert(
            "ranking_evidence_legend".to_owned(),
            context_ranking_evidence_legend_json(),
        );
    }

    let mut response = build_normalized_success_response(
        "get_review_context",
        Value::Object(normalized_payload),
        output_format,
        Vec::new(),
        packaged.truncated,
        packaged
            .truncated
            .then_some("review context capped by node, edge, file, or payload budget"),
    )?;
    inject_budget_metadata(&mut response, &stage_budget);
    inject_deprecated_input_fields(&mut response, &deprecated_change_source_fields);
    Ok(response)
}
