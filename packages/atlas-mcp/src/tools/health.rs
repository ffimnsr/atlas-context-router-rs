use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::Serialize;

use super::shared::{
    error_message, error_suggestions, failure_category, graph_issue_code, open_store,
    pending_graph_relevant_changes, tool_result_value, u64_arg,
};

#[derive(Serialize)]
struct RetrievalIndexSummary {
    available: bool,
    searchable: bool,
    state: Option<String>,
    files_discovered: i64,
    files_indexed: i64,
    chunks_written: i64,
    chunks_reused: i64,
    last_indexed_at: Option<String>,
    last_error: Option<String>,
    content_db_path: String,
}

fn collect_retrieval_index_summary(repo_root: &str, db_path: &str) -> RetrievalIndexSummary {
    use atlas_contentstore::{ContentStore, IndexState};

    let content_db_path = atlas_engine::paths::content_db_path(db_path);
    match ContentStore::open(&content_db_path) {
        Ok(mut store) => {
            let _ = store.migrate();
            match store.get_index_status(repo_root) {
                Ok(Some(status)) => {
                    let searchable = status.state == IndexState::Indexed;
                    let state = match status.state {
                        IndexState::Indexed => "indexed",
                        IndexState::Indexing => "indexing",
                        IndexState::IndexFailed => "index_failed",
                    };
                    RetrievalIndexSummary {
                        available: true,
                        searchable,
                        state: Some(state.to_owned()),
                        files_discovered: status.files_discovered,
                        files_indexed: status.files_indexed,
                        chunks_written: status.chunks_written,
                        chunks_reused: status.chunks_reused,
                        last_indexed_at: status.last_indexed_at,
                        last_error: status.last_error,
                        content_db_path,
                    }
                }
                Ok(None) => RetrievalIndexSummary {
                    available: false,
                    searchable: false,
                    state: None,
                    files_discovered: 0,
                    files_indexed: 0,
                    chunks_written: 0,
                    chunks_reused: 0,
                    last_indexed_at: None,
                    last_error: Some(
                        "content store has no retrieval index state for this repo".to_owned(),
                    ),
                    content_db_path,
                },
                Err(error) => RetrievalIndexSummary {
                    available: false,
                    searchable: false,
                    state: None,
                    files_discovered: 0,
                    files_indexed: 0,
                    chunks_written: 0,
                    chunks_reused: 0,
                    last_indexed_at: None,
                    last_error: Some(error.to_string()),
                    content_db_path,
                },
            }
        }
        Err(error) => RetrievalIndexSummary {
            available: false,
            searchable: false,
            state: None,
            files_discovered: 0,
            files_indexed: 0,
            chunks_written: 0,
            chunks_reused: 0,
            last_indexed_at: None,
            last_error: Some(error.to_string()),
            content_db_path,
        },
    }
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
        match store.stats() {
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
    let graph_error = db_open_error.as_deref().or(graph_query_error.as_deref());
    let category = failure_category(
        db_exists,
        graph_error,
        build_state_str,
        stale_index,
        retrieval_unavailable,
    );
    let ok = category == "none" && graph_built;

    let result = serde_json::json!({
        "ok": ok,
        "error_code": category,
        "message": error_message(category),
        "suggestions": error_suggestions(category),
        "repo_root": repo_root,
        "db_path": db_path,
        "db_exists": db_exists,
        "db_open_error": db_open_error,
        "graph_query_error": graph_query_error,
        "graph_built": graph_built,
        "build_state": build_state_str,
        "build_last_error": build_status.as_ref().and_then(|bs| bs.last_error.as_deref()),
        "node_count": node_count,
        "edge_count": edge_count,
        "file_count": file_count,
        "last_indexed_at": last_indexed_at,
        "stale_index": stale_index,
        "pending_graph_change_count": pending_graph_changes.len(),
        "pending_graph_changes": pending_graph_changes,
        "retrieval_index": retrieval_index,
    });

    tool_result_value(&result, output_format)
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
    struct CheckItem {
        check: &'static str,
        ok: bool,
        detail: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        issue_code: Option<&'static str>,
    }

    macro_rules! pass {
        ($name:expr, $detail:expr) => {
            CheckItem {
                check: $name,
                ok: true,
                detail: $detail.into(),
                issue_code: None,
            }
        };
    }
    macro_rules! fail {
        ($name:expr, $detail:expr) => {
            CheckItem {
                check: $name,
                ok: false,
                detail: $detail.into(),
                issue_code: None,
            }
        };
        ($name:expr, $detail:expr, $issue_code:expr) => {
            CheckItem {
                check: $name,
                ok: false,
                detail: $detail.into(),
                issue_code: Some($issue_code),
            }
        };
    }

    let mut checks: Vec<CheckItem> = Vec::new();

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
                        "corrupt_or_inconsistent_graph_rows"
                    )),
                    Err(e) => checks.push(fail!(
                        "db_integrity",
                        e.to_string(),
                        graph_issue_code(&e.to_string())
                    )),
                }
                match store.stats() {
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
                            GraphBuildState::Building => ("building (interrupted?)", false),
                            GraphBuildState::BuildFailed => ("build_failed", false),
                        };
                        let detail = if is_ok {
                            format!(
                                "state={state_str} nodes={} edges={}",
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
                            let issue_code = if matches!(bs.state, GraphBuildState::Building) {
                                "interrupted_build"
                            } else {
                                "failed_build"
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

    let all_ok = checks.iter().all(|c| c.ok);
    let ec = if all_ok { "none" } else { "checks_failed" };
    let result = serde_json::json!({
        "ok": all_ok,
        "error_code": ec,
        "message": error_message(ec),
        "suggestions": error_suggestions(ec),
        "checks": checks,
    });
    tool_result_value(&result, output_format)
}

pub(super) fn tool_db_check(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    const DEFAULT_LIMIT: usize = 100;
    let limit = u64_arg(args, "limit").unwrap_or(DEFAULT_LIMIT as u64) as usize;

    let store = open_store(db_path)?;
    let issues = store.integrity_check().context("integrity check failed")?;
    let orphans = store.orphan_nodes(limit).unwrap_or_default();
    let dangling = store.dangling_edges(limit).unwrap_or_default();

    let ok = issues.is_empty() && orphans.is_empty() && dangling.is_empty();
    let ec = if !issues.is_empty() {
        "corrupt_or_inconsistent_graph_rows"
    } else {
        "none"
    };

    #[derive(Serialize)]
    struct OrphanEntry<'a> {
        kind: &'a str,
        qualified_name: &'a str,
        file_path: &'a str,
        line_start: u32,
    }

    #[derive(Serialize)]
    struct DanglingEntry {
        id: i64,
        kind: String,
        source_qn: String,
        target_qn: String,
        missing_side: String,
    }

    let orphan_nodes: Vec<OrphanEntry<'_>> = orphans
        .iter()
        .map(|n| OrphanEntry {
            kind: n.kind.as_str(),
            qualified_name: &n.qualified_name,
            file_path: &n.file_path,
            line_start: n.line_start,
        })
        .collect();

    let dangling_edges: Vec<DanglingEntry> = dangling
        .iter()
        .map(|(id, src, tgt, kind, side)| DanglingEntry {
            id: *id,
            kind: kind.clone(),
            source_qn: src.clone(),
            target_qn: tgt.clone(),
            missing_side: side.to_string(),
        })
        .collect();

    let result = serde_json::json!({
        "ok": ok,
        "error_code": ec,
        "message": error_message(ec),
        "suggestions": error_suggestions(ec),
        "db_path": db_path,
        "integrity_issues": issues,
        "orphan_node_count": orphans.len(),
        "dangling_edge_count": dangling.len(),
        "orphan_nodes": orphan_nodes,
        "dangling_edges": dangling_edges,
    });

    tool_result_value(&result, output_format)
}

pub(super) fn tool_debug_graph(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    const DEFAULT_LIMIT: usize = 20;
    let limit = u64_arg(args, "limit").unwrap_or(DEFAULT_LIMIT as u64) as usize;

    let store = open_store(db_path)?;
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

    #[derive(Serialize)]
    struct OrphanEntry<'a> {
        kind: &'a str,
        qualified_name: &'a str,
        file_path: &'a str,
        line_start: u32,
    }

    #[derive(Serialize)]
    struct DanglingEntry {
        id: i64,
        kind: String,
        source_qn: String,
        target_qn: String,
        missing_side: String,
    }

    let orphan_nodes: Vec<OrphanEntry<'_>> = orphans
        .iter()
        .map(|n| OrphanEntry {
            kind: n.kind.as_str(),
            qualified_name: &n.qualified_name,
            file_path: &n.file_path,
            line_start: n.line_start,
        })
        .collect();

    let dangling_edges: Vec<DanglingEntry> = dangling
        .iter()
        .map(|(id, src, tgt, kind, side)| DanglingEntry {
            id: *id,
            kind: kind.clone(),
            source_qn: src.clone(),
            target_qn: tgt.clone(),
            missing_side: side.to_string(),
        })
        .collect();

    let result = serde_json::json!({
        "ok": true,
        "error_code": "none",
        "message": error_message("none"),
        "suggestions": error_suggestions("none"),
        "nodes": stats.node_count,
        "edges": stats.edge_count,
        "files": stats.file_count,
        "nodes_by_kind": stats.nodes_by_kind,
        "edges_by_kind": edge_kinds,
        "top_files_by_node_count": top_files,
        "orphan_node_count": orphans.len(),
        "dangling_edge_count": dangling.len(),
        "orphan_nodes": orphan_nodes,
        "dangling_edges": dangling_edges,
    });

    tool_result_value(&result, output_format)
}
