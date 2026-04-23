use anyhow::{Context, Result};
use atlas_core::model::ContextIntent;
use atlas_core::model::{ChangeType, ChangedFile};
use atlas_core::{
    GraphHealthInput, graph_health_error_message, graph_health_error_suggestions,
    is_schema_mismatch_error, select_graph_health_error_code,
};
use atlas_parser::ParserRegistry;
use atlas_repo::{DiffTarget, changed_files, find_repo_root, hash_file};
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use serde::Serialize;

use crate::output::{OutputFormat, render_serializable};

pub(super) const DEFAULT_OUTPUT_DESCRIPTION: &str =
    "Response body format: 'toon' (default) or 'json'";

pub(super) fn parse_mcp_intent(s: &str) -> ContextIntent {
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

pub(super) fn open_store(db_path: &str) -> Result<Store> {
    Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))
}

pub(super) fn failure_category(
    db_exists: bool,
    graph_error: Option<&str>,
    build_state: Option<&str>,
    stale_index: bool,
    retrieval_unavailable: bool,
) -> &'static str {
    select_graph_health_error_code(GraphHealthInput {
        db_exists,
        graph_error,
        build_state,
        stale_index,
        retrieval_unavailable,
    })
}

pub(super) fn error_message(error_code: &str) -> &'static str {
    graph_health_error_message(error_code)
}

pub(super) fn error_suggestions(error_code: &str) -> &'static [&'static str] {
    graph_health_error_suggestions(error_code)
}

pub(super) fn graph_issue_code(error: &str) -> &'static str {
    if is_schema_mismatch_error(error) {
        "schema_mismatch"
    } else {
        "corrupt_or_inconsistent_graph_rows"
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
    let rendered = render_serializable(value, output_format)?;
    let mut response = serde_json::json!({
        "content": [{
            "type": "text",
            "text": rendered.text,
            "mimeType": rendered.actual_format.mime_type(),
        }],
        "atlas_output_format": rendered.actual_format.as_str(),
        "atlas_requested_output_format": rendered.requested_format.as_str(),
    });

    if let Some(reason) = rendered.fallback_reason {
        response["atlas_fallback_reason"] = serde_json::Value::String(reason);
    }

    Ok(response)
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
