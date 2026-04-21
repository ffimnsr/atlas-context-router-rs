//! MCP tool definitions and dispatch.
//!
//! Each tool follows the MCP `tools/call` contract: receives `arguments` as
//! an optional JSON object and returns an MCP content envelope:
//! `{ "content": [{ "type": "text", "text": "<json>" }] }`.

use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, McpAdapter};
use atlas_core::SearchQuery;
use atlas_core::model::{ContextIntent, ContextRequest, ContextTarget};
use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_repo::{DiffTarget, changed_files, find_repo_root};
use atlas_review::{ContextEngine, query_parser};
use atlas_search::semantic as sem;
use atlas_store_sqlite::{BuildFinishStats, GraphBuildState, Store};
use camino::Utf8Path;
use serde::Serialize;

use crate::context::{compact_node, package_context_result, package_impact};
use crate::output::{OutputFormat, render_serializable, resolve_output_format};
use crate::session_tools::{
    derive_content_db_path, tool_get_context_stats, tool_get_session_status,
    tool_purge_saved_context, tool_resume_session, tool_save_context_artifact,
    tool_search_saved_context,
};

const DEFAULT_OUTPUT_DESCRIPTION: &str = "Response body format: 'toon' (default) or 'json'";

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
                    "properties": {
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "query_graph",
                "description": "Full-text search the code graph. Returns a compact, ranked list of matching symbols only; it does not return caller/callee usage edges. Use semantic=true for graph-aware expansion, then follow with symbol_neighbors, traverse_graph, or get_context when you need relationships.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text":     { "type": "string",  "description": "Search query text" },
                        "kind":     { "type": "string",  "description": "Filter by node kind (e.g. 'function', 'struct')" },
                        "language": { "type": "string",  "description": "Filter by language (e.g. 'rust', 'python')" },
                        "limit":    { "type": "integer", "description": "Maximum results to return (default 20)" },
                        "semantic": { "type": "boolean", "description": "Use graph-aware semantic expansion: expands via graph neighbours of initial FTS hits before re-ranking (default false)" },
                        "expand":   { "type": "boolean", "description": "Expand results through graph edges after ranking (default false)" },
                        "expand_hops": { "type": "integer", "description": "Max edge hops when expand=true (default 1)" },
                        "regex":    { "type": "string",  "description": "Regex pattern matched against name and qualified_name via SQL UDF. When text is empty, runs a structural scan filtered by this pattern. When both text and regex are provided, FTS5 runs first then the UDF filters inside SQLite. Supports regex crate alternation syntax (e.g. 'handle|HANDLE|Handle_'). Must be valid regex crate syntax." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
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
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes to return (default 200)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
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
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes to consider (default 200)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
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
                        "staged": { "type": "boolean", "description": "Diff staged changes only (default false)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
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
                        "files":  { "type": "array", "items": { "type": "string" }, "description": "For update: explicit list of repo-relative file paths to re-index" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
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
                        "max_nodes": { "type": "integer", "description": "Maximum nodes to return (default 100)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
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
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes (default 50)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
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
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes (default 200)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "get_context",
                "description": "Build bounded context around a symbol, file, or change-set using the context engine. Accepts a free-text query (auto-classified), an explicit file path, or a list of changed files. Returns a compact agent-optimized result with ranked nodes, edges, files, and truncation/ambiguity metadata.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":     { "type": "string",  "description": "Free-text or symbol name query (e.g. 'who calls handle_request', 'AuthService', 'src/lib.rs::fn::foo'). Alternative to file/files." },
                        "file":      { "type": "string",  "description": "Repo-relative file path target (file intent). Alternative to query/files." },
                        "files":     { "type": "array", "items": { "type": "string" }, "description": "Changed file paths for review/impact context. Alternative to query/file." },
                        "intent":    { "type": "string",  "description": "Override intent: symbol, file, review, impact, usage_lookup, refactor_safety, dead_code_check, rename_preview, dependency_removal. Inferred when omitted." },
                        "max_nodes": { "type": "integer", "description": "Maximum nodes to include (default 100)" },
                        "max_edges": { "type": "integer", "description": "Maximum edges to include (default 100)" },
                        "max_depth": { "type": "integer", "description": "Traversal depth in graph hops (default 2)" },
                        "include_saved_context": { "type": "boolean", "description": "When true, also query the content store for saved artifacts relevant to this request and include them in the result (default false)." },
                        "session_id": { "type": "string",  "description": "Restrict saved-context retrieval to artifacts from this session and apply a same-session relevance boost." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "get_session_status",
                "description": "Return the status of the current MCP session: identity, event count, and whether a resume snapshot exists.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id":    { "type": "string",  "description": "Explicit session id. Omit to use the derived id for the current repo." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "resume_session",
                "description": "Retrieve and optionally consume the resume snapshot for the current (or specified) session. Builds a snapshot on demand if one does not exist.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id":     { "type": "string",  "description": "Explicit session id. Omit to use the derived id for the current repo." },
                        "mark_consumed":  { "type": "boolean", "description": "Mark the snapshot consumed after reading (default true)." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "search_saved_context",
                "description": "Search previously saved artifacts in the content store using BM25 + trigram fallback. Returns previews (first 256 chars) and source_ids for follow-up retrieval.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":        { "type": "string",  "description": "Search query text." },
                        "session_id":   { "type": "string",  "description": "Restrict search to artifacts from this session." },
                        "source_type":  { "type": "string",  "description": "Filter by source type (e.g. 'review_context', 'mcp_artifact')." },
                        "limit":        { "type": "integer", "description": "Maximum results to return (default 10)." },
                        "output_format":{ "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "save_context_artifact",
                "description": "Index and store a large tool output or context payload. Returns a pointer (source_id) for large content, a preview for medium content, or the raw string for small content.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "content":      { "type": "string",  "description": "The content to store." },
                        "label":        { "type": "string",  "description": "Human-readable label for display and retrieval." },
                        "source_type":  { "type": "string",  "description": "Category tag (e.g. 'review_context', 'command_output'). Default: 'mcp_artifact'." },
                        "session_id":   { "type": "string",  "description": "Associate artifact with this session. Omit to use derived session." },
                        "content_type": { "type": "string",  "description": "MIME type: 'text/plain' (default), 'text/markdown', or 'application/json'." },
                        "output_format":{ "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["content", "label"]
                }
            },
            {
                "name": "get_context_stats",
                "description": "Return storage statistics for the current (or specified) session: event count, saved source count, chunk count, and DB paths.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id":    { "type": "string",  "description": "Explicit session id. Omit to use the derived id for the current repo." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "purge_saved_context",
                "description": "Delete saved artifacts. Provide session_id to delete all artifacts for that session, or omit to apply age-based cleanup (default: keep last 30 days).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id":    { "type": "string",  "description": "Delete all saved artifacts for this session." },
                        "keep_days":     { "type": "integer", "description": "For age-based cleanup: keep sources newer than this many days (default 30)." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "symbol_neighbors",
                "description": "Return the immediate graph neighbourhood of a symbol: callers, callees, call edge sites with source lines, test nodes, containment siblings, and import-linked nodes. Useful for understanding a symbol's role and exact direct usage sites without a full traversal.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "qname":     { "type": "string",  "description": "Fully-qualified name of the symbol (e.g. 'src/lib.rs::fn::my_func')." },
                        "limit":     { "type": "integer", "description": "Maximum nodes to return per relationship kind (default 10)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["qname"]
                }
            },
            {
                "name": "cross_file_links",
                "description": "Find files that reference symbols defined in the given file. Returns semantic links ordered by coupling strength (number of shared symbol references).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file":  { "type": "string",  "description": "Repo-relative file path to analyse (e.g. 'src/auth.rs')." },
                        "limit": { "type": "integer", "description": "Maximum links to return (default 20)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["file"]
                }
            },
            {
                "name": "concept_clusters",
                "description": "Cluster files related to the given seed files by shared symbol references. Returns groups of co-dependent files ordered by coupling density.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "files": { "type": "array", "items": { "type": "string" }, "description": "Seed file paths (repo-relative) to cluster around." },
                        "limit": { "type": "integer", "description": "Maximum clusters to return (default 10)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["files"]
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
    let mut adapter = McpAdapter::open(repo_root);
    if let Some(ref mut a) = adapter {
        a.before_command(name);
    }
    let result = call_inner(name, args, repo_root, db_path);
    if let Some(ref mut a) = adapter {
        a.after_command(name, result.is_ok());
    }
    // CM7: emit session event best-effort for continuity tools.
    if result.is_ok() {
        crate::session_tools::emit_session_event_best_effort(name, args, repo_root, db_path);
    }
    result
}

fn call_inner(
    name: &str,
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<serde_json::Value> {
    let output_format = resolve_output_format(args, default_output_format_for_tool(name))?;
    match name {
        "list_graph_stats" => tool_list_graph_stats(db_path, output_format),
        "query_graph" => tool_query_graph(args, db_path, output_format),
        "get_impact_radius" => tool_get_impact_radius(args, db_path, output_format),
        "get_review_context" => tool_get_review_context(args, db_path, output_format),
        "detect_changes" => tool_detect_changes(args, repo_root, db_path, output_format),
        "build_or_update_graph" => {
            tool_build_or_update_graph(args, repo_root, db_path, output_format)
        }
        "traverse_graph" => tool_traverse_graph(args, db_path, output_format),
        "get_minimal_context" => tool_get_minimal_context(args, repo_root, db_path, output_format),
        "explain_change" => tool_explain_change(args, repo_root, db_path, output_format),
        "get_context" => tool_get_context(args, db_path, output_format),
        "get_session_status" => tool_get_session_status(args, repo_root, db_path, output_format),
        "resume_session" => tool_resume_session(args, repo_root, db_path, output_format),
        "search_saved_context" => {
            tool_search_saved_context(args, repo_root, db_path, output_format)
        }
        "save_context_artifact" => {
            tool_save_context_artifact(args, repo_root, db_path, output_format)
        }
        "get_context_stats" => tool_get_context_stats(args, repo_root, db_path, output_format),
        "purge_saved_context" => tool_purge_saved_context(args, repo_root, db_path, output_format),
        "symbol_neighbors" => tool_symbol_neighbors(args, db_path, output_format),
        "cross_file_links" => tool_cross_file_links(args, db_path, output_format),
        "concept_clusters" => tool_concept_clusters(args, db_path, output_format),
        other => Err(anyhow::anyhow!("unknown tool: {other}")),
    }
}

fn default_output_format_for_tool(_name: &str) -> OutputFormat {
    OutputFormat::Toon
}

// ---------------------------------------------------------------------------
// Individual tool implementations
// ---------------------------------------------------------------------------

fn tool_list_graph_stats(db_path: &str, output_format: OutputFormat) -> Result<serde_json::Value> {
    let store = open_store(db_path)?;
    let stats = store.stats().context("stats query failed")?;
    tool_result_value(&stats, output_format)
}

fn tool_query_graph(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let text = str_arg(args, "text")?
        .map(str::to_owned)
        .unwrap_or_default();
    let kind = str_arg(args, "kind")?.map(str::to_owned);
    let language = str_arg(args, "language")?.map(str::to_owned);
    let limit = u64_arg(args, "limit").unwrap_or(20) as usize;
    let semantic = bool_arg(args, "semantic").unwrap_or(false);
    let expand = bool_arg(args, "expand").unwrap_or(false);
    let expand_hops = u64_arg(args, "expand_hops").unwrap_or(1) as u32;
    let regex = str_arg(args, "regex")?.map(str::to_owned);

    if text.trim().is_empty() && regex.is_none() {
        anyhow::bail!("query_graph requires non-empty text or a regex pattern");
    }
    if let Some(ref pat) = regex {
        if pat.trim().is_empty() {
            anyhow::bail!("regex pattern must not be empty");
        }
        regex::Regex::new(pat).map_err(|e| anyhow::anyhow!("invalid regex pattern: {e}"))?;
    }

    let store = open_store(db_path)?;

    let query = SearchQuery {
        text,
        kind,
        language,
        limit,
        graph_expand: expand,
        graph_max_hops: expand_hops,
        regex_pattern: regex,
        ..Default::default()
    };

    let results = if semantic {
        sem::expanded_search(&store, &query).context("semantic search failed")?
    } else {
        store.search(&query).context("search failed")?
    };

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

    let mut response = tool_result_value(&compact, output_format)?;
    response["atlas_result_kind"] = serde_json::Value::String("symbol_search".to_owned());
    response["atlas_usage_edges_included"] = serde_json::Value::Bool(false);
    response["atlas_relationship_tools"] =
        serde_json::json!(["symbol_neighbors", "traverse_graph", "get_context"]);
    Ok(response)
}

fn tool_get_impact_radius(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
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
    tool_result_value(&packaged, output_format)
}

fn tool_get_review_context(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
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
    tool_result_value(&packaged, output_format)
}

fn tool_detect_changes(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
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

    tool_result_value(&entries, output_format)
}

fn tool_build_or_update_graph(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let mode = str_arg(args, "mode")?.unwrap_or("build");
    let repo_root_path =
        find_repo_root(Utf8Path::new(repo_root)).context("cannot find git repo root")?;
    let repo_root_str = repo_root_path.as_str();

    /// Serialize a persisted build status to a JSON value for MCP responses.
    fn build_status_json(db_path: &str, repo_root: &str) -> serde_json::Value {
        let Ok(store) = Store::open(db_path) else {
            return serde_json::Value::Null;
        };
        let Ok(Some(bs)) = store.get_build_status(repo_root) else {
            return serde_json::Value::Null;
        };
        let state_str = match bs.state {
            GraphBuildState::Building => "building",
            GraphBuildState::Built => "built",
            GraphBuildState::BuildFailed => "build_failed",
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
    }

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

        if let Ok(s) = Store::open(db_path) {
            let _ = s.begin_build(repo_root_str);
        }

        let update_result = update_graph(
            repo_root_path.as_path(),
            db_path,
            &UpdateOptions {
                fail_fast: false,
                batch_size: config.parse_batch_size(),
                target,
            },
        );

        if let Ok(s) = Store::open(db_path) {
            match &update_result {
                Ok(sum) => {
                    let _ = s.finish_build(
                        repo_root_str,
                        BuildFinishStats {
                            files_discovered: (sum.parsed + sum.deleted + sum.renamed) as i64,
                            files_processed: sum.parsed as i64,
                            files_failed: sum.parse_errors as i64,
                            nodes_written: sum.nodes_updated as i64,
                            edges_written: sum.edges_updated as i64,
                        },
                    );
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                }
            }
        }

        let summary = update_result?;
        tool_result_value(
            &serde_json::json!({
                "mode": "update",
                "deleted": summary.deleted,
                "renamed": summary.renamed,
                "parsed": summary.parsed,
                "skipped_unsupported": summary.skipped_unsupported,
                "parse_errors": summary.parse_errors,
                "nodes_updated": summary.nodes_updated,
                "edges_updated": summary.edges_updated,
                "elapsed_ms": summary.elapsed_ms,
                "build_status": build_status_json(db_path, repo_root_str),
            }),
            output_format,
        )
    } else {
        // Default: full build
        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root))
            .unwrap_or_default();

        if let Ok(s) = Store::open(db_path) {
            let _ = s.begin_build(repo_root_str);
        }

        let build_result = build_graph(
            repo_root_path.as_path(),
            db_path,
            &BuildOptions {
                fail_fast: false,
                batch_size: config.parse_batch_size(),
            },
        );

        if let Ok(s) = Store::open(db_path) {
            match &build_result {
                Ok(sum) => {
                    let _ = s.finish_build(
                        repo_root_str,
                        BuildFinishStats {
                            files_discovered: sum.scanned as i64,
                            files_processed: sum.parsed as i64,
                            files_failed: sum.parse_errors as i64,
                            nodes_written: sum.nodes_inserted as i64,
                            edges_written: sum.edges_inserted as i64,
                        },
                    );
                }
                Err(e) => {
                    let _ = s.fail_build(repo_root_str, &e.to_string());
                }
            }
        }

        let summary = build_result?;
        tool_result_value(
            &serde_json::json!({
                "mode": "build",
                "scanned": summary.scanned,
                "skipped_unsupported": summary.skipped_unsupported,
                "skipped_unchanged": summary.skipped_unchanged,
                "parsed": summary.parsed,
                "parse_errors": summary.parse_errors,
                "nodes_inserted": summary.nodes_inserted,
                "edges_inserted": summary.edges_inserted,
                "elapsed_ms": summary.elapsed_ms,
                "build_status": build_status_json(db_path, repo_root_str),
            }),
            output_format,
        )
    }
}

fn tool_traverse_graph(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
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
    tool_result_value(&packaged, output_format)
}

fn tool_get_minimal_context(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
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

    tool_result_value(&ctx, output_format)
}

fn tool_explain_change(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
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
        return tool_result_value(
            &serde_json::json!({
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
            }),
            output_format,
        );
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

    tool_result_value(&result, output_format)
}

// ---------------------------------------------------------------------------
// Argument helpers
// ---------------------------------------------------------------------------

fn parse_mcp_intent(s: &str) -> atlas_core::model::ContextIntent {
    use atlas_core::model::ContextIntent;
    match s {
        "file" => ContextIntent::File,
        "review" => ContextIntent::Review,
        "impact" => ContextIntent::Impact,
        "usage_lookup" | "usage" => ContextIntent::UsageLookup,
        "refactor_safety" | "refactor" => ContextIntent::RefactorSafety,
        "dead_code_check" | "dead_code" => ContextIntent::DeadCodeCheck,
        "rename_preview" | "rename" => ContextIntent::RenamePreview,
        "dependency_removal" | "deps" => ContextIntent::DependencyRemoval,
        _ => ContextIntent::Symbol,
    }
}

fn tool_get_context(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    use atlas_contentstore::ContentStore;
    use atlas_core::model::{ContextIntent, ContextRequest, ContextTarget};

    let query = str_arg(args, "query")?.map(str::to_owned);
    let file = str_arg(args, "file")?.map(str::to_owned);
    let files = string_array_arg(args, "files")?;
    let intent_override = str_arg(args, "intent")?.map(str::to_owned);
    let max_nodes = u64_arg(args, "max_nodes").map(|n| n as usize);
    let max_edges = u64_arg(args, "max_edges").map(|n| n as usize);
    let max_depth = u64_arg(args, "max_depth").map(|n| n as u32);
    let include_saved_context = bool_arg(args, "include_saved_context").unwrap_or(false);
    let session_id = str_arg(args, "session_id")?.map(str::to_owned);

    let mut request = if !files.is_empty() {
        let intent = intent_override
            .as_deref()
            .map(parse_mcp_intent)
            .unwrap_or(ContextIntent::Review);
        ContextRequest {
            intent,
            target: ContextTarget::ChangedFiles { paths: files },
            ..ContextRequest::default()
        }
    } else if let Some(path) = file {
        let intent = intent_override
            .as_deref()
            .map(parse_mcp_intent)
            .unwrap_or(ContextIntent::File);
        ContextRequest {
            intent,
            target: ContextTarget::FilePath { path },
            ..ContextRequest::default()
        }
    } else if let Some(q) = query {
        let mut parsed = query_parser::parse_query(&q);
        if let Some(ref ov) = intent_override {
            parsed.intent = parse_mcp_intent(ov);
        }
        parsed
    } else {
        return Err(anyhow::anyhow!(
            "get_context requires one of: 'query', 'file', or 'files'"
        ));
    };

    if max_nodes.is_some() {
        request.max_nodes = max_nodes;
    }
    if max_edges.is_some() {
        request.max_edges = max_edges;
    }
    if max_depth.is_some() {
        request.depth = max_depth;
    }
    request.include_saved_context = include_saved_context;
    request.session_id = session_id;

    let store = open_store(db_path)?;
    let engine = ContextEngine::new(&store);

    // CM6: attach content store when saved-context retrieval is requested.
    let result = if include_saved_context {
        let content_db = derive_content_db_path(db_path);
        match ContentStore::open(&content_db) {
            Ok(mut cs) => {
                let _ = cs.migrate();
                let engine = engine.with_content_store(&cs);
                engine.build(&request).context("context engine failed")?
            }
            Err(_) => engine.build(&request).context("context engine failed")?,
        }
    } else {
        engine.build(&request).context("context engine failed")?
    };

    let packaged = package_context_result(&result);
    tool_result_value(&packaged, output_format)
}

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

// ---------------------------------------------------------------------------
// Semantic neighbourhood tools (CM9 follow-up)
// ---------------------------------------------------------------------------

fn tool_symbol_neighbors(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let qname = str_arg(args, "qname")?
        .ok_or_else(|| anyhow::anyhow!("missing required argument: qname"))?
        .to_owned();
    let limit = u64_arg(args, "limit").unwrap_or(10) as usize;

    let store = open_store(db_path)?;
    let nbhd =
        sem::symbol_neighborhood(&store, &qname, limit).context("symbol_neighborhood failed")?;
    let caller_pairs = store
        .direct_callers(&qname, limit)
        .context("direct_callers failed")?;
    let callee_pairs = store
        .direct_callees(&qname, limit)
        .context("direct_callees failed")?;

    #[derive(Serialize)]
    struct CompactCallEdge<'a> {
        from: &'a str,
        to: &'a str,
        file: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        line: Option<u32>,
        confidence: f32,
        #[serde(skip_serializing_if = "Option::is_none")]
        tier: Option<&'a str>,
    }

    fn compact_call_edge(edge: &atlas_core::Edge) -> CompactCallEdge<'_> {
        CompactCallEdge {
            from: &edge.source_qn,
            to: &edge.target_qn,
            file: &edge.file_path,
            line: edge.line,
            confidence: edge.confidence,
            tier: edge.confidence_tier.as_deref(),
        }
    }

    fn compact_unique_nodes_from_pairs<'a>(
        pairs: &'a [(atlas_core::Node, atlas_core::Edge)],
    ) -> Vec<crate::context::CompactNode<'a>> {
        let mut seen = std::collections::HashSet::new();
        let mut nodes = Vec::new();
        for (node, _) in pairs {
            if seen.insert(node.qualified_name.as_str()) {
                nodes.push(compact_node(node));
            }
        }
        nodes
    }

    #[derive(Serialize)]
    struct NeighborhoodResult<'a> {
        qname: &'a str,
        callers: Vec<crate::context::CompactNode<'a>>,
        callees: Vec<crate::context::CompactNode<'a>>,
        caller_edges: Vec<CompactCallEdge<'a>>,
        callee_edges: Vec<CompactCallEdge<'a>>,
        tests: Vec<crate::context::CompactNode<'a>>,
        siblings: Vec<crate::context::CompactNode<'a>>,
        import_neighbors: Vec<crate::context::CompactNode<'a>>,
    }

    let result = NeighborhoodResult {
        qname: &qname,
        callers: compact_unique_nodes_from_pairs(&caller_pairs),
        callees: compact_unique_nodes_from_pairs(&callee_pairs),
        caller_edges: caller_pairs
            .iter()
            .map(|(_, edge)| compact_call_edge(edge))
            .collect(),
        callee_edges: callee_pairs
            .iter()
            .map(|(_, edge)| compact_call_edge(edge))
            .collect(),
        tests: nbhd.tests.iter().map(compact_node).collect(),
        siblings: nbhd.siblings.iter().map(compact_node).collect(),
        import_neighbors: nbhd.import_neighbors.iter().map(compact_node).collect(),
    };

    tool_result_value(&result, output_format)
}

fn tool_cross_file_links(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let file = str_arg(args, "file")?
        .ok_or_else(|| anyhow::anyhow!("missing required argument: file"))?
        .to_owned();
    let limit = u64_arg(args, "limit").unwrap_or(20) as usize;

    let store = open_store(db_path)?;
    let links = sem::cross_file_links(&store, &file, limit).context("cross_file_links failed")?;

    #[derive(Serialize)]
    struct LinkResult {
        from_file: String,
        to_file: String,
        via_symbols: Vec<String>,
        strength: f64,
    }

    let result: Vec<LinkResult> = links
        .into_iter()
        .map(|l| LinkResult {
            from_file: l.from_file,
            to_file: l.to_file,
            via_symbols: l.via_symbols,
            strength: (l.strength * 10.0).round() / 10.0,
        })
        .collect();

    tool_result_value(&result, output_format)
}

fn tool_concept_clusters(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let files = string_array_arg(args, "files")?;
    if files.is_empty() {
        return Err(anyhow::anyhow!("missing required argument: files"));
    }
    let limit = u64_arg(args, "limit").unwrap_or(10) as usize;

    let store = open_store(db_path)?;
    let seed_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let clusters = sem::cluster_by_shared_symbols(&store, &seed_refs, limit)
        .context("concept_clusters failed")?;

    #[derive(Serialize)]
    struct ClusterResult {
        files: Vec<String>,
        shared_symbols: Vec<String>,
        density: f64,
    }

    let result: Vec<ClusterResult> = clusters
        .into_iter()
        .map(|c| ClusterResult {
            files: c.files,
            shared_symbols: c.shared_symbols,
            density: (c.density * 1000.0).round() / 1000.0,
        })
        .collect();

    tool_result_value(&result, output_format)
}

/// Wrap structured output in an MCP tool-result content envelope.
fn tool_result_value<T: Serialize>(
    value: &T,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let rendered = render_serializable(value, output_format)?;
    let mut response = serde_json::json!({
        "content": [{
            "type": "text",
            "text": rendered.text,
            "mimeType": rendered.actual_format.mime_type(),
        }],
        "atlas_output_format": rendered.actual_format.as_str(),
        "atlas_requested_output_format": rendered.requested_format.as_str(),
    });

    if let Some(reason) = rendered.fallback_reason {
        response["atlas_fallback_reason"] = serde_json::Value::String(reason);
    }

    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    use atlas_core::EdgeKind;
    use atlas_core::kinds::NodeKind;
    use atlas_core::model::{Edge, Node, NodeId};
    use tempfile::TempDir;

    struct McpFixture {
        _dir: TempDir,
        db_path: String,
    }

    fn make_node(kind: NodeKind, name: &str, qualified_name: &str, file_path: &str) -> Node {
        Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_owned(),
            qualified_name: qualified_name.to_owned(),
            file_path: file_path.to_owned(),
            line_start: 1,
            line_end: 5,
            language: "rust".to_owned(),
            parent_name: None,
            params: Some("()".to_owned()),
            return_type: None,
            modifiers: Some("pub".to_owned()),
            is_test: kind == NodeKind::Test,
            file_hash: format!("hash:{file_path}"),
            extra_json: serde_json::json!({}),
        }
    }

    fn make_edge(kind: EdgeKind, source_qn: &str, target_qn: &str, file_path: &str) -> Edge {
        Edge {
            id: 0,
            kind,
            source_qn: source_qn.to_owned(),
            target_qn: target_qn.to_owned(),
            file_path: file_path.to_owned(),
            line: Some(1),
            confidence: 1.0,
            confidence_tier: Some("high".to_owned()),
            extra_json: serde_json::json!({}),
        }
    }

    fn setup_mcp_fixture() -> McpFixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();

        let mut store = Store::open(&db_path).expect("open store");

        let compute = make_node(
            NodeKind::Function,
            "compute",
            "src/service.rs::fn::compute",
            "src/service.rs",
        );
        store
            .replace_file_graph(
                "src/service.rs",
                "hash:src/service.rs",
                Some("rust"),
                Some(5),
                std::slice::from_ref(&compute),
                &[],
            )
            .expect("replace service graph");

        let handle = make_node(
            NodeKind::Function,
            "handle_request",
            "src/api.rs::fn::handle_request",
            "src/api.rs",
        );
        let handle_calls_compute = make_edge(
            EdgeKind::Calls,
            "src/api.rs::fn::handle_request",
            "src/service.rs::fn::compute",
            "src/api.rs",
        );
        store
            .replace_file_graph(
                "src/api.rs",
                "hash:src/api.rs",
                Some("rust"),
                Some(5),
                std::slice::from_ref(&handle),
                &[handle_calls_compute],
            )
            .expect("replace api graph");

        let compute_test = make_node(
            NodeKind::Test,
            "compute_test",
            "tests/service_test.rs::fn::compute_test",
            "tests/service_test.rs",
        );
        let test_targets_compute = make_edge(
            EdgeKind::Tests,
            "tests/service_test.rs::fn::compute_test",
            "src/service.rs::fn::compute",
            "tests/service_test.rs",
        );
        store
            .replace_file_graph(
                "tests/service_test.rs",
                "hash:tests/service_test.rs",
                Some("rust"),
                Some(5),
                std::slice::from_ref(&compute_test),
                &[test_targets_compute],
            )
            .expect("replace test graph");

        McpFixture { _dir: dir, db_path }
    }

    fn unwrap_tool_text(resp: serde_json::Value) -> String {
        resp.get("content")
            .and_then(|c| c.as_array())
            .and_then(|c| c.first())
            .and_then(|c0| c0.get("text"))
            .and_then(|t| t.as_str())
            .expect("tool response content[0].text")
            .to_owned()
    }

    fn unwrap_tool_format(resp: &serde_json::Value) -> &str {
        resp.get("atlas_output_format")
            .and_then(|value| value.as_str())
            .expect("tool response atlas_output_format")
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
    fn tool_list_includes_get_context() {
        let list = tool_list();
        let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
        assert!(
            tools
                .iter()
                .any(|t| t.get("name") == Some(&"get_context".into())),
            "tools/list must include get_context"
        );
    }

    #[test]
    fn get_context_missing_args_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();
        let _ = Store::open(&db_path).expect("open store");

        // No query/file/files → expect error.
        let result = call(
            "get_context",
            Some(&serde_json::json!({})),
            "/ignored",
            &db_path,
        );
        assert!(
            result.is_err(),
            "empty get_context args must return an error"
        );
    }

    #[test]
    fn get_context_query_returns_packaged_result() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();

        let mut store = Store::open(&db_path).expect("open store");
        let node = Node {
            id: atlas_core::model::NodeId::UNSET,
            kind: NodeKind::Function,
            name: "compute".to_owned(),
            qualified_name: "src/math.rs::fn::compute".to_owned(),
            file_path: "src/math.rs".to_owned(),
            line_start: 1,
            line_end: 5,
            language: "rust".to_owned(),
            parent_name: None,
            params: Some("(x: i32) -> i32".to_owned()),
            return_type: Some("i32".to_owned()),
            modifiers: Some("pub".to_owned()),
            is_test: false,
            file_hash: "h1".to_owned(),
            extra_json: serde_json::json!({}),
        };
        store
            .replace_file_graph("src/math.rs", "h1", Some("rust"), Some(5), &[node], &[])
            .expect("replace_file_graph");

        let args = serde_json::json!({ "query": "compute", "output_format": "json" });
        let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        // Compact PackagedContextResult schema.
        assert!(v.get("intent").is_some(), "result must have intent");
        assert!(v.get("node_count").is_some(), "result must have node_count");
        assert!(
            v.get("nodes").and_then(|n| n.as_array()).is_some(),
            "nodes must be array"
        );
        assert!(
            v.get("truncated").is_some(),
            "result must have truncated flag"
        );
    }

    #[test]
    fn get_context_files_returns_review_intent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();
        let _ = Store::open(&db_path).expect("open store");

        let args = serde_json::json!({ "files": ["src/main.rs"], "output_format": "json" });
        let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(
            v.get("intent").and_then(|i| i.as_str()),
            Some("review"),
            "files arg must produce review intent"
        );
    }

    #[test]
    fn get_context_not_found_returns_empty_nodes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();
        let _ = Store::open(&db_path).expect("open store");

        let args = serde_json::json!({ "query": "nonexistent_xyz_unknown_symbol", "output_format": "json" });
        let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        let node_count = v.get("node_count").and_then(|n| n.as_u64()).unwrap_or(99);
        assert_eq!(node_count, 0, "not-found query must return 0 nodes");
    }

    #[test]
    fn unknown_tool_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();
        let _ = Store::open(&db_path).expect("open store");

        let result = call("unknown_tool_xyz", None, "/ignored", &db_path);
        assert!(result.is_err(), "unknown tool must return an error");
        assert!(result.unwrap_err().to_string().contains("unknown tool"));
    }

    #[test]
    fn tool_list_schema_has_required_fields() {
        let list = tool_list();
        let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
        for tool in tools {
            let name = tool
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("<missing>");
            assert!(
                tool.get("description").is_some(),
                "tool {name} must have description"
            );
            assert!(
                tool.pointer("/inputSchema/type").is_some(),
                "tool {name} must have inputSchema.type"
            );
        }
    }

    #[test]
    fn tool_list_documents_output_format() {
        let list = tool_list();
        let tools = list
            .get("tools")
            .and_then(|value| value.as_array())
            .unwrap();

        for tool in tools {
            let props = tool
                .pointer("/inputSchema/properties")
                .and_then(|value| value.as_object())
                .expect("inputSchema properties");
            assert!(
                props.contains_key("output_format"),
                "tool must document output_format"
            );
        }
    }

    #[test]
    fn tool_list_all_tools_default_to_toon() {
        let list = tool_list();
        let tools = list
            .get("tools")
            .and_then(|value| value.as_array())
            .expect("tools array");

        for tool in tools {
            let description = tool
                .pointer("/inputSchema/properties/output_format/description")
                .and_then(|value| value.as_str())
                .expect("output_format description");
            assert_eq!(description, DEFAULT_OUTPUT_DESCRIPTION);
        }
    }

    #[test]
    fn get_context_defaults_to_toon_output_format() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();

        let mut store = Store::open(&db_path).expect("open store");
        let node = Node {
            id: NodeId::UNSET,
            kind: NodeKind::Function,
            name: "compute".to_owned(),
            qualified_name: "src/math.rs::fn::compute".to_owned(),
            file_path: "src/math.rs".to_owned(),
            line_start: 1,
            line_end: 5,
            language: "rust".to_owned(),
            parent_name: None,
            params: Some("(x: i32) -> i32".to_owned()),
            return_type: Some("i32".to_owned()),
            modifiers: Some("pub".to_owned()),
            is_test: false,
            file_hash: "h1".to_owned(),
            extra_json: serde_json::json!({}),
        };
        store
            .replace_file_graph("src/math.rs", "h1", Some("rust"), Some(5), &[node], &[])
            .expect("replace_file_graph");

        let args = serde_json::json!({ "query": "compute" });
        let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
        let text = unwrap_tool_text(resp.clone());

        assert_eq!(unwrap_tool_format(&resp), "toon");
        assert!(text.contains("intent: symbol"));
    }

    #[test]
    fn explicit_json_override_beats_toon_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();

        let mut store = Store::open(&db_path).expect("open store");
        let node = Node {
            id: NodeId::UNSET,
            kind: NodeKind::Function,
            name: "compute".to_owned(),
            qualified_name: "src/math.rs::fn::compute".to_owned(),
            file_path: "src/math.rs".to_owned(),
            line_start: 1,
            line_end: 5,
            language: "rust".to_owned(),
            parent_name: None,
            params: Some("(x: i32) -> i32".to_owned()),
            return_type: Some("i32".to_owned()),
            modifiers: Some("pub".to_owned()),
            is_test: false,
            file_hash: "h1".to_owned(),
            extra_json: serde_json::json!({}),
        };
        store
            .replace_file_graph("src/math.rs", "h1", Some("rust"), Some(5), &[node], &[])
            .expect("replace_file_graph");

        let args = serde_json::json!({ "query": "compute", "output_format": "json" });
        let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
        let text = unwrap_tool_text(resp.clone());

        assert_eq!(unwrap_tool_format(&resp), "json");
        assert!(serde_json::from_str::<serde_json::Value>(&text).is_ok());
    }

    #[test]
    fn get_context_supports_toon_output_format() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();

        let mut store = Store::open(&db_path).expect("open store");
        let node = Node {
            id: NodeId::UNSET,
            kind: NodeKind::Function,
            name: "compute".to_owned(),
            qualified_name: "src/math.rs::fn::compute".to_owned(),
            file_path: "src/math.rs".to_owned(),
            line_start: 1,
            line_end: 5,
            language: "rust".to_owned(),
            parent_name: None,
            params: Some("(x: i32) -> i32".to_owned()),
            return_type: Some("i32".to_owned()),
            modifiers: Some("pub".to_owned()),
            is_test: false,
            file_hash: "h1".to_owned(),
            extra_json: serde_json::json!({}),
        };
        store
            .replace_file_graph("src/math.rs", "h1", Some("rust"), Some(5), &[node], &[])
            .expect("replace_file_graph");

        let args = serde_json::json!({ "query": "compute", "output_format": "toon" });
        let resp = call("get_context", Some(&args), "/ignored", &db_path).expect("call");
        let text = unwrap_tool_text(resp.clone());

        assert_eq!(unwrap_tool_format(&resp), "toon");
        assert!(text.contains("intent: symbol"));
        assert!(text.contains("node_count: 1"));
        assert!(!text.contains("\"intent\""));
    }

    #[test]
    fn tool_result_value_falls_back_to_json_when_toon_is_empty() {
        let rendered = super::tool_result_value(&serde_json::json!({}), OutputFormat::Toon)
            .expect("tool result");

        assert_eq!(unwrap_tool_format(&rendered), "json");
        assert!(rendered.get("atlas_fallback_reason").is_some());
    }

    #[test]
    fn query_graph_regex_param_filters_results() {
        let fixture = setup_mcp_fixture();
        // Empty text + regex: structural scan, filter by name pattern.
        let args = serde_json::json!({ "regex": "compute", "output_format": "json" });
        let response = call("query_graph", Some(&args), "/ignored", &fixture.db_path)
            .expect("query_graph regex call");
        let text = unwrap_tool_text(response);
        // All returned symbols must have "compute" in name or qualified_name.
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
        if let Some(arr) = v.as_array() {
            for item in arr {
                let qn = item["qn"].as_str().unwrap_or("");
                let name = item["name"].as_str().unwrap_or("");
                assert!(
                    qn.contains("compute") || name.contains("compute"),
                    "regex filter should only return matching symbols, got qn={qn} name={name}"
                );
            }
        }
    }

    #[test]
    fn query_graph_invalid_regex_returns_error() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "regex": "[invalid", "output_format": "json" });
        let result = call("query_graph", Some(&args), "/ignored", &fixture.db_path);
        assert!(result.is_err(), "invalid regex must return an error");
        assert!(
            result.unwrap_err().to_string().contains("invalid regex"),
            "error message should mention invalid regex"
        );
    }

    #[test]
    fn query_graph_response_carries_relationship_guidance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "text": "compute", "output_format": "json" });

        let response = call("query_graph", Some(&args), "/ignored", &fixture.db_path)
            .expect("query_graph call");

        assert_eq!(response["atlas_result_kind"], "symbol_search");
        assert_eq!(response["atlas_usage_edges_included"], false);
        assert_eq!(
            response["atlas_relationship_tools"],
            serde_json::json!(["symbol_neighbors", "traverse_graph", "get_context"])
        );
        assert_eq!(response["content"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn symbol_neighbors_includes_call_edge_sites() {
        let fixture = setup_mcp_fixture();
        let mut store = Store::open(&fixture.db_path).expect("open store");
        let handle = make_node(
            NodeKind::Function,
            "handle_request",
            "src/api.rs::fn::handle_request",
            "src/api.rs",
        );
        let first_call = make_edge(
            EdgeKind::Calls,
            "src/api.rs::fn::handle_request",
            "src/service.rs::fn::compute",
            "src/api.rs",
        );
        let mut second_call = make_edge(
            EdgeKind::Calls,
            "src/api.rs::fn::handle_request",
            "src/service.rs::fn::compute",
            "src/api.rs",
        );
        second_call.line = Some(2);
        store
            .replace_file_graph(
                "src/api.rs",
                "hash:src/api.rs",
                Some("rust"),
                Some(5),
                &[handle],
                &[first_call, second_call],
            )
            .expect("replace api graph");

        let args = serde_json::json!({
            "qname": "src/service.rs::fn::compute",
            "output_format": "json",
        });

        let response = call(
            "symbol_neighbors",
            Some(&args),
            "/ignored",
            &fixture.db_path,
        )
        .expect("symbol_neighbors call");
        let text = unwrap_tool_text(response);
        let value: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(
            value.pointer("/callers/0/qn").and_then(|v| v.as_str()),
            Some("src/api.rs::fn::handle_request")
        );
        assert_eq!(
            value.pointer("/callers/1").and_then(|v| v.as_object()),
            None,
            "caller functions should be de-duplicated"
        );
        assert_eq!(
            value
                .pointer("/caller_edges/0/from")
                .and_then(|v| v.as_str()),
            Some("src/api.rs::fn::handle_request")
        );
        assert_eq!(
            value.pointer("/caller_edges/0/to").and_then(|v| v.as_str()),
            Some("src/service.rs::fn::compute")
        );
        assert_eq!(
            value
                .pointer("/caller_edges/0/file")
                .and_then(|v| v.as_str()),
            Some("src/api.rs")
        );
        assert_eq!(
            value
                .pointer("/caller_edges/0/line")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
        assert_eq!(
            value
                .pointer("/caller_edges/1/line")
                .and_then(|v| v.as_u64()),
            Some(2),
            "edge sites should preserve duplicate render/call instances"
        );
    }

    #[test]
    fn invalid_output_format_returns_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();
        let _ = Store::open(&db_path).expect("open store");

        let args = serde_json::json!({ "query": "compute", "output_format": "xml" });
        let result = call("get_context", Some(&args), "/ignored", &db_path);

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("unsupported output_format")
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
            "output_format": "json",
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

    #[test]
    fn mcp_agent_facing_flows_pass_usability_acceptance_gate() {
        let fixture = setup_mcp_fixture();

        let query_args = serde_json::json!({ "text": "compute" });
        let query_resp = call(
            "query_graph",
            Some(&query_args),
            "/ignored",
            &fixture.db_path,
        )
        .expect("query_graph call");
        let query_text = unwrap_tool_text(query_resp.clone());
        // query_graph returns a float-scored array; toon may fall back to JSON
        // when float round-trip validation fails — that's the expected graceful path.
        let query_format = unwrap_tool_format(&query_resp);
        assert!(
            query_format == "toon" || query_format == "json",
            "expected toon or json, got {query_format}"
        );
        assert!(
            !query_text.is_empty(),
            "query_graph must return ranked results"
        );
        assert!(query_text.contains("src/service.rs::fn::compute"));
        assert_eq!(query_resp["atlas_result_kind"], "symbol_search");
        assert_eq!(query_resp["atlas_usage_edges_included"], false);
        assert!(
            query_resp["atlas_relationship_tools"]
                .as_array()
                .expect("relationship tools array")
                .iter()
                .any(|tool| tool.as_str() == Some("symbol_neighbors"))
        );

        let impact_args = serde_json::json!({ "files": ["src/service.rs"] });
        let impact_resp = call(
            "get_impact_radius",
            Some(&impact_args),
            "/ignored",
            &fixture.db_path,
        )
        .expect("get_impact_radius call");
        let impact_text = unwrap_tool_text(impact_resp.clone());
        assert_eq!(unwrap_tool_format(&impact_resp), "toon");
        assert!(impact_resp.get("atlas_fallback_reason").is_none());
        assert!(impact_text.contains("changed_file_count: 1"));
        assert!(impact_text.contains("src/api.rs::fn::handle_request"));
        assert!(impact_text.contains("tests/service_test.rs::fn::compute_test"));

        let review_args = serde_json::json!({ "files": ["src/service.rs"] });
        let review_resp = call(
            "get_review_context",
            Some(&review_args),
            "/ignored",
            &fixture.db_path,
        )
        .expect("get_review_context call");
        let review_text = unwrap_tool_text(review_resp.clone());
        assert_eq!(unwrap_tool_format(&review_resp), "toon");
        assert!(review_resp.get("atlas_fallback_reason").is_none());
        assert!(review_text.contains("intent: review"));
        assert!(review_text.contains("file_count:"));
        assert!(review_text.contains("src/service.rs"));
        assert!(review_text.contains("src/api.rs"));

        let context_args = serde_json::json!({ "query": "compute" });
        let context_resp = call(
            "get_context",
            Some(&context_args),
            "/ignored",
            &fixture.db_path,
        )
        .expect("get_context call");
        let context_text = unwrap_tool_text(context_resp.clone());
        assert_eq!(unwrap_tool_format(&context_resp), "toon");
        assert!(context_resp.get("atlas_fallback_reason").is_none());
        assert!(context_text.contains("intent: symbol"));
        assert!(context_text.contains("src/service.rs::fn::compute"));
        assert!(context_text.contains("src/api.rs::fn::handle_request"));
    }
}
