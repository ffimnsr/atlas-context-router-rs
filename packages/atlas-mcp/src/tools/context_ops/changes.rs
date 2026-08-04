use super::*;

pub(crate) fn tool_detect_changes(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let request = match validate_change_source_request("detect_changes", args, false) {
        Ok(request) => request,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let repo_scope = match resolve_repo_scope_selection("detect_changes", args, repo_root) {
        Ok(scope) => scope,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let deprecated_change_source_fields = request.deprecated_input_fields.clone();
    let resolved = resolve_change_source(request.clone(), repo_root)?;
    let changes = &resolved.changes;
    let store_opt = Store::open(db_path).ok();

    #[derive(Serialize)]
    struct ChangedEntry {
        path: String,
        change_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        old_path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        node_count: Option<usize>,
        #[serde(skip_serializing_if = "Option::is_none")]
        language: Option<String>,
        repo: serde_json::Value,
        is_added: bool,
        is_modified: bool,
        is_deleted: bool,
        is_renamed: bool,
        is_copied: bool,
    }

    let repo_aliases = repo_aliases_by_id(repo_root);
    let mut tool_warnings = Vec::new();
    let mut repo_results = Vec::new();
    let mut repo_processed_count = 0usize;
    let mut repo_skipped_count = 0usize;
    let entries: Vec<ChangedEntry> = if let Some(scope) = repo_scope.selection.as_ref() {
        let mut entries = Vec::new();
        for registration in &scope.registrations {
            match detect_changes_for_registration(registration, &request) {
                Ok(per_repo) => {
                    repo_processed_count += 1;
                    repo_results.push(json!({
                        "repo_id": registration.repo_id,
                        "display_alias": registration.display_alias,
                        "status": "ok",
                        "changed_file_count": per_repo.changes.len(),
                    }));
                    for cf in &per_repo.changes {
                        let file_nodes = store_opt
                            .as_ref()
                            .and_then(|s| s.nodes_by_file(&cf.path).ok())
                            .map(|nodes| {
                                nodes
                                    .into_iter()
                                    .filter(|node| {
                                        node_repo_id(node) == Some(registration.repo_id.as_str())
                                    })
                                    .collect::<Vec<_>>()
                            });
                        let node_count = file_nodes.as_ref().map(Vec::len);
                        let language = file_nodes
                            .as_ref()
                            .and_then(|nodes| nodes.first())
                            .map(|node| node.language.clone());
                        let change_type = match cf.change_type {
                            ChangeType::Added => "added",
                            ChangeType::Modified => "modified",
                            ChangeType::Deleted => "deleted",
                            ChangeType::Renamed => "renamed",
                            ChangeType::Copied => "copied",
                        };
                        entries.push(ChangedEntry {
                            path: cf.path.clone(),
                            change_type: change_type.to_owned(),
                            old_path: cf.old_path.clone(),
                            node_count,
                            language,
                            repo: json!({
                                "repo_id": registration.repo_id,
                                "display_alias": repo_aliases.get(&registration.repo_id).cloned().unwrap_or_else(|| registration.display_alias.clone()),
                            }),
                            is_added: matches!(cf.change_type, ChangeType::Added),
                            is_modified: matches!(cf.change_type, ChangeType::Modified),
                            is_deleted: matches!(cf.change_type, ChangeType::Deleted),
                            is_renamed: matches!(cf.change_type, ChangeType::Renamed),
                            is_copied: matches!(cf.change_type, ChangeType::Copied),
                        });
                    }
                }
                Err(error) => {
                    repo_skipped_count += 1;
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
        entries
    } else {
        changes
            .iter()
            .map(|cf| {
                let file_nodes = store_opt
                    .as_ref()
                    .and_then(|s| s.nodes_by_file(&cf.path).ok());
                let node_count = file_nodes.as_ref().map(Vec::len);
                let language = file_nodes
                    .as_ref()
                    .and_then(|nodes| nodes.first())
                    .map(|node| node.language.clone());
                let change_type = match cf.change_type {
                    ChangeType::Added => "added",
                    ChangeType::Modified => "modified",
                    ChangeType::Deleted => "deleted",
                    ChangeType::Renamed => "renamed",
                    ChangeType::Copied => "copied",
                };
                ChangedEntry {
                    path: cf.path.clone(),
                    change_type: change_type.to_owned(),
                    old_path: cf.old_path.clone(),
                    node_count,
                    language,
                    repo: json!({"repo_id": null, "display_alias": null}),
                    is_added: matches!(cf.change_type, ChangeType::Added),
                    is_modified: matches!(cf.change_type, ChangeType::Modified),
                    is_deleted: matches!(cf.change_type, ChangeType::Deleted),
                    is_renamed: matches!(cf.change_type, ChangeType::Renamed),
                    is_copied: matches!(cf.change_type, ChangeType::Copied),
                }
            })
            .collect()
    };

    let effective_changes: Vec<ChangedFile> = if repo_scope.selection.is_some() {
        entries
            .iter()
            .map(|entry| ChangedFile {
                path: entry.path.clone(),
                change_type: match entry.change_type.as_str() {
                    "added" => ChangeType::Added,
                    "modified" => ChangeType::Modified,
                    "deleted" => ChangeType::Deleted,
                    "renamed" => ChangeType::Renamed,
                    "copied" => ChangeType::Copied,
                    _ => ChangeType::Modified,
                },
                old_path: entry.old_path.clone(),
            })
            .collect()
    } else {
        changes.clone()
    };
    let (added_count, modified_count, deleted_count, renamed_count, copied_count) =
        count_change_kinds(&effective_changes);
    let mut payload = json!({
        "change_source": change_source_json(&resolved),
        "files": entries,
        "summary": {
            "changed_file_count": effective_changes.len(),
            "resolved_file_count": if repo_scope.selection.is_some() { entries.len() } else { resolved.files.len() },
            "deleted_file_count": deleted_count,
            "added_file_count": added_count,
            "modified_file_count": modified_count,
            "renamed_file_count": renamed_count,
            "copied_file_count": copied_count,
            "files_with_graph_nodes": effective_changes
                .iter()
                .filter(|cf| {
                    store_opt
                        .as_ref()
                        .and_then(|s| s.nodes_by_file(&cf.path).ok())
                        .is_some_and(|nodes| !nodes.is_empty())
                })
                .count(),
        },
    });
    if let Some(scope) = repo_scope.selection.as_ref()
        && let Some(object) = payload.as_object_mut()
    {
        object.insert(
            "repo_scope".to_owned(),
            json!({
                "selected_repo_count": scope.registrations.len(),
                "processed_repo_count": repo_processed_count,
                "skipped_repo_count": repo_skipped_count,
                "repos": repo_results,
            }),
        );
    }

    let mut response = build_normalized_success_response(
        "detect_changes",
        payload,
        output_format,
        tool_warnings,
        false,
        None,
    )?;
    let mut deprecated_fields = deprecated_change_source_fields;
    deprecated_fields.extend(repo_scope.deprecated_input_fields.iter().cloned());
    deprecated_fields.dedup();
    inject_deprecated_input_fields(&mut response, &deprecated_fields);
    Ok(response)
}
