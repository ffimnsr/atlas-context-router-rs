use anyhow::{Context, Result};
use atlas_contentstore::{ContentStore, IndexState};
use atlas_core::{
    GraphExecutionState, classify_graph_store_error, graph_health_error_message,
    graph_health_error_suggestions,
};
use atlas_repo::{
    collect_files, find_repo_root, hash_file, stable_repo_fingerprint, stable_repo_id,
};
use atlas_session::SessionStore;
use atlas_store_sqlite::{GraphBuildState, Store};
use camino::Utf8Path;
use std::path::Path;

use crate::cli::Cli;

use super::{
    db_path, derive_graph_readiness, derive_graph_readiness_open_failed, print_json,
    public_graph_stats, resolve_repo,
};

struct CheckResult {
    name: &'static str,
    ok: bool,
    detail: String,
    issue_code: Option<&'static str>,
    noncanonical_path_rows: Option<usize>,
}

impl CheckResult {
    fn pass(name: &'static str, detail: impl Into<String>) -> Self {
        Self {
            name,
            ok: true,
            detail: detail.into(),
            issue_code: None,
            noncanonical_path_rows: None,
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
            noncanonical_path_rows: None,
        }
    }
}

fn graph_issue_code(error: &str) -> &'static str {
    match classify_graph_store_error(error).as_str() {
        "schema_mismatch" => "schema_mismatch",
        "logical_inconsistency" => "logical_inconsistency",
        _ => "sqlite_corrupt",
    }
}

fn integrity_issue_code(issues: &[String], structural_problem: bool) -> &'static str {
    if structural_problem
        || issues
            .iter()
            .any(|issue| atlas_core::is_logical_inconsistency_issue(issue))
    {
        "logical_inconsistency"
    } else {
        "sqlite_corrupt"
    }
}

fn structural_orphans(store: &Store, limit: usize) -> Vec<atlas_core::Node> {
    store.structural_orphan_nodes(limit).unwrap_or_default()
}

fn structural_dangling_edges(
    store: &Store,
    limit: usize,
) -> Vec<(i64, String, String, String, &'static str)> {
    store.structural_dangling_edges(limit).unwrap_or_default()
}

fn display_check_name(name: &str) -> &str {
    match name {
        "repo_root" => "atlas_scope",
        other => other,
    }
}

fn repo_provenance_json(repo_root: &str) -> serde_json::Value {
    let repo_id = stable_repo_id(Utf8Path::new(repo_root));
    let repo_fingerprint = stable_repo_fingerprint(Utf8Path::new(repo_root), None);
    serde_json::json!({
        "repo_id": repo_id,
        "repo_fingerprint": repo_fingerprint,
        "repo_root": repo_root,
    })
}

struct DoctorReachability<'a> {
    measurement: Option<&'a atlas_engine::ReachabilityMeasurement>,
    summary: Option<&'a atlas_engine::ReachabilitySummary>,
}

fn print_doctor_report(
    cli: &Cli,
    repo_root: &str,
    checks: &[CheckResult],
    all_ok: bool,
    execution_state: Option<GraphExecutionState>,
    health_class: Option<&str>,
    reachability: DoctorReachability<'_>,
) -> Result<()> {
    if cli.json {
        let items: Vec<serde_json::Value> = checks
            .iter()
            .map(|c| {
                // Emit both the legacy CLI field names and the MCP field names
                // so one wire schema covers both surfaces (D6).
                serde_json::json!({
                    "name": c.name,
                    "check": c.name,
                    "status": if c.ok { "pass" } else { "fail" },
                    "ok": c.ok,
                    "message": c.detail,
                    "detail": c.detail,
                    "issue_code": c.issue_code,
                    "details": {
                        "label": display_check_name(c.name),
                        "ok": c.ok,
                        "issue_code": c.issue_code,
                        "noncanonical_path_rows": c.noncanonical_path_rows,
                    },
                })
            })
            .collect();
        let error_code = if all_ok { "none" } else { "checks_failed" };
        let mut payload = serde_json::json!({
            "ok": all_ok,
            "error_code": error_code,
            "recovery_mode": "block_only",
            "quarantine_path": serde_json::Value::Null,
            "rebuild_result": if all_ok { "not_needed" } else { "blocked" },
            "failure_reason": serde_json::Value::Null,
            "message": graph_health_error_message(error_code),
            "suggestions": graph_health_error_suggestions(error_code),
            "repo_provenance": repo_provenance_json(repo_root),
            "execution_state": execution_state.map(|s| s.as_str()),
            "health_class": health_class,
            "checks": items,
        });
        if let Some(measurement) = reachability.measurement
            && let Some(object) = payload.as_object_mut()
        {
            object.insert(
                "reachability_measurement".to_owned(),
                serde_json::to_value(measurement).unwrap_or(serde_json::Value::Null),
            );
        }
        if let Some(summary) = reachability.summary
            && let Some(object) = payload.as_object_mut()
        {
            object.insert(
                "reachability".to_owned(),
                serde_json::to_value(summary).unwrap_or(serde_json::Value::Null),
            );
        }
        print_json("doctor", payload)?;
    } else {
        for c in checks {
            let status = if c.ok { "PASS" } else { "FAIL" };
            println!("  [{status}] {}: {}", display_check_name(c.name), c.detail);
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
            return print_doctor_report(
                cli,
                ".",
                &checks,
                false,
                None,
                None,
                DoctorReachability {
                    measurement: None,
                    summary: None,
                },
            );
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
                        let issue_code = integrity_issue_code(&issues, false);
                        checks.push(CheckResult::fail(
                            "db_integrity",
                            issues.join("; "),
                            Some(issue_code),
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
                let source_repo_id = stable_repo_id(Utf8Path::new(&repo));
                match public_graph_stats(&store, &source_repo_id) {
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
                            GraphBuildState::Degraded => ("degraded", false),
                            GraphBuildState::Building => ("building (interrupted?)", false),
                            GraphBuildState::BuildFailed => ("build_failed", false),
                        };
                        if is_ok {
                            checks.push(CheckResult::pass(
                                "graph_build_state",
                                // Build-time telemetry, not the live graph totals;
                                // the `graph_stats` check carries live counts.
                                format!(
                                    "state={state_str} nodes_written_last_build={} edges_written_last_build={}",
                                    bs.nodes_written, bs.edges_written
                                ),
                            ));
                        } else {
                            let detail = if let Some(err) = bs.last_error {
                                format!("state={state_str} error={err}")
                            } else {
                                format!("state={state_str}")
                            };
                            let issue_code = match bs.state {
                                GraphBuildState::Building => "interrupted_build",
                                GraphBuildState::Degraded => "degraded_build",
                                _ => "failed_build",
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

    // 7. git ls-files reachable
    let tracked_files = match collect_files(Utf8Path::new(&repo), None) {
        Ok(files) => {
            checks.push(CheckResult::pass(
                "git_ls_files",
                format!("{} tracked files", files.len()),
            ));
            Some(files)
        }
        Err(e) => {
            checks.push(CheckResult::fail("git_ls_files", e.to_string(), None));
            None
        }
    };

    if let Some(files) = &tracked_files {
        let registry = super::parser_registry_from_config(&repo);
        let check = super::doctor_offloaded::check_tracked_files(files, &registry);
        checks.push(if check.ok {
            CheckResult::pass("offloaded_language_support", check.detail)
        } else {
            CheckResult::fail("offloaded_language_support", check.detail, check.issue_code)
        });
    }

    let graph_freshness = if db_exists {
        match Store::open(&db_path_str) {
            Ok(store) => {
                let registry = super::parser_registry_from_config(&repo);
                // Collect stored file hashes once; used to detect whether atlas
                // update has already indexed the current on-disk state.
                let graph_repo_root = find_repo_root(Utf8Path::new(&repo))
                    .unwrap_or_else(|_| Utf8Path::new(&repo).to_owned());
                let stored_hashes = store
                    .file_hashes_for_repo(&stable_repo_id(graph_repo_root.as_path()))
                    .unwrap_or_default();
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
                            // A file is only truly stale if the graph doesn't
                            // reflect its current on-disk state.  Files that
                            // were indexed by `atlas update` (working-tree
                            // mode) are still git-dirty but their stored hash
                            // already matches the current content, so exclude
                            // them from the stale list.
                            .filter(|rel_path| {
                                let abs_path = format!("{repo}/{rel_path}");
                                let abs_utf8 = camino::Utf8Path::new(&abs_path);
                                if abs_utf8.exists() {
                                    // File exists: stale only when hash doesn't
                                    // match what's stored in the DB.
                                    match hash_file(abs_utf8) {
                                        Ok(disk_hash) => stored_hashes
                                            .get(rel_path.as_str())
                                            .map(|db_hash| db_hash != &disk_hash)
                                            .unwrap_or(true),
                                        // Can't hash → assume stale.
                                        Err(_) => true,
                                    }
                                } else {
                                    // File deleted: stale while DB still has
                                    // a record for it.
                                    stored_hashes.contains_key(rel_path.as_str())
                                }
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

    // Shared schema/canonicality checks. These are produced by
    // `atlas-engine::doctor` so the CLI and MCP `doctor` surfaces cannot drift.
    for check in atlas_engine::doctor::shared_checks(&db_path_str) {
        checks.push(CheckResult {
            name: check.name,
            ok: check.ok,
            detail: check.message,
            issue_code: check.issue_code,
            noncanonical_path_rows: check.details.noncanonical_path_rows,
        });
    }

    if db_exists && let Ok(store) = Store::open(&db_path_str) {
        let source_repo_id = stable_repo_id(Utf8Path::new(&repo));
        let external_parsers = loaded_config
            .as_ref()
            .map(|config| config.parsers.external.as_slice())
            .unwrap_or(&[]);
        let check = atlas_engine::doctor::parser_fingerprint_check(
            &store,
            &source_repo_id,
            external_parsers,
        );
        checks.push(CheckResult {
            name: check.name,
            ok: check.ok,
            detail: check.message,
            issue_code: check.issue_code,
            noncanonical_path_rows: check.details.noncanonical_path_rows,
        });
    }

    let all_ok = checks.iter().all(|c| c.ok);
    let readiness = if db_exists {
        match Store::open(&db_path_str) {
            Ok(store) => derive_graph_readiness(&store, &repo, &db_path_str),
            Err(e) => derive_graph_readiness_open_failed(&repo, &db_path_str, &e.to_string()),
        }
    } else {
        derive_graph_readiness_open_failed(&repo, &db_path_str, "db not found")
    };
    let reachability_measurement = if db_exists {
        Store::open(&db_path_str).ok().and_then(|store| {
            let source_repo_id = stable_repo_id(Utf8Path::new(&repo));
            atlas_engine::measure_same_package_edges(
                &store,
                &source_repo_id,
                atlas_engine::DEFAULT_MEASUREMENT_SAMPLE_LIMIT,
            )
            .ok()
        })
    } else {
        None
    };
    let reachability_summary = if db_exists {
        Store::open(&db_path_str).ok().and_then(|store| {
            let source_repo_id = stable_repo_id(Utf8Path::new(&repo));
            let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))
                .map(|config| config.reachability)
                .unwrap_or_default();
            atlas_engine::diagnose_reachability(&store, &source_repo_id, &config).ok()
        })
    } else {
        None
    };
    print_doctor_report(
        cli,
        &repo,
        &checks,
        all_ok,
        Some(readiness.execution_state),
        readiness.health_class.map(|class| class.as_str()),
        DoctorReachability {
            measurement: reachability_measurement.as_ref(),
            summary: reachability_summary.as_ref(),
        },
    )?;
    if !all_ok {
        std::process::exit(1);
    }
    Ok(())
}

pub fn run_db_check(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);

    let fail_closed_json = |error_code: &'static str, failure_reason: String| -> Result<()> {
        let result = serde_json::json!({
            "db_path": db_path,
            "repo_provenance": repo_provenance_json(&repo),
            "ok": false,
            "health_class": error_code,
            "recovery_mode": "block_only",
            "quarantine_path": serde_json::Value::Null,
            "rebuild_result": "blocked",
            "failure_reason": failure_reason,
            "error_code": error_code,
            "message": graph_health_error_message(error_code),
            "suggestions": graph_health_error_suggestions(error_code),
            "integrity_issues": serde_json::Value::Null,
            "orphan_node_count": 0,
            "dangling_edge_count": 0,
            "reachability_provenance": {
                "ok": serde_json::Value::Null,
                "invalid_count": 0,
                "invalid_edges": [],
            },
            "session_db": {
                "path": atlas_engine::paths::session_db_path(&db_path),
                "exists": std::path::Path::new(&atlas_engine::paths::session_db_path(&db_path)).exists(),
                "ok": serde_json::Value::Null,
                "memory_schema": {
                    "ok": serde_json::Value::Null,
                    "issues": []
                }
            },
        });
        print_json("db_check", result)?;
        std::process::exit(1);
    };

    let store = match Store::open(&db_path) {
        Ok(store) => store,
        Err(error) => {
            if cli.json {
                return fail_closed_json(
                    classify_graph_store_error(&error.to_string()).as_str(),
                    format!("cannot open database at {db_path}: {error}"),
                );
            }
            return Err(error).with_context(|| format!("cannot open database at {db_path}"));
        }
    };

    let issues = match store.integrity_check() {
        Ok(issues) => issues,
        Err(error) => {
            if cli.json {
                return fail_closed_json(
                    classify_graph_store_error(&error.to_string()).as_str(),
                    format!("integrity check failed: {error}"),
                );
            }
            return Err(error).context("integrity check failed");
        }
    };

    const ORPHAN_LIMIT: usize = 100;
    let orphans = structural_orphans(&store, ORPHAN_LIMIT);
    let dangling = structural_dangling_edges(&store, ORPHAN_LIMIT);
    let source_repo_id = stable_repo_id(Utf8Path::new(&repo));
    let invalid_reachability = store.invalid_reachability_edges(&source_repo_id, ORPHAN_LIMIT)?;
    let reachability_provenance_ok = invalid_reachability.is_empty();

    // ICM-A1 + ICM-C1: validate the continuity-side memory and feedback
    // schemas through db check. A missing session DB is not a schema violation
    // (created on demand), but a present one must carry the full schemas.
    let session_db_path = atlas_engine::paths::session_db_path(&db_path);
    let session_db_exists = Path::new(&session_db_path).exists();
    let (
        session_db_ok,
        memory_schema_ok,
        memory_schema_issues,
        feedback_schema_ok,
        feedback_schema_issues,
        overview_schema_ok,
        overview_schema_issues,
        experience_schema_ok,
        experience_schema_issues,
    ) = if session_db_exists {
        match SessionStore::open(&session_db_path) {
            Ok(session_store) => {
                let memory_issues = session_store.memory_schema_issues();
                let feedback_issues = session_store.feedback_schema_issues();
                let mut overview_issues = session_store.overview_schema_issues();
                let experience_issues = session_store.experience_schema_issues();
                let content_db = atlas_engine::paths::content_db_path(&db_path);
                if Path::new(&content_db).exists()
                    && let Ok(content_store) = ContentStore::open(&content_db)
                {
                    for (overview_id, source_id) in session_store.overview_source_ids() {
                        if content_store
                            .get_source(&source_id)
                            .ok()
                            .flatten()
                            .is_none()
                        {
                            overview_issues.push(format!(
                                "orphan overview source_id: id={overview_id} source_id={source_id}"
                            ));
                        }
                    }
                }
                (
                    true,
                    memory_issues.is_empty(),
                    memory_issues,
                    feedback_issues.is_empty(),
                    feedback_issues,
                    overview_issues.is_empty(),
                    overview_issues,
                    experience_issues.is_empty(),
                    experience_issues,
                )
            }
            Err(e) => (
                false,
                false,
                vec![format!("cannot open session store: {e}")],
                false,
                vec![format!("cannot open session store: {e}")],
                false,
                vec![format!("cannot open session store: {e}")],
                false,
                vec![format!("cannot open session store: {e}")],
            ),
        }
    } else {
        (
            true,
            true,
            Vec::new(),
            true,
            Vec::new(),
            true,
            Vec::new(),
            true,
            Vec::new(),
        )
    };

    let ok = issues.is_empty()
        && orphans.is_empty()
        && dangling.is_empty()
        && reachability_provenance_ok
        && session_db_ok
        && memory_schema_ok
        && feedback_schema_ok
        && overview_schema_ok
        && experience_schema_ok;
    let error_code = if !issues.is_empty() || !orphans.is_empty() || !dangling.is_empty() {
        integrity_issue_code(&issues, !orphans.is_empty() || !dangling.is_empty())
    } else if !reachability_provenance_ok {
        "logical_inconsistency"
    } else if !memory_schema_ok
        || !feedback_schema_ok
        || !overview_schema_ok
        || !experience_schema_ok
    {
        "schema_mismatch"
    } else {
        "none"
    };
    let health_class = match error_code {
        "none" => Some("healthy"),
        "schema_mismatch" => Some("schema_mismatch"),
        "sqlite_corrupt" => Some("sqlite_corrupt"),
        "logical_inconsistency" => Some("logical_inconsistency"),
        _ => None,
    };

    if cli.json {
        let (graph_probe, content_probe, session_probe) =
            atlas_engine::doctor::probe_all_schemas(&db_path);
        let result = serde_json::json!({
            "db_path": db_path,
            "repo_provenance": repo_provenance_json(&repo),
            "ok": ok,
            "health_class": health_class,
            "schema_version": {
                "graph": graph_probe,
                "content": content_probe,
                "session": session_probe,
            },
            "recovery_mode": "block_only",
            "quarantine_path": serde_json::Value::Null,
            "rebuild_result": if ok { "not_needed" } else { "blocked" },
            "failure_reason": serde_json::Value::Null,
            "error_code": error_code,
            "message": graph_health_error_message(error_code),
            "suggestions": graph_health_error_suggestions(error_code),
            "integrity_issues": issues,
            "orphan_node_count": orphans.len(),
            "dangling_edge_count": dangling.len(),
            "reachability_provenance": {
                "ok": reachability_provenance_ok,
                "invalid_count": invalid_reachability.len(),
                "invalid_edges": invalid_reachability
                    .iter()
                    .map(|(id, value)| serde_json::json!({
                        "edge_id": id,
                        "reachability": value,
                    }))
                    .collect::<Vec<_>>(),
            },
            "session_db": {
                "path": session_db_path,
                "exists": session_db_exists,
                "ok": session_db_ok,
                "memory_schema": {
                    "ok": memory_schema_ok,
                    "issues": memory_schema_issues,
                },
                "feedback_schema": {
                    "ok": feedback_schema_ok,
                    "issues": feedback_schema_issues,
                },
                "overview_schema": {
                    "ok": overview_schema_ok,
                    "issues": overview_schema_issues,
                },
                "experience_schema": {
                    "ok": experience_schema_ok,
                    "issues": experience_schema_issues,
                },
            },
        });
        print_json("db_check", result)?;
    } else if ok {
        println!("Database integrity OK: {db_path}");
        if session_db_exists {
            println!("Session DB OK: {session_db_path}");
        } else {
            println!("Session DB not present: {session_db_path}");
        }
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
        if !memory_schema_ok {
            eprintln!("Memory schema FAILED: {session_db_path}");
            for issue in &memory_schema_issues {
                eprintln!("  {issue}");
            }
        }
        if !feedback_schema_ok {
            eprintln!("Feedback schema FAILED: {session_db_path}");
            for issue in &feedback_schema_issues {
                eprintln!("  {issue}");
            }
        }
        if !overview_schema_ok {
            eprintln!("Overview schema FAILED: {session_db_path}");
            for issue in &overview_schema_issues {
                eprintln!("  {issue}");
            }
        }
        if !reachability_provenance_ok {
            eprintln!(
                "Reachability provenance FAILED: {} invalid edge value(s)",
                invalid_reachability.len()
            );
            for (edge_id, value) in invalid_reachability.iter().take(10) {
                eprintln!("  edge {edge_id}: reachability={value}");
            }
        }
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests;
