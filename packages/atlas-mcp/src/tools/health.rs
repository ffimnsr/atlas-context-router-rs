use anyhow::Result;
use camino::Utf8Path;
use serde::Serialize;
use serde_json::json;
use std::sync::OnceLock;
use std::time::Instant;

use crate::tool_result::{ToolSuccessEnvelope, normalized_tool_result_value};

use super::health_retrieval::collect_retrieval_index_summary;
use super::shared::{
    derive_graph_readiness, error_code_docs, error_message, error_suggestions, failure_category,
    graph_issue_code, pending_graph_relevant_changes, tool_result_value,
};

pub(super) fn structural_orphans(
    store: &atlas_store_sqlite::Store,
    limit: usize,
) -> Vec<atlas_core::Node> {
    store.structural_orphan_nodes(limit).unwrap_or_default()
}

pub(super) fn structural_dangling_edges(
    store: &atlas_store_sqlite::Store,
    limit: usize,
) -> Vec<(i64, String, String, String, &'static str)> {
    store.structural_dangling_edges(limit).unwrap_or_default()
}

pub(super) fn integrity_issue_code(issues: &[String], structural_problem: bool) -> &'static str {
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

/// Records the first time any `broker_status` call is made.  Used to compute
/// uptime.  The transport layer calls [`mark_server_started`] at startup so
/// the timestamp is set before the first client request arrives.
static STARTED_AT: OnceLock<Instant> = OnceLock::new();

/// Mark the MCP server as started.  Call this once when the server begins
/// accepting requests.  Idempotent.
pub fn mark_server_started() {
    STARTED_AT.get_or_init(Instant::now);
}
/// `broker_status` — lightweight health/ready check for the MCP broker process.
///
/// This is intentionally separate from [`tool_status`] (which reports graph
/// readiness).  Use `broker_status` to verify that the broker itself is alive
/// and accepting requests, independently of whether a graph has been built.
pub(super) fn tool_broker_status(
    repo_root: &str,
    db_path: &str,
    worker_threads: usize,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let handler_start = Instant::now();
    let uptime_secs = STARTED_AT.get_or_init(Instant::now).elapsed().as_secs();

    let mut response = serde_json::json!({
        "ok": true,
        "pid": std::process::id(),
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": uptime_secs,
        "worker_threads_configured": worker_threads,
        "repo_root": repo_root,
        "db_path": db_path,
    });
    // Handler-only latency excludes transport framing and client round-trip.
    // Recorded so operators can distinguish broker overhead from host noise.
    if let Some(object) = response.as_object_mut() {
        object.insert(
            "handler_duration_us".to_owned(),
            serde_json::json!(handler_start.elapsed().as_micros() as u64),
        );
    }

    tool_result_value(&response, output_format)
}

pub(super) fn tool_status(
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let db_exists = std::path::Path::new(db_path).exists();
    let mut db_open_error: Option<String> = None;
    let mut graph_query_error: Option<String> = None;

    let store = if db_exists {
        match atlas_store_sqlite::Store::open(db_path) {
            Ok(store) => Some(store),
            Err(error) => {
                db_open_error = Some(error.to_string());
                None
            }
        }
    } else {
        None
    };

    let (node_count, edge_count, file_count, last_indexed_at) = if let Some(store) = store.as_ref()
    {
        match store.graph_stats() {
            Ok(stats) => (
                stats.node_count,
                stats.edge_count,
                stats.file_count,
                stats.last_indexed_at,
            ),
            Err(error) => {
                graph_query_error = Some(error.to_string());
                (0, 0, 0, None)
            }
        }
    } else {
        (0, 0, 0, None)
    };

    let build_status = if let Some(store) = store.as_ref() {
        match store.get_build_status(repo_root) {
            Ok(status) => status,
            Err(error) => {
                graph_query_error.get_or_insert_with(|| error.to_string());
                None
            }
        }
    } else {
        None
    };

    let build_state_str = build_status.as_ref().map(|bs| match bs.state {
        atlas_store_sqlite::GraphBuildState::Building => "building",
        atlas_store_sqlite::GraphBuildState::Built => "built",
        atlas_store_sqlite::GraphBuildState::Degraded => "degraded",
        atlas_store_sqlite::GraphBuildState::BuildFailed => "build_failed",
    });

    let graph_built = build_state_str == Some("built")
        || (build_state_str.is_none()
            && graph_query_error.is_none()
            && db_open_error.is_none()
            && (node_count > 0 || edge_count > 0 || file_count > 0));
    let pending_graph_changes = if graph_built {
        pending_graph_relevant_changes(repo_root, db_path).unwrap_or_default()
    } else {
        Vec::new()
    };
    let stale_index = !pending_graph_changes.is_empty();
    let retrieval_index = collect_retrieval_index_summary(repo_root, db_path);
    let retrieval_unavailable = graph_built
        && (!retrieval_index.available
            || !retrieval_index.searchable
            || retrieval_index.state.as_deref() != Some("indexed"));
    let readiness = if let Some(store) = store.as_ref() {
        derive_graph_readiness(store, repo_root, db_path)
    } else {
        use atlas_core::{GraphReadiness, GraphReadinessInput};
        GraphReadiness::derive(GraphReadinessInput {
            repo_root,
            db_path,
            db_exists,
            db_open_error: db_open_error.as_deref(),
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
    };
    let category = failure_category(
        db_exists,
        graph_built,
        readiness.health_class,
        build_state_str,
        retrieval_unavailable,
    );
    let ok = category == "none" && graph_built;
    let execution_state = readiness.execution_state;

    let (postprocess_block, postprocess_warnings) = match store.as_ref() {
        Some(store) => super::health_postprocess::status_postprocess_block(store, repo_root),
        None => (serde_json::Value::Null, Vec::new()),
    };
    let parser_fingerprint =
        super::health_parser_fingerprint::status_block(store.as_ref(), repo_root);

    let mut warnings = Vec::new();
    if stale_index {
        warnings.push(format!(
            "graph index is stale for {} graph-relevant file(s)",
            pending_graph_changes.len()
        ));
    }
    if let Some(error) = db_open_error.clone() {
        warnings.push(format!("database open failed: {error}"));
    }
    if let Some(error) = graph_query_error.clone() {
        warnings.push(format!("graph query failed: {error}"));
    }
    if let Some(error) = retrieval_index.last_error.clone() {
        warnings.push(format!("retrieval index unavailable: {error}"));
    }
    warnings.extend(postprocess_warnings);

    let build_status_value = build_status.as_ref().map(|bs| {
        json!({
            "state": build_state_str,
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
    let rebuild_result = if quarantine_path.is_some() {
        match build_state_str {
            Some("build_failed") => "rebuild_failed",
            Some("built") | Some("degraded") => "rebuilt_fresh",
            _ => "blocked",
        }
    } else {
        "not_needed"
    };

    let payload = json!({
        "postprocess": postprocess_block,
        "node_count": node_count,
        "edge_count": edge_count,
        "last_indexed_at": last_indexed_at,
        "health_class": readiness.health_class.map(|class| class.as_str()),
        "recovery_mode": "block_only",
        "quarantine_path": quarantine_path,
        "rebuild_result": rebuild_result,
        "failure_reason": build_status.as_ref().and_then(|status| status.last_error.clone()),
        "graph_state": {
            "graph_built": graph_built,
            "build_state": build_state_str,
            "build_last_error": build_status.as_ref().and_then(|bs| bs.last_error.clone()),
            "build_budget_stop_reason": build_status.as_ref().and_then(|bs| bs.budget_stop_reason.clone()),
            "stale_index": stale_index,
            "pending_graph_change_count": pending_graph_changes.len(),
            "pending_graph_changes": pending_graph_changes,
            "execution_state": execution_state.as_str(),
            "parser_fingerprint": parser_fingerprint,
            "connection_mode": "single_connection_per_store",
            "read_pool_active": false,
        },
        "db_state": {
            "path": db_path,
            "exists": db_exists,
            "open_ok": store.is_some(),
            "open_error": db_open_error,
            "query_error": graph_query_error,
            "build_status": build_status_value,
        },
        "indexed_file_count": file_count,
        "failure_category": category,
        "ready": ok,
        "ready_scope": "retrieval",
        "safe_for_symbol_lookup": matches!(execution_state, atlas_core::GraphExecutionState::Fresh | atlas_core::GraphExecutionState::Stale),
        "safe_for_analysis": matches!(execution_state, atlas_core::GraphExecutionState::Fresh),
        "retrieval_index": retrieval_index,
        "summary": {
            "health_class": readiness.health_class.map(|class| class.as_str()),
            "message": error_message(category),
            "suggestions": error_suggestions(category),
            "mcp_suggestions": mcp_graph_recovery_suggestions(category),
            "error_code_docs": error_code_docs(category),
        },
        "warnings": warnings,
    });

    let envelope = ToolSuccessEnvelope::new("status", payload);
    normalized_tool_result_value(&envelope, output_format)
}

pub(super) fn mcp_graph_recovery_suggestions(category: &str) -> Vec<&'static str> {
    match category {
        "missing_graph_db"
        | "noncanonical_path_rows"
        | "schema_mismatch"
        | "sqlite_corrupt"
        | "logical_inconsistency"
        | "corrupt_or_inconsistent_graph_rows"
        | "interrupted_build"
        | "degraded_build"
        | "failed_build" => vec!["call build_graph with {}"],
        "stale_index" => {
            vec!["call update_graph with {\"change_source\":{\"kind\":\"working_tree\"}}"]
        }
        "retrieval_index_unavailable" => vec![
            "call build_graph with {} for a full refresh",
            "or call update_graph with {\"change_source\":{\"kind\":\"working_tree\"}} after edits",
        ],
        _ => Vec::new(),
    }
}

pub(super) fn tool_doctor(
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    use atlas_contentstore::{ContentStore, IndexState};
    use atlas_repo::{collect_files, find_repo_root};
    use atlas_store_sqlite::GraphBuildState;

    #[derive(Serialize)]
    struct LegacyCheckItem {
        check: &'static str,
        label: &'static str,
        ok: bool,
        detail: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        issue_code: Option<&'static str>,
        noncanonical_path_rows: Option<usize>,
    }

    fn check_label(check: &'static str) -> &'static str {
        match check {
            "repo_root" => "atlas_scope",
            other => other,
        }
    }

    fn fix_hint_for(issue_code: Option<&str>, name: &str) -> Option<String> {
        match (issue_code, name) {
            (Some("stale_index"), _) => Some(
                "run update_graph with change_source=working_tree to refresh graph state"
                    .to_owned(),
            ),
            (Some("retrieval_index_unavailable"), _) => {
                Some("rebuild retrieval index after graph build completes".to_owned())
            }
            (Some("noncanonical_path_rows"), _) => {
                Some("rebuild graph/content state from canonical repo paths".to_owned())
            }
            (Some("failed_build"), _)
            | (Some("degraded_build"), _)
            | (Some("interrupted_build"), _) => {
                Some("rerun build_graph and inspect build warnings".to_owned())
            }
            (Some("parser_fingerprint_missing"), _)
            | (Some("parser_fingerprint_stale"), _) => Some(
                "call build_graph with {} to rebuild the graph with the current parser configuration"
                    .to_owned(),
            ),
            (None, "config_file") => Some("run atlas init to create .atlas/config.toml".to_owned()),
            (None, "db_file") => Some("run atlas init then build_graph".to_owned()),
            _ => None,
        }
    }

    macro_rules! pass {
        ($name:expr, $detail:expr) => {
            LegacyCheckItem {
                check: $name,
                label: check_label($name),
                ok: true,
                detail: $detail.into(),
                issue_code: None,
                noncanonical_path_rows: None,
            }
        };
    }
    macro_rules! fail {
        ($name:expr, $detail:expr) => {
            LegacyCheckItem {
                check: $name,
                label: check_label($name),
                ok: false,
                detail: $detail.into(),
                issue_code: None,
                noncanonical_path_rows: None,
            }
        };
        ($name:expr, $detail:expr, $issue_code:expr) => {
            LegacyCheckItem {
                check: $name,
                label: check_label($name),
                ok: false,
                detail: $detail.into(),
                issue_code: Some($issue_code),
                noncanonical_path_rows: None,
            }
        };
    }

    let mut checks: Vec<LegacyCheckItem> = Vec::new();

    checks.push(pass!("repo_root", repo_root));

    match find_repo_root(Utf8Path::new(repo_root)) {
        Ok(root) => checks.push(pass!("git_root", root.as_str())),
        Err(e) => checks.push(fail!("git_root", e.to_string())),
    }

    let atlas_dir = atlas_engine::paths::atlas_dir(repo_root);
    if atlas_dir.exists() {
        checks.push(pass!("atlas_dir", atlas_dir.display().to_string()));
    } else {
        checks.push(fail!(
            "atlas_dir",
            format!("{} not found — run `atlas init`", atlas_dir.display())
        ));
    }

    let config_path = atlas_engine::paths::config_path(repo_root);
    if config_path.exists() {
        checks.push(pass!("config_file", config_path.display().to_string()));
        match atlas_engine::Config::load(&atlas_dir) {
            Ok(cfg) => checks.push(pass!(
                "mcp_serve_config",
                format!(
                    "workers={} timeout_ms={}",
                    cfg.mcp_worker_threads(),
                    cfg.mcp_tool_timeout_ms()
                )
            )),
            Err(e) => checks.push(fail!("mcp_serve_config", e.to_string())),
        }
    } else {
        checks.push(fail!(
            "config_file",
            format!("{} not found — run `atlas init`", config_path.display())
        ));
    }

    let db_exists = std::path::Path::new(db_path).exists();
    if db_exists {
        checks.push(pass!("db_file", db_path));
    } else {
        checks.push(fail!(
            "db_file",
            format!("{db_path} not found — run `atlas init`")
        ));
    }

    if db_exists {
        match atlas_store_sqlite::Store::open(db_path) {
            Ok(store) => {
                checks.push(pass!("db_open", db_path));
                match store.integrity_check() {
                    Ok(issues) if issues.is_empty() => checks.push(pass!("db_integrity", "ok")),
                    Ok(issues) => checks.push(fail!(
                        "db_integrity",
                        issues.join("; "),
                        integrity_issue_code(&issues, false)
                    )),
                    Err(e) => checks.push(fail!(
                        "db_integrity",
                        e.to_string(),
                        graph_issue_code(&e.to_string())
                    )),
                }
                match store.graph_stats() {
                    Ok(st) => checks.push(pass!(
                        "graph_stats",
                        format!(
                            "files={} nodes={} edges={}",
                            st.file_count, st.node_count, st.edge_count
                        )
                    )),
                    Err(e) => checks.push(fail!(
                        "graph_stats",
                        e.to_string(),
                        graph_issue_code(&e.to_string())
                    )),
                }
                match store.get_build_status(repo_root) {
                    Ok(Some(bs)) => {
                        let (state_str, is_ok) = match bs.state {
                            GraphBuildState::Built => ("built", true),
                            GraphBuildState::Degraded => ("degraded", false),
                            GraphBuildState::Building => ("building (interrupted?)", false),
                            GraphBuildState::BuildFailed => ("build_failed", false),
                        };
                        let detail = if is_ok {
                            // Build-time telemetry, not the live graph totals:
                            // `graph_stats` is the canonical live count.
                            format!(
                                "state={state_str} nodes_written_last_build={} edges_written_last_build={}",
                                bs.nodes_written, bs.edges_written
                            )
                        } else if let Some(err) = bs.last_error {
                            format!("state={state_str} error={err}")
                        } else {
                            format!("state={state_str}")
                        };
                        if is_ok {
                            checks.push(pass!("graph_build_state", detail));
                        } else {
                            let issue_code = match bs.state {
                                GraphBuildState::Building => "interrupted_build",
                                GraphBuildState::Degraded => "degraded_build",
                                _ => "failed_build",
                            };
                            checks.push(fail!("graph_build_state", detail, issue_code));
                        }
                    }
                    Ok(None) => checks.push(pass!("graph_build_state", "no build recorded yet")),
                    Err(e) => checks.push(fail!(
                        "graph_build_state",
                        e.to_string(),
                        graph_issue_code(&e.to_string())
                    )),
                }
                if let Some(check) =
                    super::health_parser_fingerprint::doctor_check(repo_root, db_path)
                {
                    if check.ok {
                        checks.push(pass!("parser_fingerprint", check.detail));
                    } else {
                        let issue_code = check.issue_code.unwrap_or("parser_fingerprint_missing");
                        checks.push(fail!("parser_fingerprint", check.detail, issue_code));
                    }
                }
            }
            Err(e) => checks.push(fail!(
                "db_open",
                e.to_string(),
                graph_issue_code(&e.to_string())
            )),
        }
    }

    match collect_files(Utf8Path::new(repo_root), None) {
        Ok(files) => checks.push(pass!(
            "git_ls_files",
            format!("{} tracked files", files.len())
        )),
        Err(e) => checks.push(fail!("git_ls_files", e.to_string())),
    }

    let freshness_files = pending_graph_relevant_changes(repo_root, db_path).unwrap_or_default();
    if freshness_files.is_empty() {
        checks.push(pass!("graph_freshness", "up_to_date"));
    } else {
        let preview = freshness_files
            .iter()
            .take(5)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let detail = if freshness_files.len() > 5 {
            format!(
                "{} pending graph-relevant files: {} (+{} more)",
                freshness_files.len(),
                preview,
                freshness_files.len() - 5
            )
        } else {
            format!(
                "{} pending graph-relevant files: {}",
                freshness_files.len(),
                preview
            )
        };
        checks.push(fail!("graph_freshness", detail, "stale_index"));
    }

    {
        let content_db = atlas_engine::paths::content_db_path(db_path);
        match ContentStore::open(&content_db) {
            Ok(mut cs) => {
                let _ = cs.migrate();
                match cs.get_index_status(repo_root) {
                    Ok(Some(status)) => {
                        let state_str = match status.state {
                            IndexState::Indexed => "indexed",
                            IndexState::Indexing => "indexing (interrupted?)",
                            IndexState::IndexFailed => "index_failed",
                        };
                        let ok = status.state == IndexState::Indexed;
                        let detail = if ok {
                            format!(
                                "state={state_str} files={} chunks={}",
                                status.files_indexed, status.chunks_written
                            )
                        } else if let Some(err) = status.last_error {
                            format!("state={state_str} error={err}")
                        } else {
                            format!("state={state_str}")
                        };
                        if ok {
                            checks.push(pass!("retrieval_index", detail));
                        } else {
                            checks.push(fail!(
                                "retrieval_index",
                                detail,
                                "retrieval_index_unavailable"
                            ));
                        }
                    }
                    Ok(None) => checks.push(fail!(
                        "retrieval_index",
                        "no index run recorded yet",
                        "retrieval_index_unavailable"
                    )),
                    Err(e) => checks.push(fail!(
                        "retrieval_index",
                        e.to_string(),
                        "retrieval_index_unavailable"
                    )),
                }
            }
            Err(_) => checks.push(fail!(
                "retrieval_index",
                "content store not initialised",
                "retrieval_index_unavailable"
            )),
        }
    }

    // Shared schema/canonicality checks. Produced by
    // `atlas-engine::doctor` so CLI and MCP `doctor` cannot drift.
    for check in atlas_engine::doctor::shared_checks(db_path) {
        checks.push(if check.ok {
            LegacyCheckItem {
                check: check.name,
                label: check_label(check.name),
                ok: true,
                detail: check.message,
                issue_code: None,
                noncanonical_path_rows: check.details.noncanonical_path_rows,
            }
        } else {
            LegacyCheckItem {
                check: check.name,
                label: check_label(check.name),
                ok: false,
                detail: check.message,
                issue_code: check.issue_code,
                noncanonical_path_rows: check.details.noncanonical_path_rows,
            }
        });
    }

    let ok_count = checks.iter().filter(|c| c.ok).count();
    let fail_count = checks.len().saturating_sub(ok_count);
    let warnings = checks
        .iter()
        .filter(|check| !check.ok)
        .filter_map(|check| {
            check
                .issue_code
                .map(|issue| format!("{}: {issue}", check.check))
        })
        .collect::<Vec<_>>();
    let normalized_checks = checks
        .iter()
        .map(|check| {
            json!({
                "name": check.check,
                "check": check.check,
                "status": if check.ok { "pass" } else { "fail" },
                "ok": check.ok,
                "message": &check.detail,
                "detail": &check.detail,
                "issue_code": check.issue_code,
                "details": {
                    "label": check.label,
                    "ok": check.ok,
                    "issue_code": check.issue_code,
                    "noncanonical_path_rows": check.noncanonical_path_rows,
                },
                "fix_hint": fix_hint_for(check.issue_code, check.check),
            })
        })
        .collect::<Vec<_>>();
    let overall_status = if fail_count == 0 { "ok" } else { "fail" };
    let readiness = if db_exists {
        match atlas_store_sqlite::Store::open(db_path) {
            Ok(store) => derive_graph_readiness(&store, repo_root, db_path),
            Err(error) => {
                use atlas_core::{GraphReadiness, GraphReadinessInput};
                GraphReadiness::derive(GraphReadinessInput {
                    repo_root,
                    db_path,
                    db_exists,
                    db_open_error: Some(&error.to_string()),
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
        }
    } else {
        use atlas_core::{GraphReadiness, GraphReadinessInput};
        GraphReadiness::derive(GraphReadinessInput {
            repo_root,
            db_path,
            db_exists,
            db_open_error: Some("db not found"),
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
    };

    let reachability_measurement = if db_exists {
        use atlas_repo::stable_repo_id;
        atlas_store_sqlite::Store::open(db_path)
            .ok()
            .and_then(|store| {
                let source_repo_id = stable_repo_id(Utf8Path::new(repo_root));
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
        use atlas_repo::stable_repo_id;
        atlas_store_sqlite::Store::open(db_path)
            .ok()
            .and_then(|store| {
                let source_repo_id = stable_repo_id(Utf8Path::new(repo_root));
                let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
                    .map(|config| config.reachability)
                    .unwrap_or_default();
                atlas_engine::diagnose_reachability(&store, &source_repo_id, &config).ok()
            })
    } else {
        None
    };

    let mut payload = json!({
        "overall_status": overall_status,
        "health_class": readiness.health_class.map(|class| class.as_str()),
        "recovery_mode": "block_only",
        "quarantine_path": serde_json::Value::Null,
        "rebuild_result": if fail_count == 0 { "not_needed" } else { "blocked" },
        "failure_reason": serde_json::Value::Null,
        "checks": normalized_checks,
        "summary": {
            "total_count": checks.len(),
            "pass_count": ok_count,
            "fail_count": fail_count,
            "health_class": readiness.health_class.map(|class| class.as_str()),
            "message": error_message(if fail_count == 0 { "none" } else { "checks_failed" }),
            "suggestions": error_suggestions(if fail_count == 0 { "none" } else { "checks_failed" }),
            "error_code_docs": error_code_docs(if fail_count == 0 { "none" } else { "checks_failed" }),
        },
        "warnings": warnings,
    });

    if let Some(measurement) = reachability_measurement {
        payload["reachability_measurement"] =
            serde_json::to_value(measurement).unwrap_or(serde_json::Value::Null);
    }
    if let Some(summary) = reachability_summary {
        payload["reachability"] = serde_json::to_value(summary).unwrap_or(serde_json::Value::Null);
    }

    let envelope = ToolSuccessEnvelope::new("doctor", payload);
    normalized_tool_result_value(&envelope, output_format)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::NodeKind;
    use atlas_core::{Edge, EdgeKind, Node, NodeId};

    fn open_store() -> atlas_store_sqlite::Store {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test.sqlite");
        atlas_store_sqlite::Store::open(db_path.to_str().unwrap()).unwrap()
    }

    fn source_node(qn: &str) -> Node {
        Node {
            id: NodeId::UNSET,
            kind: NodeKind::Function,
            name: "source".to_owned(),
            qualified_name: qn.to_owned(),
            file_path: "src/lib.rs".to_owned(),
            line_start: 1,
            line_end: 1,
            language: "rust".to_owned(),
            parent_name: Some("src/lib.rs".to_owned()),
            params: Some("()".to_owned()),
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "hash".to_owned(),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        }
    }

    fn dangling_edge(kind: EdgeKind, target_qn: &str) -> Edge {
        Edge {
            id: 0,
            kind,
            source_qn: "src/lib.rs::fn::source".to_owned(),
            target_qn: target_qn.to_owned(),
            file_path: "src/lib.rs".to_owned(),
            line: Some(1),
            confidence: 1.0,
            confidence_tier: None,
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        }
    }

    #[test]
    fn structural_dangling_edges_ignores_nonstructural_calls() {
        let mut store = open_store();
        store
            .replace_file_graph_for_repo(
                "repo_test",
                "src/lib.rs",
                "hash",
                Some("rust"),
                None,
                &[source_node("src/lib.rs::fn::source")],
                &[dangling_edge(EdgeKind::Calls, "Path::new")],
            )
            .unwrap();

        assert!(structural_dangling_edges(&store, 100).is_empty());
    }

    #[test]
    fn structural_dangling_edges_keeps_structural_contains() {
        let mut store = open_store();
        store
            .replace_file_graph_for_repo(
                "repo_test",
                "src/lib.rs",
                "hash",
                Some("rust"),
                None,
                &[source_node("src/lib.rs::fn::source")],
                &[dangling_edge(EdgeKind::Contains, "src/lib.rs::fn::missing")],
            )
            .unwrap();

        let dangling = structural_dangling_edges(&store, 100);
        assert_eq!(dangling.len(), 1);
        assert_eq!(dangling[0].3, "contains");
    }

    #[test]
    fn broker_status_reports_supplied_worker_thread_count() {
        let response = tool_broker_status("repo", "db", 7, crate::output::OutputFormat::Json)
            .expect("broker status");
        assert_eq!(
            response["structuredContent"]["worker_threads_configured"],
            7
        );
    }
}
