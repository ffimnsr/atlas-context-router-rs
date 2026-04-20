//! MCP tool definitions and dispatch.
//!
//! Each tool follows the MCP `tools/call` contract: receives `arguments` as
//! an optional JSON object and returns an MCP content envelope:
//! `{ "content": [{ "type": "text", "text": "<json>" }] }`.

use anyhow::{Context, Result};
use atlas_core::SearchQuery;
use atlas_core::model::{ContextIntent, ContextRequest, ContextTarget};
use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_repo::{DiffTarget, changed_files, find_repo_root};
use atlas_review::ContextEngine;
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use serde::Serialize;

use crate::context::{compact_node, package_context_result, package_impact};

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
            },
            {
                "name": "explain_change",
                "description": "Advanced impact analysis for a set of changed files: risk level, changed-symbol breakdown by change kind (api/signature/internal), boundary violations, test coverage gaps, and a compact summary. Deterministic, LLM-free.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "files":     { "type": "array", "items": { "type": "string" }, "description": "Repo-relative changed file paths. Required unless using base/staged." },
                        "base":      { "type": "string",  "description": "Base git ref (e.g. 'origin/main'). Infers changed files from git diff when files not provided." },
                        "staged":    { "type": "boolean", "description": "Diff staged changes only (default false). Used when inferring files from git." },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit for impact (default 5)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes (default 200)" }
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
        "explain_change" => tool_explain_change(args, repo_root, db_path),
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
    let engine = ContextEngine::new(&store);
    let request = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles { paths: files },
        max_nodes: Some(max_nodes),
        depth: Some(max_depth),
        ..ContextRequest::default()
    };
    let result = engine.build(&request).context("context engine failed")?;
    let packaged = package_context_result(&result);
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

        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
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
            "renamed": summary.renamed,
            "parsed": summary.parsed,
            "skipped_unsupported": summary.skipped_unsupported,
            "parse_errors": summary.parse_errors,
            "nodes_updated": summary.nodes_updated,
            "edges_updated": summary.edges_updated,
            "elapsed_ms": summary.elapsed_ms,
        }))?)
    } else {
        // Default: full build
        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
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

fn tool_explain_change(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<serde_json::Value> {
    let max_depth = u64_arg(args, "max_depth").unwrap_or(5) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;

    // Resolve changed file list: explicit list takes priority; fall back to git diff.
    let mut files = string_array_arg(args, "files")?;
    if files.is_empty() {
        let staged = bool_arg(args, "staged").unwrap_or(false);
        let base = str_arg(args, "base")?.map(str::to_owned);
        let repo_root_path =
            find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;
        let diff_target = if staged {
            DiffTarget::Staged
        } else if let Some(b) = base {
            DiffTarget::BaseRef(b)
        } else {
            DiffTarget::WorkingTree
        };
        let changes = changed_files(repo_root_path.as_path(), &diff_target)
            .context("cannot detect changed files")?;
        files = changes
            .into_iter()
            .filter(|cf| cf.change_type != atlas_core::ChangeType::Deleted)
            .map(|cf| cf.path)
            .collect();
    }

    if files.is_empty() {
        return tool_result(serde_json::to_string_pretty(&serde_json::json!({
            "risk_level": "low",
            "changed_file_count": 0,
            "changed_symbol_count": 0,
            "changed_by_kind": { "api_change": 0, "signature_change": 0, "internal_change": 0 },
            "changed_symbols": [],
            "impacted_file_count": 0,
            "impacted_node_count": 0,
            "boundary_violations": [],
            "test_impact": { "affected_test_count": 0, "uncovered_symbol_count": 0, "uncovered_symbols": [] },
            "summary": "No changed files detected."
        }))?);
    }

    let store = open_store(db_path)?;
    let file_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let base_impact = store
        .impact_radius(&file_refs, max_depth, max_nodes)
        .context("impact_radius query failed")?;

    let advanced = atlas_impact::analyze(base_impact);

    // Summarise changed symbols by change kind.
    let mut api_count: usize = 0;
    let mut sig_count: usize = 0;
    let mut internal_count: usize = 0;

    #[derive(Serialize)]
    struct ChangedSymbol<'a> {
        qn: &'a str,
        kind: &'a str,
        file: &'a str,
        line: u32,
        change_kind: &'a str,
        lang: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        sig: Option<&'a str>,
    }

    let changed_symbols: Vec<ChangedSymbol<'_>> = advanced
        .scored_nodes
        .iter()
        .filter_map(|sn| sn.change_kind.map(|ck| (&sn.node, ck)))
        .map(|(n, ck)| {
            let ck_str = match ck {
                atlas_core::ChangeKind::ApiChange => {
                    api_count += 1;
                    "api_change"
                }
                atlas_core::ChangeKind::SignatureChange => {
                    sig_count += 1;
                    "signature_change"
                }
                atlas_core::ChangeKind::InternalChange => {
                    internal_count += 1;
                    "internal_change"
                }
            };
            ChangedSymbol {
                qn: &n.qualified_name,
                kind: n.kind.as_str(),
                file: &n.file_path,
                line: n.line_start,
                change_kind: ck_str,
                lang: &n.language,
                sig: n.params.as_deref(),
            }
        })
        .collect();

    let changed_symbol_count = changed_symbols.len();

    #[derive(Serialize)]
    struct BoundaryViolationCompact<'a> {
        kind: &'a str,
        description: &'a str,
        nodes: &'a [String],
    }

    let boundary_violations: Vec<BoundaryViolationCompact<'_>> = advanced
        .boundary_violations
        .iter()
        .map(|bv| BoundaryViolationCompact {
            kind: match bv.kind {
                atlas_core::BoundaryKind::CrossModule => "cross_module",
                atlas_core::BoundaryKind::CrossPackage => "cross_package",
            },
            description: &bv.description,
            nodes: &bv.nodes,
        })
        .collect();

    let affected_test_count = advanced.test_impact.affected_tests.len();
    let uncovered: Vec<&str> = advanced
        .test_impact
        .uncovered_changed_nodes
        .iter()
        .map(|n| n.qualified_name.as_str())
        .collect();
    let uncovered_count = uncovered.len();

    // Build deterministic summary text.
    let risk_str = advanced.risk_level.to_string();
    let impacted_file_count = advanced.base.impacted_files.len();
    let impacted_node_count = advanced.base.impacted_nodes.len();

    let mut summary_parts: Vec<String> = Vec::new();
    summary_parts.push(format!("Risk: {}.", risk_str));
    if api_count > 0 {
        summary_parts.push(format!("{} api change(s).", api_count));
    }
    if sig_count > 0 {
        summary_parts.push(format!("{} signature change(s).", sig_count));
    }
    if internal_count > 0 {
        summary_parts.push(format!("{} internal change(s).", internal_count));
    }
    summary_parts.push(format!(
        "Affects {} file(s), {} node(s).",
        impacted_file_count, impacted_node_count
    ));
    if !boundary_violations.is_empty() {
        summary_parts.push(format!(
            "{} boundary violation(s).",
            boundary_violations.len()
        ));
    }
    if uncovered_count > 0 {
        summary_parts.push(format!(
            "{} changed symbol(s) lack test coverage.",
            uncovered_count
        ));
    }
    let summary = summary_parts.join(" ");

    #[derive(Serialize)]
    struct ExplainChangeResult<'a> {
        risk_level: &'a str,
        changed_file_count: usize,
        changed_symbol_count: usize,
        changed_by_kind: serde_json::Value,
        changed_symbols: Vec<ChangedSymbol<'a>>,
        impacted_file_count: usize,
        impacted_node_count: usize,
        boundary_violations: Vec<BoundaryViolationCompact<'a>>,
        test_impact: serde_json::Value,
        summary: &'a str,
    }

    let result = ExplainChangeResult {
        risk_level: &risk_str,
        changed_file_count: files.len(),
        changed_symbol_count,
        changed_by_kind: serde_json::json!({
            "api_change": api_count,
            "signature_change": sig_count,
            "internal_change": internal_count,
        }),
        changed_symbols,
        impacted_file_count,
        impacted_node_count,
        boundary_violations,
        test_impact: serde_json::json!({
            "affected_test_count": affected_test_count,
            "uncovered_symbol_count": uncovered_count,
            "uncovered_symbols": uncovered,
        }),
        summary: &summary,
    };

    tool_result(serde_json::to_string_pretty(&result)?)
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

#[cfg(test)]
mod tests {
    use super::*;

    use atlas_core::kinds::NodeKind;
    use atlas_core::model::{Node, NodeId};

    fn unwrap_tool_text(resp: serde_json::Value) -> String {
        resp.get("content")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
            .and_then(|c0| c0.get("text"))
            .and_then(|t| t.as_str())
            .expect("tool response content[0].text")
            .to_owned()
    }

    #[test]
    fn tool_list_includes_explain_change() {
        let list = tool_list();
        let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
        assert!(
            tools
                .iter()
                .any(|t| t.get("name") == Some(&"explain_change".into()))
        );
    }

    #[test]
    fn explain_change_reports_change_kind_counts() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();

        let mut store = Store::open(&db_path).expect("open store");
        let node = Node {
            id: NodeId::UNSET,
            kind: NodeKind::Function,
            name: "foo".to_owned(),
            qualified_name: "src/a.rs::fn::foo".to_owned(),
            file_path: "src/a.rs".to_owned(),
            line_start: 1,
            line_end: 3,
            language: "rust".to_owned(),
            parent_name: None,
            params: Some("x: i32".to_owned()),
            return_type: Some("i32".to_owned()),
            modifiers: Some("pub".to_owned()),
            is_test: false,
            file_hash: "h1".to_owned(),
            extra_json: serde_json::json!({}),
        };
        store
            .replace_file_graph("src/a.rs", "h1", Some("rust"), Some(10), &[node], &[])
            .expect("replace_file_graph");

        let args = serde_json::json!({
            "files": ["src/a.rs"],
            "max_depth": 5,
            "max_nodes": 200,
        });
        let resp = call("explain_change", Some(&args), "/ignored", &db_path).expect("call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(
            v.get("changed_file_count").and_then(|n| n.as_u64()),
            Some(1)
        );
        assert_eq!(
            v.get("changed_symbol_count").and_then(|n| n.as_u64()),
            Some(1)
        );
        assert_eq!(
            v.pointer("/changed_by_kind/signature_change")
                .and_then(|n| n.as_u64()),
            Some(1)
        );
        assert_eq!(
            v.pointer("/changed_symbols/0/change_kind")
                .and_then(|s| s.as_str()),
            Some("signature_change")
        );
        assert_eq!(
            v.pointer("/changed_symbols/0/qn").and_then(|s| s.as_str()),
            Some("src/a.rs::fn::foo")
        );
    }
}
