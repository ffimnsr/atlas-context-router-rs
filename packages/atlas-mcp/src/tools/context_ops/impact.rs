use super::*;

pub(crate) fn tool_get_impact_radius(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let request = match validate_change_source_request("get_impact_radius", args, true) {
        Ok(request) => request,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let repo_scope = match resolve_repo_scope_selection("get_impact_radius", args, repo_root) {
        Ok(scope) => scope,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let deprecated_change_source_fields = request.deprecated_input_fields.clone();
    let resolved = resolve_change_source(request.clone(), repo_root)?;
    let max_depth = u64_arg(args, "max_depth").unwrap_or(5) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;

    let store = open_store(db_path)?;
    let policy = load_budget_policy(repo_root)?;
    let mut tool_warnings = Vec::new();
    let result = if let Some(scope) = repo_scope.selection.as_ref() {
        let mut seed_files = Vec::new();
        let mut seed_qnames = Vec::new();
        let mut repo_results = Vec::new();
        for registration in &scope.registrations {
            match detect_changes_for_registration(registration, &request) {
                Ok(per_repo) => {
                    seed_files.extend(per_repo.files.clone().into_iter().map(|path| {
                        json!({
                            "path": path,
                            "repo": {
                                "repo_id": registration.repo_id,
                                "display_alias": registration.display_alias,
                            }
                        })
                    }));
                    seed_qnames.extend(impact_seed_qnames_for_repo(
                        &store,
                        &registration.repo_id,
                        &per_repo.files,
                    )?);
                    repo_results.push(json!({
                        "repo_id": registration.repo_id,
                        "display_alias": registration.display_alias,
                        "status": "ok",
                        "changed_file_count": per_repo.files.len(),
                    }));
                }
                Err(error) => {
                    tool_warnings.push(format!(
                        "skipped repo {}: {}",
                        registration.display_alias, error
                    ));
                    repo_results.push(json!({
                        "repo_id": registration.repo_id,
                        "display_alias": registration.display_alias,
                        "status": "skipped",
                        "error": error.to_string(),
                    }));
                }
            }
        }
        let seed_refs: Vec<&str> = seed_qnames.iter().map(String::as_str).collect();
        let impact = store
            .traverse_from_qnames(
                &seed_refs,
                max_depth,
                max_nodes,
                policy.graph_traversal.edges.default_limit,
            )
            .context("impact_radius query failed")?;
        let advanced = atlas_impact::analyze(impact.clone());
        let packaged = package_impact(&impact, &resolved.files);
        let mut payload = as_object_map(serde_json::to_value(&packaged)?);
        payload.insert("seed_files".to_owned(), Value::Array(seed_files));
        payload.insert(
            "changed_symbols".to_owned(),
            serde_json::to_value(&packaged.changed_nodes)?,
        );
        payload.insert(
            "impacted_symbols".to_owned(),
            serde_json::to_value(&packaged.impacted_nodes)?,
        );
        payload.insert(
            "boundary_summary".to_owned(),
            boundary_summary_json(&advanced),
        );
        payload.insert(
            "repo_scope".to_owned(),
            json!({
                "selected_repo_count": scope.registrations.len(),
                "processed_repo_count": repo_results.iter().filter(|entry| entry.get("status") == Some(&Value::String("ok".to_owned()))).count(),
                "repos": repo_results,
            }),
        );
        payload.insert(
            "summary".to_owned(),
            json!({
                "changed_file_count": packaged.changed_file_count,
                "changed_symbol_count": packaged.changed_node_count,
                "impacted_symbol_count": packaged.impacted_node_count,
                "impacted_file_count": packaged.impacted_file_count,
                "relevant_edge_count": packaged.relevant_edge_count,
                "seed_budget_count": packaged.seed_budgets.len(),
                "traversal_budget_applied": packaged.traversal_budget.is_some(),
                "cross_repo_boundary": advanced.boundary_violations.iter().any(|violation| violation.kind == atlas_core::BoundaryKind::CrossRepo),
            }),
        );
        payload.remove("changed_file_count");
        payload.remove("changed_node_count");
        payload.remove("changed_nodes");
        payload.remove("impacted_node_count");
        payload.remove("impacted_nodes");
        payload.remove("impacted_file_count");
        payload.remove("relevant_edge_count");
        payload.remove("budget_status");
        insert_change_source_payload(&mut payload, &resolved);
        let mut response = build_normalized_success_response(
            "get_impact_radius",
            Value::Object(payload),
            output_format,
            tool_warnings.clone(),
            packaged.truncated,
            packaged
                .truncated
                .then_some("node or edge caps limited impact result"),
        )?;
        inject_budget_metadata(&mut response, &packaged.budget);
        let mut deprecated_fields = deprecated_change_source_fields.clone();
        deprecated_fields.extend(repo_scope.deprecated_input_fields.iter().cloned());
        deprecated_fields.dedup();
        inject_deprecated_input_fields(&mut response, &deprecated_fields);
        return Ok(response);
    } else {
        let file_refs: Vec<&str> = resolved.files.iter().map(String::as_str).collect();
        store
            .impact_radius(
                &file_refs,
                max_depth,
                max_nodes,
                policy.graph_traversal.edges.default_limit,
            )
            .context("impact_radius query failed")?
    };

    let advanced = atlas_impact::analyze(result.clone());
    let packaged = package_impact(&result, &resolved.files);
    let mut payload = as_object_map(serde_json::to_value(&packaged)?);
    payload.insert("seed_files".to_owned(), json!(resolved.files));
    payload.insert(
        "changed_symbols".to_owned(),
        serde_json::to_value(&packaged.changed_nodes)?,
    );
    payload.insert(
        "impacted_symbols".to_owned(),
        serde_json::to_value(&packaged.impacted_nodes)?,
    );
    payload.insert(
        "boundary_summary".to_owned(),
        boundary_summary_json(&advanced),
    );
    payload.insert(
        "summary".to_owned(),
        json!({
            "changed_file_count": packaged.changed_file_count,
            "changed_symbol_count": packaged.changed_node_count,
            "impacted_symbol_count": packaged.impacted_node_count,
            "impacted_file_count": packaged.impacted_file_count,
            "relevant_edge_count": packaged.relevant_edge_count,
            "seed_budget_count": packaged.seed_budgets.len(),
            "traversal_budget_applied": packaged.traversal_budget.is_some(),
            "cross_repo_boundary": advanced.boundary_violations.iter().any(|violation| violation.kind == atlas_core::BoundaryKind::CrossRepo),
        }),
    );
    payload.remove("changed_file_count");
    payload.remove("changed_node_count");
    payload.remove("changed_nodes");
    payload.remove("impacted_node_count");
    payload.remove("impacted_nodes");
    payload.remove("impacted_file_count");
    payload.remove("relevant_edge_count");
    payload.remove("budget_status");

    insert_change_source_payload(&mut payload, &resolved);
    let mut response = build_normalized_success_response(
        "get_impact_radius",
        Value::Object(payload),
        output_format,
        tool_warnings,
        packaged.truncated,
        packaged
            .truncated
            .then_some("node or edge caps limited impact result"),
    )?;
    inject_budget_metadata(&mut response, &result.budget);
    let mut deprecated_fields = deprecated_change_source_fields;
    deprecated_fields.extend(repo_scope.deprecated_input_fields.iter().cloned());
    deprecated_fields.dedup();
    inject_deprecated_input_fields(&mut response, &deprecated_fields);
    Ok(response)
}
