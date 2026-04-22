use anyhow::{Context, Result};
use atlas_contentstore::{ContentStore, IndexState};
use atlas_core::{
    graph_health_error_message, graph_health_error_suggestions, is_schema_mismatch_error,
};
use atlas_repo::{collect_files, find_repo_root};
use atlas_store_sqlite::{GraphBuildState, Store};
use camino::Utf8Path;

use crate::cli::{Cli, Command};

use super::{db_path, print_json, resolve_repo};

struct CheckResult {
    name: &'static str,
    ok: bool,
    detail: String,
    issue_code: Option<&'static str>,
}

impl CheckResult {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: true,
            detail: detail.into(),
            issue_code: None,
        }
    }
    fn fail(
        name: &'static str,
        detail: impl Into<String>,
        issue_code: Option<&'static str>,
    ) -> Self {
        Self {
            name,
            ok: false,
            detail: detail.into(),
            issue_code,
        }
    }
}

fn graph_issue_code(error: &str) -> &'static str {
    if is_schema_mismatch_error(error) {
        "schema_mismatch"
    } else {
        "corrupt_or_inconsistent_graph_rows"
    }
}

fn print_doctor_report(cli: &Cli, checks: &[CheckResult], all_ok: bool) -> Result<()> {
    if cli.json {
        let items: Vec<serde_json::Value> = checks
            .iter()
            .map(|c| {
                serde_json::json!({
                    "check": c.name,
                    "ok": c.ok,
                    "detail": c.detail,
                    "issue_code": c.issue_code,
                })
            })
            .collect();
        let error_code = if all_ok { "none" } else { "checks_failed" };
        print_json(
            "doctor",
            serde_json::json!({
                "ok": all_ok,
                "error_code": error_code,
                "message": graph_health_error_message(error_code),
                "suggestions": graph_health_error_suggestions(error_code),
                "checks": items,
            }),
        )?;
    } else {
        for c in checks {
            let status = if c.ok { "PASS" } else { "FAIL" };
            println!("  [{status}] {}: {}", c.name, c.detail);
        }
        println!();
        if all_ok {
            println!("All checks passed.");
        } else {
            eprintln!("Some checks failed.");
        }
    }
    Ok(())
}

pub fn run_doctor(cli: &Cli) -> Result<()> {
    let mut checks: Vec<CheckResult> = Vec::new();

    // 1. Repo root
    let repo = match resolve_repo(cli) {
        Ok(r) => {
            checks.push(CheckResult::pass("repo_root", &r));
            r
        }
        Err(e) => {
            checks.push(CheckResult::fail("repo_root", e.to_string(), None));
            return print_doctor_report(cli, &checks, false);
        }
    };

    // 2. Git repo root detection
    match find_repo_root(Utf8Path::new(&repo)) {
        Ok(root) => checks.push(CheckResult::pass("git_root", root.as_str())),
        Err(e) => checks.push(CheckResult::fail("git_root", e.to_string(), None)),
    }

    // 3. .atlas dir
    let atlas_dir = atlas_engine::paths::atlas_dir(&repo);
    if atlas_dir.exists() {
        checks.push(CheckResult::pass(
            "atlas_dir",
            atlas_dir.display().to_string(),
        ));
    } else {
        checks.push(CheckResult::fail(
            "atlas_dir",
            format!("{} not found — run `atlas init`", atlas_dir.display()),
            None,
        ));
    }

    // 4. Config file
    let config_path = atlas_engine::paths::config_path(&repo);
    let mut loaded_config: Option<atlas_engine::Config> = None;
    if config_path.exists() {
        checks.push(CheckResult::pass(
            "config_file",
            config_path.display().to_string(),
        ));
        match atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo)) {
            Ok(config) => {
                checks.push(CheckResult::pass(
                    "mcp_serve_config",
                    format!(
                        "workers={} timeout_ms={}",
                        config.mcp_worker_threads(),
                        config.mcp_tool_timeout_ms()
                    ),
                ));
                loaded_config = Some(config);
            }
            Err(e) => {
                checks.push(CheckResult::fail("mcp_serve_config", e.to_string(), None));
            }
        }
    } else {
        checks.push(CheckResult::fail(
            "config_file",
            format!("{} not found — run `atlas init`", config_path.display()),
            None,
        ));
    }

    // 5. DB file exists
    let db_path_str = db_path(cli, &repo);
    let db_exists = std::path::Path::new(&db_path_str).exists();
    if db_exists {
        checks.push(CheckResult::pass("db_file", &db_path_str));
    } else {
        checks.push(CheckResult::fail(
            "db_file",
            format!("{db_path_str} not found — run `atlas init`"),
            None,
        ));
    }

    // 6. DB open + integrity + stats
    if db_exists {
        match Store::open(&db_path_str) {
            Ok(store) => {
                checks.push(CheckResult::pass("db_open", &db_path_str));
                match store.integrity_check() {
                    Ok(issues) if issues.is_empty() => {
                        checks.push(CheckResult::pass("db_integrity", "ok"));
                    }
                    Ok(issues) => {
                        checks.push(CheckResult::fail(
                            "db_integrity",
                            issues.join("; "),
                            Some("corrupt_or_inconsistent_graph_rows"),
                        ));
                    }
                    Err(e) => {
                        let detail = e.to_string();
                        checks.push(CheckResult::fail(
                            "db_integrity",
                            &detail,
                            Some(graph_issue_code(&detail)),
                        ));
                    }
                }
                match store.stats() {
                    Ok(stats) => {
                        checks.push(CheckResult::pass(
                            "graph_stats",
                            format!(
                                "files={} nodes={} edges={}",
                                stats.file_count, stats.node_count, stats.edge_count
                            ),
                        ));
                    }
                    Err(e) => {
                        let detail = e.to_string();
                        checks.push(CheckResult::fail(
                            "graph_stats",
                            &detail,
                            Some(graph_issue_code(&detail)),
                        ));
                    }
                }

                // 6b. Graph build lifecycle state.
                match store.get_build_status(&repo) {
                    Ok(Some(bs)) => {
                        let (state_str, is_ok) = match bs.state {
                            GraphBuildState::Built => ("built", true),
                            GraphBuildState::Building => ("building (interrupted?)", false),
                            GraphBuildState::BuildFailed => ("build_failed", false),
                        };
                        if is_ok {
                            checks.push(CheckResult::pass(
                                "graph_build_state",
                                format!(
                                    "state={state_str} nodes={} edges={}",
                                    bs.nodes_written, bs.edges_written
                                ),
                            ));
                        } else {
                            let detail = if let Some(err) = bs.last_error {
                                format!("state={state_str} error={err}")
                            } else {
                                format!("state={state_str}")
                            };
                            let issue_code = if matches!(bs.state, GraphBuildState::Building) {
                                "interrupted_build"
                            } else {
                                "failed_build"
                            };
                            checks.push(CheckResult::fail(
                                "graph_build_state",
                                detail,
                                Some(issue_code),
                            ));
                        }
                    }
                    Ok(None) => {
                        checks.push(CheckResult::pass(
                            "graph_build_state",
                            "no build recorded yet",
                        ));
                    }
                    Err(e) => {
                        let detail = e.to_string();
                        checks.push(CheckResult::fail(
                            "graph_build_state",
                            &detail,
                            Some(graph_issue_code(&detail)),
                        ));
                    }
                }
            }
            Err(e) => {
                let detail = e.to_string();
                checks.push(CheckResult::fail(
                    "db_open",
                    &detail,
                    Some(graph_issue_code(&detail)),
                ));
            }
        }
    }

    let _ = loaded_config;

    // 7. git ls-files reachable
    match collect_files(Utf8Path::new(&repo), None) {
        Ok(files) => {
            checks.push(CheckResult::pass(
                "git_ls_files",
                format!("{} tracked files", files.len()),
            ));
        }
        Err(e) => {
            checks.push(CheckResult::fail("git_ls_files", e.to_string(), None));
        }
    }

    let graph_freshness = if db_exists {
        match Store::open(&db_path_str) {
            Ok(store) => {
                let registry = atlas_parser::ParserRegistry::with_defaults();
                match atlas_repo::changed_files(
                    Utf8Path::new(&repo),
                    &atlas_repo::DiffTarget::WorkingTree,
                ) {
                    Ok(changes) => {
                        let mut files: Vec<String> = changes
                            .iter()
                            .filter(|change| {
                                registry.supports(&change.path)
                                    || change
                                        .old_path
                                        .as_deref()
                                        .is_some_and(|old_path| registry.supports(old_path))
                                    || store
                                        .nodes_by_file(&change.path)
                                        .map(|nodes| !nodes.is_empty())
                                        .unwrap_or(false)
                                    || change.old_path.as_deref().is_some_and(|old_path| {
                                        store
                                            .nodes_by_file(old_path)
                                            .map(|nodes| !nodes.is_empty())
                                            .unwrap_or(false)
                                    })
                            })
                            .flat_map(|change| {
                                std::iter::once(change.path.clone()).chain(change.old_path.clone())
                            })
                            .collect();
                        files.sort();
                        files.dedup();
                        files
                    }
                    Err(_) => Vec::new(),
                }
            }
            Err(_) => Vec::new(),
        }
    } else {
        Vec::new()
    };
    if graph_freshness.is_empty() {
        checks.push(CheckResult::pass("graph_freshness", "up_to_date"));
    } else {
        let preview = graph_freshness
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let detail = if graph_freshness.len() > 5 {
            format!(
                "{} pending graph-relevant files: {} (+{} more)",
                graph_freshness.len(),
                preview,
                graph_freshness.len() - 5
            )
        } else {
            format!(
                "{} pending graph-relevant files: {}",
                graph_freshness.len(),
                preview
            )
        };
        checks.push(CheckResult::fail(
            "graph_freshness",
            detail,
            Some("stale_index"),
        ));
    }

    // 8. Content DB retrieval index state (best-effort; missing DB is not fatal).
    {
        let content_db = atlas_engine::paths::content_db_path(&db_path_str);
        match ContentStore::open(&content_db) {
            Ok(mut cs) => {
                let _ = cs.migrate();
                match cs.get_index_status(&repo) {
                    Ok(Some(status)) => {
                        let state_str = match status.state {
                            IndexState::Indexed => "indexed",
                            IndexState::Indexing => "indexing (interrupted?)",
                            IndexState::IndexFailed => "index_failed",
                        };
                        let searchable = status.state == IndexState::Indexed;
                        checks.push(if searchable {
                            CheckResult::pass(
                                "retrieval_index",
                                format!(
                                    "state={state_str} files={} chunks={}",
                                    status.files_indexed, status.chunks_written
                                ),
                            )
                        } else {
                            let detail = if let Some(err) = status.last_error {
                                format!("state={state_str} error={err}")
                            } else {
                                format!("state={state_str}")
                            };
                            CheckResult::fail(
                                "retrieval_index",
                                detail,
                                Some("retrieval_index_unavailable"),
                            )
                        });
                    }
                    Ok(None) => {
                        checks.push(CheckResult::fail(
                            "retrieval_index",
                            "no index run recorded yet",
                            Some("retrieval_index_unavailable"),
                        ));
                    }
                    Err(e) => {
                        checks.push(CheckResult::fail(
                            "retrieval_index",
                            e.to_string(),
                            Some("retrieval_index_unavailable"),
                        ));
                    }
                }
            }
            // Content DB not yet created — not an error at this point.
            Err(_) => {
                checks.push(CheckResult::fail(
                    "retrieval_index",
                    "content store not initialised",
                    Some("retrieval_index_unavailable"),
                ));
            }
        }
    }

    let all_ok = checks.iter().all(|c| c.ok);
    print_doctor_report(cli, &checks, all_ok)?;
    if !all_ok {
        std::process::exit(1);
    }
    Ok(())
}

pub fn run_db_check(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);

    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let issues = store.integrity_check().context("integrity check failed")?;

    const ORPHAN_LIMIT: usize = 100;
    let orphans = store.orphan_nodes(ORPHAN_LIMIT).unwrap_or_default();
    let dangling = store.dangling_edges(ORPHAN_LIMIT).unwrap_or_default();

    let ok = issues.is_empty() && orphans.is_empty() && dangling.is_empty();

    if cli.json {
        let result = serde_json::json!({
            "db_path": db_path,
            "ok": ok,
            "error_code": if ok { "none" } else { "corrupt_or_inconsistent_graph_rows" },
            "message": graph_health_error_message(if ok { "none" } else { "corrupt_or_inconsistent_graph_rows" }),
            "suggestions": graph_health_error_suggestions(if ok { "none" } else { "corrupt_or_inconsistent_graph_rows" }),
            "integrity_issues": issues,
            "orphan_node_count": orphans.len(),
            "dangling_edge_count": dangling.len(),
        });
        print_json("db_check", result)?;
    } else if ok {
        println!("Database integrity OK: {db_path}");
    } else {
        if !issues.is_empty() {
            eprintln!("Database integrity FAILED: {db_path}");
            for issue in &issues {
                eprintln!("  {issue}");
            }
        }
        if !orphans.is_empty() {
            eprintln!("Orphan nodes (no edges): {}", orphans.len());
            for n in orphans.iter().take(10) {
                eprintln!(
                    "  {} {} ({})",
                    n.kind.as_str(),
                    n.qualified_name,
                    n.file_path
                );
            }
            if orphans.len() > 10 {
                eprintln!("  … and {} more", orphans.len() - 10);
            }
        }
        if !dangling.is_empty() {
            eprintln!("Dangling edges (missing endpoint): {}", dangling.len());
            for (id, src, tgt, kind, side) in dangling.iter().take(10) {
                eprintln!("  edge {id} kind={kind} missing {side}: {src} -> {tgt}");
            }
            if dangling.len() > 10 {
                eprintln!("  … and {} more", dangling.len() - 10);
            }
        }
        std::process::exit(1);
    }

    Ok(())
}

pub fn run_debug_graph(cli: &Cli) -> Result<()> {
    let limit = match &cli.command {
        Command::DebugGraph { limit } => *limit,
        _ => unreachable!(),
    };

    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let stats = store.stats().context("cannot read graph stats")?;
    let edge_kinds = store.edge_kind_stats().context("edge kind stats failed")?;
    let top_files = store
        .top_files_by_node_count(10)
        .context("top files query failed")?;
    let orphans = store
        .orphan_nodes(limit)
        .context("orphan node query failed")?;
    let dangling = store
        .dangling_edges(limit)
        .context("dangling edge query failed")?;

    if cli.json {
        print_json(
            "debug_graph",
            serde_json::json!({
                "ok": true,
                "error_code": "none",
                "message": graph_health_error_message("none"),
                "suggestions": graph_health_error_suggestions("none"),
                "nodes": stats.node_count,
                "edges": stats.edge_count,
                "files": stats.file_count,
                "nodes_by_kind": stats.nodes_by_kind,
                "edges_by_kind": edge_kinds,
                "top_files_by_node_count": top_files,
                "orphan_nodes": orphans.iter().map(|n| serde_json::json!({
                    "kind": n.kind.as_str(),
                    "qualified_name": n.qualified_name,
                    "file_path": n.file_path,
                    "line_start": n.line_start,
                })).collect::<Vec<_>>(),
                "dangling_edges": dangling.iter().map(|(id, src, tgt, kind, side)| serde_json::json!({
                    "id": id,
                    "kind": kind,
                    "source_qn": src,
                    "target_qn": tgt,
                    "missing_side": side,
                })).collect::<Vec<_>>(),
            }),
        )?;
    } else {
        println!("Graph summary");
        println!("  Nodes : {}", stats.node_count);
        println!("  Edges : {}", stats.edge_count);
        println!("  Files : {}", stats.file_count);

        if !stats.nodes_by_kind.is_empty() {
            println!("\nNodes by kind:");
            for (kind, count) in &stats.nodes_by_kind {
                println!("  {kind:<16} {count}");
            }
        }

        if !edge_kinds.is_empty() {
            println!("\nEdges by kind:");
            for (kind, count) in &edge_kinds {
                println!("  {kind:<16} {count}");
            }
        }

        if !top_files.is_empty() {
            println!("\nTop files by node count:");
            for (path, count) in &top_files {
                println!("  {count:>6}  {path}");
            }
        }

        println!("\nData integrity:");
        if orphans.is_empty() {
            println!("  Orphan nodes    : 0 (OK)");
        } else {
            println!("  Orphan nodes    : {} (no edges)", orphans.len());
            for n in orphans.iter().take(5) {
                println!(
                    "    {} {} ({}:{})",
                    n.kind.as_str(),
                    n.qualified_name,
                    n.file_path,
                    n.line_start
                );
            }
            if orphans.len() > 5 {
                println!(
                    "    … and {} more (use --limit to show more)",
                    orphans.len() - 5
                );
            }
        }
        if dangling.is_empty() {
            println!("  Dangling edges  : 0 (OK)");
        } else {
            println!("  Dangling edges  : {} (missing endpoint)", dangling.len());
            for (id, src, tgt, kind, side) in dangling.iter().take(5) {
                println!("    edge {id} [{kind}] missing {side}: {src} -> {tgt}");
            }
            if dangling.len() > 5 {
                println!("    … and {} more", dangling.len() - 5);
            }
        }
    }

    Ok(())
}
