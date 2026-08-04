use super::*;

pub(crate) fn tool_explain_change(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let policy = load_budget_policy(repo_root)?;
    let max_depth = u64_arg(args, "max_depth").unwrap_or(5) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;

    let request = match validate_change_source_request("explain_change", args, true) {
        Ok(request) => request,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let deprecated_change_source_fields = request.deprecated_input_fields.clone();
    let resolved = resolve_change_source(request, repo_root)?;
    let files = resolved.files.clone();

    if files.is_empty() {
        let summary = atlas_review::empty_explain_change_summary();
        let mut payload = as_object_map(serde_json::to_value(&summary)?);
        let summary_text = payload
            .remove("summary")
            .unwrap_or_else(|| Value::String(String::new()));
        payload.insert("changed_files".to_owned(), json!([]));
        payload.insert(
            "change_kinds".to_owned(),
            payload
                .get("changed_by_kind")
                .cloned()
                .unwrap_or_else(|| json!({})),
        );
        payload.insert("coverage_gaps".to_owned(), json!([]));
        payload.insert(
            "summary".to_owned(),
            json!({
                "text": summary_text,
                "changed_file_count": 0,
                "changed_symbol_count": 0,
                "impacted_file_count": 0,
                "impacted_node_count": 0,
            }),
        );
        payload.remove("changed_file_count");
        payload.remove("changed_symbol_count");
        payload.remove("changed_by_kind");
        payload.remove("impacted_file_count");
        payload.remove("impacted_node_count");
        payload.remove("summary_text");
        insert_change_source_payload(&mut payload, &resolved);
        let mut response = build_normalized_success_response(
            "explain_change",
            Value::Object(payload),
            output_format,
            Vec::new(),
            false,
            None,
        )?;
        inject_deprecated_input_fields(&mut response, &deprecated_change_source_fields);
        return Ok(response);
    }

    let store = open_store(db_path)?;
    let changes: Vec<atlas_core::model::ChangedFile> = files
        .iter()
        .cloned()
        .map(|path| atlas_core::model::ChangedFile {
            path,
            change_type: atlas_core::ChangeType::Modified,
            old_path: None,
        })
        .collect();
    let summary = atlas_review::build_explain_change_summary(
        &store, &changes, &files, max_depth, max_nodes, &policy,
    )
    .context("explain_change summary generation failed")?;

    let mut payload = as_object_map(serde_json::to_value(&summary)?);
    let summary_text = payload
        .remove("summary")
        .unwrap_or_else(|| Value::String(String::new()));
    let coverage_gaps = summary
        .test_impact
        .uncovered_symbols
        .iter()
        .map(|symbol| json!({ "symbol": symbol }))
        .collect::<Vec<_>>();
    payload.insert(
        "changed_files".to_owned(),
        json!(summary.diff_summary.files),
    );
    payload.insert(
        "change_kinds".to_owned(),
        payload
            .get("changed_by_kind")
            .cloned()
            .unwrap_or_else(|| json!({})),
    );
    payload.insert("coverage_gaps".to_owned(), Value::Array(coverage_gaps));
    payload.insert(
        "summary".to_owned(),
        json!({
            "text": summary_text,
            "changed_file_count": summary.changed_file_count,
            "changed_symbol_count": summary.changed_symbol_count,
            "impacted_file_count": summary.impacted_file_count,
            "impacted_node_count": summary.impacted_node_count,
        }),
    );
    payload.remove("changed_file_count");
    payload.remove("changed_symbol_count");
    payload.remove("changed_by_kind");
    payload.remove("impacted_file_count");
    payload.remove("impacted_node_count");
    payload.remove("summary_text");

    insert_change_source_payload(&mut payload, &resolved);
    let mut response = build_normalized_success_response(
        "explain_change",
        Value::Object(payload),
        output_format,
        Vec::new(),
        false,
        None,
    )?;
    inject_deprecated_input_fields(&mut response, &deprecated_change_source_fields);
    Ok(response)
}
