//! Canonical graph-readiness derivation shared by MCP tools, the CLI, and the
//! wake-up pack builder (ICM-D).
//!
//! The readiness record is computed from the graph store's build status,
//! stats, health class, pending graph-relevant working-tree changes, and the
//! retrieval index. This module owns the derivation so every surface reports
//! the same readiness block.

use serde::Serialize;

use atlas_contentstore::{ContentStore, IndexState};
use atlas_core::{
    GraphReadiness, GraphReadinessInput, GraphStoreHealthClass,
    model::{ChangeType, ChangedFile},
};
use atlas_parser::ParserRegistry;
use atlas_repo::{DiffTarget, changed_files, find_repo_root, hash_file};
use atlas_store_sqlite::{GraphBuildState, Store};
use camino::Utf8Path;

/// Compact freshness warning attached to graph-backed results when pending
/// graph-relevant changes overlap the result's files.
#[derive(Serialize)]
pub struct FreshnessWarning {
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

/// Files changed in the working tree that are not yet reflected in the graph.
pub fn pending_graph_relevant_changes(repo_root: &str, db_path: &str) -> Option<Vec<String>> {
    let repo_root_path = find_repo_root(Utf8Path::new(repo_root)).ok()?;
    let changes = changed_files(repo_root_path.as_path(), &DiffTarget::WorkingTree).ok()?;
    if changes.is_empty() {
        return Some(Vec::new());
    }

    let store = Store::open(db_path).ok()?;
    let mut registry = ParserRegistry::with_defaults();
    // Honor [parsers.external] so external-language files count as
    // graph-relevant for hook-triggered refresh decisions.
    if let Ok(config) =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root_path.as_str()))
        && let Err(error) = registry.register_externals(&config.parsers.external)
    {
        tracing::warn!("external parsers unavailable: {error:#}");
    }

    Some(unique_sorted_paths(
        changes
            .iter()
            .filter(|change| {
                change_is_pending_in_graph(&store, &registry, repo_root_path.as_path(), change)
            })
            .flat_map(|change| std::iter::once(change.path.clone()).chain(change.old_path.clone())),
    ))
}

/// Compact freshness warning when pending graph-relevant changes overlap the
/// files backing a graph-backed result.
pub fn compute_freshness_warning(
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
            "run update_graph to refresh the graph",
            "run detect_changes to inspect pending graph-relevant files",
        ],
    })
}

/// Derive canonical [`GraphReadiness`] from an already-open store.
///
/// This is the shared readiness derivation path for all MCP tool handlers, the
/// CLI, and wake-up packs. Call this after `Store::open` succeeds; use the
/// result to gate graph-backed operations via [`GraphReadiness::check_tool`].
pub fn derive_graph_readiness(store: &Store, repo_root: &str, db_path: &str) -> GraphReadiness {
    let db_exists = std::path::Path::new(db_path).exists();

    let mut graph_error = None;
    let (build_state_str, build_last_error, recovery_mode, quarantine_path) =
        match store.get_build_status(repo_root) {
            Ok(Some(bs)) => {
                let state = match bs.state {
                    GraphBuildState::Building => "building",
                    GraphBuildState::Built => "built",
                    GraphBuildState::Degraded => "degraded",
                    GraphBuildState::BuildFailed => "build_failed",
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
pub fn derive_graph_readiness_open_failed(
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
