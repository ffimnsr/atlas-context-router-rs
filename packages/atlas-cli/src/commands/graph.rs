use std::fs;

use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter};
use atlas_core::GraphStats;
use atlas_core::model::ChangedFile;
use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_repo::{changed_files, find_repo_root};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

use crate::cli::{Cli, Command};

use super::{
    augment_changes_with_node_counts, change_tag, db_path, detect_changes_target, print_json,
    resolve_repo,
};

fn status_payload(
    repo: &str,
    db_path: &str,
    stats: &GraphStats,
    base: &Option<String>,
    staged: bool,
    changes: &[ChangedFile],
    store: Option<&Store>,
) -> serde_json::Value {
    serde_json::json!({
        "repo_root": repo,
        "db_path": db_path,
        "diff_target": {
            "base": base,
            "staged": staged,
            "kind": if staged { "staged" } else if base.is_some() { "base_ref" } else { "working_tree" },
        },
        "indexed_file_count": stats.file_count,
        "node_count": stats.node_count,
        "edge_count": stats.edge_count,
        "nodes_by_kind": stats.nodes_by_kind,
        "languages": stats.languages,
        "last_indexed_at": stats.last_indexed_at,
        "changed_file_count": changes.len(),
        "changed_files": augment_changes_with_node_counts(changes, store),
    })
}

pub fn run_init(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let atlas_dir = atlas_engine::paths::atlas_dir(&repo);
    fs::create_dir_all(&atlas_dir)
        .with_context(|| format!("cannot create {}", atlas_dir.display()))?;

    let db_path = db_path(cli, &repo);
    Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let config_path = atlas_engine::paths::config_path(&repo);
    let config_created = atlas_engine::Config::write_default(&atlas_dir)
        .with_context(|| format!("cannot write config to {}", config_path.display()))?;

    if cli.json {
        print_json(
            "init",
            serde_json::json!({
                "atlas_dir": atlas_dir.display().to_string(),
                "db_path": db_path,
                "config_path": config_path.display().to_string(),
                "config_created": config_created,
            }),
        )?;
    } else {
        println!("Initialized atlas in {}", atlas_dir.display());
        println!("Database: {db_path}");
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
    let stats = store.stats().context("cannot read stats")?;
    let changes = changed_files(repo_root, &detect_changes_target(&base, staged))
        .context("cannot detect changed files")?;

    if cli.json {
        print_json(
            "status",
            status_payload(
                &repo,
                &db_path,
                &stats,
                &base,
                staged,
                &changes,
                Some(&store),
            ),
        )?;
    } else {
        println!("Repo root : {repo}");
        println!("Database  : {db_path}");
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

        let summary = build_graph(
            repo_root_path.as_path(),
            &db_path,
            &BuildOptions {
                fail_fast,
                batch_size: config.parse_batch_size(),
            },
        )?;

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

        let summary = update_graph(
            repo_root_path.as_path(),
            &db_path,
            &UpdateOptions {
                fail_fast,
                batch_size: config.parse_batch_size(),
                target,
            },
        )?;

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
