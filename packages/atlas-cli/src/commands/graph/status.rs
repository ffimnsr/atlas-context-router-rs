use crate::cli::{Cli, Command};
use anyhow::{Context, Result};
use atlas_contentstore::{ContentStore, IndexState};
use atlas_core::model::{ChangeType, ChangedFile};
use atlas_core::{
    GraphHealthInput, GraphReadiness, GraphReadinessInput, GraphStats, graph_health_error_message,
    graph_health_error_suggestions, select_graph_health_error_code,
};
use atlas_parser::ParserRegistry;
use atlas_repo::{
    changed_files, find_repo_root, hash_file, stable_repo_fingerprint, stable_repo_id,
};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

use super::super::{
    augment_changes_with_node_counts, change_tag, db_path, derive_graph_readiness_open_failed,
    detect_changes_target, print_json, public_graph_stats, resolve_repo,
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
    health_class: Option<String>,
    graph_built: bool,
    build_state: Option<String>,
    build_last_error: Option<String>,
    graph_query_error: Option<String>,
    pending_graph_changes: Vec<String>,
    retrieval_index: serde_json::Value,
    execution_state: atlas_core::GraphExecutionState,
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
    let retrieval_index = retrieval_index_value(ctx.repo, ctx.db_path);
    let retrieval_unavailable = graph_built
        && (!retrieval_index["available"].as_bool().unwrap_or(false)
            || !retrieval_index["searchable"].as_bool().unwrap_or(false)
            || retrieval_index["state"].as_str() != Some("indexed"));
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
        recovery_mode: build_status
            .as_ref()
            .and_then(|bs| bs.recovery_mode.as_deref()),
        quarantine_path: build_status
            .as_ref()
            .and_then(|bs| bs.quarantine_path.as_deref()),
        pending_graph_changes: &pending_graph_changes,
        indexed_file_count: ctx.stats.file_count,
        graph_has_content,
        last_indexed_at: ctx.stats.last_indexed_at.as_deref(),
        retrieval_unavailable,
    });

    let error_code = select_graph_health_error_code(GraphHealthInput {
        db_exists: true,
        graph_built,
        health_class: readiness.health_class,
        build_state,
        retrieval_unavailable,
    });

    StatusDiagnostics {
        ok: error_code == "none" && graph_built,
        error_code,
        health_class: readiness
            .health_class
            .map(|class| class.as_str().to_owned()),
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
            "recovery_mode": bs.recovery_mode,
            "quarantine_path": bs.quarantine_path,
        })
    });
    let quarantine_path = build_status
        .as_ref()
        .and_then(|status| status.quarantine_path.clone());
    let rebuild_result = if let Some(path) = &quarantine_path {
        let _ = path;
        match diagnostics.build_state.as_deref() {
            Some("build_failed") => "rebuild_failed",
            Some("built") | Some("degraded") => "rebuilt_fresh",
            _ => "blocked",
        }
    } else {
        "not_needed"
    };
    let repo_id = stable_repo_id(Utf8Path::new(ctx.repo));
    let repo_fingerprint = stable_repo_fingerprint(Utf8Path::new(ctx.repo), None);
    serde_json::json!({
        "ok": diagnostics.ok,
        "error_code": diagnostics.error_code,
        "health_class": diagnostics.health_class,
        "recovery_mode": "block_only",
        "quarantine_path": quarantine_path,
        "rebuild_result": rebuild_result,
        "failure_reason": diagnostics.build_last_error.clone(),
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

fn status_fail_closed_payload(
    repo: &str,
    db_path: &str,
    config: &atlas_engine::Config,
    base: &Option<String>,
    staged: bool,
    readiness: &GraphReadiness,
) -> serde_json::Value {
    let repo_id = stable_repo_id(Utf8Path::new(repo));
    let repo_fingerprint = stable_repo_fingerprint(Utf8Path::new(repo), None);
    serde_json::json!({
        "ok": false,
        "error_code": &readiness.error_code,
        "health_class": readiness.health_class.map(|class| class.as_str()),
        "recovery_mode": "block_only",
        "quarantine_path": serde_json::Value::Null,
        "rebuild_result": "blocked",
        "failure_reason": &readiness.db_open_error,
        "message": &readiness.message,
        "suggestions": &readiness.suggestions,
        "repo_root": repo,
        "repo_provenance": {
            "repo_id": repo_id,
            "repo_fingerprint": repo_fingerprint,
            "repo_root": repo,
        },
        "db_path": db_path,
        "mcp": {
            "worker_threads": config.mcp_worker_threads(),
            "tool_timeout_ms": config.mcp_tool_timeout_ms(),
            "tool_timeout_ms_by_tool": config.mcp_tool_timeout_ms_by_tool(),
        },
        "diff_target": {
            "base": base,
            "staged": staged,
            "kind": if staged { "staged" } else if base.is_some() { "base_ref" } else { "working_tree" },
        },
        "indexed_file_count": 0,
        "node_count": 0,
        "edge_count": 0,
        "nodes_by_kind": Vec::<(String, i64)>::new(),
        "languages": Vec::<String>::new(),
        "last_indexed_at": serde_json::Value::Null,
        "graph_built": readiness.graph_built,
        "build_state": &readiness.build_state,
        "build_last_error": &readiness.build_last_error,
        "graph_query_error": &readiness.db_open_error,
        "stale_index": readiness.stale_index,
        "pending_graph_change_count": readiness.pending_graph_changes.len(),
        "pending_graph_changes": &readiness.pending_graph_changes,
        "execution_state": readiness.execution_state.as_str(),
        "retrieval_index": retrieval_index_value(repo, db_path),
        "changed_file_count": 0,
        "changed_files": Vec::<serde_json::Value>::new(),
        "build_status": serde_json::Value::Null,
    })
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

    let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
    let store = match Store::open(&db_path) {
        Ok(store) => store,
        Err(error) => {
            let readiness = derive_graph_readiness_open_failed(&repo, &db_path, &error.to_string());
            if cli.json {
                print_json(
                    "status",
                    status_fail_closed_payload(&repo, &db_path, &config, &base, staged, &readiness),
                )?;
            } else {
                eprintln!("Error: {}", readiness.message);
                for suggestion in &readiness.suggestions {
                    eprintln!("  → {suggestion}");
                }
            }
            anyhow::bail!("{}", readiness.message)
        }
    };
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
