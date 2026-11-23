use super::*;

pub(crate) fn tool_cross_file_links(
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
    let file = atlas_repo::normalize_repo_file_path(camino::Utf8Path::new(repo_root), &file)
        .map_err(|error| anyhow::anyhow!("invalid file path: {error}"))?
        .canonical;
    let requested_limit = u64_arg(args, "limit").unwrap_or(20) as usize;
    let include_weak = bool_arg(args, "include_weak").unwrap_or(false);
    let limit = budgets.resolve_limit(
        policy.review_context_extraction.files,
        "review_context_extraction.max_files",
        Some(requested_limit),
    );

    let store = open_store(db_path)?;
    let links = sem::cross_file_links_with_options(&store, &file, limit, include_weak)
        .context("cross_file_links failed")?;

    let linked_files: Vec<_> = links
        .into_iter()
        .enumerate()
        .map(|(idx, l)| {
            let coupling_metric = super::analysis_payloads::round_one_decimal(l.strength);
            json!({
                "file": l.to_file,
                "via_symbols": l.via_symbols,
                "coupling_metric": coupling_metric,
                "rank": idx + 1,
                "confidence_tier": l.confidence_tier,
                "reachability": l.reachability,
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
            "max_strength": super::analysis_payloads::round_metric(max_strength * 10.0),
            "total_strength": super::analysis_payloads::round_metric(total_strength * 10.0),
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

pub(crate) fn tool_concept_clusters(
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
    let files = files
        .iter()
        .map(|path| {
            atlas_repo::normalize_repo_file_path(camino::Utf8Path::new(repo_root), path)
                .map(|resolved| resolved.canonical)
                .map_err(|error| anyhow::anyhow!("invalid file path '{path}': {error}"))
        })
        .collect::<Result<Vec<_>>>()?;
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
