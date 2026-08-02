use std::fmt::Display;
use std::fs;

use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter};
use atlas_contentstore::{ContentStore, IndexState};
use atlas_core::GraphStats;
use atlas_core::model::{ChangeType, ChangedFile};
use atlas_core::{
    GraphHealthInput, GraphReadiness, GraphReadinessInput, graph_health_error_message,
    graph_health_error_suggestions, select_graph_health_error_code,
};
use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_parser::ParserRegistry;
use atlas_repo::{
    RepoRegistration, RepoRegistry, RepoRelationshipKind, changed_files, find_repo_root, hash_file,
    phase1_multi_repo_supported, stable_repo_fingerprint, stable_repo_id,
};
use atlas_session::SessionStore;
use atlas_store_sqlite::{BuildFinishStats, Store};
use camino::Utf8Path;
use tracing::debug;

use crate::cli::{Cli, Command};

use super::{
    augment_changes_with_node_counts, change_tag, db_path, detect_changes_target, print_json,
    public_graph_stats, resolve_repo,
};

struct StatusPayloadContext<'a> {
    repo: &'a str,
    db_path: &'a str,
    stats: &'a GraphStats,
    config: &'a atlas_engine::Config,
    base: &'a Option<String>,
    staged: bool,
    changes: &'a [ChangedFile],
    store: Option<&'a Store>,
}

struct StatusDiagnostics {
    ok: bool,
    error_code: &'static str,
    graph_built: bool,
    build_state: Option<String>,
    build_last_error: Option<String>,
    graph_query_error: Option<String>,
    pending_graph_changes: Vec<String>,
    retrieval_index: serde_json::Value,
    execution_state: atlas_core::GraphExecutionState,
}

const MAX_MULTI_REPO_SELECTION: usize = 32;

#[derive(Default)]
struct MultiRepoBudgetAggregate {
    selected_repo_count: usize,
    processed_repo_count: usize,
    failed_repo_count: usize,
    skipped_repo_count: usize,
    excluded_manual_repo_count: usize,
    budget_hit_repo_count: usize,
    files_accepted: usize,
    files_skipped_by_byte_budget: usize,
    bytes_accepted: u64,
    bytes_skipped: u64,
}

fn excluded_manual_repo_count(registry: &RepoRegistry) -> usize {
    registry
        .registrations
        .iter()
        .filter(|entry| entry.enabled)
        .filter(|entry| entry.relationship.kind == RepoRelationshipKind::Manual)
        .count()
}

fn print_summary_value(label: &str, value: impl Display) {
    println!("  {label:<20}: {value}");
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

fn graph_relevant_changed_files(
    store: &Store,
    repo_root: &Utf8Path,
    changes: &[ChangedFile],
) -> Vec<String> {
    let registry = ParserRegistry::with_defaults();
    let mut files: Vec<String> = changes
        .iter()
        .filter(|change| change_is_pending_in_graph(store, &registry, repo_root, change))
        .flat_map(|change| std::iter::once(change.path.clone()).chain(change.old_path.clone()))
        .collect();
    files.sort();
    files.dedup();
    files
}

fn retrieval_index_value(repo: &str, db_path: &str) -> serde_json::Value {
    let content_db_path = atlas_engine::paths::content_db_path(db_path);
    let content_db_path_string = content_db_path.clone();
    match ContentStore::open(&content_db_path) {
        Ok(mut store) => {
            let _ = store.migrate();
            match store.get_index_status(repo) {
                Ok(Some(status)) => {
                    let searchable = status.state == IndexState::Indexed;
                    let state = match status.state {
                        IndexState::Indexed => "indexed",
                        IndexState::Indexing => "indexing",
                        IndexState::IndexFailed => "index_failed",
                    };
                    serde_json::json!({
                        "available": true,
                        "searchable": searchable,
                        "state": state,
                        "files_discovered": status.files_discovered,
                        "files_indexed": status.files_indexed,
                        "chunks_written": status.chunks_written,
                        "chunks_reused": status.chunks_reused,
                        "last_indexed_at": status.last_indexed_at,
                        "last_error": status.last_error,
                        "content_db_path": content_db_path_string,
                    })
                }
                Ok(None) => serde_json::json!({
                    "available": false,
                    "searchable": false,
                    "state": serde_json::Value::Null,
                    "files_discovered": 0,
                    "files_indexed": 0,
                    "chunks_written": 0,
                    "chunks_reused": 0,
                    "last_indexed_at": serde_json::Value::Null,
                    "last_error": "content store has no retrieval index state for this repo",
                    "content_db_path": content_db_path_string,
                }),
                Err(error) => serde_json::json!({
                    "available": false,
                    "searchable": false,
                    "state": serde_json::Value::Null,
                    "files_discovered": 0,
                    "files_indexed": 0,
                    "chunks_written": 0,
                    "chunks_reused": 0,
                    "last_indexed_at": serde_json::Value::Null,
                    "last_error": error.to_string(),
                    "content_db_path": content_db_path_string,
                }),
            }
        }
        Err(error) => serde_json::json!({
            "available": false,
            "searchable": false,
            "state": serde_json::Value::Null,
            "files_discovered": 0,
            "files_indexed": 0,
            "chunks_written": 0,
            "chunks_reused": 0,
            "last_indexed_at": serde_json::Value::Null,
            "last_error": error.to_string(),
            "content_db_path": content_db_path_string,
        }),
    }
}

fn collect_status_diagnostics(ctx: &StatusPayloadContext<'_>) -> StatusDiagnostics {
    let mut graph_query_error: Option<String> = None;
    let build_status = match ctx.store.map(|store| store.get_build_status(ctx.repo)) {
        Some(Ok(status)) => status,
        Some(Err(error)) => {
            graph_query_error = Some(error.to_string());
            None
        }
        None => None,
    };
    let build_state = build_status.as_ref().map(|bs| match bs.state {
        atlas_store_sqlite::GraphBuildState::Building => "building",
        atlas_store_sqlite::GraphBuildState::Built => "built",
        atlas_store_sqlite::GraphBuildState::Degraded => "degraded",
        atlas_store_sqlite::GraphBuildState::BuildFailed => "build_failed",
    });
    let graph_built = build_state == Some("built")
        || (build_state.is_none()
            && graph_query_error.is_none()
            && (ctx.stats.node_count > 0 || ctx.stats.edge_count > 0 || ctx.stats.file_count > 0));
    let pending_graph_changes = ctx
        .store
        .map(|store| graph_relevant_changed_files(store, Utf8Path::new(ctx.repo), ctx.changes))
        .unwrap_or_default();
    let stale_index = graph_built && !pending_graph_changes.is_empty();
    let retrieval_index = retrieval_index_value(ctx.repo, ctx.db_path);
    let retrieval_unavailable = graph_built
        && (!retrieval_index["available"].as_bool().unwrap_or(false)
            || !retrieval_index["searchable"].as_bool().unwrap_or(false)
            || retrieval_index["state"].as_str() != Some("indexed"));
    let error_code = select_graph_health_error_code(GraphHealthInput {
        db_exists: true,
        graph_error: graph_query_error.as_deref(),
        build_state,
        stale_index,
        retrieval_unavailable,
    });

    // Derive canonical execution state via GraphReadiness.
    let graph_has_content =
        ctx.stats.node_count > 0 || ctx.stats.edge_count > 0 || ctx.stats.file_count > 0;
    let readiness = GraphReadiness::derive(GraphReadinessInput {
        repo_root: ctx.repo,
        db_path: ctx.db_path,
        db_exists: true,
        db_open_error: None,
        build_state,
        build_last_error: build_status
            .as_ref()
            .and_then(|bs| bs.last_error.as_deref()),
        graph_error: graph_query_error.as_deref(),
        pending_graph_changes: &pending_graph_changes,
        indexed_file_count: ctx.stats.file_count,
        graph_has_content,
        last_indexed_at: ctx.stats.last_indexed_at.as_deref(),
        retrieval_unavailable,
    });

    StatusDiagnostics {
        ok: error_code == "none" && graph_built,
        error_code,
        graph_built,
        build_state: build_state.map(str::to_owned),
        build_last_error: build_status.and_then(|status| status.last_error),
        graph_query_error,
        pending_graph_changes,
        retrieval_index,
        execution_state: readiness.execution_state,
    }
}

fn status_payload(ctx: StatusPayloadContext<'_>) -> serde_json::Value {
    let build_status = ctx
        .store
        .and_then(|s| s.get_build_status(ctx.repo).ok().flatten());
    let diagnostics = collect_status_diagnostics(&ctx);
    let build_state_val = build_status.as_ref().map(|bs| {
        let state_str = match bs.state {
            atlas_store_sqlite::GraphBuildState::Building => "building",
            atlas_store_sqlite::GraphBuildState::Built => "built",
            atlas_store_sqlite::GraphBuildState::Degraded => "degraded",
            atlas_store_sqlite::GraphBuildState::BuildFailed => "build_failed",
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
    });
    let repo_id = stable_repo_id(Utf8Path::new(ctx.repo));
    let repo_fingerprint = stable_repo_fingerprint(Utf8Path::new(ctx.repo), None);
    serde_json::json!({
        "ok": diagnostics.ok,
        "error_code": diagnostics.error_code,
        "message": graph_health_error_message(diagnostics.error_code),
        "suggestions": graph_health_error_suggestions(diagnostics.error_code),
        "repo_root": ctx.repo,
        "repo_provenance": {
            "repo_id": repo_id,
            "repo_fingerprint": repo_fingerprint,
            "repo_root": ctx.repo,
        },
        "db_path": ctx.db_path,
        "mcp": {
            "worker_threads": ctx.config.mcp_worker_threads(),
            "tool_timeout_ms": ctx.config.mcp_tool_timeout_ms(),
            "tool_timeout_ms_by_tool": ctx.config.mcp_tool_timeout_ms_by_tool(),
        },
        "diff_target": {
            "base": ctx.base,
            "staged": ctx.staged,
            "kind": if ctx.staged { "staged" } else if ctx.base.is_some() { "base_ref" } else { "working_tree" },
        },
        "indexed_file_count": ctx.stats.file_count,
        "node_count": ctx.stats.node_count,
        "edge_count": ctx.stats.edge_count,
        "nodes_by_kind": ctx.stats.nodes_by_kind,
        "languages": ctx.stats.languages,
        "last_indexed_at": ctx.stats.last_indexed_at,
        "graph_built": diagnostics.graph_built,
        "build_state": diagnostics.build_state,
        "build_last_error": diagnostics.build_last_error,
        "graph_query_error": diagnostics.graph_query_error,
        "stale_index": !diagnostics.pending_graph_changes.is_empty(),
        "pending_graph_change_count": diagnostics.pending_graph_changes.len(),
        "pending_graph_changes": diagnostics.pending_graph_changes,
        "execution_state": diagnostics.execution_state.as_str(),
        "retrieval_index": diagnostics.retrieval_index,
        "changed_file_count": ctx.changes.len(),
        "changed_files": augment_changes_with_node_counts(ctx.changes, ctx.store),
        "build_status": build_state_val,
    })
}

pub fn run_init(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    debug!(repo_root = %repo, "init: resolved repo root");
    let atlas_dir = atlas_engine::paths::atlas_dir(&repo);
    fs::create_dir_all(&atlas_dir)
        .with_context(|| format!("cannot create {}", atlas_dir.display()))?;
    debug!(atlas_dir = %atlas_dir.display(), "init: ensured atlas directory");

    let db_path = db_path(cli, &repo);
    Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    debug!(db_path = %db_path, "init: opened graph database");

    let content_db_path = atlas_engine::paths::content_db_path(&db_path);
    let mut content_store = ContentStore::open(&content_db_path)
        .with_context(|| format!("cannot open content store at {content_db_path}"))?;
    content_store
        .migrate()
        .with_context(|| format!("cannot migrate content store at {content_db_path}"))?;
    debug!(content_db_path = %content_db_path, "init: opened content store");

    let session_db_path = atlas_engine::paths::session_db_path(&db_path);
    SessionStore::open(&session_db_path)
        .with_context(|| format!("cannot open session store at {session_db_path}"))?;
    debug!(session_db_path = %session_db_path, "init: opened session store");

    let config_path = atlas_engine::paths::config_path(&repo);
    let profile = match &cli.command {
        Command::Init { profile } => match profile.as_str() {
            "minimal" => atlas_engine::config::ConfigTemplateProfile::Minimal,
            "standard" => atlas_engine::config::ConfigTemplateProfile::Standard,
            "full" => atlas_engine::config::ConfigTemplateProfile::Full,
            other => anyhow::bail!("unsupported init profile: {other}"),
        },
        _ => unreachable!(),
    };
    let config_created = atlas_engine::Config::write_template(&atlas_dir, profile)
        .with_context(|| format!("cannot write config to {}", config_path.display()))?;
    debug!(config_path = %config_path.display(), config_created, profile = profile.as_str(), "init: prepared config template");

    let repo_registry = super::repo::bootstrap_and_save_registry(Utf8Path::new(&repo))
        .context("cannot bootstrap repo registry")?;
    let repo_registry_path = atlas_repo::registry_path(Utf8Path::new(&repo));
    if let Ok(mut store) = Store::open(&db_path) {
        atlas_engine::refresh_repo_registry_graph(&mut store, &repo_registry)
            .context("cannot refresh synthetic repo registry graph")?;
    }
    debug!(registry_path = %repo_registry_path, registrations = repo_registry.registrations.len(), "init: prepared repo registry");

    if cli.json {
        print_json(
            "init",
            serde_json::json!({
                "atlas_dir": atlas_dir.display().to_string(),
                "db_path": db_path,
                "content_db_path": content_db_path,
                "session_db_path": session_db_path,
                "config_path": config_path.display().to_string(),
                "config_created": config_created,
                "config_profile": profile.as_str(),
                "repo_registry_path": repo_registry_path.to_string(),
                "repo_registrations": repo_registry.registrations.len(),
                "repo_registry_warnings": repo_registry.warnings,
            }),
        )?;
    } else if super::init_wizard::should_run(cli.json) {
        let repo_root = std::path::Path::new(&repo);
        super::init_wizard::run(repo_root)?;
    } else {
        println!("Initialized atlas in {}", atlas_dir.display());
        println!("Database: {db_path}");
        println!("Content : {content_db_path}");
        println!("Session : {session_db_path}");
        println!("Registry: {repo_registry_path}");
        if config_created {
            println!("Config  : {} ({})", config_path.display(), profile.as_str());
        }
    }
    Ok(())
}

pub fn run_status(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let repo_root = repo_root_path.as_path();
    let db_path = db_path(cli, &repo);

    let (base, staged) = match &cli.command {
        Command::Status { base, staged } => (base.clone(), *staged),
        _ => unreachable!(),
    };

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
    let source_repo_id = stable_repo_id(repo_root);
    let stats = public_graph_stats(&store, &source_repo_id).context("cannot read stats")?;
    let changes = changed_files(repo_root, &detect_changes_target(&base, staged))
        .context("cannot detect changed files")?;

    if cli.json {
        print_json(
            "status",
            status_payload(StatusPayloadContext {
                repo: &repo,
                db_path: &db_path,
                stats: &stats,
                config: &config,
                base: &base,
                staged,
                changes: &changes,
                store: Some(&store),
            }),
        )?;
    } else {
        println!("Atlas scope: {repo}");
        println!("Git root   : {repo_root}");
        println!("Database  : {db_path}");
        println!(
            "MCP serve : workers={} default_timeout_ms={} per_tool_overrides={}",
            config.mcp_worker_threads(),
            config.mcp_tool_timeout_ms(),
            config.mcp_tool_timeout_ms_by_tool().len()
        );
        println!("Files     : {}", stats.file_count);
        println!("Nodes     : {}", stats.node_count);
        println!("Edges     : {}", stats.edge_count);
        if !stats.languages.is_empty() {
            println!("Languages : {}", stats.languages.join(", "));
        }
        if !stats.nodes_by_kind.is_empty() {
            println!("Nodes by kind:");
            for (kind, count) in &stats.nodes_by_kind {
                println!("  {kind:<14} {count}");
            }
        }
        if let Some(ts) = &stats.last_indexed_at {
            println!("Last indexed: {ts}");
        }
        if let Ok(Some(bs)) = store.get_build_status(&repo) {
            let state_str = match bs.state {
                atlas_store_sqlite::GraphBuildState::Building => "building (interrupted?)",
                atlas_store_sqlite::GraphBuildState::Built => "built",
                atlas_store_sqlite::GraphBuildState::Degraded => "degraded",
                atlas_store_sqlite::GraphBuildState::BuildFailed => "build_failed",
            };
            println!("Build state : {state_str}");
            if let Some(err) = &bs.last_error {
                println!("Build error : {err}");
            }
        }
        if base.is_some() || staged || !changes.is_empty() {
            println!("Changed files: {}", changes.len());
            for cf in &changes {
                let node_info = store
                    .nodes_by_file(&cf.path)
                    .ok()
                    .map(|nodes| format!(" [{} nodes]", nodes.len()))
                    .unwrap_or_default();
                if let Some(old) = &cf.old_path {
                    println!(
                        "  {}  {old} -> {}{node_info}",
                        change_tag(cf.change_type),
                        cf.path
                    );
                } else {
                    println!("  {}  {}{node_info}", change_tag(cf.change_type), cf.path);
                }
            }
        }
    }
    Ok(())
}

pub fn run_build(cli: &Cli) -> Result<()> {
    let (fail_fast, dry_run, selected_repo_id, all_repos) = match &cli.command {
        Command::Build {
            fail_fast,
            dry_run,
            repo_id,
            all_repos,
        } => (*fail_fast, *dry_run, repo_id.clone(), *all_repos),
        _ => unreachable!(),
    };
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("build");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let db_path = db_path(cli, &repo);

        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
        let build_budget = config.build_run_budget()?;

        if all_repos || selected_repo_id.is_some() {
            return run_registered_builds(
                cli,
                Utf8Path::new(&repo),
                &db_path,
                &config,
                fail_fast,
                dry_run,
                selected_repo_id.as_deref(),
                all_repos,
            );
        }

        let source_repo_id = stable_repo_id(repo_root_path.as_path());

        // Record lifecycle: building.
        if !dry_run && let Ok(store) = Store::open(&db_path) {
            let _ = store.begin_build_for_repo(&source_repo_id, repo_root_path.as_str());
        }

        let build_result = build_graph(
            repo_root_path.as_path(),
            &db_path,
            &BuildOptions {
                fail_fast,
                dry_run,
                batch_size: config.parse_batch_size(),
                budget: build_budget,
                source_repo_id: Some(source_repo_id.clone()),
                namespace_qualified_names: false,
            },
        );

        // Record lifecycle: built or build_failed.
        if !dry_run && let Ok(store) = Store::open(&db_path) {
            match &build_result {
                Ok(s) => {
                    let state =
                        if matches!(s.budget.budget_status, atlas_core::BudgetStatus::Blocked) {
                            atlas_store_sqlite::GraphBuildState::BuildFailed
                        } else if s.is_degraded() {
                            atlas_store_sqlite::GraphBuildState::Degraded
                        } else {
                            atlas_store_sqlite::GraphBuildState::Built
                        };
                    let _ = store.finish_build_for_repo(
                        &source_repo_id,
                        repo_root_path.as_str(),
                        BuildFinishStats {
                            state,
                            files_discovered: s.scanned as i64,
                            files_processed: s.parsed as i64,
                            files_accepted: s.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: s
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: s.parse_errors as i64,
                            bytes_accepted: s.budget_counters.bytes_accepted as i64,
                            bytes_skipped: s.budget_counters.bytes_skipped as i64,
                            nodes_written: s.nodes_inserted as i64,
                            edges_written: s.edges_inserted as i64,
                            budget_stop_reason: s.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                }
                Err(e) => {
                    let _ = store.fail_build_for_repo(
                        &source_repo_id,
                        repo_root_path.as_str(),
                        &e.to_string(),
                    );
                }
            }
        }

        let summary = build_result?;

        if cli.json {
            print_json(
                "build",
                serde_json::json!({
                    "dry_run": dry_run,
                    "scanned": summary.scanned,
                    "skipped_unsupported": summary.skipped_unsupported,
                    "skipped_unchanged": summary.skipped_unchanged,
                    "parsed": summary.parsed,
                    "parse_errors": summary.parse_errors,
                    "chunk_upsert_failures": summary.chunk_upsert_failures,
                    "call_target_reconcile_failures": summary.call_target_reconcile_failures,
                    "nodes_inserted": summary.nodes_inserted,
                    "edges_inserted": summary.edges_inserted,
                    "warnings": summary.warnings,
                    "budget": summary.budget,
                    "budget_counters": summary.budget_counters,
                    "elapsed_ms": summary.elapsed_ms,
                    "nodes_per_sec": if summary.elapsed_ms > 0 {
                        (summary.nodes_inserted as f64 / summary.elapsed_ms as f64 * 1000.0).round() as u64
                    } else { summary.nodes_inserted as u64 },
                }),
            )?;
        } else {
            let nodes_per_sec = if summary.elapsed_ms > 0 {
                format!(
                    "{:.0} nodes/s",
                    summary.nodes_inserted as f64 / summary.elapsed_ms as f64 * 1000.0
                )
            } else {
                String::from("—")
            };
            println!(
                "{} ({:.2}s, {nodes_per_sec})",
                if dry_run {
                    "Build dry run complete"
                } else {
                    "Build complete"
                },
                summary.elapsed_ms as f64 / 1000.0
            );
            print_summary_value("Scanned", summary.scanned);
            print_summary_value("Unsupported skipped", summary.skipped_unsupported);
            print_summary_value("Unchanged skipped", summary.skipped_unchanged);
            print_summary_value("Parsed", summary.parsed);
            if summary.parse_errors > 0 {
                print_summary_value("Errors", summary.parse_errors);
            }
            if summary.chunk_upsert_failures > 0 {
                print_summary_value("Chunk indexing failures", summary.chunk_upsert_failures);
            }
            if summary.call_target_reconcile_failures > 0 {
                print_summary_value(
                    "Call-target reconcile failures",
                    summary.call_target_reconcile_failures,
                );
            }
            print_summary_value("Files accepted", summary.budget_counters.files_accepted);
            if summary.budget_counters.files_skipped_by_byte_budget > 0 {
                print_summary_value(
                    "Byte-budget skipped",
                    summary.budget_counters.files_skipped_by_byte_budget,
                );
            }
            print_summary_value("Nodes inserted", summary.nodes_inserted);
            print_summary_value("Edges inserted", summary.edges_inserted);
            if let Some(reason) = &summary.budget_counters.budget_stop_reason {
                print_summary_value("Budget stop reason", reason);
            }
            for warning in &summary.warnings {
                print_summary_value("Warning", warning);
            }
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("build", result.is_ok());
    }
    result
}

fn enabled_registration_targets(
    registry_root: &Utf8Path,
    selected_repo_id: Option<&str>,
    all_repos: bool,
) -> Result<(Vec<RepoRegistration>, usize)> {
    let registry = RepoRegistry::load(registry_root)
        .with_context(|| "repo registry missing; run `atlas init` or `atlas repo sync` first")?;
    let excluded_manual = if all_repos {
        excluded_manual_repo_count(&registry)
    } else {
        0
    };
    let mut registrations: Vec<RepoRegistration> = registry
        .registrations
        .into_iter()
        .filter(|entry| entry.enabled)
        .collect();
    if let Some(repo_id) = selected_repo_id {
        registrations.retain(|entry| entry.repo_id == repo_id);
        anyhow::ensure!(
            !registrations.is_empty(),
            "enabled repo id '{repo_id}' is not registered"
        );
    } else if all_repos {
        registrations.retain(|entry| phase1_multi_repo_supported(entry.relationship.kind));
        anyhow::ensure!(
            registrations.len() <= MAX_MULTI_REPO_SELECTION,
            "all_repos scope exceeds max supported repo fan-out ({MAX_MULTI_REPO_SELECTION})"
        );
    } else {
        registrations.retain(|entry| entry.relationship.kind == RepoRelationshipKind::Root);
    }
    Ok((registrations, excluded_manual))
}

fn build_state_for_summary(
    summary: &atlas_engine::BuildSummary,
) -> atlas_store_sqlite::GraphBuildState {
    if matches!(
        summary.budget.budget_status,
        atlas_core::BudgetStatus::Blocked
    ) {
        atlas_store_sqlite::GraphBuildState::BuildFailed
    } else if summary.is_degraded() {
        atlas_store_sqlite::GraphBuildState::Degraded
    } else {
        atlas_store_sqlite::GraphBuildState::Built
    }
}

#[allow(clippy::too_many_arguments)]
fn run_registered_builds(
    cli: &Cli,
    registry_root: &Utf8Path,
    db_path: &str,
    config: &atlas_engine::Config,
    fail_fast: bool,
    dry_run: bool,
    selected_repo_id: Option<&str>,
    all_repos: bool,
) -> Result<()> {
    let (registrations, excluded_manual) =
        enabled_registration_targets(registry_root, selected_repo_id, all_repos)?;
    let build_budget = config.build_run_budget()?;
    let mut results = Vec::new();
    let mut aggregate = MultiRepoBudgetAggregate {
        selected_repo_count: registrations.len(),
        excluded_manual_repo_count: excluded_manual,
        ..MultiRepoBudgetAggregate::default()
    };

    for registration in registrations {
        if !registration.root.exists() {
            aggregate.failed_repo_count += 1;
            aggregate.skipped_repo_count += 1;
            results.push(serde_json::json!({
                "repo_id": registration.repo_id,
                "display_alias": registration.display_alias,
                "status": "skipped",
                "ok": false,
                "error": "repo root missing",
            }));
            continue;
        }
        if !dry_run && let Ok(store) = Store::open(db_path) {
            let _ = store.begin_build_for_repo(&registration.repo_id, registration.root.as_str());
        }
        let result = build_graph(
            registration.root.as_path(),
            db_path,
            &BuildOptions {
                fail_fast,
                dry_run,
                batch_size: config.parse_batch_size(),
                budget: build_budget,
                source_repo_id: Some(registration.repo_id.clone()),
                namespace_qualified_names: registration.relationship.kind
                    != RepoRelationshipKind::Root,
            },
        );
        match result {
            Ok(summary) => {
                if !dry_run && let Ok(store) = Store::open(db_path) {
                    let _ = store.finish_build_for_repo(
                        &registration.repo_id,
                        registration.root.as_str(),
                        BuildFinishStats {
                            state: build_state_for_summary(&summary),
                            files_discovered: summary.scanned as i64,
                            files_processed: summary.parsed as i64,
                            files_accepted: summary.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: summary
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: summary.parse_errors as i64,
                            bytes_accepted: summary.budget_counters.bytes_accepted as i64,
                            bytes_skipped: summary.budget_counters.bytes_skipped as i64,
                            nodes_written: summary.nodes_inserted as i64,
                            edges_written: summary.edges_inserted as i64,
                            budget_stop_reason: summary.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                }
                aggregate.processed_repo_count += 1;
                if summary.budget.budget_hit {
                    aggregate.budget_hit_repo_count += 1;
                }
                aggregate.files_accepted += summary.budget_counters.files_accepted;
                aggregate.files_skipped_by_byte_budget +=
                    summary.budget_counters.files_skipped_by_byte_budget;
                aggregate.bytes_accepted += summary.budget_counters.bytes_accepted;
                aggregate.bytes_skipped += summary.budget_counters.bytes_skipped;
                results.push(serde_json::json!({
                    "repo_id": registration.repo_id,
                    "display_alias": registration.display_alias,
                    "status": if summary.is_degraded() { "degraded" } else { "ok" },
                    "ok": true,
                    "parsed": summary.parsed,
                    "nodes_inserted": summary.nodes_inserted,
                    "edges_inserted": summary.edges_inserted,
                    "budget_status": summary.budget.budget_status,
                    "budget_hit": summary.budget.budget_hit,
                    "files_accepted": summary.budget_counters.files_accepted,
                    "files_skipped_by_byte_budget": summary.budget_counters.files_skipped_by_byte_budget,
                    "bytes_accepted": summary.budget_counters.bytes_accepted,
                    "bytes_skipped": summary.budget_counters.bytes_skipped,
                    "budget_stop_reason": summary.budget_counters.budget_stop_reason,
                    "warnings": summary.warnings,
                }));
            }
            Err(error) => {
                aggregate.failed_repo_count += 1;
                if !dry_run && let Ok(store) = Store::open(db_path) {
                    let _ = store.fail_build_for_repo(
                        &registration.repo_id,
                        registration.root.as_str(),
                        &error.to_string(),
                    );
                }
                results.push(serde_json::json!({
                    "repo_id": registration.repo_id,
                    "display_alias": registration.display_alias,
                    "status": "error",
                    "ok": false,
                    "error": error.to_string(),
                }));
            }
        }
    }

    if cli.json {
        print_json(
            "build",
            serde_json::json!({
                "dry_run": dry_run,
                "partial_success": aggregate.failed_repo_count > 0,
                "repo_scope": {
                    "selected_repo_count": aggregate.selected_repo_count,
                    "processed_repo_count": aggregate.processed_repo_count,
                    "failed_repo_count": aggregate.failed_repo_count,
                    "skipped_repo_count": aggregate.skipped_repo_count,
                    "excluded_manual_repo_count": aggregate.excluded_manual_repo_count,
                },
                "budget_summary": {
                    "budget_hit_repo_count": aggregate.budget_hit_repo_count,
                    "files_accepted": aggregate.files_accepted,
                    "files_skipped_by_byte_budget": aggregate.files_skipped_by_byte_budget,
                    "bytes_accepted": aggregate.bytes_accepted,
                    "bytes_skipped": aggregate.bytes_skipped,
                },
                "repos": results,
            }),
        )
    } else {
        println!(
            "Build complete for {} repo(s); processed={} failures={} skipped={}",
            aggregate.selected_repo_count,
            aggregate.processed_repo_count,
            aggregate.failed_repo_count,
            aggregate.skipped_repo_count,
        );
        if aggregate.excluded_manual_repo_count > 0 {
            println!(
                "Phase-1 rollout: excluded {} manual repo(s) from --all-repos. Use --repo-id to target them explicitly.",
                aggregate.excluded_manual_repo_count
            );
        }
        println!(
            "Budget summary: accepted_files={} skipped_files={} accepted_bytes={} skipped_bytes={} budget_hit_repos={}",
            aggregate.files_accepted,
            aggregate.files_skipped_by_byte_budget,
            aggregate.bytes_accepted,
            aggregate.bytes_skipped,
            aggregate.budget_hit_repo_count,
        );
        for result in results {
            println!("  {}", result);
        }
        Ok(())
    }
}

fn update_state_for_summary(
    summary: &atlas_engine::UpdateSummary,
) -> atlas_store_sqlite::GraphBuildState {
    if matches!(
        summary.budget.budget_status,
        atlas_core::BudgetStatus::Blocked
    ) {
        atlas_store_sqlite::GraphBuildState::BuildFailed
    } else if summary.is_degraded() {
        atlas_store_sqlite::GraphBuildState::Degraded
    } else {
        atlas_store_sqlite::GraphBuildState::Built
    }
}

#[allow(clippy::too_many_arguments)]
fn run_registered_updates(
    cli: &Cli,
    registry_root: &Utf8Path,
    db_path: &str,
    config: &atlas_engine::Config,
    fail_fast: bool,
    dry_run: bool,
    selected_repo_id: Option<&str>,
    all_repos: bool,
    affected_repos: bool,
) -> Result<()> {
    let (registrations, excluded_manual) =
        enabled_registration_targets(registry_root, selected_repo_id, all_repos || affected_repos)?;
    let build_budget = config.build_run_budget()?;
    let mut results = Vec::new();
    let mut aggregate = MultiRepoBudgetAggregate {
        selected_repo_count: registrations.len(),
        excluded_manual_repo_count: excluded_manual,
        ..MultiRepoBudgetAggregate::default()
    };

    for registration in registrations {
        if !registration.root.exists() {
            aggregate.failed_repo_count += 1;
            aggregate.skipped_repo_count += 1;
            results.push(serde_json::json!({
                "repo_id": registration.repo_id,
                "display_alias": registration.display_alias,
                "status": "skipped",
                "ok": false,
                "error": "repo root missing",
            }));
            continue;
        }
        let target = match &cli.command {
            Command::Update {
                base,
                staged,
                files,
                ..
            } => {
                if !files.is_empty() && selected_repo_id.is_some() {
                    UpdateTarget::Files(files.clone())
                } else if *staged {
                    UpdateTarget::Staged
                } else if let Some(base_ref) = base {
                    UpdateTarget::BaseRef(base_ref.clone())
                } else {
                    UpdateTarget::WorkingTree
                }
            }
            _ => UpdateTarget::WorkingTree,
        };

        if affected_repos {
            let diff_target = match &target {
                UpdateTarget::Staged => atlas_repo::DiffTarget::Staged,
                UpdateTarget::BaseRef(base) => atlas_repo::DiffTarget::BaseRef(base.clone()),
                _ => atlas_repo::DiffTarget::WorkingTree,
            };
            let changes = match changed_files(registration.root.as_path(), &diff_target) {
                Ok(changes) => changes,
                Err(error) => {
                    aggregate.failed_repo_count += 1;
                    aggregate.skipped_repo_count += 1;
                    results.push(serde_json::json!({
                        "repo_id": registration.repo_id,
                        "display_alias": registration.display_alias,
                        "status": "skipped",
                        "ok": false,
                        "error": error.to_string(),
                    }));
                    continue;
                }
            };
            if changes.is_empty() {
                aggregate.skipped_repo_count += 1;
                results.push(serde_json::json!({
                    "repo_id": registration.repo_id,
                    "display_alias": registration.display_alias,
                    "status": "skipped",
                    "ok": true,
                    "skipped": "no_changes",
                }));
                continue;
            }
        }

        if !dry_run && let Ok(store) = Store::open(db_path) {
            let _ = store.begin_build_for_repo(&registration.repo_id, registration.root.as_str());
        }
        let result = update_graph(
            registration.root.as_path(),
            db_path,
            &UpdateOptions {
                fail_fast,
                dry_run,
                batch_size: config.parse_batch_size(),
                target,
                budget: build_budget,
                source_repo_id: Some(registration.repo_id.clone()),
                namespace_qualified_names: registration.relationship.kind
                    != RepoRelationshipKind::Root,
            },
        );
        match result {
            Ok(summary) => {
                if !dry_run && let Ok(store) = Store::open(db_path) {
                    let _ = store.finish_build_for_repo(
                        &registration.repo_id,
                        registration.root.as_str(),
                        BuildFinishStats {
                            state: update_state_for_summary(&summary),
                            files_discovered: (summary.parsed + summary.deleted + summary.renamed)
                                as i64,
                            files_processed: summary.parsed as i64,
                            files_accepted: summary.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: summary
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: summary.parse_errors as i64,
                            bytes_accepted: summary.budget_counters.bytes_accepted as i64,
                            bytes_skipped: summary.budget_counters.bytes_skipped as i64,
                            nodes_written: summary.nodes_updated as i64,
                            edges_written: summary.edges_updated as i64,
                            budget_stop_reason: summary.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                }
                aggregate.processed_repo_count += 1;
                if summary.budget.budget_hit {
                    aggregate.budget_hit_repo_count += 1;
                }
                aggregate.files_accepted += summary.budget_counters.files_accepted;
                aggregate.files_skipped_by_byte_budget +=
                    summary.budget_counters.files_skipped_by_byte_budget;
                aggregate.bytes_accepted += summary.budget_counters.bytes_accepted;
                aggregate.bytes_skipped += summary.budget_counters.bytes_skipped;
                results.push(serde_json::json!({
                    "repo_id": registration.repo_id,
                    "display_alias": registration.display_alias,
                    "status": if summary.is_degraded() { "degraded" } else { "ok" },
                    "ok": true,
                    "parsed": summary.parsed,
                    "deleted": summary.deleted,
                    "renamed": summary.renamed,
                    "nodes_updated": summary.nodes_updated,
                    "edges_updated": summary.edges_updated,
                    "budget_status": summary.budget.budget_status,
                    "budget_hit": summary.budget.budget_hit,
                    "files_accepted": summary.budget_counters.files_accepted,
                    "files_skipped_by_byte_budget": summary.budget_counters.files_skipped_by_byte_budget,
                    "bytes_accepted": summary.budget_counters.bytes_accepted,
                    "bytes_skipped": summary.budget_counters.bytes_skipped,
                    "budget_stop_reason": summary.budget_counters.budget_stop_reason,
                    "warnings": summary.warnings,
                }));
            }
            Err(error) => {
                aggregate.failed_repo_count += 1;
                if !dry_run && let Ok(store) = Store::open(db_path) {
                    let _ = store.fail_build_for_repo(
                        &registration.repo_id,
                        registration.root.as_str(),
                        &error.to_string(),
                    );
                }
                results.push(serde_json::json!({
                    "repo_id": registration.repo_id,
                    "display_alias": registration.display_alias,
                    "status": "error",
                    "ok": false,
                    "error": error.to_string(),
                }));
            }
        }
    }

    if cli.json {
        print_json(
            "update",
            serde_json::json!({
                "dry_run": dry_run,
                "partial_success": aggregate.failed_repo_count > 0,
                "repo_scope": {
                    "selected_repo_count": aggregate.selected_repo_count,
                    "processed_repo_count": aggregate.processed_repo_count,
                    "failed_repo_count": aggregate.failed_repo_count,
                    "skipped_repo_count": aggregate.skipped_repo_count,
                    "excluded_manual_repo_count": aggregate.excluded_manual_repo_count,
                },
                "budget_summary": {
                    "budget_hit_repo_count": aggregate.budget_hit_repo_count,
                    "files_accepted": aggregate.files_accepted,
                    "files_skipped_by_byte_budget": aggregate.files_skipped_by_byte_budget,
                    "bytes_accepted": aggregate.bytes_accepted,
                    "bytes_skipped": aggregate.bytes_skipped,
                },
                "repos": results,
            }),
        )
    } else {
        println!(
            "Update complete for {} repo(s); processed={} failures={} skipped={}",
            aggregate.selected_repo_count,
            aggregate.processed_repo_count,
            aggregate.failed_repo_count,
            aggregate.skipped_repo_count,
        );
        if aggregate.excluded_manual_repo_count > 0 {
            println!(
                "Phase-1 rollout: excluded {} manual repo(s) from broad multi-repo update. Use --repo-id to target them explicitly.",
                aggregate.excluded_manual_repo_count
            );
        }
        println!(
            "Budget summary: accepted_files={} skipped_files={} accepted_bytes={} skipped_bytes={} budget_hit_repos={}",
            aggregate.files_accepted,
            aggregate.files_skipped_by_byte_budget,
            aggregate.bytes_accepted,
            aggregate.bytes_skipped,
            aggregate.budget_hit_repo_count,
        );
        for result in results {
            println!("  {}", result);
        }
        Ok(())
    }
}

pub fn run_update(cli: &Cli) -> Result<()> {
    let (fail_fast, dry_run, selected_repo_id, all_repos, affected_repos) = match &cli.command {
        Command::Update {
            fail_fast,
            dry_run,
            repo_id,
            all_repos,
            affected_repos,
            ..
        } => (
            *fail_fast,
            *dry_run,
            repo_id.clone(),
            *all_repos,
            *affected_repos,
        ),
        _ => unreachable!(),
    };
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("update");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let db_path = db_path(cli, &repo);

        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
        let build_budget = config.build_run_budget()?;

        if all_repos || affected_repos || selected_repo_id.is_some() {
            return run_registered_updates(
                cli,
                Utf8Path::new(&repo),
                &db_path,
                &config,
                fail_fast,
                dry_run,
                selected_repo_id.as_deref(),
                all_repos,
                affected_repos,
            );
        }

        let explicit_files: Vec<String> = match &cli.command {
            Command::Update { files, .. } => files.clone(),
            _ => vec![],
        };

        let target = if !explicit_files.is_empty() {
            UpdateTarget::Files(explicit_files)
        } else {
            match &cli.command {
                Command::Update { base, staged, .. } => {
                    if *staged {
                        UpdateTarget::Staged
                    } else if let Some(base_ref) = base {
                        UpdateTarget::BaseRef(base_ref.clone())
                    } else {
                        UpdateTarget::WorkingTree
                    }
                }
                _ => UpdateTarget::WorkingTree,
            }
        };

        let source_repo_id = stable_repo_id(repo_root_path.as_path());

        // Record lifecycle: building.
        if !dry_run && let Ok(store) = Store::open(&db_path) {
            let _ = store.begin_build_for_repo(&source_repo_id, repo_root_path.as_str());
        }

        let update_result = update_graph(
            repo_root_path.as_path(),
            &db_path,
            &UpdateOptions {
                fail_fast,
                dry_run,
                batch_size: config.parse_batch_size(),
                target,
                budget: build_budget,
                source_repo_id: Some(source_repo_id.clone()),
                namespace_qualified_names: false,
            },
        );

        // Record lifecycle: built or build_failed.
        if !dry_run && let Ok(store) = Store::open(&db_path) {
            match &update_result {
                Ok(s) => {
                    let state =
                        if matches!(s.budget.budget_status, atlas_core::BudgetStatus::Blocked) {
                            atlas_store_sqlite::GraphBuildState::BuildFailed
                        } else if s.is_degraded() {
                            atlas_store_sqlite::GraphBuildState::Degraded
                        } else {
                            atlas_store_sqlite::GraphBuildState::Built
                        };
                    let _ = store.finish_build_for_repo(
                        &source_repo_id,
                        repo_root_path.as_str(),
                        BuildFinishStats {
                            state,
                            files_discovered: (s.parsed + s.deleted + s.renamed) as i64,
                            files_processed: s.parsed as i64,
                            files_accepted: s.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: s
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: s.parse_errors as i64,
                            bytes_accepted: s.budget_counters.bytes_accepted as i64,
                            bytes_skipped: s.budget_counters.bytes_skipped as i64,
                            nodes_written: s.nodes_updated as i64,
                            edges_written: s.edges_updated as i64,
                            budget_stop_reason: s.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                }
                Err(e) => {
                    let _ = store.fail_build_for_repo(
                        &source_repo_id,
                        repo_root_path.as_str(),
                        &e.to_string(),
                    );
                }
            }
        }

        let summary = update_result?;

        if cli.json {
            print_json(
                "update",
                serde_json::json!({
                    "dry_run": dry_run,
                    "deleted": summary.deleted,
                    "renamed": summary.renamed,
                    "parsed": summary.parsed,
                    "skipped_unsupported": summary.skipped_unsupported,
                    "parse_errors": summary.parse_errors,
                    "chunk_upsert_failures": summary.chunk_upsert_failures,
                    "call_target_reconcile_failures": summary.call_target_reconcile_failures,
                    "nodes_updated": summary.nodes_updated,
                    "edges_updated": summary.edges_updated,
                    "warnings": summary.warnings,
                    "budget": summary.budget,
                    "budget_counters": summary.budget_counters,
                    "elapsed_ms": summary.elapsed_ms,
                    "nodes_per_sec": if summary.elapsed_ms > 0 {
                        (summary.nodes_updated as f64 / summary.elapsed_ms as f64 * 1000.0).round() as u64
                    } else { summary.nodes_updated as u64 },
                }),
            )?;
        } else {
            let nodes_per_sec = if summary.elapsed_ms > 0 {
                format!(
                    "{:.0} nodes/s",
                    summary.nodes_updated as f64 / summary.elapsed_ms as f64 * 1000.0
                )
            } else {
                String::from("—")
            };
            println!(
                "{} ({:.2}s, {nodes_per_sec})",
                if dry_run {
                    "Update dry run complete"
                } else {
                    "Update complete"
                },
                summary.elapsed_ms as f64 / 1000.0
            );
            print_summary_value("Deleted", summary.deleted);
            if summary.renamed > 0 {
                print_summary_value("Renamed", summary.renamed);
            }
            print_summary_value("Parsed", summary.parsed);
            if summary.skipped_unsupported > 0 {
                print_summary_value("Unsupported skipped", summary.skipped_unsupported);
            }
            print_summary_value("Files accepted", summary.budget_counters.files_accepted);
            if summary.budget_counters.files_skipped_by_byte_budget > 0 {
                print_summary_value(
                    "Byte-budget skipped",
                    summary.budget_counters.files_skipped_by_byte_budget,
                );
            }
            if summary.parse_errors > 0 {
                print_summary_value("Errors", summary.parse_errors);
            }
            if summary.chunk_upsert_failures > 0 {
                print_summary_value("Chunk indexing failures", summary.chunk_upsert_failures);
            }
            if summary.call_target_reconcile_failures > 0 {
                print_summary_value(
                    "Call-target reconcile failures",
                    summary.call_target_reconcile_failures,
                );
            }
            print_summary_value("Nodes", summary.nodes_updated);
            print_summary_value("Edges", summary.edges_updated);
            if let Some(reason) = &summary.budget_counters.budget_stop_reason {
                print_summary_value("Budget stop reason", reason);
            }
            for warning in &summary.warnings {
                print_summary_value("Warning", warning);
            }
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("update", result.is_ok());
    }
    result
}

pub fn run_watch(cli: &Cli) -> Result<()> {
    use atlas_engine::{WatchRunner, config};
    use std::time::Duration;

    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let db_path = db_path(cli, &repo);

    let engine_config = config::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;

    let (debounce_ms, watch_json) = match &cli.command {
        Command::Watch { debounce_ms, json } => (*debounce_ms, *json),
        _ => (200, false),
    };
    let json_output = cli.json || watch_json;
    let debounce = Duration::from_millis(debounce_ms);

    let mut runner = WatchRunner::new(
        repo_root_path.as_path(),
        db_path.clone(),
        debounce,
        engine_config.parse_batch_size(),
    )
    .context("cannot start watch runner")?;

    if !json_output {
        println!(
            "Watching '{}' (debounce {}ms) — press Ctrl+C to stop",
            repo_root_path, debounce_ms
        );
    }

    runner.run(|result| {
        if json_output {
            let obj = serde_json::json!({
                "schema_version": "atlas_cli.v1",
                "command": "watch",
                "data": {
                    "files_updated": result.files_updated,
                    "observed_events": result.observed_events,
                    "coalesced_events": result.coalesced_events,
                    "dropped_events": result.dropped_events,
                    "recovery_mode": result.recovery_mode,
                    "nodes_updated": result.nodes_updated,
                    "errors": result.errors,
                    "elapsed_ms": result.elapsed_ms,
                    "error_messages": result.error_messages,
                }
            });
            println!("{}", serde_json::to_string(&obj).unwrap_or_default());
        } else if result.errors > 0 {
            if result.dropped_events > 0 {
                eprintln!(
                    "watch: backlog overflowed; recovered via {} after dropping {} raw event(s)",
                    result.recovery_mode, result.dropped_events,
                );
            }
            eprintln!(
                "watch: {} file(s) — {} event(s) observed — {} coalesced — {} node(s) updated — {} error(s) [{} ms]",
                result.files_updated,
                result.observed_events,
                result.coalesced_events,
                result.nodes_updated,
                result.errors,
                result.elapsed_ms,
            );
            for msg in &result.error_messages {
                eprintln!("  {msg}");
            }
        } else {
            if result.dropped_events > 0 {
                println!(
                    "watch: backlog overflowed; recovered via {} after dropping {} raw event(s)",
                    result.recovery_mode, result.dropped_events,
                );
            }
            println!(
                "watch: {} file(s) — {} event(s) observed — {} coalesced — {} node(s) updated [{} ms]",
                result.files_updated,
                result.observed_events,
                result.coalesced_events,
                result.nodes_updated,
                result.elapsed_ms,
            );
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_repo::{RepoRelationship, TrustState, VcsMetadata};
    use camino::{Utf8Path, Utf8PathBuf};

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
    fn enabled_registration_targets_all_repos_excludes_manual_registrations() {
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

        let (selected, excluded_manual) = enabled_registration_targets(root, None, true).unwrap();

        assert_eq!(selected.len(), 2);
        assert_eq!(excluded_manual, 1);
        assert!(
            selected
                .iter()
                .all(|entry| entry.relationship.kind != RepoRelationshipKind::Manual)
        );
    }

    #[test]
    fn enabled_registration_targets_rejects_excessive_all_repo_fanout() {
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

        let error = enabled_registration_targets(root, None, true).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("all_repos scope exceeds max supported repo fan-out")
        );
    }
}
