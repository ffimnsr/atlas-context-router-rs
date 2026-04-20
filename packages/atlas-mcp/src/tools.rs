//! MCP tool definitions and dispatch.
//!
//! Each tool follows the MCP `tools/call` contract: receives `arguments` as
//! an optional JSON object and returns an MCP content envelope:
//! `{ "content": [{ "type": "text", "text": "<json>" }] }`.

use anyhow::{Context, Result};
use atlas_core::SearchQuery;
use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_repo::{DiffTarget, changed_files, find_repo_root};
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use serde::Serialize;

use crate::context::{compact_node, package_impact, package_review};

// ---------------------------------------------------------------------------
// Tool schema list
// ---------------------------------------------------------------------------

/// Return the MCP `tools/list` response body.
pub fn tool_list() -> serde_json::Value {
    serde_json::json!({
        "tools": [
            {
                "name": "list_graph_stats",
                "description": "Return node/edge counts and language breakdown for the indexed graph.",
                "inputSchema": {
                    "type": "object",
                    "properties": {},
                    "required": []
                }
            },
            {
                "name": "query_graph",
                "description": "Full-text search the code graph. Returns a compact, ranked list of matching symbols.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text":     { "type": "string",  "description": "Search query text" },
                        "kind":     { "type": "string",  "description": "Filter by node kind (e.g. 'function', 'struct')" },
                        "language": { "type": "string",  "description": "Filter by language (e.g. 'rust', 'python')" },
                        "limit":    { "type": "integer", "description": "Maximum results to return (default 20)" }
                    },
                    "required": ["text"]
                }
            },
            {
                "name": "get_impact_radius",
                "description": "Compute nodes and files affected when the given files change. Returns compact, capped results.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "files":     { "type": "array",   "items": { "type": "string" }, "description": "Repo-relative changed file paths" },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit (default 5)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes to return (default 200)" }
                    },
                    "required": ["files"]
                }
            },
            {
                "name": "get_review_context",
                "description": "Assemble review context for the given files: changed symbols, impacted neighbors, critical edges, and risk summary. Agent-optimized output.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "files": { "type": "array", "items": { "type": "string" }, "description": "Repo-relative changed file paths" },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit (default 3)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes to consider (default 200)" }
                    },
                    "required": ["files"]
                }
            },
            {
                "name": "detect_changes",
                "description": "List files changed since a base git ref, with per-file node counts from the graph.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "base":   { "type": "string",  "description": "Base ref (e.g. 'origin/main'). Omit to diff working tree." },
                        "staged": { "type": "boolean", "description": "Diff staged changes only (default false)" }
                    },
                    "required": []
                }
            },
            {
                "name": "build_or_update_graph",
                "description": "Scan, parse, and persist (or incrementally update) the code graph. Use mode='build' for a full scan or mode='update' for a git-diff-based incremental update.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "mode":   { "type": "string",  "description": "'build' (full scan, default) or 'update' (incremental)" },
                        "base":   { "type": "string",  "description": "For update: base git ref (e.g. 'origin/main')" },
                        "staged": { "type": "boolean", "description": "For update: diff staged changes only" },
                        "files":  { "type": "array", "items": { "type": "string" }, "description": "For update: explicit list of repo-relative file paths to re-index" }
                    },
                    "required": []
                }
            },
            {
                "name": "traverse_graph",
                "description": "Bi-directional graph traversal from a specific symbol (qualified name). Returns all nodes reachable within depth hops.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "from_qn":   { "type": "string",  "description": "Qualified name of the starting node (e.g. 'src/lib.rs::fn::my_func')" },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit (default 3)" },
                        "max_nodes": { "type": "integer", "description": "Maximum nodes to return (default 100)" }
                    },
                    "required": ["from_qn"]
                }
            },
            {
                "name": "get_minimal_context",
                "description": "Auto-detect changed files from git, then return a compact review bundle: changed symbols, immediate impact, risk flags. Lower token overhead than get_review_context.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "base":      { "type": "string",  "description": "Base git ref (e.g. 'origin/main'). Omit to diff working tree." },
                        "staged":    { "type": "boolean", "description": "Diff staged changes only (default false)" },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit (default 2)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes (default 50)" }
                    },
                    "required": []
                }
            }
        ]
    })
}

// ---------------------------------------------------------------------------
// Tool dispatch
// ---------------------------------------------------------------------------

/// Dispatch a `tools/call` invocation.
pub fn call(
    name: &str,
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<serde_json::Value> {
    match name {
        "list_graph_stats" => tool_list_graph_stats(db_path),
        "query_graph" => tool_query_graph(args, db_path),
        "get_impact_radius" => tool_get_impact_radius(args, db_path),
        "get_review_context" => tool_get_review_context(args, db_path),
        "detect_changes" => tool_detect_changes(args, repo_root, db_path),
        "build_or_update_graph" => tool_build_or_update_graph(args, repo_root, db_path),
        "traverse_graph" => tool_traverse_graph(args, db_path),
        "get_minimal_context" => tool_get_minimal_context(args, repo_root, db_path),
        other => Err(anyhow::anyhow!("unknown tool: {other}")),
    }
}

// ---------------------------------------------------------------------------
// Individual tool implementations
// ---------------------------------------------------------------------------

fn tool_list_graph_stats(db_path: &str) -> Result<serde_json::Value> {
    let store = open_store(db_path)?;
    let stats = store.stats().context("stats query failed")?;
    tool_result(serde_json::to_string_pretty(&stats)?)
}

fn tool_query_graph(args: Option<&serde_json::Value>, db_path: &str) -> Result<serde_json::Value> {
    let text = str_arg(args, "text")?
        .ok_or_else(|| anyhow::anyhow!("missing required argument: text"))?
        .to_owned();
    let kind = str_arg(args, "kind")?.map(str::to_owned);
    let language = str_arg(args, "language")?.map(str::to_owned);
    let limit = u64_arg(args, "limit").unwrap_or(20) as usize;

    let store = open_store(db_path)?;
    let results = store
        .search(&SearchQuery {
            text,
            kind,
            language,
            limit,
            ..Default::default()
        })
        .context("search failed")?;

    // Compact output: strip raw node blob, keep scored compact node.
    #[derive(Serialize)]
    struct CompactResult<'a> {
        score: f64,
        #[serde(flatten)]
        node: crate::context::CompactNode<'a>,
    }

    let compact: Vec<CompactResult<'_>> = results
        .iter()
        .map(|r| CompactResult {
            score: (r.score * 1000.0).round() / 1000.0,
            node: compact_node(&r.node),
        })
        .collect();

    tool_result(serde_json::to_string_pretty(&compact)?)
}

fn tool_get_impact_radius(
    args: Option<&serde_json::Value>,
    db_path: &str,
) -> Result<serde_json::Value> {
    let files = string_array_arg(args, "files")?;
    if files.is_empty() {
        return Err(anyhow::anyhow!("missing required argument: files"));
    }
    let max_depth = u64_arg(args, "max_depth").unwrap_or(5) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;

    let store = open_store(db_path)?;
    let file_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let result = store
        .impact_radius(&file_refs, max_depth, max_nodes)
        .context("impact_radius query failed")?;

    let packaged = package_impact(&result, &files);
    tool_result(serde_json::to_string_pretty(&packaged)?)
}

fn tool_get_review_context(
    args: Option<&serde_json::Value>,
    db_path: &str,
) -> Result<serde_json::Value> {
    let files = string_array_arg(args, "files")?;
    if files.is_empty() {
        return Err(anyhow::anyhow!("missing required argument: files"));
    }
    let max_depth = u64_arg(args, "max_depth").unwrap_or(3) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;

    let store = open_store(db_path)?;
    let file_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let impact = store
        .impact_radius(&file_refs, max_depth, max_nodes)
        .context("impact_radius query failed")?;
    let ctx = atlas_review::assemble_review_context(&impact, &files, max_depth, max_nodes);
    let packaged = package_review(&ctx);
    tool_result(serde_json::to_string_pretty(&packaged)?)
}

fn tool_detect_changes(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<serde_json::Value> {
    let base = str_arg(args, "base")?.map(str::to_owned);
    let staged = bool_arg(args, "staged").unwrap_or(false);

    let repo_root_path =
        find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;

    let target = if staged {
        DiffTarget::Staged
    } else if let Some(ref b) = base {
        DiffTarget::BaseRef(b.clone())
    } else {
        DiffTarget::WorkingTree
    };

    let changes =
        changed_files(repo_root_path.as_path(), &target).context("cannot detect changed files")?;

    // Augment with per-file node counts when the DB is reachable.
    let store_opt = Store::open(db_path).ok();

    #[derive(Serialize)]
    struct ChangedEntry<'a> {
        path: &'a str,
        change_type: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        old_path: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        node_count: Option<usize>,
    }

    let entries: Vec<ChangedEntry<'_>> = changes
        .iter()
        .map(|cf| {
            let node_count = store_opt
                .as_ref()
                .and_then(|s| s.nodes_by_file(&cf.path).ok())
                .map(|ns| ns.len());
            ChangedEntry {
                path: &cf.path,
                change_type: match cf.change_type {
                    atlas_core::ChangeType::Added => "added",
                    atlas_core::ChangeType::Modified => "modified",
                    atlas_core::ChangeType::Deleted => "deleted",
                    atlas_core::ChangeType::Renamed => "renamed",
                    atlas_core::ChangeType::Copied => "copied",
                },
                old_path: cf.old_path.as_deref(),
                node_count,
            }
        })
        .collect();

    tool_result(serde_json::to_string_pretty(&entries)?)
}

fn tool_build_or_update_graph(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<serde_json::Value> {
    let mode = str_arg(args, "mode")?.unwrap_or("build");
    let repo_root_path =
        find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;

    if mode == "update" {
        let base = str_arg(args, "base")?.map(str::to_owned);
        let staged = bool_arg(args, "staged").unwrap_or(false);
        let files = string_array_arg(args, "files")?;

        let target = if !files.is_empty() {
            UpdateTarget::Files(files)
        } else if staged {
            UpdateTarget::Staged
        } else if let Some(b) = base {
            UpdateTarget::BaseRef(b)
        } else {
            UpdateTarget::WorkingTree
        };

        let config = atlas_engine::Config::load(
            &atlas_engine::paths::atlas_dir(repo_root),
        )
        .unwrap_or_default();

        let summary = update_graph(
            repo_root_path.as_path(),
            db_path,
            &UpdateOptions {
                fail_fast: false,
                batch_size: config.parse_batch_size(),
                target,
            },
        )?;
        tool_result(serde_json::to_string_pretty(&serde_json::json!({
            "mode": "update",
            "deleted": summary.deleted,
            "parsed": summary.parsed,
            "skipped_unsupported": summary.skipped_unsupported,
            "parse_errors": summary.parse_errors,
            "nodes_updated": summary.nodes_updated,
            "edges_updated": summary.edges_updated,
            "elapsed_ms": summary.elapsed_ms,
        }))?)
    } else {
        // Default: full build
        let config = atlas_engine::Config::load(
            &atlas_engine::paths::atlas_dir(repo_root),
        )
        .unwrap_or_default();

        let summary = build_graph(
            repo_root_path.as_path(),
            db_path,
            &BuildOptions {
                fail_fast: false,
                batch_size: config.parse_batch_size(),
            },
        )?;
        tool_result(serde_json::to_string_pretty(&serde_json::json!({
            "mode": "build",
            "scanned": summary.scanned,
            "skipped_unsupported": summary.skipped_unsupported,
            "skipped_unchanged": summary.skipped_unchanged,
            "parsed": summary.parsed,
            "parse_errors": summary.parse_errors,
            "nodes_inserted": summary.nodes_inserted,
            "edges_inserted": summary.edges_inserted,
            "elapsed_ms": summary.elapsed_ms,
        }))?)
    }
}

fn tool_traverse_graph(
    args: Option<&serde_json::Value>,
    db_path: &str,
) -> Result<serde_json::Value> {
    let from_qn = str_arg(args, "from_qn")?
        .ok_or_else(|| anyhow::anyhow!("missing required argument: from_qn"))?
        .to_owned();
    let max_depth = u64_arg(args, "max_depth").unwrap_or(3) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(100) as usize;

    let store = open_store(db_path)?;
    let result = store
        .traverse_from_qnames(&[from_qn.as_str()], max_depth, max_nodes)
        .context("traverse_from_qnames failed")?;

    let seeds = vec![from_qn];
    let packaged = package_impact(&result, &seeds);
    tool_result(serde_json::to_string_pretty(&packaged)?)
}

fn tool_get_minimal_context(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<serde_json::Value> {
    let base = str_arg(args, "base")?.map(str::to_owned);
    let staged = bool_arg(args, "staged").unwrap_or(false);
    let max_depth = u64_arg(args, "max_depth").unwrap_or(2) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(50) as usize;

    let repo_root_path =
        find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;

    let diff_target = if staged {
        DiffTarget::Staged
    } else if let Some(ref b) = base {
        DiffTarget::BaseRef(b.clone())
    } else {
        DiffTarget::WorkingTree
    };

    let changes = changed_files(repo_root_path.as_path(), &diff_target)
        .context("cannot detect changed files")?;

    let changed_file_paths: Vec<String> = changes
        .iter()
        .filter(|cf| cf.change_type != atlas_core::ChangeType::Deleted)
        .map(|cf| cf.path.clone())
        .collect();

    let store = open_store(db_path)?;
    let file_refs: Vec<&str> = changed_file_paths.iter().map(String::as_str).collect();
    let impact = store
        .impact_radius(&file_refs, max_depth, max_nodes)
        .context("impact_radius failed")?;

    let packaged = package_impact(&impact, &changed_file_paths);

    #[derive(Serialize)]
    struct MinimalContext<'a> {
        changed_file_count: usize,
        deleted_file_count: usize,
        changed_files: Vec<&'a str>,
        impact: crate::context::PackagedImpact<'a>,
    }

    let deleted_count = changes
        .iter()
        .filter(|cf| cf.change_type == atlas_core::ChangeType::Deleted)
        .count();

    let ctx = MinimalContext {
        changed_file_count: changed_file_paths.len(),
        deleted_file_count: deleted_count,
        changed_files: changed_file_paths.iter().map(String::as_str).collect(),
        impact: packaged,
    };

    tool_result(serde_json::to_string_pretty(&ctx)?)
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

fn str_arg<'a>(args: Option<&'a serde_json::Value>, key: &str) -> Result<Option<&'a str>> {
    Ok(args.and_then(|a| a.get(key)).and_then(|v| v.as_str()))
}

fn u64_arg(args: Option<&serde_json::Value>, key: &str) -> Option<u64> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_u64())
}

fn bool_arg(args: Option<&serde_json::Value>, key: &str) -> Option<bool> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_bool())
}

fn string_array_arg(args: Option<&serde_json::Value>, key: &str) -> Result<Vec<String>> {
    Ok(args
        .and_then(|a| a.get(key))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Response envelope
// ---------------------------------------------------------------------------

fn open_store(db_path: &str) -> Result<Store> {
    Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))
}

/// Wrap text in an MCP tool-result content envelope.
fn tool_result(text: String) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "content": [{ "type": "text", "text": text }]
    }))
}
