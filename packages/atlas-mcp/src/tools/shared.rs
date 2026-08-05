use anyhow::{Context, Result};
use atlas_contentstore::{ContentStore, IndexState};
use atlas_core::model::ContextIntent;
use atlas_core::model::{ChangeType, ChangedFile, ContextRequest, ContextTarget};
use atlas_core::{
    BudgetPolicy, BudgetReport, GraphHealthInput, GraphReadiness, GraphReadinessInput,
    GraphStoreHealthClass, classify_graph_store_error, error_code_docs_ref,
    graph_health_error_message, graph_health_error_suggestions, select_graph_health_error_code,
};
use atlas_parser::ParserRegistry;
use atlas_repo::{
    CanonicalRepoPath, DiffTarget, RepoRegistration, RepoRegistry, changed_files, find_repo_root,
    hash_file, phase1_multi_repo_supported,
};
use atlas_review::query_parser;
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use serde::Serialize;
use std::collections::BTreeMap;

use crate::output::OutputFormat;
use crate::tool_result::{
    InputShapeErrorSpec, ToolErrorPayload, input_shape_error_payload,
    tool_result_value as build_tool_result_value,
};

pub(super) const DEFAULT_OUTPUT_DESCRIPTION: &str =
    "Response body format. Atlas MCP responses are JSON-only.";

pub(crate) fn parse_mcp_intent(s: &str) -> ContextIntent {
    match s {
        "file" => ContextIntent::File,
        "review" => ContextIntent::Review,
        "impact" => ContextIntent::Impact,
        "usage_lookup" | "usage" => ContextIntent::UsageLookup,
        "refactor_safety" | "refactor" => ContextIntent::RefactorSafety,
        "dead_code_check" | "dead_code" => ContextIntent::DeadCodeCheck,
        "rename_preview" | "rename" => ContextIntent::RenamePreview,
        "dependency_removal" | "deps" => ContextIntent::DependencyRemoval,
        _ => ContextIntent::Symbol,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct McpQueryGrammar {
    pub(crate) kind: &'static str,
    pub(crate) accepted: bool,
    pub(crate) source_text: String,
    pub(crate) normalized_text: String,
    pub(crate) parsed_request: ContextRequest,
}

pub(crate) fn mcp_supported_query_grammar_examples() -> &'static [&'static str] {
    &[
        "compute",
        "src/service.rs::fn::compute",
        "who calls compute",
        "what breaks if I change compute",
        "tests for compute",
    ]
}

fn strip_case_insensitive_prefix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if text.len() < prefix.len() {
        return None;
    }
    let head = text.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(text[prefix.len()..].trim())
    } else {
        None
    }
}

fn target_to_lookup_text(target: &ContextTarget) -> Option<String> {
    match target {
        ContextTarget::QualifiedName { qname } => Some(qname.clone()),
        ContextTarget::SymbolName { name } => Some(name.clone()),
        ContextTarget::FilePath { path } => Some(path.clone()),
        ContextTarget::ChangedFiles { paths } => paths.first().cloned(),
        ContextTarget::ChangedSymbols { qnames } => qnames.first().cloned(),
        ContextTarget::EdgeQuerySeed { source_qname, .. } => Some(source_qname.clone()),
    }
}

fn looks_code_like_query(trimmed: &str) -> bool {
    trimmed.contains("::")
        || trimmed.contains('/')
        || trimmed.contains('_')
        || trimmed.contains('(')
        || trimmed.contains('.')
        || trimmed.chars().any(|ch| ch.is_ascii_uppercase())
}

fn looks_plain_identifier(trimmed: &str) -> bool {
    !trimmed.is_empty()
        && !trimmed.contains(char::is_whitespace)
        && trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | ':' | '/' | '.'))
}

pub(crate) fn mcp_query_looks_like_unstructured_description(query: &str) -> bool {
    let trimmed = query.trim();
    if trimmed.is_empty() || !trimmed.contains(char::is_whitespace) {
        return false;
    }
    if strip_case_insensitive_prefix(trimmed, "tests for ").is_some() {
        return false;
    }
    if looks_code_like_query(trimmed) {
        return false;
    }

    let parsed = query_parser::parse_query(trimmed);
    if parsed.intent != ContextIntent::Symbol {
        return false;
    }

    let words = trimmed
        .split_whitespace()
        .map(|word| word.trim_matches(|ch: char| !ch.is_alphanumeric()))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let natural_language_cues = [
        "please", "show", "find", "explain", "review", "check", "tell", "help", "look", "what",
        "why", "how",
    ];
    if words.len() >= 4
        || words.iter().any(|word| {
            natural_language_cues
                .iter()
                .any(|cue| word.eq_ignore_ascii_case(cue))
        })
    {
        return true;
    }

    matches!(parsed.target, ContextTarget::SymbolName { ref name } if name == trimmed)
}

pub(crate) fn parse_mcp_query_grammar(query: &str) -> McpQueryGrammar {
    let trimmed = query.trim();

    if let Some(rest) = strip_case_insensitive_prefix(trimmed, "tests for ") {
        let parsed_target = query_parser::parse_query(rest);
        let normalized_text =
            target_to_lookup_text(&parsed_target.target).unwrap_or_else(|| rest.trim().to_owned());
        let mut parsed_request = parsed_target;
        parsed_request.intent = ContextIntent::UsageLookup;
        parsed_request.include_callers = true;
        parsed_request.include_callees = false;
        parsed_request.include_tests = true;
        return McpQueryGrammar {
            kind: "tests_for",
            accepted: !normalized_text.is_empty()
                && !mcp_query_looks_like_unstructured_description(rest),
            source_text: trimmed.to_owned(),
            normalized_text,
            parsed_request,
        };
    }

    let parsed_request = query_parser::parse_query(trimmed);
    let lower = trimmed.to_ascii_lowercase();
    let kind = if lower.starts_with("who calls ") {
        "who_calls"
    } else if lower.starts_with("what breaks") {
        "what_breaks"
    } else if matches!(&parsed_request.target, ContextTarget::QualifiedName { qname } if qname == trimmed)
    {
        "exact_qualified_name"
    } else if matches!(&parsed_request.target, ContextTarget::SymbolName { name } if name == trimmed)
        && looks_plain_identifier(trimmed)
    {
        "plain_identifier"
    } else if mcp_query_looks_like_unstructured_description(trimmed) {
        "unsupported_description"
    } else {
        "structured_query"
    };
    let normalized_text = match kind {
        "who_calls" | "what_breaks" => {
            target_to_lookup_text(&parsed_request.target).unwrap_or_else(|| trimmed.to_owned())
        }
        "exact_qualified_name" | "plain_identifier" => trimmed.to_owned(),
        "unsupported_description" | "structured_query" => trimmed.to_owned(),
        _ => target_to_lookup_text(&parsed_request.target).unwrap_or_else(|| trimmed.to_owned()),
    };

    McpQueryGrammar {
        kind,
        accepted: kind != "unsupported_description",
        source_text: trimmed.to_owned(),
        normalized_text,
        parsed_request,
    }
}

pub(super) fn str_arg<'a>(
    args: Option<&'a serde_json::Value>,
    key: &str,
) -> Result<Option<&'a str>> {
    Ok(args.and_then(|a| a.get(key)).and_then(|v| v.as_str()))
}

pub(super) fn u64_arg(args: Option<&serde_json::Value>, key: &str) -> Option<u64> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_u64())
}

pub(super) fn bool_arg(args: Option<&serde_json::Value>, key: &str) -> Option<bool> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_bool())
}

pub(super) fn string_array_arg(args: Option<&serde_json::Value>, key: &str) -> Result<Vec<String>> {
    Ok(args
        .and_then(|a| a.get(key))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

pub(crate) fn open_store(db_path: &str) -> Result<Store> {
    Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))
}

pub(super) fn load_budget_policy(repo_root: &str) -> Result<BudgetPolicy> {
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    config.budget_policy()
}

pub(super) fn load_embedding_config(
    repo_root: &str,
) -> Result<Option<atlas_search::embed::EmbeddingConfig>> {
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    Ok(config.embedding_backend()?.map(|backend| {
        atlas_search::embed::EmbeddingConfig::new(
            backend.url,
            backend.model,
            backend.timeout_secs,
            backend.max_retries,
            backend.retry_backoff_ms,
        )
    }))
}

pub(super) fn failure_category(
    db_exists: bool,
    graph_built: bool,
    health_class: Option<GraphStoreHealthClass>,
    build_state: Option<&str>,
    retrieval_unavailable: bool,
) -> &'static str {
    select_graph_health_error_code(GraphHealthInput {
        db_exists,
        graph_built,
        health_class,
        build_state,
        retrieval_unavailable,
    })
}

pub(super) fn error_message(error_code: &str) -> &'static str {
    graph_health_error_message(error_code)
}

pub(super) fn error_suggestions(error_code: &str) -> &'static [&'static str] {
    graph_health_error_suggestions(error_code)
}

pub(super) fn error_code_docs(error_code: &str) -> String {
    error_code_docs_ref(error_code)
}

pub(super) fn graph_issue_code(error: &str) -> &'static str {
    match classify_graph_store_error(error).as_str() {
        "schema_mismatch" => "schema_mismatch",
        "logical_inconsistency" => "logical_inconsistency",
        _ => "sqlite_corrupt",
    }
}

pub(super) fn resolve_kind_alias(input: &str) -> String {
    match input.to_ascii_lowercase().as_str() {
        "fn" | "func" | "function" => "function",
        "method" | "meth" => "method",
        "class" => "class",
        "struct" | "record" => "struct",
        "interface" | "iface" => "interface",
        "trait" => "trait",
        "enum" => "enum",
        "module" | "mod" => "module",
        "variable" | "var" | "field" => "variable",
        "constant" | "const" => "constant",
        "test" => "test",
        "import" | "use" => "import",
        "package" | "pkg" => "package",
        "file" => "file",
        other => other,
    }
    .to_owned()
}

pub(super) fn tool_result_value<T: Serialize>(
    value: &T,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    build_tool_result_value(value, output_format)
}

pub(super) fn inject_budget_metadata(response: &mut serde_json::Value, budget: &BudgetReport) {
    response["budget_status"] = serde_json::json!(budget.budget_status);
    response["budget_hit"] = serde_json::json!(budget.budget_hit);
    response["budget_name"] = serde_json::json!(&budget.budget_name);
    response["budget_limit"] = serde_json::json!(budget.budget_limit);
    response["budget_observed"] = serde_json::json!(budget.budget_observed);
    response["partial"] = serde_json::json!(budget.partial);
    response["safe_to_answer"] = serde_json::json!(budget.safe_to_answer);
}

pub(crate) fn inject_deprecated_input_fields(
    response: &mut serde_json::Value,
    deprecated_input_fields: &[String],
) {
    if deprecated_input_fields.is_empty() {
        return;
    }
    response["_meta"]["deprecated_input_fields"] = serde_json::json!(deprecated_input_fields);
}

pub(crate) const MAX_MULTI_REPO_SELECTION: usize = 32;

#[derive(Clone, Debug)]
pub(crate) struct RepoScopeSelection {
    pub repo_ids: Vec<String>,
    pub registrations: Vec<RepoRegistration>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedRepoScopeKind {
    Current,
    RepoId,
    All,
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedRepoScopeSelection {
    #[allow(dead_code)]
    pub kind: ResolvedRepoScopeKind,
    pub selection: Option<RepoScopeSelection>,
    pub deprecated_input_fields: Vec<String>,
}

pub(crate) fn repo_aliases_by_id(repo_root: &str) -> BTreeMap<String, String> {
    RepoRegistry::load_or_bootstrap(Utf8Path::new(repo_root))
        .map(|registry| {
            registry
                .registrations
                .into_iter()
                .map(|entry| (entry.repo_id, entry.display_alias))
                .collect()
        })
        .unwrap_or_default()
}

fn repo_scope_error_payload(
    tool_name: &str,
    message: impl Into<String>,
    detail: impl Into<String>,
    offending_fields: Vec<String>,
    retry_example: serde_json::Value,
) -> Box<ToolErrorPayload> {
    Box::new(input_shape_error_payload(
        tool_name,
        message,
        detail,
        InputShapeErrorSpec {
            offending_fields,
            normalization_performed: Vec::new(),
            accepted_argument_families: vec![
                "repo_scope.kind=current".to_owned(),
                "repo_scope.kind=repo_id".to_owned(),
                "repo_scope.kind=all".to_owned(),
            ],
            retry_example: Some(retry_example),
            fail_closed_reason: Some(
                "Atlas refused to guess between conflicting repository scope selectors".to_owned(),
            ),
            retry_guidance: Some("Provide exactly one repo scope selector, then retry.".to_owned()),
            extra_details: Some(serde_json::json!({
                "accepted_repo_scope_shapes": [
                    { "repo_scope": { "kind": "current" } },
                    { "repo_scope": { "kind": "repo_id", "repo_id": "atlas-core" } },
                    { "repo_scope": { "kind": "all" } }
                ]
            })),
        },
    ))
}

pub(crate) fn resolve_repo_scope_selection(
    tool_name: &str,
    args: Option<&serde_json::Value>,
    repo_root: &str,
) -> std::result::Result<ResolvedRepoScopeSelection, Box<ToolErrorPayload>> {
    let repo_scope_value = args.and_then(|value| value.get("repo_scope"));
    let repo_scope_object = repo_scope_value.and_then(|value| value.as_object());

    if args.is_some_and(|value| value.get("repo_id").is_some() || value.get("all_repos").is_some())
    {
        return Err(repo_scope_error_payload(
            tool_name,
            "legacy repo scope fields are no longer supported",
            "Use repo_scope={ kind: 'current' | 'repo_id' | 'all' } and remove top-level repo_id/all_repos fields.",
            vec!["repo_id".to_owned(), "all_repos".to_owned()],
            serde_json::json!({ "repo_scope": { "kind": "current" } }),
        ));
    }

    if repo_scope_value.is_some() && repo_scope_object.is_none() {
        return Err(repo_scope_error_payload(
            tool_name,
            "invalid repo_scope selector",
            "repo_scope must be an object with required kind field",
            vec!["repo_scope".to_owned()],
            serde_json::json!({ "repo_scope": { "kind": "current" } }),
        ));
    }

    let (kind, target_repo_id, deprecated_input_fields) = if let Some(scope) = repo_scope_object {
        let kind = scope
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                repo_scope_error_payload(
                    tool_name,
                    "repo_scope.kind is required",
                    "repo_scope object requires kind=current, kind=repo_id, or kind=all",
                    vec!["repo_scope.kind".to_owned()],
                    serde_json::json!({ "repo_scope": { "kind": "current" } }),
                )
            })?;
        match kind {
            "current" => (ResolvedRepoScopeKind::Current, None, Vec::new()),
            "repo_id" => {
                let repo_id = scope
                    .get("repo_id")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        repo_scope_error_payload(
                            tool_name,
                            "repo_scope.kind='repo_id' requires non-empty repo_id",
                            "repo_scope repo_id selector requires repo_scope.repo_id",
                            vec!["repo_scope.kind".to_owned(), "repo_scope.repo_id".to_owned()],
                            serde_json::json!({ "repo_scope": { "kind": "repo_id", "repo_id": "atlas-core" } }),
                        )
                    })?;
                (
                    ResolvedRepoScopeKind::RepoId,
                    Some(repo_id.to_owned()),
                    Vec::new(),
                )
            }
            "all" => (ResolvedRepoScopeKind::All, None, Vec::new()),
            other => {
                return Err(repo_scope_error_payload(
                    tool_name,
                    format!("invalid repo_scope.kind '{other}'"),
                    "repo_scope.kind must be one of: current, repo_id, all",
                    vec!["repo_scope.kind".to_owned()],
                    serde_json::json!({ "repo_scope": { "kind": "current" } }),
                ));
            }
        }
    } else {
        (ResolvedRepoScopeKind::Current, None, Vec::new())
    };

    if kind == ResolvedRepoScopeKind::Current {
        return Ok(ResolvedRepoScopeSelection {
            kind,
            selection: None,
            deprecated_input_fields,
        });
    }

    let registry = RepoRegistry::load(Utf8Path::new(repo_root)).with_context(|| {
        "repo registry missing; run `atlas init` or `atlas repo sync` before multi-repo MCP queries"
    }).map_err(|error| {
        repo_scope_error_payload(
            tool_name,
            "repo registry missing for requested repo scope",
            error.to_string(),
            vec!["repo_scope".to_owned()],
            serde_json::json!({ "repo_scope": { "kind": "current" } }),
        )
    })?;

    let selection = if kind == ResolvedRepoScopeKind::All {
        let registrations: Vec<RepoRegistration> = registry
            .registrations
            .into_iter()
            .filter(|entry| entry.enabled)
            .filter(|entry| phase1_multi_repo_supported(entry.relationship.kind))
            .collect();
        if registrations.len() > MAX_MULTI_REPO_SELECTION {
            return Err(repo_scope_error_payload(
                tool_name,
                "all repo scope exceeds max supported fan-out",
                format!(
                    "all repo scope exceeds max supported repo fan-out ({MAX_MULTI_REPO_SELECTION})"
                ),
                vec!["repo_scope".to_owned()],
                serde_json::json!({ "repo_scope": { "kind": "current" } }),
            ));
        }
        let repo_ids = registrations
            .iter()
            .map(|entry| entry.repo_id.clone())
            .collect();
        RepoScopeSelection {
            repo_ids,
            registrations,
        }
    } else {
        let target = target_repo_id.expect("repo_id target required");
        let registration = registry
            .registrations
            .into_iter()
            .find(|entry| entry.repo_id == target)
            .ok_or_else(|| {
                repo_scope_error_payload(
                    tool_name,
                    format!("repo id '{target}' is not registered"),
                    format!("repo id '{target}' is not registered"),
                    vec!["repo_scope.repo_id".to_owned()],
                    serde_json::json!({ "repo_scope": { "kind": "current" } }),
                )
            })?;
        if !registration.enabled {
            return Err(repo_scope_error_payload(
                tool_name,
                format!("repo id '{target}' is disabled"),
                format!("repo id '{target}' is disabled"),
                vec!["repo_scope.repo_id".to_owned()],
                serde_json::json!({ "repo_scope": { "kind": "current" } }),
            ));
        }
        RepoScopeSelection {
            repo_ids: vec![registration.repo_id.clone()],
            registrations: vec![registration],
        }
    };

    Ok(ResolvedRepoScopeSelection {
        kind,
        selection: Some(selection),
        deprecated_input_fields,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResolvedChangeSourceKind {
    WorkingTree,
    Staged,
    Base,
    Files,
}

impl ResolvedChangeSourceKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::WorkingTree => "working_tree",
            Self::Staged => "staged",
            Self::Base => "base",
            Self::Files => "files",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedChangeSourceSelection {
    pub kind: ResolvedChangeSourceKind,
    pub files: Vec<String>,
    pub base: Option<String>,
    pub deprecated_input_fields: Vec<String>,
}

fn change_source_examples(allow_explicit_files: bool) -> Vec<serde_json::Value> {
    let mut examples = Vec::new();
    if allow_explicit_files {
        examples.push(serde_json::json!({
            "change_source": {
                "kind": "files",
                "files": ["src/service.rs"]
            }
        }));
    }
    examples.push(serde_json::json!({
        "change_source": {
            "kind": "base",
            "base": "origin/main"
        }
    }));
    examples.push(serde_json::json!({
        "change_source": {
            "kind": "staged"
        }
    }));
    examples.push(serde_json::json!({
        "change_source": {
            "kind": "working_tree"
        }
    }));
    examples
}

fn accepted_change_source_families(allow_explicit_files: bool) -> Vec<String> {
    let mut families = Vec::new();
    if allow_explicit_files {
        families.push("change_source.kind=files".to_owned());
    }
    families.extend([
        "change_source.kind=base".to_owned(),
        "change_source.kind=staged".to_owned(),
        "change_source.kind=working_tree".to_owned(),
    ]);
    families
}

fn change_source_error_payload(
    tool_name: &str,
    message: impl Into<String>,
    detail: impl Into<String>,
    allow_explicit_files: bool,
    offending_fields: Vec<String>,
    present_mode_families: Vec<String>,
) -> Box<ToolErrorPayload> {
    let examples = change_source_examples(allow_explicit_files);
    let retry_example = examples.first().cloned();
    let accepted_shapes = if allow_explicit_files {
        serde_json::json!([
            { "change_source": { "kind": "files", "files": ["src/service.rs"] } },
            { "change_source": { "kind": "base", "base": "origin/main" } },
            { "change_source": { "kind": "staged" } },
            { "change_source": { "kind": "working_tree" } }
        ])
    } else {
        serde_json::json!([
            { "change_source": { "kind": "base", "base": "origin/main" } },
            { "change_source": { "kind": "staged" } },
            { "change_source": { "kind": "working_tree" } }
        ])
    };
    Box::new(input_shape_error_payload(
        tool_name,
        message,
        detail,
        InputShapeErrorSpec {
            offending_fields,
            normalization_performed: Vec::new(),
            accepted_argument_families: accepted_change_source_families(allow_explicit_files),
            retry_example,
            fail_closed_reason: Some(
                "Atlas refused to guess between conflicting change-source selectors".to_owned(),
            ),
            retry_guidance: Some(
                "Provide exactly one change_source selector and retry.".to_owned(),
            ),
            extra_details: Some(serde_json::json!({
                "present_mode_families": present_mode_families,
                "accepted_change_source_shapes": accepted_shapes,
            })),
        },
    ))
}

fn canonicalize_change_source_files(
    files: &[String],
    tool_name: &str,
    field_name: &str,
    allow_explicit_files: bool,
) -> std::result::Result<Vec<String>, Box<ToolErrorPayload>> {
    files
        .iter()
        .map(|path| {
            CanonicalRepoPath::from_repo_relative(path)
                .with_context(|| format!("invalid explicit file path '{path}'"))
                .map(|path| path.as_str().to_owned())
                .map_err(|error| {
                    change_source_error_payload(
                        tool_name,
                        format!("invalid {field_name} path"),
                        error.to_string(),
                        allow_explicit_files,
                        vec![field_name.to_owned()],
                        vec!["files".to_owned()],
                    )
                })
        })
        .collect()
}

pub(crate) fn resolve_change_source_selection(
    tool_name: &str,
    args: Option<&serde_json::Value>,
    allow_explicit_files: bool,
) -> std::result::Result<ResolvedChangeSourceSelection, Box<ToolErrorPayload>> {
    let change_source_value = args.and_then(|value| value.get("change_source"));
    let change_source_object = change_source_value.and_then(|value| value.as_object());
    let mut offending_legacy_fields = Vec::new();
    if args.is_some_and(|value| value.get("mode").is_some()) {
        offending_legacy_fields.push("mode".to_owned());
    }
    if args.is_some_and(|value| value.get("files").is_some()) {
        offending_legacy_fields.push("files".to_owned());
    }
    if args.is_some_and(|value| value.get("base").is_some()) {
        offending_legacy_fields.push("base".to_owned());
    }
    if args.is_some_and(|value| value.get("staged").is_some()) {
        offending_legacy_fields.push("staged".to_owned());
    }
    if args.is_some_and(|value| value.get("working_tree").is_some()) {
        offending_legacy_fields.push("working_tree".to_owned());
    }
    if !offending_legacy_fields.is_empty() {
        return Err(change_source_error_payload(
            tool_name,
            "legacy change_source fields are no longer supported",
            "Use change_source={ kind: 'files' | 'base' | 'staged' | 'working_tree', ... } and remove top-level mode/files/base/staged/working_tree fields.",
            allow_explicit_files,
            offending_legacy_fields,
            Vec::new(),
        ));
    }

    if change_source_value.is_some() && change_source_object.is_none() {
        return Err(change_source_error_payload(
            tool_name,
            "invalid change_source selector",
            "change_source must be an object with kind=working_tree, kind=staged, kind=base, or kind=files",
            allow_explicit_files,
            vec!["change_source".to_owned()],
            Vec::new(),
        ));
    }

    if let Some(change_source) = change_source_object {
        let kind = change_source
            .get("kind")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                change_source_error_payload(
                    tool_name,
                    "change_source.kind is required",
                    "change_source object requires kind=working_tree, kind=staged, kind=base, or kind=files",
                    allow_explicit_files,
                    vec!["change_source.kind".to_owned()],
                    Vec::new(),
                )
            })?;

        return match kind {
            "working_tree" => Ok(ResolvedChangeSourceSelection {
                kind: ResolvedChangeSourceKind::WorkingTree,
                files: Vec::new(),
                base: None,
                deprecated_input_fields: Vec::new(),
            }),
            "staged" => Ok(ResolvedChangeSourceSelection {
                kind: ResolvedChangeSourceKind::Staged,
                files: Vec::new(),
                base: None,
                deprecated_input_fields: Vec::new(),
            }),
            "base" => {
                let base = change_source
                    .get("base")
                    .and_then(|value| value.as_str())
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| {
                        change_source_error_payload(
                            tool_name,
                            "change_source.kind='base' requires non-empty change_source.base",
                            "base change_source selector requires change_source.base",
                            allow_explicit_files,
                            vec![
                                "change_source.kind".to_owned(),
                                "change_source.base".to_owned(),
                            ],
                            vec!["base".to_owned()],
                        )
                    })?
                    .to_owned();
                Ok(ResolvedChangeSourceSelection {
                    kind: ResolvedChangeSourceKind::Base,
                    files: Vec::new(),
                    base: Some(base),
                    deprecated_input_fields: Vec::new(),
                })
            }
            "files" => {
                if !allow_explicit_files {
                    return Err(change_source_error_payload(
                        tool_name,
                        "invalid change_source.kind 'files'",
                        "this tool does not accept change_source.kind='files'",
                        allow_explicit_files,
                        vec!["change_source.kind".to_owned()],
                        vec!["files".to_owned()],
                    ));
                }
                let raw_files = change_source
                    .get("files")
                    .and_then(|value| value.as_array())
                    .ok_or_else(|| {
                        change_source_error_payload(
                            tool_name,
                            "change_source.kind='files' requires non-empty change_source.files",
                            "files change_source selector requires change_source.files array",
                            allow_explicit_files,
                            vec![
                                "change_source.kind".to_owned(),
                                "change_source.files".to_owned(),
                            ],
                            vec!["files".to_owned()],
                        )
                    })?
                    .iter()
                    .filter_map(|value| value.as_str().map(str::to_owned))
                    .collect::<Vec<_>>();
                if raw_files.is_empty() {
                    return Err(change_source_error_payload(
                        tool_name,
                        "change_source.kind='files' requires non-empty change_source.files",
                        "files change_source selector requires non-empty change_source.files array",
                        allow_explicit_files,
                        vec![
                            "change_source.kind".to_owned(),
                            "change_source.files".to_owned(),
                        ],
                        vec!["files".to_owned()],
                    ));
                }
                let files = canonicalize_change_source_files(
                    &raw_files,
                    tool_name,
                    "change_source.files",
                    allow_explicit_files,
                )?;
                Ok(ResolvedChangeSourceSelection {
                    kind: ResolvedChangeSourceKind::Files,
                    files,
                    base: None,
                    deprecated_input_fields: Vec::new(),
                })
            }
            other => Err(change_source_error_payload(
                tool_name,
                format!("invalid change_source.kind '{other}'"),
                "change_source.kind must be one of: working_tree, staged, base, files",
                allow_explicit_files,
                vec!["change_source.kind".to_owned()],
                Vec::new(),
            )),
        };
    }

    Err(change_source_error_payload(
        tool_name,
        "change_source is required",
        "Provide change_source={ kind: 'files' | 'base' | 'staged' | 'working_tree', ... }.",
        allow_explicit_files,
        vec!["change_source".to_owned()],
        Vec::new(),
    ))
}

#[derive(Serialize)]
pub(super) struct FreshnessWarning {
    pub stale: bool,
    pub changed_files: Vec<String>,
    pub stale_result_files: Vec<String>,
    pub warning: String,
    pub suggested_recovery: Vec<&'static str>,
}

fn unique_sorted_paths(paths: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut paths: Vec<String> = paths.into_iter().collect();
    paths.sort();
    paths.dedup();
    paths
}

fn file_has_graph_facts(store: &Store, path: &str) -> bool {
    store
        .nodes_by_file(path)
        .map(|nodes| !nodes.is_empty())
        .unwrap_or(false)
}

fn graph_contains_file_state(store: &Store, path: &str) -> bool {
    store.file_hash(path).ok().flatten().is_some() || file_has_graph_facts(store, path)
}

fn graph_matches_worktree_path(store: &Store, repo_root: &Utf8Path, path: &str) -> bool {
    let worktree_hash = hash_file(&repo_root.join(path));
    let indexed_hash = store.file_hash(path).ok().flatten();

    match worktree_hash {
        Ok(current_hash) => indexed_hash.as_deref() == Some(current_hash.as_str()),
        Err(_) => !graph_contains_file_state(store, path),
    }
}

fn change_can_affect_graph_facts(
    store: &Store,
    registry: &ParserRegistry,
    change: &ChangedFile,
) -> bool {
    registry.supports(&change.path)
        || change
            .old_path
            .as_deref()
            .is_some_and(|old_path| registry.supports(old_path))
        || file_has_graph_facts(store, &change.path)
        || change
            .old_path
            .as_deref()
            .is_some_and(|old_path| file_has_graph_facts(store, old_path))
}

fn change_is_pending_in_graph(
    store: &Store,
    registry: &ParserRegistry,
    repo_root: &Utf8Path,
    change: &ChangedFile,
) -> bool {
    if !change_can_affect_graph_facts(store, registry, change) {
        return false;
    }

    match change.change_type {
        ChangeType::Added | ChangeType::Modified => {
            !graph_matches_worktree_path(store, repo_root, &change.path)
        }
        ChangeType::Deleted => graph_contains_file_state(store, &change.path),
        ChangeType::Renamed | ChangeType::Copied => {
            let new_path_pending = !graph_matches_worktree_path(store, repo_root, &change.path);
            let old_path_pending = change
                .old_path
                .as_deref()
                .is_some_and(|old_path| graph_contains_file_state(store, old_path));
            new_path_pending || old_path_pending
        }
    }
}

pub(super) fn pending_graph_relevant_changes(
    repo_root: &str,
    db_path: &str,
) -> Option<Vec<String>> {
    let repo_root_path = find_repo_root(Utf8Path::new(repo_root)).ok()?;
    let changes = changed_files(repo_root_path.as_path(), &DiffTarget::WorkingTree).ok()?;
    if changes.is_empty() {
        return Some(Vec::new());
    }

    let store = Store::open(db_path).ok()?;
    let registry = ParserRegistry::with_defaults();

    Some(unique_sorted_paths(
        changes
            .iter()
            .filter(|change| {
                change_is_pending_in_graph(&store, &registry, repo_root_path.as_path(), change)
            })
            .flat_map(|change| std::iter::once(change.path.clone()).chain(change.old_path.clone())),
    ))
}

pub(super) fn compute_freshness_warning(
    repo_root: &str,
    db_path: &str,
    relevant_files: &[String],
) -> Option<FreshnessWarning> {
    if relevant_files.is_empty() {
        return None;
    }

    let changed_files = pending_graph_relevant_changes(repo_root, db_path)?;
    if changed_files.is_empty() {
        return None;
    }

    let stale_result_files = unique_sorted_paths(
        relevant_files
            .iter()
            .filter(|path| changed_files.iter().any(|changed| changed == *path))
            .cloned(),
    );
    if stale_result_files.is_empty() {
        return None;
    }

    let warning = if stale_result_files.len() == 1 {
        format!(
            "Graph-backed answer may be stale: pending graph-relevant changes affect {}.",
            stale_result_files[0]
        )
    } else {
        format!(
            "Graph-backed answer may be stale: pending graph-relevant changes affect {} files in this result.",
            stale_result_files.len()
        )
    };

    Some(FreshnessWarning {
        stale: true,
        changed_files,
        stale_result_files,
        warning,
        suggested_recovery: vec![
            "run build_or_update_graph to refresh the graph",
            "run detect_changes to inspect pending graph-relevant files",
        ],
    })
}

// ── Canonical graph readiness helpers ────────────────────────────────────────

/// Derive canonical [`GraphReadiness`] from an already-open store.
///
/// This is the shared readiness derivation path for all MCP tool handlers.
/// Call this after `Store::open` succeeds; use the result to gate
/// graph-backed operations via [`GraphReadiness::check_tool`].
pub(crate) fn derive_graph_readiness(
    store: &Store,
    repo_root: &str,
    db_path: &str,
) -> GraphReadiness {
    let db_exists = std::path::Path::new(db_path).exists();

    let mut graph_error = None;
    let (build_state_str, build_last_error, recovery_mode, quarantine_path) =
        match store.get_build_status(repo_root) {
            Ok(Some(bs)) => {
                let state = match bs.state {
                    atlas_store_sqlite::GraphBuildState::Building => "building",
                    atlas_store_sqlite::GraphBuildState::Built => "built",
                    atlas_store_sqlite::GraphBuildState::Degraded => "degraded",
                    atlas_store_sqlite::GraphBuildState::BuildFailed => "build_failed",
                };
                (
                    Some(state.to_owned()),
                    bs.last_error,
                    bs.recovery_mode,
                    bs.quarantine_path,
                )
            }
            Ok(None) => (None, None, None, None),
            Err(error) => {
                graph_error = Some(error.to_string());
                (None, None, None, None)
            }
        };

    let (file_count, graph_has_content, last_indexed_at) = match store.stats() {
        Ok(s) => {
            let has_content = s.node_count > 0 || s.edge_count > 0 || s.file_count > 0;
            (s.file_count, has_content, s.last_indexed_at)
        }
        Err(e) => {
            graph_error.get_or_insert_with(|| e.to_string());
            (0, false, None)
        }
    };
    if graph_error.is_none() {
        match store.graph_store_health_class() {
            Ok(Some(GraphStoreHealthClass::SchemaMismatch)) => {
                graph_error = Some(
                    "schema_mismatch: graph store schema does not match current Atlas build"
                        .to_owned(),
                );
            }
            Ok(Some(GraphStoreHealthClass::SqliteCorrupt)) => {
                graph_error = Some(
                    "sqlite_corrupt: graph integrity check reported physical corruption".to_owned(),
                );
            }
            Ok(Some(GraphStoreHealthClass::LogicalInconsistency)) => {
                graph_error = Some(
                    "logical_inconsistency: graph invariant scan found unsafe rows".to_owned(),
                );
            }
            Ok(_) => {}
            Err(error) => {
                graph_error = Some(error.to_string());
            }
        }
    }

    let pending = pending_graph_relevant_changes(repo_root, db_path).unwrap_or_default();

    let content_db_path = atlas_engine::paths::content_db_path(db_path);
    let retrieval_unavailable = match ContentStore::open(&content_db_path) {
        Ok(mut cs) => {
            let _ = cs.migrate();
            match cs.get_index_status(repo_root) {
                Ok(Some(s)) => s.state != IndexState::Indexed,
                _ => true,
            }
        }
        Err(_) => true,
    };

    GraphReadiness::derive(GraphReadinessInput {
        repo_root,
        db_path,
        db_exists,
        db_open_error: None,
        build_state: build_state_str.as_deref(),
        build_last_error: build_last_error.as_deref(),
        graph_error: graph_error.as_deref(),
        recovery_mode: recovery_mode.as_deref(),
        quarantine_path: quarantine_path.as_deref(),
        pending_graph_changes: &pending,
        indexed_file_count: file_count,
        graph_has_content,
        last_indexed_at: last_indexed_at.as_deref(),
        retrieval_unavailable,
    })
}

/// Derive [`GraphReadiness`] when the store could not be opened.
///
/// Use this when `Store::open` fails; the open error is passed into the
/// readiness record so blocked messages are consistent.
pub(crate) fn derive_graph_readiness_open_failed(
    repo_root: &str,
    db_path: &str,
    open_error: &str,
) -> GraphReadiness {
    let db_exists = std::path::Path::new(db_path).exists();
    GraphReadiness::derive(GraphReadinessInput {
        repo_root,
        db_path,
        db_exists,
        db_open_error: Some(open_error),
        build_state: None,
        build_last_error: None,
        graph_error: None,
        recovery_mode: None,
        quarantine_path: None,
        pending_graph_changes: &[],
        indexed_file_count: 0,
        graph_has_content: false,
        last_indexed_at: None,
        retrieval_unavailable: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_repo::{
        RepoRegistration, RepoRegistry, RepoRelationship, RepoRelationshipKind, TrustState,
        VcsMetadata, stable_repo_id,
    };
    use camino::{Utf8Path, Utf8PathBuf};
    use serde_json::json;

    fn registration(root: &Utf8Path, alias: &str, kind: RepoRelationshipKind) -> RepoRegistration {
        RepoRegistration {
            repo_id: stable_repo_id(root),
            root: root.to_path_buf(),
            display_alias: alias.to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind,
                parent_repo_id: None,
                parent_path: None,
            },
            trust_state: TrustState::Trusted,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: Vec::new(),
        }
    }

    #[test]
    fn resolve_repo_scope_selection_all_kind_excludes_manual_registrations() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let sub = root.join("submodule");
        let manual = root.join("../manual-sibling");
        let mut registry = RepoRegistry::new(stable_repo_id(root));
        registry.registrations = vec![
            registration(root, ".", RepoRelationshipKind::Root),
            registration(sub.as_path(), "submodule", RepoRelationshipKind::Submodule),
            registration(manual.as_path(), "manual", RepoRelationshipKind::Manual),
        ];
        registry.save(root).unwrap();

        let resolved = resolve_repo_scope_selection(
            "test_tool",
            Some(&json!({"repo_scope": {"kind": "all"}})),
            root.as_str(),
        )
        .unwrap();
        let selection = resolved.selection.expect("selection");

        assert_eq!(resolved.kind, ResolvedRepoScopeKind::All);
        assert!(resolved.deprecated_input_fields.is_empty());
        assert_eq!(selection.registrations.len(), 2);
        assert!(
            selection
                .registrations
                .iter()
                .all(|entry| entry.relationship.kind != RepoRelationshipKind::Manual)
        );
    }

    #[test]
    fn resolve_repo_scope_selection_supports_repo_scope_object_current() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let resolved = resolve_repo_scope_selection(
            "test_tool",
            Some(&json!({"repo_scope": {"kind": "current"}})),
            root.as_str(),
        )
        .unwrap();

        assert_eq!(resolved.kind, ResolvedRepoScopeKind::Current);
        assert!(resolved.selection.is_none());
        assert!(resolved.deprecated_input_fields.is_empty());
    }

    #[test]
    fn resolve_repo_scope_selection_supports_repo_scope_object_repo_id() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let sub = root.join("submodule");
        let mut registry = RepoRegistry::new(stable_repo_id(root));
        registry.registrations = vec![
            registration(root, ".", RepoRelationshipKind::Root),
            registration(sub.as_path(), "submodule", RepoRelationshipKind::Submodule),
        ];
        registry.save(root).unwrap();
        let sub_repo_id = stable_repo_id(sub.as_path());

        let resolved = resolve_repo_scope_selection(
            "test_tool",
            Some(&json!({"repo_scope": {"kind": "repo_id", "repo_id": sub_repo_id}})),
            root.as_str(),
        )
        .unwrap();
        let selection = resolved.selection.expect("selection");

        assert_eq!(resolved.kind, ResolvedRepoScopeKind::RepoId);
        assert!(resolved.deprecated_input_fields.is_empty());
        assert_eq!(selection.repo_ids.len(), 1);
        assert_eq!(selection.repo_ids[0], stable_repo_id(sub.as_path()));
    }

    #[test]
    fn resolve_repo_scope_selection_rejects_mixed_repo_scope_and_legacy_fields() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let error = resolve_repo_scope_selection(
            "test_tool",
            Some(&json!({
                "repo_scope": {"kind": "current"},
                "all_repos": true
            })),
            root.as_str(),
        )
        .unwrap_err();

        assert_eq!(
            error.message,
            "legacy repo scope fields are no longer supported"
        );
        assert_eq!(
            error
                .details
                .as_ref()
                .and_then(|details| details.get("offending_fields"))
                .cloned(),
            Some(json!(["repo_id", "all_repos"]))
        );
    }

    #[test]
    fn resolve_repo_scope_selection_rejects_excessive_all_repo_fanout() {
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        let mut registry = RepoRegistry::new(stable_repo_id(root));
        registry.registrations = (0..(MAX_MULTI_REPO_SELECTION + 1))
            .map(|index| {
                let repo_root = Utf8PathBuf::from(format!("{}/repo-{index}", root.as_str()));
                registration(
                    repo_root.as_path(),
                    &format!("repo-{index}"),
                    RepoRelationshipKind::Submodule,
                )
            })
            .collect();
        registry.save(root).unwrap();

        let error = resolve_repo_scope_selection(
            "test_tool",
            Some(&json!({"repo_scope": {"kind": "all"}})),
            root.as_str(),
        )
        .unwrap_err();

        assert_eq!(
            error.message,
            "all repo scope exceeds max supported fan-out"
        );
        assert!(
            error
                .details
                .as_ref()
                .and_then(|details| details.get("detail"))
                .and_then(|detail| detail.as_str())
                .is_some_and(|detail| detail.contains("max supported repo fan-out"))
        );
    }

    #[test]
    fn resolve_change_source_selection_supports_change_source_object_files() {
        let resolved = resolve_change_source_selection(
            "test_tool",
            Some(&json!({
                "change_source": {
                    "kind": "files",
                    "files": ["src/./lib.rs"]
                }
            })),
            true,
        )
        .unwrap();

        assert_eq!(resolved.kind, ResolvedChangeSourceKind::Files);
        assert_eq!(resolved.files, vec!["src/lib.rs"]);
        assert!(resolved.deprecated_input_fields.is_empty());
    }

    #[test]
    fn resolve_change_source_selection_rejects_legacy_mode() {
        let error = resolve_change_source_selection(
            "test_tool",
            Some(&json!({"mode": "working_tree"})),
            false,
        )
        .unwrap_err();

        assert_eq!(
            error.message,
            "legacy change_source fields are no longer supported"
        );
    }

    #[test]
    fn resolve_change_source_selection_rejects_missing_base_for_base_kind() {
        let error = resolve_change_source_selection(
            "test_tool",
            Some(&json!({
                "change_source": {
                    "kind": "base"
                }
            })),
            false,
        )
        .unwrap_err();

        assert_eq!(
            error.message,
            "change_source.kind='base' requires non-empty change_source.base"
        );
    }

    #[test]
    fn resolve_change_source_selection_rejects_mixed_change_source_and_legacy_fields() {
        let error = resolve_change_source_selection(
            "test_tool",
            Some(&json!({
                "change_source": {"kind": "staged"},
                "staged": true
            })),
            false,
        )
        .unwrap_err();

        assert_eq!(
            error.message,
            "legacy change_source fields are no longer supported"
        );
        assert_eq!(
            error
                .details
                .as_ref()
                .and_then(|details| details.get("offending_fields"))
                .cloned(),
            Some(json!(["staged"]))
        );
    }
}
