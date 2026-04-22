use anyhow::{Context, Result};
use camino::Utf8Path;
use serde::Serialize;

use super::shared::{
    error_message, error_suggestions, failure_category, open_store, tool_result_value, u64_arg,
};

pub(super) fn tool_status(
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let db_exists = std::path::Path::new(db_path).exists();
    let store_result = if db_exists {
        Some(atlas_store_sqlite::Store::open(db_path))
    } else {
        None
    };
    let db_open_ok = store_result.as_ref().map(|r| r.is_ok()).unwrap_or(false);
    let store = store_result.and_then(|r| r.ok());

    let (node_count, edge_count, file_count, last_indexed_at) = store
        .as_ref()
        .and_then(|s| s.stats().ok())
        .map(|st| {
            (
                st.node_count,
                st.edge_count,
                st.file_count,
                st.last_indexed_at,
            )
        })
        .unwrap_or((0, 0, 0, None));

    let build_status = store
        .as_ref()
        .and_then(|s| s.get_build_status(repo_root).ok().flatten());

    let build_state_str = build_status.as_ref().map(|bs| match bs.state {
        atlas_store_sqlite::GraphBuildState::Building => "building",
        atlas_store_sqlite::GraphBuildState::Built => "built",
        atlas_store_sqlite::GraphBuildState::BuildFailed => "build_failed",
    });

    let graph_built = build_state_str == Some("built");
    let category = failure_category(
        db_exists,
        db_open_ok,
        build_status.as_ref().map(|bs| &bs.state),
    );
    let ok = db_exists && db_open_ok && graph_built;

    let result = serde_json::json!({
        "ok": ok,
        "error_code": category,
        "message": error_message(category),
        "suggestions": error_suggestions(category),
        "repo_root": repo_root,
        "db_path": db_path,
        "db_exists": db_exists,
        "graph_built": graph_built,
        "build_state": build_state_str,
        "build_last_error": build_status.as_ref().and_then(|bs| bs.last_error.as_deref()),
        "node_count": node_count,
        "edge_count": edge_count,
        "file_count": file_count,
        "last_indexed_at": last_indexed_at,
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
    }

    macro_rules! pass {
        ($name:expr, $detail:expr) => {
            CheckItem {
                check: $name,
                ok: true,
                detail: $detail.into(),
            }
        };
    }
    macro_rules! fail {
        ($name:expr, $detail:expr) => {
            CheckItem {
                check: $name,
                ok: false,
                detail: $detail.into(),
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
                    Ok(issues) => checks.push(fail!("db_integrity", issues.join("; "))),
                    Err(e) => checks.push(fail!("db_integrity", e.to_string())),
                }
                match store.stats() {
                    Ok(st) => checks.push(pass!(
                        "graph_stats",
                        format!(
                            "files={} nodes={} edges={}",
                            st.file_count, st.node_count, st.edge_count
                        )
                    )),
                    Err(e) => checks.push(fail!("graph_stats", e.to_string())),
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
                            checks.push(fail!("graph_build_state", detail));
                        }
                    }
                    Ok(None) => checks.push(pass!("graph_build_state", "no build recorded yet")),
                    Err(e) => checks.push(fail!("graph_build_state", e.to_string())),
                }
            }
            Err(e) => checks.push(fail!("db_open", e.to_string())),
        }
    }

    match collect_files(Utf8Path::new(repo_root), None) {
        Ok(files) => checks.push(pass!(
            "git_ls_files",
            format!("{} tracked files", files.len())
        )),
        Err(e) => checks.push(fail!("git_ls_files", e.to_string())),
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
                            checks.push(fail!("retrieval_index", detail));
                        }
                    }
                    Ok(None) => checks.push(pass!("retrieval_index", "no index run recorded yet")),
                    Err(e) => checks.push(fail!("retrieval_index", e.to_string())),
                }
            }
            Err(_) => checks.push(pass!("retrieval_index", "content store not initialised")),
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
