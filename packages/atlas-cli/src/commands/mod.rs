mod changes;
mod context_cmd;
mod docs;
mod graph;
mod graph_objects;
mod history;
mod hook;
mod init_wizard;
mod insights;
mod maintenance;
mod platform;
mod postprocess;
mod query;
mod reasoning;
mod session;

pub use changes::{run_detect_changes, run_explain_change, run_impact, run_review_context};
pub use context_cmd::{run_context, run_shell};
pub use docs::run_docs_section;
pub use graph::{run_build, run_init, run_status, run_update, run_watch};
pub use graph_objects::{run_communities, run_flows};
pub use history::run_history;
pub use hook::run_hook;
pub use insights::run_insights;
pub use maintenance::{
    run_db_check, run_debug_config, run_debug_graph, run_doctor, run_embedding_config, run_migrate,
    run_purge_noncanonical, run_selfupdate,
};
pub use platform::{run_completions, run_install, run_serve, run_serve_daemon, run_serve_http};
pub use postprocess::run_postprocess;
pub use query::{run_embed, run_explain_query, run_query};
pub use reasoning::{run_analyze, run_refactor};
pub use session::run_session;

pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const GIT_HASH: &str = env!("GIT_HASH");
pub(crate) const GIT_COMMIT_DATE: &str = env!("GIT_COMMIT_DATE");
pub(crate) const GIT_DIRTY: &str = env!("GIT_DIRTY");
pub(crate) const CARGO_PROFILE: &str = env!("CARGO_PROFILE");
pub(crate) const RUSTC_VERSION: &str = env!("RUSTC_VERSION");
pub(crate) const BUILD_DATE: &str = env!("BUILD_DATE");
pub(crate) const LONG_VERSION: &str = env!("ATLAS_LONG_VERSION");

pub fn run_version(cli: &Cli) -> Result<()> {
    if cli.json {
        print_json(
            "version",
            serde_json::json!({
                "version": VERSION,
                "git_hash": GIT_HASH,
                "git_commit_date": GIT_COMMIT_DATE,
                "git_dirty": GIT_DIRTY == "true",
                "cargo_profile": CARGO_PROFILE,
                "rustc_version": RUSTC_VERSION,
                "build_date": BUILD_DATE,
                "long_version": LONG_VERSION
            }),
        )
    } else {
        println!("atlas {LONG_VERSION}");
        Ok(())
    }
}

use std::io;
use std::io::IsTerminal;

use anyhow::{Context, Result};
use atlas_contentstore::{ContentStore, IndexState};
use atlas_core::model::{ChangeType, ChangedFile};
use atlas_core::{
    BudgetPolicy, GraphReadiness, GraphReadinessInput, GraphToolRequirement, ReadinessOverride,
    ReadinessVerdict,
};
use atlas_parser::ParserRegistry;
use atlas_repo::{DiffTarget, changed_files, find_repo_root, hash_file};
use atlas_store_sqlite::{GraphBuildState, Store};
use camino::Utf8Path;

use crate::cli::Cli;

pub(crate) const MACHINE_SCHEMA_VERSION: &str = "atlas_cli.v1";

pub(crate) fn json_envelope(command: &str, data: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": MACHINE_SCHEMA_VERSION,
        "command": command,
        "data": data,
    })
}

pub(crate) fn print_json(command: &str, data: serde_json::Value) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&json_envelope(command, data))?
    );
    Ok(())
}

pub(crate) fn load_budget_policy(repo_root: &str) -> Result<BudgetPolicy> {
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    config.budget_policy()
}

pub(crate) fn load_embedding_config(
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

pub(crate) fn detect_changes_target(base: &Option<String>, staged: bool) -> DiffTarget {
    if staged {
        DiffTarget::Staged
    } else if let Some(base_ref) = base {
        DiffTarget::BaseRef(base_ref.clone())
    } else {
        DiffTarget::WorkingTree
    }
}

pub(crate) fn change_tag(change_type: ChangeType) -> &'static str {
    match change_type {
        ChangeType::Added => "A",
        ChangeType::Modified => "M",
        ChangeType::Deleted => "D",
        ChangeType::Renamed => "R",
        ChangeType::Copied => "C",
    }
}

pub(crate) fn augment_changes_with_node_counts(
    changes: &[atlas_core::model::ChangedFile],
    store: Option<&Store>,
) -> Vec<serde_json::Value> {
    changes
        .iter()
        .map(|cf| {
            let node_count = store
                .and_then(|s| s.nodes_by_file(&cf.path).ok())
                .map(|ns| ns.len());
            let mut value = serde_json::to_value(cf).unwrap_or_default();
            if let Some(count) = node_count {
                value["node_count"] = serde_json::json!(count);
            }
            value
        })
        .collect()
}

pub(crate) fn colorize(text: &str, ansi: &str) -> String {
    if io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
        format!("\x1b[{ansi}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub(crate) fn query_display_path(node: &atlas_core::Node) -> String {
    node.extra_json
        .as_object()
        .and_then(|extra| {
            extra
                .get("owner_manifest_path")
                .or_else(|| extra.get("workspace_manifest_path"))
        })
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| node.file_path.clone())
}

pub(crate) fn resolve_repo(cli: &Cli) -> Result<String> {
    let cwd = std::env::current_dir().context("cannot determine cwd")?;
    resolve_repo_with_cwd(cli, &cwd)
}

fn resolve_repo_with_cwd(cli: &Cli, cwd: &std::path::Path) -> Result<String> {
    if let Some(r) = &cli.repo {
        // Expand leading `~/` or a bare `~` to the home directory.
        let expanded = if r == "~" {
            dirs_home()?
        } else if let Some(rest) = r.strip_prefix("~/") {
            format!("{}/{rest}", dirs_home()?)
        } else {
            r.clone()
        };
        return Ok(expanded);
    }

    let cwd_utf8 =
        Utf8Path::from_path(cwd).ok_or_else(|| anyhow::anyhow!("cwd is not valid UTF-8"))?;
    Ok(find_repo_root(cwd_utf8)
        .map(|root| root.into_string())
        .unwrap_or_else(|_| cwd.to_string_lossy().into_owned()))
}

fn dirs_home() -> Result<String> {
    std::env::var("HOME")
        .or_else(|_| {
            #[allow(deprecated)]
            std::env::home_dir()
                .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))
                .map(|p| p.to_string_lossy().into_owned())
        })
        .context("cannot expand ~: HOME not set and home directory not detectable")
}

pub(crate) fn db_path(cli: &Cli, repo: &str) -> String {
    if let Some(p) = &cli.db {
        return p.clone();
    }
    atlas_engine::paths::default_db_path(repo)
}

// ── Canonical graph readiness helpers ────────────────────────────────────────

/// Returns `true` when the changed file could affect graph facts stored for it.
fn change_affects_graph(store: &Store, registry: &ParserRegistry, change: &ChangedFile) -> bool {
    let has_facts = |path: &str| {
        store
            .nodes_by_file(path)
            .map(|ns| !ns.is_empty())
            .unwrap_or(false)
    };
    registry.supports(&change.path)
        || change
            .old_path
            .as_deref()
            .is_some_and(|p| registry.supports(p))
        || has_facts(&change.path)
        || change.old_path.as_deref().is_some_and(has_facts)
}

fn graph_matches_path(store: &Store, repo_root: &Utf8Path, path: &str) -> bool {
    let indexed = store.file_hash(path).ok().flatten();
    match hash_file(&repo_root.join(path)) {
        Ok(current) => indexed.as_deref() == Some(current.as_str()),
        Err(_) => {
            store.file_hash(path).ok().flatten().is_none()
                && store
                    .nodes_by_file(path)
                    .map(|ns| ns.is_empty())
                    .unwrap_or(true)
        }
    }
}

fn file_in_graph(store: &Store, path: &str) -> bool {
    store.file_hash(path).ok().flatten().is_some()
        || store
            .nodes_by_file(path)
            .map(|ns| !ns.is_empty())
            .unwrap_or(false)
}

/// Compute graph-relevant pending working-tree changes for readiness detection.
///
/// Returns an empty `Vec` when git diff or store queries fail; readiness
/// callers treat an empty list as "not stale".
fn pending_graph_changes(store: &Store, repo_root: &str) -> Vec<String> {
    let Ok(root) = find_repo_root(Utf8Path::new(repo_root)) else {
        return Vec::new();
    };
    let Ok(changes) = changed_files(root.as_path(), &DiffTarget::WorkingTree) else {
        return Vec::new();
    };
    if changes.is_empty() {
        return Vec::new();
    }

    let registry = ParserRegistry::with_defaults();
    let mut files: Vec<String> = changes
        .iter()
        .filter(|c| change_affects_graph(store, &registry, c))
        .filter(|c| match c.change_type {
            ChangeType::Added | ChangeType::Modified => {
                !graph_matches_path(store, root.as_path(), &c.path)
            }
            ChangeType::Deleted => file_in_graph(store, &c.path),
            ChangeType::Renamed | ChangeType::Copied => {
                !graph_matches_path(store, root.as_path(), &c.path)
                    || c.old_path
                        .as_deref()
                        .is_some_and(|old| file_in_graph(store, old))
            }
        })
        .flat_map(|c| std::iter::once(c.path.clone()).chain(c.old_path.clone()))
        .collect();
    files.sort();
    files.dedup();
    files
}

/// Derive canonical [`GraphReadiness`] from an already-opened store.
///
/// This is the single shared derivation path for all CLI commands.  Call this
/// after `Store::open` succeeds, then use [`check_graph_readiness`] to gate
/// graph-backed operations.
pub(crate) fn derive_graph_readiness(
    store: &Store,
    repo_root: &str,
    db_path: &str,
) -> GraphReadiness {
    let db_exists = std::path::Path::new(db_path).exists();

    let (build_state_str, build_last_error) = match store.get_build_status(repo_root) {
        Ok(Some(bs)) => {
            let state = match bs.state {
                GraphBuildState::Building => "building",
                GraphBuildState::Built => "built",
                GraphBuildState::Degraded => "degraded",
                GraphBuildState::BuildFailed => "build_failed",
            };
            (Some(state.to_owned()), bs.last_error)
        }
        _ => (None, None),
    };

    let (stats, graph_error) = match store.stats() {
        Ok(s) => (s, None),
        Err(e) => {
            let dummy = atlas_core::GraphStats {
                file_count: 0,
                node_count: 0,
                edge_count: 0,
                nodes_by_kind: Vec::new(),
                languages: Vec::new(),
                last_indexed_at: None,
            };
            (dummy, Some(e.to_string()))
        }
    };

    let pending = pending_graph_changes(store, repo_root);

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
        pending_graph_changes: &pending,
        indexed_file_count: stats.file_count,
        graph_has_content: stats.node_count > 0 || stats.edge_count > 0 || stats.file_count > 0,
        last_indexed_at: stats.last_indexed_at.as_deref(),
        retrieval_unavailable,
    })
}

/// Derive [`GraphReadiness`] when the store could not be opened.
///
/// Used by command entry points that fail at `Store::open`; the open error is
/// included in the readiness record so blocked messages are consistent.
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
        pending_graph_changes: &[],
        indexed_file_count: 0,
        graph_has_content: false,
        last_indexed_at: None,
        retrieval_unavailable: true,
    })
}

/// Build [`ReadinessOverride`] from `--allow-stale` / `--allow-partial` flags.
pub(crate) fn readiness_overrides(allow_stale: bool, allow_partial: bool) -> ReadinessOverride {
    ReadinessOverride {
        allow_stale,
        allow_partial,
    }
}

/// Check graph readiness before a graph-backed operation.
///
/// Returns `Ok(Option<warning>)` when the tool is allowed to proceed.
/// When blocked, prints an appropriate error (JSON or text) and returns `Err`.
///
/// Callers should `eprintln!` the returned warning when it is `Some`.
pub(crate) fn check_graph_readiness(
    readiness: &GraphReadiness,
    requirement: GraphToolRequirement,
    overrides: ReadinessOverride,
    command: &str,
    cli: &Cli,
) -> Result<Option<String>> {
    let verdict = readiness.check_tool(requirement, overrides);
    match verdict {
        ReadinessVerdict::Allowed { warning, .. } => Ok(warning),
        ReadinessVerdict::Blocked {
            execution_state,
            reason,
            suggestions,
        } => {
            if cli.json {
                print_json(
                    command,
                    serde_json::json!({
                        "error": reason,
                        "execution_state": execution_state.as_str(),
                        "suggestions": suggestions,
                    }),
                )?;
            } else {
                eprintln!("Error: {reason}");
                for s in &suggestions {
                    eprintln!("  → {s}");
                }
            }
            anyhow::bail!("{reason}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Command};
    use camino::Utf8Path;
    use std::process::Command as ProcessCommand;

    const GIT_TEST_NAME: &str = "Atlas Test";
    const GIT_TEST_EMAIL: &str = "test@atlas";

    fn cli_with_repo(repo: &str) -> Cli {
        Cli {
            repo: Some(repo.to_owned()),
            db: None,
            verbose: false,
            json: false,
            command: Command::Doctor,
        }
    }

    fn cli_no_repo() -> Cli {
        Cli {
            repo: None,
            db: None,
            verbose: false,
            json: false,
            command: Command::Doctor,
        }
    }

    fn git(dir: &std::path::Path, args: &[&str]) {
        let status = ProcessCommand::new("git")
            .current_dir(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", GIT_TEST_NAME)
            .env("GIT_AUTHOR_EMAIL", GIT_TEST_EMAIL)
            .env("GIT_COMMITTER_NAME", GIT_TEST_NAME)
            .env("GIT_COMMITTER_EMAIL", GIT_TEST_EMAIL)
            .status()
            .expect("git command");
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn resolve_repo_absolute_path_returned_as_is() {
        let cli = cli_with_repo("/tmp/my-project");
        assert_eq!(resolve_repo(&cli).unwrap(), "/tmp/my-project");
    }

    #[test]
    fn resolve_repo_tilde_expands_to_home() {
        let home = std::env::var("HOME").expect("HOME must be set for this test");
        let cli = cli_with_repo("~");
        assert_eq!(resolve_repo(&cli).unwrap(), home);
    }

    #[test]
    fn resolve_repo_tilde_slash_expands_to_home_subpath() {
        let home = std::env::var("HOME").expect("HOME must be set for this test");
        let cli = cli_with_repo("~/projects/foo");
        assert_eq!(resolve_repo(&cli).unwrap(), format!("{home}/projects/foo"));
    }

    #[test]
    fn resolve_repo_no_repo_prefers_git_root() {
        let dir = tempfile::tempdir().unwrap();
        let repo_root = dir.path();
        let nested = repo_root.join("packages").join("app");
        std::fs::create_dir_all(&nested).unwrap();
        git(repo_root, &["init", "--quiet"]);

        let cli = cli_no_repo();
        let expected = Utf8Path::from_path(&repo_root.canonicalize().unwrap())
            .unwrap()
            .to_string();
        assert_eq!(resolve_repo_with_cwd(&cli, &nested).unwrap(), expected);
    }

    #[test]
    fn resolve_repo_no_repo_falls_back_to_cwd_outside_git() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("scratch");
        std::fs::create_dir_all(&nested).unwrap();

        let cli = cli_no_repo();
        let expected = nested.to_string_lossy().into_owned();
        assert_eq!(resolve_repo_with_cwd(&cli, &nested).unwrap(), expected);
    }

    #[test]
    fn resolve_repo_does_not_expand_tilde_in_middle_of_path() {
        // A path like "/home/user/~foo" must not be touched.
        let cli = cli_with_repo("/home/user/~foo");
        assert_eq!(resolve_repo(&cli).unwrap(), "/home/user/~foo");
    }
}
