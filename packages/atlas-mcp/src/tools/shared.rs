use anyhow::{Context, Result};
use atlas_core::model::ContextIntent;
use atlas_core::{
    BudgetPolicy, BudgetReport, GraphHealthInput, GraphStoreHealthClass,
    classify_graph_store_error, error_code_docs_ref, graph_health_error_message,
    graph_health_error_suggestions, select_graph_health_error_code,
};
use atlas_repo::{RepoRegistration, RepoRegistry, phase1_multi_repo_supported};
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

// Query grammar parsing lives in `atlas-review` so MCP and CLI share one
// implementation of intent classification, normalization, and rejection.
pub(crate) type McpQueryGrammar = atlas_review::QueryGrammar;

pub(crate) fn mcp_supported_query_grammar_examples() -> &'static [&'static str] {
    atlas_review::query_grammar_examples()
}

pub(crate) fn mcp_query_looks_like_unstructured_description(query: &str) -> bool {
    atlas_review::query_looks_like_unstructured_description(query)
}

pub(crate) fn parse_mcp_query_grammar(query: &str) -> McpQueryGrammar {
    atlas_review::parse_query_grammar(query)
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

/// Load the configured runtime token counter for context payload budgets.
///
/// Missing/invalid config yields the default heuristic counter; a
/// `fail_closed` tokenizer config propagates its load error. Returns the
/// full load result so callers can surface fallback metadata.
pub(super) fn load_token_counter(
    repo_root: &str,
) -> Result<atlas_engine::config::TokenCounterLoadResult> {
    let atlas_dir = atlas_engine::paths::atlas_dir(repo_root);
    let config = atlas_engine::Config::load(&atlas_dir).unwrap_or_default();
    let atlas_dir_utf8 = camino::Utf8Path::from_path(&atlas_dir)
        .ok_or_else(|| anyhow::anyhow!("atlas dir is not valid UTF-8: {}", atlas_dir.display()))?;
    config.token_counter(atlas_dir_utf8)
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
    atlas_core::kinds::normalize_kind_alias(input)
}

pub(crate) fn node_repo_id(node: &atlas_core::Node) -> Option<&str> {
    node.extra_json
        .as_object()
        .and_then(|extra| extra.get("repo_id"))
        .and_then(|value| value.as_str())
}

pub(crate) fn node_repo_json(
    node: &atlas_core::Node,
    repo_aliases: &BTreeMap<String, String>,
) -> serde_json::Value {
    let repo_id = node_repo_id(node);
    serde_json::json!({
        "repo_id": repo_id,
        "display_alias": repo_id.and_then(|id| repo_aliases.get(id)).cloned(),
    })
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

// ── Canonical graph readiness helpers ────────────────────────────────────────
//
// The readiness derivation moved to `atlas-agent-events` (ICM-D) so the CLI
// and the wake-up pack builder share one implementation; this module re-exports
// it so MCP tool handlers keep the same call sites.

pub(crate) use atlas_agent_events::graph_readiness::{
    compute_freshness_warning, derive_graph_readiness, derive_graph_readiness_open_failed,
    pending_graph_relevant_changes,
};

#[cfg(test)]
mod tests {
    use super::change_source::*;
    use super::repo_scope::*;
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
        // Direct-relative candidates must exist on disk for the normalizer.
        let temp = tempfile::tempdir().unwrap();
        let root = Utf8Path::from_path(temp.path()).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "// placeholder").unwrap();

        let resolved = resolve_change_source_selection(
            "test_tool",
            root.as_str(),
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
            "/repo",
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
            "/repo",
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
            "/repo",
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

mod change_source;
mod repo_scope;

pub(crate) use self::change_source::ResolvedChangeSourceKind;
pub(crate) use self::change_source::resolve_change_source_selection;
pub(crate) use self::repo_scope::repo_aliases_by_id;
pub(crate) use self::repo_scope::resolve_repo_scope_selection;
