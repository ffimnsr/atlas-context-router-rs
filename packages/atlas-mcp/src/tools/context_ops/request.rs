use super::*;

pub(super) fn resolve_diff_target(request: &ChangeSourceRequest) -> DiffTarget {
    match request.kind {
        ResolvedChangeSourceKind::Staged => DiffTarget::Staged,
        ResolvedChangeSourceKind::Base => {
            DiffTarget::BaseRef(request.base.clone().expect("base kind requires base ref"))
        }
        ResolvedChangeSourceKind::WorkingTree => DiffTarget::WorkingTree,
        ResolvedChangeSourceKind::Files => unreachable!("files kind does not use git diff target"),
    }
}

pub(super) fn validate_change_source_request(
    tool_name: &str,
    args: Option<&serde_json::Value>,
    allow_explicit_files: bool,
) -> std::result::Result<ChangeSourceRequest, Box<ToolErrorPayload>> {
    let resolved = resolve_change_source_selection(tool_name, args, allow_explicit_files)?;
    Ok(ChangeSourceRequest {
        kind: resolved.kind,
        files: resolved.files,
        base: resolved.base,
        deprecated_input_fields: resolved.deprecated_input_fields,
    })
}

pub(super) fn build_operation_error(
    message: impl Into<String>,
    detail: impl Into<String>,
    offending_fields: Vec<&str>,
    retry_example: Value,
) -> Box<ToolErrorPayload> {
    Box::new(input_shape_error_payload(
        "build_or_update_graph",
        message,
        detail,
        InputShapeErrorSpec {
            offending_fields: offending_fields.into_iter().map(str::to_owned).collect(),
            normalization_performed: Vec::new(),
            accepted_argument_families: vec![
                "operation.kind=build".to_owned(),
                "operation.kind=update".to_owned(),
            ],
            retry_example: Some(retry_example),
            fail_closed_reason: Some(
                "Atlas refused to guess between conflicting build/update operation selectors"
                    .to_owned(),
            ),
            retry_guidance: Some(
                "Provide exactly one build_or_update_graph operation shape and retry.".to_owned(),
            ),
            extra_details: Some(json!({
                "accepted_operation_shapes": [
                    { "operation": { "kind": "build" } },
                    { "operation": { "kind": "update", "change_source": { "kind": "working_tree" } } },
                    { "operation": { "kind": "update", "change_source": { "kind": "staged" } } },
                    { "operation": { "kind": "update", "change_source": { "kind": "base", "base": "origin/main" } } },
                    { "operation": { "kind": "update", "change_source": { "kind": "files", "files": ["src/lib.rs"] } } }
                ]
            })),
        },
    ))
}

pub(super) fn material_legacy_build_update_fields(
    args: Option<&Value>,
) -> std::result::Result<LegacyBuildUpdateFields, Box<ToolErrorPayload>> {
    let present_fields = ["mode", "base", "staged", "files"]
        .into_iter()
        .filter(|field| args.is_some_and(|value| value.get(field).is_some()))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    Ok(LegacyBuildUpdateFields { present_fields })
}

pub(super) fn validate_build_operation_request(
    args: Option<&Value>,
) -> std::result::Result<BuildOperationRequest, Box<ToolErrorPayload>> {
    let LegacyBuildUpdateFields {
        present_fields: legacy_present_fields,
    } = material_legacy_build_update_fields(args)?;
    let operation_value = args.and_then(|value| value.get("operation"));
    let operation_object = operation_value.and_then(|value| value.as_object());

    if operation_value.is_some() && operation_object.is_none() {
        return Err(build_operation_error(
            "invalid operation selector",
            "operation must be an object with kind=build or kind=update",
            vec!["operation"],
            json!({ "operation": { "kind": "build" } }),
        ));
    }

    if operation_object.is_some() && !legacy_present_fields.is_empty() {
        let mut offending_fields = vec!["operation"];
        offending_fields.extend(legacy_present_fields.iter().map(String::as_str));
        return Err(build_operation_error(
            "conflicting build operation selectors",
            "operation cannot be combined with legacy mode, base, staged, or files fields",
            offending_fields,
            json!({ "operation": { "kind": "build" } }),
        ));
    }

    if let Some(operation) = operation_object {
        let kind = operation
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                build_operation_error(
                    "operation.kind is required",
                    "operation object requires kind=build or kind=update",
                    vec!["operation.kind"],
                    json!({ "operation": { "kind": "build" } }),
                )
            })?;
        return match kind {
            "build" => {
                if operation.get("change_source").is_some() {
                    Err(build_operation_error(
                        "operation.kind='build' cannot include change_source",
                        "build operation does not accept change_source because it performs a full build",
                        vec!["operation.kind", "operation.change_source"],
                        json!({ "operation": { "kind": "build" } }),
                    ))
                } else {
                    Ok(BuildOperationRequest {
                        kind: BuildOperationKind::Build,
                        change_source: None,
                        deprecated_input_fields: Vec::new(),
                    })
                }
            }
            "update" => {
                let nested_change_source = operation.get("change_source").ok_or_else(|| {
                    build_operation_error(
                        "operation.kind='update' requires operation.change_source",
                        "update operation requires explicit operation.change_source; use working_tree, staged, base, or files",
                        vec!["operation.kind", "operation.change_source"],
                        json!({ "operation": { "kind": "update", "change_source": { "kind": "working_tree" } } }),
                    )
                })?;
                let nested_args = json!({ "change_source": nested_change_source.clone() });
                let change_source = validate_change_source_request(
                    "build_or_update_graph",
                    Some(&nested_args),
                    true,
                )?;
                Ok(BuildOperationRequest {
                    kind: BuildOperationKind::Update,
                    change_source: Some(change_source),
                    deprecated_input_fields: Vec::new(),
                })
            }
            other => Err(build_operation_error(
                format!("invalid operation.kind '{other}'"),
                "operation.kind must be one of: build, update",
                vec!["operation.kind"],
                json!({ "operation": { "kind": "build" } }),
            )),
        };
    }

    if !legacy_present_fields.is_empty() {
        let offending_fields = legacy_present_fields
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        return Err(build_operation_error(
            "legacy build_or_update_graph fields are no longer supported",
            "Use operation={ kind: 'build' } or operation={ kind: 'update', change_source: ... } and remove top-level mode/base/staged/files fields.",
            offending_fields,
            json!({ "operation": { "kind": "build" } }),
        ));
    }

    Ok(BuildOperationRequest {
        kind: BuildOperationKind::Build,
        change_source: None,
        deprecated_input_fields: Vec::new(),
    })
}

pub(super) fn resolve_change_source(
    request: ChangeSourceRequest,
    repo_root: &str,
) -> Result<ResolvedChangeSource> {
    let ChangeSourceRequest {
        kind,
        files,
        base,
        deprecated_input_fields: _,
    } = request;

    if kind == ResolvedChangeSourceKind::Files {
        return Ok(ResolvedChangeSource {
            kind,
            files,
            changes: Vec::new(),
            deleted_files: Vec::new(),
            base,
        });
    }

    let repo_root_path =
        find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;
    let repo_root_path = repo_root_path.as_path();

    let diff_target = resolve_diff_target(&ChangeSourceRequest {
        kind,
        files: Vec::new(),
        base: base.clone(),
        deprecated_input_fields: Vec::new(),
    });
    let changes =
        changed_files(repo_root_path, &diff_target).context("cannot detect changed files")?;
    let files: Vec<String> = changes
        .iter()
        .filter(|cf| cf.change_type != ChangeType::Deleted)
        .map(|cf| cf.path.clone())
        .collect();
    let deleted_files: Vec<String> = changes
        .iter()
        .filter(|cf| cf.change_type == ChangeType::Deleted)
        .map(|cf| cf.path.clone())
        .collect();

    Ok(ResolvedChangeSource {
        kind,
        files,
        changes,
        deleted_files,
        base,
    })
}

pub(super) fn change_source_json(resolved: &ResolvedChangeSource) -> Value {
    json!({
        "kind": resolved.kind.as_str(),
        "resolved_files": &resolved.files,
        "deleted_files": &resolved.deleted_files,
        "base": &resolved.base,
    })
}

pub(super) fn node_repo_id(node: &atlas_core::Node) -> Option<&str> {
    node.extra_json
        .as_object()
        .and_then(|extra| extra.get("repo_id"))
        .and_then(|value| value.as_str())
}

pub(super) fn changed_repo_summary_json(
    changed_nodes: &[atlas_core::Node],
    repo_aliases: &std::collections::BTreeMap<String, String>,
) -> Value {
    let mut counts = std::collections::BTreeMap::<String, usize>::new();
    for node in changed_nodes {
        if let Some(repo_id) = node_repo_id(node) {
            *counts.entry(repo_id.to_owned()).or_default() += 1;
        }
    }
    Value::Array(
        counts
            .into_iter()
            .map(|(repo_id, changed_symbol_count)| {
                json!({
                    "repo_id": repo_id,
                    "display_alias": repo_aliases.get(&repo_id).cloned(),
                    "changed_symbol_count": changed_symbol_count,
                })
            })
            .collect(),
    )
}

pub(super) fn boundary_summary_json(advanced: &atlas_core::AdvancedImpactResult) -> Value {
    let cross_module_count = advanced
        .boundary_violations
        .iter()
        .find(|violation| violation.kind == atlas_core::BoundaryKind::CrossModule)
        .map(|violation| violation.nodes.len())
        .unwrap_or(0);
    let cross_package_count = advanced
        .boundary_violations
        .iter()
        .find(|violation| violation.kind == atlas_core::BoundaryKind::CrossPackage)
        .map(|violation| violation.nodes.len())
        .unwrap_or(0);
    let cross_repo_count = advanced
        .boundary_violations
        .iter()
        .find(|violation| violation.kind == atlas_core::BoundaryKind::CrossRepo)
        .map(|violation| violation.nodes.len())
        .unwrap_or(0);
    json!({
        "cross_module": cross_module_count > 0,
        "cross_module_count": cross_module_count,
        "cross_package": cross_package_count > 0,
        "cross_package_count": cross_package_count,
        "cross_repo": cross_repo_count > 0,
        "cross_repo_count": cross_repo_count,
        "violations": advanced.boundary_violations,
    })
}

pub(super) fn cross_repo_context_hops_json(edges: &[Value], store: &Store) -> Value {
    let hop_count = edges
        .iter()
        .filter(|edge| {
            let from = edge.get("from").and_then(Value::as_str);
            let to = edge.get("to").and_then(Value::as_str);
            let from_node = from.and_then(|qname| store.node_by_qname(qname).ok().flatten());
            let to_node = to.and_then(|qname| store.node_by_qname(qname).ok().flatten());
            let from_repo = from_node.as_ref().and_then(node_repo_id);
            let to_repo = to_node.as_ref().and_then(node_repo_id);
            from_repo.is_some() && to_repo.is_some() && from_repo != to_repo
        })
        .count();
    json!({
        "enabled": hop_count > 0,
        "edge_count": hop_count,
    })
}

pub(super) fn impact_seed_qnames_for_repo(
    store: &Store,
    repo_id: &str,
    files: &[String],
) -> Result<Vec<String>> {
    let mut qnames = Vec::new();
    for file in files {
        for node in store.nodes_by_file(file)? {
            if node_repo_id(&node) == Some(repo_id) {
                qnames.push(node.qualified_name);
            }
        }
    }
    qnames.sort();
    qnames.dedup();
    Ok(qnames)
}

pub(super) fn detect_changes_for_registration(
    registration: &RepoRegistration,
    request: &ChangeSourceRequest,
) -> Result<ResolvedChangeSource> {
    resolve_change_source(
        ChangeSourceRequest {
            kind: request.kind,
            files: request.files.clone(),
            base: request.base.clone(),
            deprecated_input_fields: request.deprecated_input_fields.clone(),
        },
        registration.root.as_str(),
    )
}

pub(super) fn insert_change_source_payload(
    payload: &mut Map<String, Value>,
    resolved: &ResolvedChangeSource,
) {
    payload.insert("change_source".to_owned(), change_source_json(resolved));
}

pub(super) fn as_object_map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

pub(super) fn build_normalized_success_response(
    tool_name: &str,
    payload: Value,
    output_format: crate::output::OutputFormat,
    warnings: Vec<String>,
    truncated: bool,
    truncation_reason: Option<&str>,
) -> Result<Value> {
    let envelope = ToolSuccessEnvelope::new(tool_name, payload)
        .with_warnings(warnings)
        .with_truncation(truncated, truncation_reason.map(str::to_owned));
    normalized_tool_result_value(&envelope, output_format)
}
