use std::fs;

use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter};
use atlas_contentstore::{ContentStore, IndexState};
use atlas_core::GraphStats;
use atlas_core::model::ChangedFile;
use atlas_core::{
    GraphHealthInput, graph_health_error_message, graph_health_error_suggestions,
    select_graph_health_error_code,
};
use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_parser::ParserRegistry;
use atlas_repo::{changed_files, find_repo_root};
use atlas_session::SessionStore;
use atlas_store_sqlite::{BuildFinishStats, Store};
use camino::Utf8Path;

use crate::cli::{Cli, Command};

use super::{
    augment_changes_with_node_counts, change_tag, db_path, detect_changes_target, print_json,
    resolve_repo,
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
}

fn file_has_graph_facts(store: &Store, path: &str) -> bool {
    store
        .nodes_by_file(path)
        .map(|nodes| !nodes.is_empty())
        .unwrap_or(false)
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

fn graph_relevant_changed_files(store: &Store, changes: &[ChangedFile]) -> Vec<String> {
    let registry = ParserRegistry::with_defaults();
    let mut files: Vec<String> = changes
        .iter()
        .filter(|change| change_can_affect_graph_facts(store, &registry, change))
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
        atlas_store_sqlite::GraphBuildState::BuildFailed => "build_failed",
    });
    let graph_built = build_state == Some("built")
        || (build_state.is_none()
            && graph_query_error.is_none()
            && (ctx.stats.node_count > 0 || ctx.stats.edge_count > 0 || ctx.stats.file_count > 0));
    let pending_graph_changes = ctx
        .store
        .map(|store| graph_relevant_changed_files(store, ctx.changes))
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

    StatusDiagnostics {
        ok: error_code == "none" && graph_built,
        error_code,
        graph_built,
        build_state: build_state.map(str::to_owned),
        build_last_error: build_status.and_then(|status| status.last_error),
        graph_query_error,
        pending_graph_changes,
        retrieval_index,
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
            atlas_store_sqlite::GraphBuildState::BuildFailed => "build_failed",
        };
        serde_json::json!({
            "state": state_str,
            "files_discovered": bs.files_discovered,
            "files_processed": bs.files_processed,
            "files_failed": bs.files_failed,
            "nodes_written": bs.nodes_written,
            "edges_written": bs.edges_written,
            "last_built_at": bs.last_built_at,
            "last_error": bs.last_error,
        })
    });
    serde_json::json!({
        "ok": diagnostics.ok,
        "error_code": diagnostics.error_code,
        "message": graph_health_error_message(diagnostics.error_code),
        "suggestions": graph_health_error_suggestions(diagnostics.error_code),
        "repo_root": ctx.repo,
        "db_path": ctx.db_path,
        "mcp": {
            "worker_threads": ctx.config.mcp_worker_threads(),
            "tool_timeout_ms": ctx.config.mcp_tool_timeout_ms(),
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
        "retrieval_index": diagnostics.retrieval_index,
        "changed_file_count": ctx.changes.len(),
        "changed_files": augment_changes_with_node_counts(ctx.changes, ctx.store),
        "build_status": build_state_val,
    })
}

pub fn run_init(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let atlas_dir = atlas_engine::paths::atlas_dir(&repo);
    fs::create_dir_all(&atlas_dir)
        .with_context(|| format!("cannot create {}", atlas_dir.display()))?;

    let db_path = db_path(cli, &repo);
    Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let content_db_path = atlas_engine::paths::content_db_path(&db_path);
    let mut content_store = ContentStore::open(&content_db_path)
        .with_context(|| format!("cannot open content store at {content_db_path}"))?;
    content_store
        .migrate()
        .with_context(|| format!("cannot migrate content store at {content_db_path}"))?;

    let session_db_path = atlas_engine::paths::session_db_path(&db_path);
    SessionStore::open(&session_db_path)
        .with_context(|| format!("cannot open session store at {session_db_path}"))?;

    let config_path = atlas_engine::paths::config_path(&repo);
    let config_created = atlas_engine::Config::write_default(&atlas_dir)
        .with_context(|| format!("cannot write config to {}", config_path.display()))?;

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
        if config_created {
            println!("Config  : {}", config_path.display());
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
    let stats = store.stats().context("cannot read stats")?;
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
        println!("Repo root : {repo}");
        println!("Database  : {db_path}");
        println!(
            "MCP serve : workers={} timeout_ms={}",
            config.mcp_worker_threads(),
            config.mcp_tool_timeout_ms()
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
    let fail_fast = matches!(&cli.command, Command::Build { fail_fast } if *fail_fast);
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

        // Record lifecycle: building.
        if let Ok(store) = Store::open(&db_path) {
            let _ = store.begin_build(repo_root_path.as_str());
        }

        let build_result = build_graph(
            repo_root_path.as_path(),
            &db_path,
            &BuildOptions {
                fail_fast,
                batch_size: config.parse_batch_size(),
            },
        );

        // Record lifecycle: built or build_failed.
        if let Ok(store) = Store::open(&db_path) {
            match &build_result {
                Ok(s) => {
                    let _ = store.finish_build(
                        repo_root_path.as_str(),
                        BuildFinishStats {
                            files_discovered: s.scanned as i64,
                            files_processed: s.parsed as i64,
                            files_failed: s.parse_errors as i64,
                            nodes_written: s.nodes_inserted as i64,
                            edges_written: s.edges_inserted as i64,
                        },
                    );
                }
                Err(e) => {
                    let _ = store.fail_build(repo_root_path.as_str(), &e.to_string());
                }
            }
        }

        let summary = build_result?;

        if cli.json {
            print_json(
                "build",
                serde_json::json!({
                    "scanned": summary.scanned,
                    "skipped_unsupported": summary.skipped_unsupported,
                    "skipped_unchanged": summary.skipped_unchanged,
                    "parsed": summary.parsed,
                    "parse_errors": summary.parse_errors,
                    "nodes_inserted": summary.nodes_inserted,
                    "edges_inserted": summary.edges_inserted,
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
                "Build complete ({:.2}s, {nodes_per_sec})",
                summary.elapsed_ms as f64 / 1000.0
            );
            println!("  Scanned             : {}", summary.scanned);
            println!("  Unsupported skipped : {}", summary.skipped_unsupported);
            println!("  Unchanged skipped   : {}", summary.skipped_unchanged);
            println!("  Parsed              : {}", summary.parsed);
            if summary.parse_errors > 0 {
                println!("  Errors              : {}", summary.parse_errors);
            }
            println!("  Nodes inserted      : {}", summary.nodes_inserted);
            println!("  Edges inserted      : {}", summary.edges_inserted);
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("build", result.is_ok());
    }
    result
}

pub fn run_update(cli: &Cli) -> Result<()> {
    let fail_fast = matches!(
        &cli.command,
        Command::Update { fail_fast, .. } if *fail_fast
    );
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

        // Record lifecycle: building.
        if let Ok(store) = Store::open(&db_path) {
            let _ = store.begin_build(repo_root_path.as_str());
        }

        let update_result = update_graph(
            repo_root_path.as_path(),
            &db_path,
            &UpdateOptions {
                fail_fast,
                batch_size: config.parse_batch_size(),
                target,
            },
        );

        // Record lifecycle: built or build_failed.
        if let Ok(store) = Store::open(&db_path) {
            match &update_result {
                Ok(s) => {
                    let _ = store.finish_build(
                        repo_root_path.as_str(),
                        BuildFinishStats {
                            files_discovered: (s.parsed + s.deleted + s.renamed) as i64,
                            files_processed: s.parsed as i64,
                            files_failed: s.parse_errors as i64,
                            nodes_written: s.nodes_updated as i64,
                            edges_written: s.edges_updated as i64,
                        },
                    );
                }
                Err(e) => {
                    let _ = store.fail_build(repo_root_path.as_str(), &e.to_string());
                }
            }
        }

        let summary = update_result?;

        if cli.json {
            print_json(
                "update",
                serde_json::json!({
                    "deleted": summary.deleted,
                    "renamed": summary.renamed,
                    "parsed": summary.parsed,
                    "skipped_unsupported": summary.skipped_unsupported,
                    "parse_errors": summary.parse_errors,
                    "nodes_updated": summary.nodes_updated,
                    "edges_updated": summary.edges_updated,
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
                "Update complete ({:.2}s, {nodes_per_sec})",
                summary.elapsed_ms as f64 / 1000.0
            );
            println!("  Deleted  : {}", summary.deleted);
            if summary.renamed > 0 {
                println!("  Renamed  : {}", summary.renamed);
            }
            println!("  Parsed   : {}", summary.parsed);
            if summary.skipped_unsupported > 0 {
                println!("  Unsupported skipped : {}", summary.skipped_unsupported);
            }
            if summary.parse_errors > 0 {
                println!("  Errors   : {}", summary.parse_errors);
            }
            println!("  Nodes    : {}", summary.nodes_updated);
            println!("  Edges    : {}", summary.edges_updated);
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
                    "nodes_updated": result.nodes_updated,
                    "errors": result.errors,
                    "elapsed_ms": result.elapsed_ms,
                    "error_messages": result.error_messages,
                }
            });
            println!("{}", serde_json::to_string(&obj).unwrap_or_default());
        } else if result.errors > 0 {
            eprintln!(
                "watch: {} file(s) — {} node(s) updated — {} error(s) [{} ms]",
                result.files_updated, result.nodes_updated, result.errors, result.elapsed_ms,
            );
            for msg in &result.error_messages {
                eprintln!("  {msg}");
            }
        } else {
            println!(
                "watch: {} file(s) — {} node(s) updated [{} ms]",
                result.files_updated, result.nodes_updated, result.elapsed_ms,
            );
        }
    })
}
