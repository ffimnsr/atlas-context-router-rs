use anyhow::{Context, Result};
use atlas_adapters::derive_content_db_path;
use atlas_core::SearchQuery;
use atlas_core::model::{ChangeType, ChangedFile, ContextIntent, ContextRequest, ContextTarget};
use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_repo::{
    CanonicalRepoPath, DiffTarget, RepoRegistration, changed_files, find_repo_root, stable_repo_id,
};
use atlas_review::ContextEngine;
use atlas_search::semantic as sem;
use atlas_store_sqlite::{BuildFinishStats, GraphBuildState, Store};
use camino::Utf8Path;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

use super::shared::{
    ResolvedChangeSourceKind, bool_arg, error_code_docs, error_message, error_suggestions,
    inject_budget_metadata, inject_deprecated_input_fields, load_budget_policy,
    mcp_query_looks_like_unstructured_description, mcp_supported_query_grammar_examples,
    open_store, parse_mcp_intent, parse_mcp_query_grammar, repo_aliases_by_id,
    resolve_change_source_selection, resolve_repo_scope_selection, str_arg, u64_arg,
};
use crate::context::{enforce_mcp_response_budget, package_context_result, package_impact};
use crate::session_tools::{
    decision_hits_json, record_mcp_decision_best_effort, search_decisions_best_effort,
};
use crate::tool_result::{
    InputShapeErrorSpec, ToolErrorPayload, ToolSuccessEnvelope, input_shape_error_payload,
    normalized_tool_result_value, tool_execution_error_value,
};

fn context_ranking_evidence_legend_json() -> serde_json::Value {
    atlas_core::context_ranking_evidence_legend()
}

fn context_decision_lookup_query(request: &ContextRequest) -> Option<String> {
    match &request.target {
        ContextTarget::QualifiedName { qname } => Some(qname.clone()),
        ContextTarget::SymbolName { name } => Some(name.clone()),
        ContextTarget::FilePath { path } => Some(path.clone()),
        ContextTarget::ChangedFiles { paths } => {
            let joined = paths.iter().take(3).cloned().collect::<Vec<_>>().join(" ");
            (!joined.is_empty()).then_some(joined)
        }
        ContextTarget::ChangedSymbols { qnames } => {
            let joined = qnames.iter().take(3).cloned().collect::<Vec<_>>().join(" ");
            (!joined.is_empty()).then_some(joined)
        }
        ContextTarget::EdgeQuerySeed { source_qname, .. } => Some(source_qname.clone()),
    }
}

#[derive(Clone, Debug)]
struct ChangeSourceRequest {
    kind: ResolvedChangeSourceKind,
    files: Vec<String>,
    base: Option<String>,
    deprecated_input_fields: Vec<String>,
}

struct ResolvedChangeSource {
    kind: ResolvedChangeSourceKind,
    files: Vec<String>,
    changes: Vec<ChangedFile>,
    deleted_files: Vec<String>,
    base: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BuildOperationKind {
    Build,
    Update,
}

impl BuildOperationKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Build => "build",
            Self::Update => "update",
        }
    }
}

#[derive(Clone, Debug)]
struct BuildOperationRequest {
    kind: BuildOperationKind,
    change_source: Option<ChangeSourceRequest>,
    deprecated_input_fields: Vec<String>,
}

struct LegacyBuildUpdateFields {
    present_fields: Vec<String>,
}

fn resolve_diff_target(request: &ChangeSourceRequest) -> DiffTarget {
    match request.kind {
        ResolvedChangeSourceKind::Staged => DiffTarget::Staged,
        ResolvedChangeSourceKind::Base => {
            DiffTarget::BaseRef(request.base.clone().expect("base kind requires base ref"))
        }
        ResolvedChangeSourceKind::WorkingTree => DiffTarget::WorkingTree,
        ResolvedChangeSourceKind::Files => unreachable!("files kind does not use git diff target"),
    }
}

fn validate_change_source_request(
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

fn build_operation_error(
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

fn material_legacy_build_update_fields(
    args: Option<&Value>,
) -> std::result::Result<LegacyBuildUpdateFields, Box<ToolErrorPayload>> {
    let present_fields = ["mode", "base", "staged", "files"]
        .into_iter()
        .filter(|field| args.is_some_and(|value| value.get(field).is_some()))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    Ok(LegacyBuildUpdateFields { present_fields })
}

fn validate_build_operation_request(
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

fn resolve_change_source(
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

fn change_source_json(resolved: &ResolvedChangeSource) -> Value {
    json!({
        "kind": resolved.kind.as_str(),
        "resolved_files": &resolved.files,
        "deleted_files": &resolved.deleted_files,
        "base": &resolved.base,
    })
}

fn node_repo_id(node: &atlas_core::Node) -> Option<&str> {
    node.extra_json
        .as_object()
        .and_then(|extra| extra.get("repo_id"))
        .and_then(|value| value.as_str())
}

fn changed_repo_summary_json(
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

fn boundary_summary_json(advanced: &atlas_core::AdvancedImpactResult) -> Value {
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

fn cross_repo_context_hops_json(edges: &[Value], store: &Store) -> Value {
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

fn impact_seed_qnames_for_repo(
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

fn detect_changes_for_registration(
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

fn insert_change_source_payload(payload: &mut Map<String, Value>, resolved: &ResolvedChangeSource) {
    payload.insert("change_source".to_owned(), change_source_json(resolved));
}

fn as_object_map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

fn build_normalized_success_response(
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GetContextTargetKind {
    Query,
    File,
    Files,
}

impl GetContextTargetKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Query => "query",
            Self::File => "file",
            Self::Files => "files",
        }
    }
}

struct ParsedGetContextTarget {
    kind: GetContextTargetKind,
    target: ContextTarget,
    parsed_request: Option<atlas_core::model::ContextRequest>,
    query: Option<String>,
    file: Option<String>,
    files: Vec<String>,
    deprecated_input_fields: Vec<String>,
}

fn get_context_target_error(
    message: impl Into<String>,
    detail: impl Into<String>,
    offending_fields: Vec<&str>,
    retry_example: Value,
) -> Box<ToolErrorPayload> {
    Box::new(input_shape_error_payload(
        "get_context",
        message,
        detail,
        InputShapeErrorSpec {
            offending_fields: offending_fields.into_iter().map(str::to_owned).collect(),
            normalization_performed: Vec::new(),
            accepted_argument_families: vec![
                "target.kind=query".to_owned(),
                "target.kind=file".to_owned(),
                "target.kind=files".to_owned(),
            ],
            retry_example: Some(retry_example),
            fail_closed_reason: Some(
                "Atlas refused to guess between conflicting get_context target selectors"
                    .to_owned(),
            ),
            retry_guidance: Some(
                "Provide exactly one get_context target selector and retry.".to_owned(),
            ),
            extra_details: Some(json!({
                "accepted_target_shapes": [
                    { "target": { "kind": "query", "query": "handle_request" } },
                    { "target": { "kind": "file", "file": "src/lib.rs" } },
                    { "target": { "kind": "files", "files": ["src/lib.rs"] } }
                ]
            })),
        },
    ))
}

fn context_query_looks_like_unstructured_description(query: &str) -> bool {
    mcp_query_looks_like_unstructured_description(query)
}

fn parse_get_context_target(
    args: Option<&Value>,
) -> std::result::Result<ParsedGetContextTarget, Box<ToolErrorPayload>> {
    if args.is_some_and(|value| {
        value.get("query").is_some() || value.get("file").is_some() || value.get("files").is_some()
    }) {
        return Err(get_context_target_error(
            "legacy get_context target fields are no longer supported",
            "Use target={ kind: 'query' | 'file' | 'files', ... } and remove top-level query/file/files fields.",
            vec!["query", "file", "files"],
            json!({ "target": { "kind": "query", "query": "handle_request" } }),
        ));
    }

    let target_value = args.and_then(|value| value.get("target"));
    let target_object = target_value.and_then(|value| value.as_object());

    if target_value.is_some() && target_object.is_none() {
        return Err(get_context_target_error(
            "invalid target selector",
            "target must be an object with kind=query, kind=file, or kind=files",
            vec!["target"],
            json!({ "target": { "kind": "query", "query": "handle_request" } }),
        ));
    }

    if let Some(target) = target_object {
        let kind = target
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                get_context_target_error(
                    "target.kind is required",
                    "target object requires kind=query, kind=file, or kind=files",
                    vec!["target.kind"],
                    json!({ "target": { "kind": "query", "query": "handle_request" } }),
                )
            })?;
        match kind {
            "query" => {
                let query = target
                    .get("query")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        get_context_target_error(
                            "target.kind='query' requires non-empty target.query",
                            "query target requires target.query",
                            vec!["target.kind", "target.query"],
                            json!({ "target": { "kind": "query", "query": "handle_request" } }),
                        )
                    })?
                    .to_owned();
                if context_query_looks_like_unstructured_description(&query) {
                    return Err(get_context_target_error(
                        "target.query must be exact identifier, qualified name, or supported intent phrase",
                        format!(
                            "query target does not accept natural-language-only descriptions. Supported grammar: {}.",
                            mcp_supported_query_grammar_examples().join(", ")
                        ),
                        vec!["target.query"],
                        json!({ "target": { "kind": "query", "query": "who calls handle_request" } }),
                    ));
                }
                let parsed = parse_mcp_query_grammar(&query);
                Ok(ParsedGetContextTarget {
                    kind: GetContextTargetKind::Query,
                    target: parsed.parsed_request.target.clone(),
                    parsed_request: Some(parsed.parsed_request),
                    query: Some(query),
                    file: None,
                    files: Vec::new(),
                    deprecated_input_fields: Vec::new(),
                })
            }
            "file" => {
                let file = target
                    .get("file")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        get_context_target_error(
                            "target.kind='file' requires non-empty target.file",
                            "file target requires target.file",
                            vec!["target.kind", "target.file"],
                            json!({ "target": { "kind": "file", "file": "src/lib.rs" } }),
                        )
                    })?;
                let file = CanonicalRepoPath::from_repo_relative(file)
                    .map_err(|error| {
                        get_context_target_error(
                            "invalid target.file path",
                            error.to_string(),
                            vec!["target.file"],
                            json!({ "target": { "kind": "file", "file": "src/lib.rs" } }),
                        )
                    })?
                    .as_str()
                    .to_owned();
                Ok(ParsedGetContextTarget {
                    kind: GetContextTargetKind::File,
                    target: ContextTarget::FilePath { path: file.clone() },
                    parsed_request: None,
                    query: None,
                    file: Some(file),
                    files: Vec::new(),
                    deprecated_input_fields: Vec::new(),
                })
            }
            "files" => {
                let files = target
                    .get("files")
                    .and_then(|value| value.as_array())
                    .ok_or_else(|| {
                        get_context_target_error(
                            "target.kind='files' requires non-empty target.files",
                            "files target requires target.files array",
                            vec!["target.kind", "target.files"],
                            json!({ "target": { "kind": "files", "files": ["src/lib.rs"] } }),
                        )
                    })?
                    .iter()
                    .filter_map(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(|path| {
                        CanonicalRepoPath::from_repo_relative(path)
                            .map(|path| path.as_str().to_owned())
                            .map_err(|error| {
                                get_context_target_error(
                                    "invalid target.files path",
                                    error.to_string(),
                                    vec!["target.files"],
                                    json!({ "target": { "kind": "files", "files": ["src/lib.rs"] } }),
                                )
                            })
                    })
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                if files.is_empty() {
                    return Err(get_context_target_error(
                        "target.kind='files' requires non-empty target.files",
                        "files target requires target.files array",
                        vec!["target.kind", "target.files"],
                        json!({ "target": { "kind": "files", "files": ["src/lib.rs"] } }),
                    ));
                }
                Ok(ParsedGetContextTarget {
                    kind: GetContextTargetKind::Files,
                    target: ContextTarget::ChangedFiles {
                        paths: files.clone(),
                    },
                    parsed_request: None,
                    query: None,
                    file: None,
                    files,
                    deprecated_input_fields: Vec::new(),
                })
            }
            other => Err(get_context_target_error(
                format!("invalid target.kind '{other}'"),
                "target.kind must be one of: query, file, files",
                vec!["target.kind"],
                json!({ "target": { "kind": "query", "query": "handle_request" } }),
            )),
        }
    } else {
        Err(get_context_target_error(
            "get_context requires target",
            "Provide target={ kind: 'query' | 'file' | 'files', ... }.",
            vec!["target"],
            json!({ "target": { "kind": "query", "query": "handle_request" } }),
        ))
    }
}

fn count_change_kinds(changes: &[ChangedFile]) -> (usize, usize, usize, usize, usize) {
    let mut added = 0;
    let mut modified = 0;
    let mut deleted = 0;
    let mut renamed = 0;
    let mut copied = 0;
    for change in changes {
        match change.change_type {
            ChangeType::Added => added += 1,
            ChangeType::Modified => modified += 1,
            ChangeType::Deleted => deleted += 1,
            ChangeType::Renamed => renamed += 1,
            ChangeType::Copied => copied += 1,
        }
    }
    (added, modified, deleted, renamed, copied)
}

pub(super) fn tool_get_impact_radius(
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

pub(super) fn tool_get_review_context(
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

pub(super) fn tool_detect_changes(
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

pub(super) fn tool_build_or_update_graph(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let operation = match validate_build_operation_request(args) {
        Ok(operation) => operation,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let deprecated_operation_fields = operation.deprecated_input_fields.clone();
    let repo_root_path =
        find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;
    let repo_root_str = repo_root_path.as_str();

    fn build_status_json(db_path: &str, repo_root: &str) -> serde_json::Value {
        let Ok(store) = Store::open(db_path) else {
            return serde_json::Value::Null;
        };
        let Ok(Some(bs)) = store.get_build_status(repo_root) else {
            return serde_json::Value::Null;
        };
        let state_str = match bs.state {
            GraphBuildState::Building => "building",
            GraphBuildState::Built => "built",
            GraphBuildState::Degraded => "degraded",
            GraphBuildState::BuildFailed => "build_failed",
        };
        serde_json::json!({
            "state": state_str,
            "files_discovered": bs.files_discovered,
            "files_processed": bs.files_processed,
            "files_accepted": bs.files_accepted,
            "files_skipped_by_byte_budget": bs.files_skipped_by_byte_budget,
            "files_failed": bs.files_failed,
            "bytes_accepted": bs.bytes_accepted,
            "bytes_skipped": bs.bytes_skipped,
            "nodes_written": bs.nodes_written,
            "edges_written": bs.edges_written,
            "budget_stop_reason": bs.budget_stop_reason,
            "last_built_at": bs.last_built_at,
            "last_error": bs.last_error,
        })
    }

    fn budget_status_label(status: atlas_core::BudgetStatus) -> &'static str {
        match status {
            atlas_core::BudgetStatus::WithinBudget => "within_budget",
            atlas_core::BudgetStatus::OverrideClamped => "override_clamped",
            atlas_core::BudgetStatus::PartialResult => "partial_result",
            atlas_core::BudgetStatus::Blocked => "blocked",
        }
    }

    if operation.kind == BuildOperationKind::Update {
        let change_source = operation
            .change_source
            .clone()
            .expect("validated update operation requires change_source");
        let base = change_source.base.clone();
        let (target_kind, target) = match change_source.kind {
            ResolvedChangeSourceKind::Files => {
                ("files", UpdateTarget::Files(change_source.files.clone()))
            }
            ResolvedChangeSourceKind::Staged => ("staged", UpdateTarget::Staged),
            ResolvedChangeSourceKind::Base => (
                "base",
                UpdateTarget::BaseRef(base.clone().expect("base kind requires base ref")),
            ),
            ResolvedChangeSourceKind::WorkingTree => ("working_tree", UpdateTarget::WorkingTree),
        };

        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
            .unwrap_or_default();
        let build_budget = config.build_run_budget()?;

        if let Ok(s) = Store::open(db_path) {
            let _ = s.begin_build(repo_root_str);
        }

        crate::progress::report("detecting changed files", None);
        if crate::progress::is_canceled() {
            return Err(anyhow::anyhow!("canceled"));
        }
        crate::progress::report("updating graph", Some(10));

        let update_result = update_graph(
            repo_root_path.as_path(),
            db_path,
            &UpdateOptions {
                fail_fast: false,
                dry_run: false,
                batch_size: config.parse_batch_size(),
                target,
                budget: build_budget,
                source_repo_id: Some(stable_repo_id(repo_root_path.as_path())),
                namespace_qualified_names: false,
            },
        );

        if let Ok(s) = Store::open(db_path) {
            match &update_result {
                Ok(sum) => {
                    let state =
                        if matches!(sum.budget.budget_status, atlas_core::BudgetStatus::Blocked) {
                            GraphBuildState::BuildFailed
                        } else if sum.is_degraded() {
                            GraphBuildState::Degraded
                        } else {
                            GraphBuildState::Built
                        };
                    let _ = s.finish_build(
                        repo_root_str,
                        BuildFinishStats {
                            state,
                            files_discovered: (sum.parsed + sum.deleted + sum.renamed) as i64,
                            files_processed: sum.parsed as i64,
                            files_accepted: sum.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: sum
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: sum.parse_errors as i64,
                            bytes_accepted: sum.budget_counters.bytes_accepted as i64,
                            bytes_skipped: sum.budget_counters.bytes_skipped as i64,
                            nodes_written: sum.nodes_updated as i64,
                            edges_written: sum.edges_updated as i64,
                            budget_stop_reason: sum.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                }
            }
        }

        crate::progress::report("writing results", Some(90));
        let summary = update_result?;
        crate::progress::report("update complete", Some(100));

        let status = if matches!(
            summary.budget.budget_status,
            atlas_core::BudgetStatus::Blocked
        ) {
            "blocked"
        } else if summary.is_degraded() {
            "degraded"
        } else {
            "completed"
        };
        let warnings = summary.warnings.clone();
        let payload = json!({
            "mode": operation.kind.as_str(),
            "status": status,
            "source": {
                "target_kind": target_kind,
                "base_ref": base,
                "staged": matches!(change_source.kind, ResolvedChangeSourceKind::Staged),
            },
            "files_scanned": summary.parsed + summary.deleted + summary.renamed,
            "files_changed": summary.parsed + summary.deleted + summary.renamed,
            "files_parsed": summary.parsed,
            "files_deleted": summary.deleted,
            "files_renamed": summary.renamed,
            "files_skipped_unsupported": summary.skipped_unsupported,
            "files_skipped_unchanged": 0,
            "parse_error_count": summary.parse_errors,
            "chunk_upsert_failure_count": summary.chunk_upsert_failures,
            "call_target_reconcile_failure_count": summary.call_target_reconcile_failures,
            "nodes_written": summary.nodes_updated,
            "edges_written": summary.edges_updated,
            "duration_ms": summary.elapsed_ms as u64,
            "warnings": warnings,
            "stages": [
                {
                    "name": "detect_changes",
                    "status": "completed",
                    "item_count": summary.parsed + summary.deleted + summary.renamed,
                    "details": {
                        "target_kind": target_kind,
                    }
                },
                {
                    "name": "update_graph",
                    "status": status,
                    "item_count": summary.parsed,
                    "details": {
                        "skipped_unsupported": summary.skipped_unsupported,
                        "parse_errors": summary.parse_errors,
                    }
                },
                {
                    "name": "persist_graph",
                    "status": status,
                    "item_count": summary.nodes_updated + summary.edges_updated,
                    "details": {
                        "nodes_written": summary.nodes_updated,
                        "edges_written": summary.edges_updated,
                    }
                }
            ],
            "summary": {
                "budget_status": budget_status_label(summary.budget.budget_status),
                "budget_hit": summary.budget.budget_hit,
                "partial": summary.budget.partial,
                "safe_to_answer": summary.budget.safe_to_answer,
                "budget_counters": summary.budget_counters,
            },
            "build_status": build_status_json(db_path, repo_root_str),
        });
        let envelope = ToolSuccessEnvelope::new("build_or_update_graph", payload);
        let mut response = normalized_tool_result_value(&envelope, output_format)?;
        inject_budget_metadata(&mut response, &summary.budget);
        inject_deprecated_input_fields(&mut response, &deprecated_operation_fields);
        Ok(response)
    } else {
        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
            .unwrap_or_default();
        let build_budget = config.build_run_budget()?;

        if let Ok(s) = Store::open(db_path) {
            let _ = s.begin_build(repo_root_str);
        }

        crate::progress::report("scanning repository files", None);
        if crate::progress::is_canceled() {
            return Err(anyhow::anyhow!("canceled"));
        }
        crate::progress::report("building graph", Some(10));

        let build_result = build_graph(
            repo_root_path.as_path(),
            db_path,
            &BuildOptions {
                fail_fast: false,
                dry_run: false,
                batch_size: config.parse_batch_size(),
                budget: build_budget,
                source_repo_id: Some(stable_repo_id(repo_root_path.as_path())),
                namespace_qualified_names: false,
            },
        );

        if let Ok(s) = Store::open(db_path) {
            match &build_result {
                Ok(sum) => {
                    let state =
                        if matches!(sum.budget.budget_status, atlas_core::BudgetStatus::Blocked) {
                            GraphBuildState::BuildFailed
                        } else if sum.is_degraded() {
                            GraphBuildState::Degraded
                        } else {
                            GraphBuildState::Built
                        };
                    let _ = s.finish_build(
                        repo_root_str,
                        BuildFinishStats {
                            state,
                            files_discovered: sum.scanned as i64,
                            files_processed: sum.parsed as i64,
                            files_accepted: sum.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: sum
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: sum.parse_errors as i64,
                            bytes_accepted: sum.budget_counters.bytes_accepted as i64,
                            bytes_skipped: sum.budget_counters.bytes_skipped as i64,
                            nodes_written: sum.nodes_inserted as i64,
                            edges_written: sum.edges_inserted as i64,
                            budget_stop_reason: sum.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                }
            }
        }

        crate::progress::report("writing results", Some(90));
        let summary = build_result?;
        crate::progress::report("build complete", Some(100));

        let status = if matches!(
            summary.budget.budget_status,
            atlas_core::BudgetStatus::Blocked
        ) {
            "blocked"
        } else if summary.is_degraded() {
            "degraded"
        } else {
            "completed"
        };
        let warnings = summary.warnings.clone();
        let payload = json!({
            "mode": operation.kind.as_str(),
            "status": status,
            "source": {
                "target_kind": "full_build",
                "base_ref": Value::Null,
                "staged": false,
            },
            "files_scanned": summary.scanned,
            "files_changed": 0,
            "files_parsed": summary.parsed,
            "files_deleted": 0,
            "files_renamed": 0,
            "files_skipped_unsupported": summary.skipped_unsupported,
            "files_skipped_unchanged": summary.skipped_unchanged,
            "parse_error_count": summary.parse_errors,
            "chunk_upsert_failure_count": summary.chunk_upsert_failures,
            "call_target_reconcile_failure_count": summary.call_target_reconcile_failures,
            "nodes_written": summary.nodes_inserted,
            "edges_written": summary.edges_inserted,
            "duration_ms": summary.elapsed_ms as u64,
            "warnings": warnings,
            "stages": [
                {
                    "name": "scan_repo",
                    "status": "completed",
                    "item_count": summary.scanned,
                    "details": {
                        "files_scanned": summary.scanned,
                    }
                },
                {
                    "name": "parse_repo",
                    "status": status,
                    "item_count": summary.parsed,
                    "details": {
                        "skipped_unsupported": summary.skipped_unsupported,
                        "skipped_unchanged": summary.skipped_unchanged,
                        "parse_errors": summary.parse_errors,
                    }
                },
                {
                    "name": "persist_graph",
                    "status": status,
                    "item_count": summary.nodes_inserted + summary.edges_inserted,
                    "details": {
                        "nodes_written": summary.nodes_inserted,
                        "edges_written": summary.edges_inserted,
                    }
                }
            ],
            "summary": {
                "budget_status": budget_status_label(summary.budget.budget_status),
                "budget_hit": summary.budget.budget_hit,
                "partial": summary.budget.partial,
                "safe_to_answer": summary.budget.safe_to_answer,
                "budget_counters": summary.budget_counters,
            },
            "build_status": build_status_json(db_path, repo_root_str),
        });
        let envelope = ToolSuccessEnvelope::new("build_or_update_graph", payload);
        let mut response = normalized_tool_result_value(&envelope, output_format)?;
        inject_budget_metadata(&mut response, &summary.budget);
        inject_deprecated_input_fields(&mut response, &deprecated_operation_fields);
        Ok(response)
    }
}

pub(super) fn tool_get_minimal_context(
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

pub(super) fn tool_explain_change(
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

pub(super) fn tool_get_context(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    use atlas_contentstore::ContentStore;

    let target = match parse_get_context_target(args) {
        Ok(target) => target,
        Err(payload) => return tool_execution_error_value(output_format, &payload),
    };
    let intent_override = str_arg(args, "intent")?.map(str::to_owned);
    let max_nodes = u64_arg(args, "max_nodes").map(|n| n as usize);
    let max_edges = u64_arg(args, "max_edges").map(|n| n as usize);
    let max_files = u64_arg(args, "max_files").map(|n| n as usize);
    let max_depth = u64_arg(args, "max_depth").map(|n| n as u32);
    let code_spans = bool_arg(args, "code_spans");
    let tests = bool_arg(args, "tests");
    let imports = bool_arg(args, "imports");
    let neighbors = bool_arg(args, "neighbors");
    let semantic = bool_arg(args, "semantic").unwrap_or(false);
    let include_saved_context = bool_arg(args, "include_saved_context").unwrap_or(false);
    let allow_cross_repo_edges = bool_arg(args, "allow_cross_repo_edges").unwrap_or(false);
    let session_id = str_arg(args, "session_id")?.map(str::to_owned);
    let agent_id = str_arg(args, "agent_id")?.map(str::to_owned);
    let merge_agent_partitions = bool_arg(args, "merge_agent_partitions").unwrap_or(false);
    let token_budget = u64_arg(args, "token_budget").map(|n| n as usize);

    let mut request = match target.kind {
        GetContextTargetKind::Files => {
            let intent = intent_override
                .as_deref()
                .map(parse_mcp_intent)
                .unwrap_or(ContextIntent::Review);
            ContextRequest {
                intent,
                target: target.target.clone(),
                ..ContextRequest::default()
            }
        }
        GetContextTargetKind::File => {
            let intent = intent_override
                .as_deref()
                .map(parse_mcp_intent)
                .unwrap_or(ContextIntent::File);
            ContextRequest {
                intent,
                target: target.target.clone(),
                ..ContextRequest::default()
            }
        }
        GetContextTargetKind::Query => {
            let mut parsed = target
                .parsed_request
                .clone()
                .expect("query target parsed request");
            if let Some(ref ov) = intent_override {
                parsed.intent = parse_mcp_intent(ov);
            }
            parsed
        }
    };

    if max_nodes.is_some() {
        request.max_nodes = max_nodes;
    }
    if max_edges.is_some() {
        request.max_edges = max_edges;
    }
    if max_files.is_some() {
        request.max_files = max_files;
    }
    if max_depth.is_some() {
        request.depth = max_depth;
    }
    if let Some(v) = code_spans {
        request.include_code_spans = v;
    }
    if let Some(v) = tests {
        request.include_tests = v;
    }
    if let Some(v) = imports {
        request.include_imports = v;
    }
    if let Some(v) = neighbors {
        request.include_neighbors = v;
    }
    request.include_saved_context = include_saved_context;
    request.allow_cross_repo_edges = allow_cross_repo_edges;
    request.session_id = session_id;
    request.agent_id = agent_id.clone();
    request.merge_agent_partitions = merge_agent_partitions;
    if token_budget.is_some() {
        request.token_budget = token_budget;
    }

    let store = open_store(db_path)?;
    let policy = load_budget_policy(repo_root)?;

    // --semantic: when target is a SymbolName, run graph-aware semantic search
    // first to resolve the best-matching qualified name, then build context
    // around the resolved node instead of doing a fuzzier name lookup.
    if semantic && let ContextTarget::SymbolName { ref name } = request.target {
        let sq = SearchQuery {
            text: name.clone(),
            limit: 5,
            graph_expand: true,
            graph_max_hops: 1,
            ..Default::default()
        };
        let hits = sem::context_boosted_search(&store, &sq, &[], &[]).unwrap_or_default();
        if let Some(top) = hits.into_iter().next() {
            request.target = ContextTarget::QualifiedName {
                qname: top.node.qualified_name,
            };
        }
    }

    let engine = ContextEngine::new(&store).with_budget_policy(policy);

    let result = if include_saved_context {
        let content_db = derive_content_db_path(db_path);
        match ContentStore::open(&content_db) {
            Ok(mut cs) => {
                let _ = cs.migrate();
                let engine = engine.with_content_store(&cs);
                engine.build(&request).context("context engine failed")?
            }
            Err(_) => engine.build(&request).context("context engine failed")?,
        }
    } else {
        engine.build(&request).context("context engine failed")?
    };

    let include_context_ranking_evidence = output_format == crate::output::OutputFormat::Json;
    let packaged = package_context_result(&result, include_context_ranking_evidence);
    let mut packaged_value = serde_json::to_value(&packaged)?;
    let linked_decisions = context_decision_lookup_query(&request)
        .map(|query| {
            let hits = search_decisions_best_effort(
                repo_root,
                db_path,
                request.session_id.as_deref(),
                &query,
                3,
            );
            (query, hits)
        })
        .filter(|(_, hits)| !hits.is_empty());
    if let Some((query, hits)) = linked_decisions.as_ref() {
        packaged_value["linked_decisions"] = decision_hits_json(hits);
        packaged_value["decision_lookup_query"] = serde_json::Value::String(query.clone());
    }
    let context_files: Vec<String> = match &request.target {
        ContextTarget::ChangedFiles { paths } => paths.clone(),
        ContextTarget::FilePath { path } => vec![path.clone()],
        _ => Vec::new(),
    }
    .into_iter()
    .chain(result.files.iter().map(|file| file.path.clone()))
    .chain(result.nodes.iter().map(|node| node.node.file_path.clone()))
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect();

    let mut omitted: Vec<&str> = Vec::new();
    if !result.request.include_tests {
        omitted.push("tests");
    }
    if !result.request.include_code_spans {
        omitted.push("code_spans");
    }
    if !result.request.include_neighbors {
        omitted.push("neighbors");
    }

    let response_budget_limit = policy
        .mcp_cli_payload_serialization
        .mcp_response_bytes
        .default_limit;

    let response_budget_limit = response_budget_limit.saturating_sub(500);
    let stage_budget = if let Some(response_budget) =
        enforce_mcp_response_budget(&mut packaged_value, output_format, response_budget_limit)?
    {
        result.budget.clone().merge(response_budget)
    } else {
        result.budget.clone()
    };
    let mode = match &request.target {
        ContextTarget::ChangedFiles { .. } => "change_context",
        ContextTarget::FilePath { .. } => "file_context",
        ContextTarget::QualifiedName { .. } | ContextTarget::SymbolName { .. } => "symbol_context",
        ContextTarget::ChangedSymbols { .. } => "change_context",
        ContextTarget::EdgeQuerySeed { .. } => "symbol_context",
    };
    let assets = packaged_value
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
    let ranked_symbols = packaged_value
        .get("nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|node| {
            json!({
                "qn": node.get("qn").cloned().unwrap_or(Value::Null),
                "reason": node.get("reason").cloned().unwrap_or(Value::Null),
                "distance": node.get("distance").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();
    let ranked_edges = packaged_value
        .get("edges")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|edge| {
            json!({
                "from": edge.get("from").cloned().unwrap_or(Value::Null),
                "to": edge.get("to").cloned().unwrap_or(Value::Null),
                "kind": edge.get("kind").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();
    let ranked_files = packaged_value
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|file| {
            json!({
                "path": file.get("path").cloned().unwrap_or(Value::Null),
                "reason": file.get("reason").cloned().unwrap_or(Value::Null),
            })
        })
        .collect::<Vec<_>>();
    let ambiguity_candidates = packaged_value
        .get("ambiguity_candidates")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let repo_aliases = repo_aliases_by_id(repo_root);
    let ambiguity_candidates_detailed = ambiguity_candidates
        .iter()
        .filter_map(Value::as_str)
        .map(|qname| {
            let node = store.node_by_qname(qname).ok().flatten();
            let repo_id = node.as_ref().and_then(node_repo_id);
            json!({
                "qualified_name": qname,
                "file_path": node.as_ref().map(|node| node.file_path.clone()),
                "kind": node.as_ref().map(|node| node.kind.as_str().to_owned()),
                "repo": {
                    "repo_id": repo_id,
                    "display_alias": repo_id.and_then(|id| repo_aliases.get(id)).cloned(),
                }
            })
        })
        .collect::<Vec<_>>();
    let ambiguity = json!({
        "query": packaged_value.get("ambiguity_query").cloned().unwrap_or(Value::Null),
        "candidates": serde_json::Value::Array(ambiguity_candidates),
        "candidates_detailed": ambiguity_candidates_detailed,
    });
    let mut normalized_payload = as_object_map(packaged_value.clone());
    normalized_payload.remove("saved_context_sources");
    normalized_payload.insert("mode".to_owned(), json!(mode));
    normalized_payload.insert(
        "target".to_owned(),
        json!({
            "kind": target.kind.as_str(),
            "query": target.query,
            "file": target.file,
            "files": target.files,
        }),
    );
    normalized_payload.insert(
        "query".to_owned(),
        match &request.target {
            ContextTarget::QualifiedName { qname } => json!(qname),
            ContextTarget::SymbolName { name } => json!(name),
            ContextTarget::EdgeQuerySeed { source_qname, .. } => json!(source_qname),
            _ => Value::Null,
        },
    );
    normalized_payload.insert(
        "file".to_owned(),
        match &request.target {
            ContextTarget::FilePath { path } => json!(path),
            _ => Value::Null,
        },
    );
    normalized_payload.insert(
        "files".to_owned(),
        match &request.target {
            ContextTarget::ChangedFiles { paths } => json!(paths),
            _ => json!([]),
        },
    );
    normalized_payload.insert("ranked_symbols".to_owned(), Value::Array(ranked_symbols));
    normalized_payload.insert("ranked_edges".to_owned(), Value::Array(ranked_edges));
    normalized_payload.insert("ranked_files".to_owned(), Value::Array(ranked_files));
    normalized_payload.insert("assets".to_owned(), Value::Array(assets));
    normalized_payload.insert("ambiguity".to_owned(), ambiguity);
    if include_context_ranking_evidence {
        normalized_payload.insert(
            "ranking_evidence_legend".to_owned(),
            context_ranking_evidence_legend_json(),
        );
    }
    normalized_payload.insert("context_files".to_owned(), json!(context_files));
    normalized_payload.insert(
        "cross_repo_context_hops".to_owned(),
        cross_repo_context_hops_json(
            packaged_value
                .get("edges")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
            &store,
        ),
    );
    normalized_payload.insert(
        "detail_controls".to_owned(),
        serde_json::json!({
            "max_files": result.request.max_files,
            "max_nodes": result.request.max_nodes,
            "max_edges": result.request.max_edges,
            "code_spans": result.request.include_code_spans,
            "tests": result.request.include_tests,
            "imports": result.request.include_imports,
            "neighbors": result.request.include_neighbors,
            "semantic": semantic,
            "allow_cross_repo_edges": result.request.allow_cross_repo_edges,
            "agent_id": result.request.agent_id,
            "merge_agent_partitions": result.request.merge_agent_partitions,
            "omitted_sections": omitted,
        }),
    );
    normalized_payload.insert(
        "agent_scope".to_owned(),
        serde_json::json!({
            "agent_id": result.request.agent_id,
            "merge_agent_partitions": result.request.merge_agent_partitions,
        }),
    );
    let missing_lookup_hint = "No graph nodes matched this request. Possible causes: \
         (1) the graph has not been built yet — run build_or_update_graph first; \
         (2) 'query' contained a natural-language phrase instead of a symbol name or \
         qualified name — try a short exact identifier (e.g. 'BalancesTab') or \
         use query_graph with regex for pattern matching; \
         (3) the file path is wrong or the file has no indexed symbols.";
    let lookup = if result.nodes.is_empty() {
        serde_json::json!({
            "status": "node_not_found",
            "error_code": "node_not_found",
            "error_code_docs": error_code_docs("node_not_found"),
            "message": error_message("node_not_found"),
            "suggestions": error_suggestions("node_not_found"),
            "hint": missing_lookup_hint,
        })
    } else {
        serde_json::json!({
            "status": "ok",
            "error_code": Value::Null,
            "error_code_docs": Value::Null,
            "message": Value::Null,
            "suggestions": [],
            "hint": Value::Null,
        })
    };
    normalized_payload.insert("lookup".to_owned(), lookup);
    let warnings = if result.nodes.is_empty() {
        vec![error_message("node_not_found").to_owned()]
    } else {
        Vec::new()
    };

    let mut response = build_normalized_success_response(
        "get_context",
        Value::Object(normalized_payload),
        output_format,
        warnings,
        packaged.truncated,
        packaged
            .truncated
            .then_some("context capped by node, edge, file, or payload budget"),
    )?;
    inject_budget_metadata(&mut response, &stage_budget);
    inject_deprecated_input_fields(&mut response, &target.deprecated_input_fields);
    if let Some((query, hits)) = linked_decisions {
        let source_ids = hits
            .iter()
            .flat_map(|hit| hit.decision.source_ids.iter().cloned())
            .take(5)
            .collect::<Vec<_>>();
        record_mcp_decision_best_effort(
            repo_root,
            db_path,
            &format!("reuse prior decision for context: {query}"),
            Some("stored decision memory matched current context request"),
            serde_json::json!({
                "query": query,
                "conclusion": "prior decision reused for context request",
                "source_ids": source_ids,
                "evidence": hits.iter().take(3).map(|hit| serde_json::json!({
                    "decision_id": hit.decision.decision_id,
                    "summary": hit.decision.summary,
                    "relevance_score": hit.relevance_score,
                })).collect::<Vec<_>>(),
            }),
        );
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::context_query_looks_like_unstructured_description;

    #[test]
    fn natural_language_query_detection_rejects_plain_descriptions() {
        assert!(context_query_looks_like_unstructured_description(
            "please show me authentication flow"
        ));
    }

    #[test]
    fn natural_language_query_detection_allows_code_like_queries() {
        assert!(!context_query_looks_like_unstructured_description(
            "who calls handle_request"
        ));
        assert!(!context_query_looks_like_unstructured_description(
            "src/lib.rs::fn::handle_request"
        ));
        assert!(!context_query_looks_like_unstructured_description(
            "handle_request"
        ));
    }
}
