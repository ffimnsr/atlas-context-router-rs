use super::*;

pub(crate) fn tool_get_minimal_context(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let request = match validate_change_source_request("get_minimal_context", args, false) {
        Ok(request) => request,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let max_depth = u64_arg(args, "max_depth").unwrap_or(2) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(50) as usize;

    let deprecated_change_source_fields = request.deprecated_input_fields.clone();
    let resolved = resolve_change_source(request, repo_root)?;
    let changes = &resolved.changes;

    let changed_file_paths: Vec<String> = changes
        .iter()
        .filter(|cf| cf.change_type != atlas_core::ChangeType::Deleted)
        .map(|cf| cf.path.clone())
        .collect();

    let store = open_store(db_path)?;
    let policy = load_budget_policy(repo_root)?;
    let file_refs: Vec<&str> = changed_file_paths.iter().map(String::as_str).collect();
    let impact = store
        .impact_radius(
            &file_refs,
            max_depth,
            max_nodes,
            policy.graph_traversal.edges.default_limit,
        )
        .context("impact_radius failed")?;

    let packaged = package_impact(&impact, &changed_file_paths);

    let deleted_count = changes
        .iter()
        .filter(|cf| cf.change_type == atlas_core::ChangeType::Deleted)
        .count();

    let mut risk_flags = Vec::new();
    if deleted_count > 0 {
        risk_flags.push("deleted_files_present");
    }
    if impact.impacted_files.len() > changed_file_paths.len() {
        risk_flags.push("transitive_file_impact");
    }
    if impact.impacted_nodes.len() > impact.changed_nodes.len() {
        risk_flags.push("transitive_symbol_impact");
    }
    if packaged.truncated {
        risk_flags.push("truncated");
    }
    if impact
        .impacted_nodes
        .iter()
        .any(|node| node.is_test || node.qualified_name.contains("test"))
    {
        risk_flags.push("test_impact");
    }

    let payload = json!({
        "change_source": change_source_json(&resolved),
        "changed_symbols": packaged.changed_nodes,
        "immediate_impact": {
            "impacted_symbols": packaged.impacted_nodes,
            "impacted_files": packaged.impacted_files,
            "relevant_edges": packaged.relevant_edges,
        },
        "risk_flags": risk_flags,
        "summary": {
            "changed_file_count": changed_file_paths.len(),
            "deleted_file_count": deleted_count,
            "changed_symbol_count": packaged.changed_node_count,
            "impacted_symbol_count": packaged.impacted_node_count,
            "impacted_file_count": packaged.impacted_file_count,
            "truncated": packaged.truncated,
        }
    });

    let mut payload = payload;
    if let Some(object) = payload.as_object_mut() {
        object.insert("change_source".to_owned(), change_source_json(&resolved));
    }
    let mut response = build_normalized_success_response(
        "get_minimal_context",
        payload,
        output_format,
        Vec::new(),
        packaged.truncated,
        packaged
            .truncated
            .then_some("minimal context capped by node or edge budgets"),
    )?;
    inject_budget_metadata(&mut response, &impact.budget);
    inject_deprecated_input_fields(&mut response, &deprecated_change_source_fields);
    Ok(response)
}
