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
use atlas_reasoning::ReasoningEngine;
use atlas_repo::{DiffTarget, changed_files, find_repo_root};
use atlas_review::{
    ContextEngine, ResolvedTarget, normalize_qn_kind_tokens, query_parser, resolve_target,
};
use atlas_search::search as fts_search;
use atlas_search::semantic as sem;
use atlas_store_sqlite::{BuildFinishStats, GraphBuildState, Store};
use camino::Utf8Path;
use serde::Serialize;

use crate::context::{compact_node, package_context_result, package_impact};
use crate::discovery_tools::{
    tool_search_content, tool_search_files, tool_search_templates, tool_search_text_assets,
};
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
                "description": "Full-text search the code graph by symbol name or identifier. Returns a compact, ranked list of matching symbols only; it does not return caller/callee usage edges. IMPORTANT: text is matched against indexed symbol names and qualified names (identifiers like 'BalancesTab', 'useFilteredBalances'), NOT against natural language — use short exact symbol names, not descriptive phrases. Follow up with symbol_neighbors, traverse_graph, or get_context when you need relationships.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text":     { "type": "string",  "description": "Symbol name or identifier to search (e.g. 'BalancesTab', 'useFilteredBalances'). FTS matches against indexed symbol names and qualified names — use short exact identifiers, NOT natural language phrases. Multi-word phrases rarely produce hits." },
                        "kind":     { "type": "string",  "description": "Filter by node kind (e.g. 'function', 'struct')" },
                        "language": { "type": "string",  "description": "Filter by language (e.g. 'rust', 'python')" },
                        "limit":    { "type": "integer", "description": "Maximum results to return (default 20)" },
                        "semantic": { "type": "boolean", "description": "Graph-neighbour expansion on top of FTS: re-ranks initial FTS hits using graph edges (default false). NOT vector/embedding search — still requires FTS to find at least one initial symbol-name hit. If FTS returns nothing (e.g. text was a phrase not a symbol name), semantic expansion also returns nothing. Use regex instead for pattern-based fallback." },
                        "expand":   { "type": "boolean", "description": "Expand results through graph edges after ranking (default false). Subsumed by semantic=true; setting both is redundant." },
                        "expand_hops": { "type": "integer", "description": "Max edge hops when expand=true (default 1)" },
                        "regex":    { "type": "string",  "description": "Regex pattern matched against name and qualified_name via SQL UDF. Three modes: (1) regex-only structural scan when text is empty — filters every node in the DB; (2) text+regex: FTS5 runs first then the UDF post-filters its candidates inside SQLite; (3) invalid pattern returns an error with details. Supports regex crate alternation syntax (e.g. 'handle|HANDLE|Handle_'). Must be valid regex crate syntax." },
                        "subpath":  { "type": "string",  "description": "Restrict results to nodes whose file_path starts with this prefix (e.g. 'src/auth', 'packages/atlas-core'). Filtering happens in SQL before ranking." },
                        "fuzzy":    { "type": "boolean", "description": "Enable fuzzy (edit-distance) name-matching boost for near-miss symbol names (default false). Adds +4 score to symbols whose name is within edit-distance threshold of the query. Useful for typo recovery; prefer a single short identifier for best results." },
                        "hybrid":   { "type": "boolean", "description": "Enable hybrid FTS + vector retrieval with Reciprocal Rank Fusion (default false). Requires ATLAS_EMBED_URL to be set; falls back to FTS-only when no embedding backend is configured." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "batch_query_graph",
                "description": "Run multiple query_graph searches in a single round-trip. Provide EITHER 'text' (a space- or comma-separated list of symbol names that is auto-split into one query per token, e.g. 'BalancesTab, compute, handleRequest') OR 'queries' (an explicit array of query objects). Returns an array of per-query results. Each token/query uses the same symbol-name FTS as query_graph — pass short exact identifiers, not natural-language phrases. Max 20 tokens/queries per call.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                            "description": "Space- or comma-separated symbol names to look up. Each delimiter-separated token becomes an independent FTS query (e.g. 'BalancesTab, compute, handleRequest' or 'BalancesTab compute handleRequest'). Mutually exclusive with 'queries'; if both are given, 'text' wins."
                        },
                        "queries": {
                            "type": "array",
                            "description": "Array of query objects (max 20). Each object accepts the same fields as query_graph: text, kind, language, limit, semantic, expand, expand_hops, regex, subpath, fuzzy, hybrid. Use when per-query options differ.",
                            "maxItems": 20,
                            "items": {
                                "type": "object",
                                "properties": {
                                    "text":        { "type": "string",  "description": "Symbol name or identifier to search (e.g. 'BalancesTab'). Required unless 'regex' is set." },
                                    "kind":        { "type": "string",  "description": "Filter by node kind (e.g. 'function', 'struct')" },
                                    "language":    { "type": "string",  "description": "Filter by language (e.g. 'rust', 'typescript')" },
                                    "limit":       { "type": "integer", "description": "Maximum results for this query (default 20)" },
                                    "semantic":    { "type": "boolean", "description": "Graph-neighbour expansion on top of FTS (default false). Requires FTS to find at least one hit first." },
                                    "expand":      { "type": "boolean", "description": "Expand results through graph edges after ranking (default false)" },
                                    "expand_hops": { "type": "integer", "description": "Max edge hops when expand=true (default 1)" },
                                    "regex":       { "type": "string",  "description": "Regex pattern matched against name and qualified_name via SQL UDF. Must be valid regex crate syntax." },
                                    "subpath":     { "type": "string",  "description": "Restrict results to nodes whose file_path starts with this prefix." },
                                    "fuzzy":       { "type": "boolean", "description": "Enable fuzzy name-matching boost (default false)." },
                                    "hybrid":      { "type": "boolean", "description": "Enable hybrid FTS + vector retrieval (default false). Requires ATLAS_EMBED_URL." }
                                },
                                "required": []
                            }
                        },
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
                "description": "Build bounded context around a symbol, file, or change-set. Provide EITHER 'query' (a symbol name, qualified name, or structured intent phrase like 'who calls MyFunc') OR 'file' (a repo-relative path) OR 'files' (a list of changed paths). Returns ranked nodes, edges, files, and truncation/ambiguity metadata. IMPORTANT: 'query' is matched against indexed symbol names — it does NOT accept natural-language descriptions. Use short exact identifiers or intent phrases.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":     { "type": "string",  "description": "Symbol name, qualified name, or intent phrase. Examples: 'AuthService' (symbol), 'src/lib.rs::fn::foo' (qualified name), 'who calls handle_request' (usage lookup), 'what breaks MyFunc' (impact). Do NOT pass natural-language descriptions — they will not match any graph nodes. Alternative to file/files." },
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
            },
            {
                "name": "search_files",
                "description": "Discover files by name or path glob. Use this when you need config files, templates, SQL, Markdown, or other non-code assets that are not indexed as graph symbols. For symbol/relationship questions use query_graph instead.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "pattern":        { "type": "string",  "description": "Glob pattern matched against file names and repo-relative paths (e.g. '*.sql', '**/*.toml', 'config/*')." },
                        "globs":          { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters: only files whose repo-relative path matches at least one of these globs are considered." },
                        "exclude_globs":  { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters: files matching any of these globs are skipped (e.g. ['**/generated/**', '**/*.min.js'])." },
                        "subpath":        { "type": "string",  "description": "Scope the walk to a repo sub-directory (e.g. 'packages/api'). Useful for monorepos." },
                        "case_sensitive": { "type": "boolean", "description": "Match pattern case-sensitively (default false)." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["pattern"]
                }
            },
            {
                "name": "search_content",
                "description": "Search file contents by literal string or regex. Use this when you need to find text that is not a symbol name (e.g. error messages, config keys, comments, SQL queries). Generated and vendored files are excluded by default. For symbol/relationship questions use query_graph instead.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":              { "type": "string",  "description": "Text to search for. Literal string by default; set is_regex=true for regex patterns." },
                        "globs":              { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters: only files matching at least one glob are searched." },
                        "exclude_globs":      { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters: files matching any of these globs are skipped." },
                        "exclude_generated":  { "type": "boolean", "description": "Skip generated/vendor files (node_modules, dist, *.min.js, etc.). Default true." },
                        "is_regex":           { "type": "boolean", "description": "Treat query as a regex pattern (default false). Literal queries are case-insensitive by default." },
                        "context_lines":      { "type": "integer", "description": "Lines of context to include before and after each match (default 0)." },
                        "max_results":        { "type": "integer", "description": "Maximum match lines to return (default 50)." },
                        "subpath":            { "type": "string",  "description": "Scope the walk to a repo sub-directory (e.g. 'services/auth'). Useful for monorepos." },
                        "output_format":      { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "search_templates",
                "description": "Discover template files (HTML, Jinja2, Handlebars, Tera, Mako, Mustache, Twig, Liquid, ERB, HAML, Pug) by extension. Narrows by `kind` when you know the template engine. Prefer this over search_files for template-specific discovery. For symbol/relationship questions use query_graph instead.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "kind":           { "type": "string",  "description": "Template engine: html, jinja, handlebars, tera, mako, mustache, twig, liquid, erb, haml, pug. Omit to search all template types." },
                        "globs":          { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters." },
                        "exclude_globs":  { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters." },
                        "subpath":        { "type": "string",  "description": "Scope the walk to a repo sub-directory. Useful for monorepos." },
                        "case_sensitive": { "type": "boolean", "description": "Match case-sensitively (default false)." },
                        "max_results":    { "type": "integer", "description": "Maximum files to return (default 100)." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "search_text_assets",
                "description": "Discover SQL, config (TOML/YAML/INI), environment (.env), and prompt files. Use `kind` to narrow to a specific asset type. These files are not indexed as graph symbols; use query_graph for symbol/relationship questions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "kind":           { "type": "string",  "description": "Asset type: sql, config, env, prompt. Omit to search all text asset types." },
                        "globs":          { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters." },
                        "exclude_globs":  { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters." },
                        "subpath":        { "type": "string",  "description": "Scope the walk to a repo sub-directory. Useful for monorepos." },
                        "case_sensitive": { "type": "boolean", "description": "Match case-sensitively (default false)." },
                        "max_results":    { "type": "integer", "description": "Maximum files to return (default 100)." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "status",
                "description": "Return a compact graph health summary: build state, node/edge counts, last-indexed timestamp, and a machine-readable failure category. Call this before query_graph or get_context to verify the graph is healthy and up-to-date. Succeeds even when the graph DB is missing.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "doctor",
                "description": "Run full repo health checks: git root detection, .atlas dir, config file, DB open/integrity, graph build state, tracked file count, and retrieval-index state. Returns an array of per-check results with pass/fail and detail. Call before trusting graph-backed context after a fresh clone or suspected corruption.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "db_check",
                "description": "Run SQLite integrity check and scan for orphan nodes (no edges) and dangling edges (missing endpoint). Returns ok=true when all checks pass. Use to diagnose corrupt or inconsistent graph rows.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit":         { "type": "integer", "description": "Maximum orphan/dangling samples to return (default 100)." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "debug_graph",
                "description": "Return internal graph diagnostics: node/edge counts by kind, top files by node count, orphan nodes, and dangling edges. Use to investigate structural anomalies or unexpected empty results.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit":         { "type": "integer", "description": "Maximum orphan/dangling samples to return (default 20)." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "explain_query",
                "description": "Explain how a query_graph invocation would be executed: tokenisation, FTS phrase construction, regex validation, and expected search path. Use to diagnose why query_graph returns no results or to validate a regex pattern before running it.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text":          { "type": "string",  "description": "Symbol name or identifier — same as query_graph 'text'." },
                        "kind":          { "type": "string",  "description": "Node kind filter (e.g. 'function', 'struct') — same as query_graph 'kind'." },
                        "language":      { "type": "string",  "description": "Language filter (e.g. 'rust') — same as query_graph 'language'." },
                        "limit":         { "type": "integer", "description": "Result limit (default 20)." },
                        "semantic":      { "type": "boolean", "description": "Whether semantic expansion would be applied (default false)." },
                        "regex":         { "type": "string",  "description": "Regex pattern — validated and explained. Regex-only (text empty): structural scan. text+regex: FTS5 first then UDF post-filter. Invalid pattern: error with details." },
                        "subpath":       { "type": "string",  "description": "File-path prefix filter — same as query_graph 'subpath'." },
                        "fuzzy":         { "type": "boolean", "description": "Whether fuzzy name-matching boost would be active (default false)." },
                        "hybrid":        { "type": "boolean", "description": "Whether hybrid FTS + vector retrieval would be used (default false). Requires ATLAS_EMBED_URL." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "analyze_safety",
                "description": "Score how safe it is to refactor a symbol. Returns fan-in, fan-out, test adjacency, cross-module caller count, and a 0–1 safety score with band (safe/moderate/risky) and suggested validations.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol":        { "type": "string",  "description": "Fully-qualified symbol name (e.g. 'src/auth.rs::fn::verify_token')." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["symbol"]
                }
            },
            {
                "name": "analyze_remove",
                "description": "Analyse the impact of removing one or more symbols. Returns impacted symbols, files, and tests separated by confidence tier (Definite/Probable/Weak), plus uncertainty flags.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbols":       { "type": "array",   "items": { "type": "string" }, "description": "Fully-qualified symbol names to remove." },
                        "max_depth":     { "type": "integer", "description": "Traversal depth limit (default 3)." },
                        "max_nodes":     { "type": "integer", "description": "Maximum impacted nodes to return (default 200)." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["symbols"]
                }
            },
            {
                "name": "analyze_dead_code",
                "description": "Detect dead-code candidates: nodes with no inbound semantic edges that are not public, not tests, and not in the entrypoint allowlist. Returns candidates with certainty tiers and blocker flags.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "allowlist":     { "type": "array",   "items": { "type": "string" }, "description": "Qualified names to exclude from dead-code candidates even when they have no inbound edges." },
                        "subpath":       { "type": "string",  "description": "Restrict scan to nodes whose file_path starts with this prefix (e.g. 'src/internal')." },
                        "limit":         { "type": "integer", "description": "Maximum candidates to return (default 500)." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "analyze_dependency",
                "description": "Check whether removing a symbol is safe by verifying it has no remaining semantic references. Returns removable verdict, blocking callers, confidence tier, and suggested cleanups.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol":        { "type": "string",  "description": "Fully-qualified symbol name to check (e.g. 'src/lib.rs::fn::legacy_parse')." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["symbol"]
                }
            },
            {
                "name": "resolve_symbol",
                "description": "Resolve a symbol name to its exact qualified_name in the graph. Eliminates the manual workflow of query_graph → copy qualified_name → call symbol_neighbors. Returns the best match, an ambiguity list when multiple symbols match, and follow-up suggestions. Accepts public kind aliases (e.g. 'function'/'fn', 'struct'/'record') that are mapped to the compact tokens used in qualified names.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name":          { "type": "string",  "description": "Symbol name to resolve (e.g. 'LoadIdentityMessages', 'compute'). Matched against indexed symbol names using FTS." },
                        "kind":          { "type": "string",  "description": "Optional kind filter. Accepts public aliases: 'function'/'fn'/'func', 'method', 'class', 'struct'/'record', 'interface'/'iface', 'trait', 'enum', 'module'/'mod', 'variable'/'var', 'constant'/'const', 'test', 'import', 'package'/'pkg', 'file'." },
                        "file":          { "type": "string",  "description": "Optional file path filter. Only returns matches whose file_path contains this string (e.g. 'internal/requestctx/context.go' or 'src/')." },
                        "language":      { "type": "string",  "description": "Optional language filter (e.g. 'rust', 'go', 'typescript')." },
                        "limit":         { "type": "integer", "description": "Maximum matches to return (default 10)." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["name"]
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
    let mut response = match name {
        "list_graph_stats" => tool_list_graph_stats(db_path, output_format),
        "query_graph" => tool_query_graph(args, db_path, output_format),
        "batch_query_graph" => tool_batch_query_graph(args, db_path, output_format),
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
        "search_files" => tool_search_files(args, repo_root, output_format),
        "search_content" => tool_search_content(args, repo_root, output_format),
        "search_templates" => tool_search_templates(args, repo_root, output_format),
        "search_text_assets" => tool_search_text_assets(args, repo_root, output_format),
        "status" => tool_status(repo_root, db_path, output_format),
        "doctor" => tool_doctor(repo_root, db_path, output_format),
        "db_check" => tool_db_check(args, db_path, output_format),
        "debug_graph" => tool_debug_graph(args, db_path, output_format),
        "explain_query" => tool_explain_query(args, db_path, output_format),
        "resolve_symbol" => tool_resolve_symbol(args, db_path, output_format),
        "analyze_safety" => tool_analyze_safety(args, db_path, output_format),
        "analyze_remove" => tool_analyze_remove(args, db_path, output_format),
        "analyze_dead_code" => tool_analyze_dead_code(args, db_path, output_format),
        "analyze_dependency" => tool_analyze_dependency(args, db_path, output_format),
        other => return Err(anyhow::anyhow!("unknown tool: {other}")),
    }?;

    // MCP7: inject compact provenance envelope into every successful response.
    inject_provenance(&mut response, repo_root, db_path);

    Ok(response)
}

fn default_output_format_for_tool(_name: &str) -> OutputFormat {
    OutputFormat::Toon
}

/// Inject compact provenance metadata into every MCP tool response (MCP7).
///
/// Adds `atlas_provenance` at the top level of the response object, alongside
/// `atlas_output_format`. Provenance never goes inside `content[0].text` so it
/// does not bloat TOON/JSON payload rendering.
///
/// Falls back silently when the store cannot be opened (e.g. no graph built
/// yet, or tool does not touch the DB).
fn inject_provenance(response: &mut serde_json::Value, repo_root: &str, db_path: &str) {
    let (indexed_file_count, last_indexed_at) = if let Ok(store) = Store::open(db_path) {
        if let Ok(meta) = store.provenance_meta() {
            (meta.indexed_file_count, meta.last_indexed_at)
        } else {
            (0, None)
        }
    } else {
        (0, None)
    };

    response["atlas_provenance"] = serde_json::json!({
        "repo_root": repo_root,
        "db_path": db_path,
        "indexed_file_count": indexed_file_count,
        "last_indexed_at": last_indexed_at,
    });
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
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let fuzzy = bool_arg(args, "fuzzy").unwrap_or(false);
    let hybrid = bool_arg(args, "hybrid").unwrap_or(false);

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
        subpath,
        graph_expand: expand,
        graph_max_hops: expand_hops,
        regex_pattern: regex,
        fuzzy_match: fuzzy,
        hybrid,
        ..Default::default()
    };

    let results = if semantic {
        sem::expanded_search(&store, &query).context("semantic search failed")?
    } else if fuzzy || hybrid {
        // Use the full atlas_search::search path which implements fuzzy edit-distance
        // fallback and hybrid FTS + vector retrieval with RRF.
        fts_search(&store, &query).context("search failed")?
    } else {
        store.search(&query).context("search failed")?
    };

    // Determine active query mode for response metadata.
    let active_query_mode = match (
        query.text.trim().is_empty(),
        query.regex_pattern.is_some(),
        semantic,
        hybrid,
    ) {
        (true, true, _, _) => "regex_structural_scan",
        (false, false, false, true) => "fts5_vector_hybrid",
        (false, false, true, _) => "fts5_graph_expand",
        (false, true, false, _) => "fts5_regex_filter",
        (false, true, true, _) => "fts5_regex_filter_graph_expand",
        _ => "fts5",
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
    // MCP7: truncation metadata — agents need to know when results are capped.
    response["atlas_result_count"] = serde_json::json!(compact.len());
    response["atlas_truncated"] = serde_json::json!(compact.len() == limit);
    response["atlas_query_mode"] = serde_json::Value::String(active_query_mode.to_owned());
    if compact.is_empty() && semantic {
        // FTS found no symbol names matching the query text; graph expansion
        // had nothing to seed from. Guide the LLM toward productive next steps.
        response["atlas_hint"] = serde_json::Value::String(
            "FTS found no symbol names matching the query text. \
             FTS searches indexed identifiers, not natural language phrases. \
             Try: (1) a short exact symbol name like 'BalancesTab'; \
             (2) the regex param for pattern matching (e.g. regex='Balance'); \
             (3) get_context with a file path; \
             (4) list_graph_stats to confirm the graph has been built."
                .to_owned(),
        );
    }
    Ok(response)
}

fn tool_batch_query_graph(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    const MAX_QUERIES: usize = 20;

    // If top-level `text` is provided, split by whitespace into per-token queries.
    // Otherwise fall back to the explicit `queries` array.
    let text_phrase = str_arg(args, "text")?.filter(|s| !s.trim().is_empty());
    let synthesized: Vec<serde_json::Value>;
    let queries_val: &[serde_json::Value] = if let Some(phrase) = text_phrase {
        synthesized = phrase
            .split(|c: char| c.is_whitespace() || c == ',')
            .filter(|tok| !tok.is_empty())
            .map(|tok| serde_json::json!({ "text": tok }))
            .collect();
        &synthesized
    } else {
        let arr = args
            .and_then(|a| a.get("queries"))
            .and_then(|v| v.as_array())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "batch_query_graph requires either a 'text' string \
                     (space-separated tokens) or a non-empty 'queries' array"
                )
            })?;
        arr.as_slice()
    };

    if queries_val.is_empty() {
        anyhow::bail!(
            "batch_query_graph requires either a 'text' string \
             (space-separated tokens) or a non-empty 'queries' array"
        );
    }
    if queries_val.len() > MAX_QUERIES {
        anyhow::bail!(
            "batch_query_graph exceeds the maximum of {MAX_QUERIES} queries per call; \
             split into smaller batches"
        );
    }

    // Open store once for all queries.
    let store = open_store(db_path)?;

    #[derive(Serialize)]
    struct BatchItem {
        query_index: usize,
        text: String,
        items: Vec<BatchResultNode>,
        #[serde(skip_serializing_if = "Option::is_none")]
        atlas_hint: Option<String>,
    }

    #[derive(Serialize)]
    struct BatchResultNode {
        score: f64,
        name: String,
        qualified_name: String,
        kind: String,
        file_path: String,
        line_start: u32,
        language: String,
    }

    let mut batch_results: Vec<BatchItem> = Vec::with_capacity(queries_val.len());

    for (idx, q) in queries_val.iter().enumerate() {
        let q_args = Some(q);
        let text = str_arg(q_args, "text")?
            .map(str::to_owned)
            .unwrap_or_default();
        let kind = str_arg(q_args, "kind")?.map(str::to_owned);
        let language = str_arg(q_args, "language")?.map(str::to_owned);
        let limit = u64_arg(q_args, "limit").unwrap_or(20) as usize;
        let semantic = bool_arg(q_args, "semantic").unwrap_or(false);
        let expand = bool_arg(q_args, "expand").unwrap_or(false);
        let expand_hops = u64_arg(q_args, "expand_hops").unwrap_or(1) as u32;
        let regex = str_arg(q_args, "regex")?.map(str::to_owned);
        let subpath = str_arg(q_args, "subpath")?.map(str::to_owned);
        let fuzzy = bool_arg(q_args, "fuzzy").unwrap_or(false);
        let hybrid = bool_arg(q_args, "hybrid").unwrap_or(false);

        if text.trim().is_empty() && regex.is_none() {
            anyhow::bail!("query at index {idx} requires non-empty 'text' or a 'regex' pattern");
        }
        if let Some(ref pat) = regex {
            if pat.trim().is_empty() {
                anyhow::bail!("query at index {idx}: regex pattern must not be empty");
            }
            regex::Regex::new(pat)
                .map_err(|e| anyhow::anyhow!("query at index {idx}: invalid regex pattern: {e}"))?;
        }

        let query = SearchQuery {
            text: text.clone(),
            kind,
            language,
            limit,
            subpath,
            graph_expand: expand,
            graph_max_hops: expand_hops,
            regex_pattern: regex,
            fuzzy_match: fuzzy,
            hybrid,
            ..Default::default()
        };

        let results = if semantic {
            sem::expanded_search(&store, &query).context("semantic search failed")?
        } else {
            store.search(&query).context("search failed")?
        };

        let items: Vec<BatchResultNode> = results
            .iter()
            .map(|r| BatchResultNode {
                score: (r.score * 1000.0).round() / 1000.0,
                name: r.node.name.clone(),
                qualified_name: r.node.qualified_name.clone(),
                kind: r.node.kind.as_str().to_owned(),
                file_path: r.node.file_path.clone(),
                line_start: r.node.line_start,
                language: r.node.language.clone(),
            })
            .collect();

        let atlas_hint = if items.is_empty() && semantic {
            Some(
                "FTS found no symbol names matching the query text. \
                 FTS searches indexed identifiers, not natural language phrases. \
                 Try: (1) a short exact symbol name like 'BalancesTab'; \
                 (2) the regex param for pattern matching (e.g. regex='Balance'); \
                 (3) get_context with a file path; \
                 (4) list_graph_stats to confirm the graph has been built."
                    .to_owned(),
            )
        } else {
            None
        };

        batch_results.push(BatchItem {
            query_index: idx,
            text,
            items,
            atlas_hint,
        });
    }

    let mut response = tool_result_value(&batch_results, output_format)?;
    response["atlas_result_kind"] = serde_json::Value::String("batch_symbol_search".to_owned());
    response["atlas_query_count"] =
        serde_json::Value::Number(serde_json::Number::from(batch_results.len()));
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
    let mut response = tool_result_value(&packaged, output_format)?;
    // When the engine returned nothing, guide the LLM toward productive next steps.
    if result.nodes.is_empty() {
        response["atlas_error_code"] = serde_json::Value::String("node_not_found".to_owned());
        response["atlas_message"] =
            serde_json::Value::String(error_message("node_not_found").to_owned());
        response["atlas_suggestions"] = serde_json::json!(error_suggestions("node_not_found"));
        response["atlas_hint"] = serde_json::Value::String(
            "No graph nodes matched this request. Possible causes: \
             (1) the graph has not been built yet — run build_or_update_graph first; \
             (2) 'query' contained a natural-language phrase instead of a symbol name or \
             qualified name — try a short exact identifier (e.g. 'BalancesTab') or \
             use query_graph with regex for pattern matching; \
             (3) the file path is wrong or the file has no indexed symbols."
                .to_owned(),
        );
    }
    Ok(response)
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

    let all_empty = result.callers.is_empty()
        && result.callees.is_empty()
        && result.tests.is_empty()
        && result.siblings.is_empty()
        && result.import_neighbors.is_empty();

    let mut response = tool_result_value(&result, output_format)?;

    // If every collection is empty, check whether the symbol exists at all so
    // agents receive a consistent "node_not_found" signal rather than silent
    // empty arrays.
    if all_empty {
        let exists = store
            .node_by_qname(&qname)
            .map(|n| n.is_some())
            .unwrap_or(false);
        if !exists {
            response["atlas_error_code"] = serde_json::Value::String("node_not_found".to_owned());
            response["atlas_message"] =
                serde_json::Value::String(error_message("node_not_found").to_owned());
            response["atlas_suggestions"] = serde_json::json!(error_suggestions("node_not_found"));
        }
    }

    Ok(response)
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

// ---------------------------------------------------------------------------
// MCP8: Health and debug tools
// ---------------------------------------------------------------------------

/// Machine-readable failure category for a repo health check.
///
/// Returned in `status` responses when the graph is not healthy so agents can
/// branch on specific failure modes without parsing human-readable strings.
fn failure_category(
    db_exists: bool,
    db_open_ok: bool,
    build_state: Option<&atlas_store_sqlite::GraphBuildState>,
) -> &'static str {
    if !db_exists {
        return "missing_graph_db";
    }
    if !db_open_ok {
        return "corrupt_or_inconsistent_graph_rows";
    }
    match build_state {
        Some(atlas_store_sqlite::GraphBuildState::Building) => "interrupted_build",
        Some(atlas_store_sqlite::GraphBuildState::BuildFailed) => "failed_build",
        _ => "none",
    }
}

/// Human-readable explanation for a machine-readable error code.
fn error_message(error_code: &str) -> &'static str {
    match error_code {
        "none" => "Graph is healthy and up-to-date.",
        "missing_graph_db" => "Graph database not found. Run `atlas build` to create it.",
        "corrupt_or_inconsistent_graph_rows" => {
            "Graph database has integrity issues. Run `atlas build` to rebuild from scratch."
        }
        "interrupted_build" => {
            "Previous build was interrupted and did not complete. Run `atlas build` to restart."
        }
        "failed_build" => {
            "Last build failed. Check build_last_error for details, then run `atlas build` to retry."
        }
        "node_not_found" => "No graph nodes matched this request.",
        "checks_failed" => "One or more health checks failed.",
        _ => "An unknown error occurred.",
    }
}

/// Actionable next steps for a machine-readable error code.
fn error_suggestions(error_code: &str) -> &'static [&'static str] {
    match error_code {
        "none" => &[],
        "missing_graph_db" => &[
            "run `atlas build` to create the graph",
            "run `atlas init` if the project is new",
        ],
        "corrupt_or_inconsistent_graph_rows" => {
            &["run `atlas build` to rebuild the graph from scratch"]
        }
        "interrupted_build" => &["run `atlas build` to restart the interrupted build"],
        "failed_build" => &[
            "check the build_last_error field for details",
            "run `atlas build` to retry",
        ],
        "node_not_found" => &[
            "verify the symbol name with query_graph or resolve_symbol",
            "run status to confirm the graph is built",
            "run build_or_update_graph to index the repo first",
        ],
        "checks_failed" => &["inspect the checks array for details"],
        _ => &[],
    }
}

fn tool_status(
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let db_exists = std::path::Path::new(db_path).exists();

    // Attempt to open the store; failures are surfaced via the health fields,
    // not as errors — so agents always receive a valid response.
    let store_result = if db_exists {
        Some(Store::open(db_path))
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

fn tool_doctor(
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
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

    // 1. Git repo root
    match find_repo_root(Utf8Path::new(repo_root)) {
        Ok(root) => checks.push(pass!("git_root", root.as_str())),
        Err(e) => checks.push(fail!("git_root", e.to_string())),
    }

    // 2. .atlas dir
    let atlas_dir = atlas_engine::paths::atlas_dir(repo_root);
    if atlas_dir.exists() {
        checks.push(pass!("atlas_dir", atlas_dir.display().to_string()));
    } else {
        checks.push(fail!(
            "atlas_dir",
            format!("{} not found — run `atlas init`", atlas_dir.display())
        ));
    }

    // 3. Config file
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

    // 4. DB file
    let db_exists = std::path::Path::new(db_path).exists();
    if db_exists {
        checks.push(pass!("db_file", db_path));
    } else {
        checks.push(fail!(
            "db_file",
            format!("{db_path} not found — run `atlas init`")
        ));
    }

    // 5. DB open + integrity + build state
    if db_exists {
        match Store::open(db_path) {
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

    // 6. git ls-files
    match collect_files(Utf8Path::new(repo_root), None) {
        Ok(files) => checks.push(pass!(
            "git_ls_files",
            format!("{} tracked files", files.len())
        )),
        Err(e) => checks.push(fail!("git_ls_files", e.to_string())),
    }

    // 7. Retrieval/content index
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

fn tool_db_check(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
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

fn tool_debug_graph(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
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

fn tool_explain_query(
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
    let regex = str_arg(args, "regex")?.map(str::to_owned);
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let fuzzy = bool_arg(args, "fuzzy").unwrap_or(false);
    let hybrid = bool_arg(args, "hybrid").unwrap_or(false);

    if text.trim().is_empty() && regex.is_none() {
        anyhow::bail!("explain_query requires non-empty text or a regex pattern");
    }

    // Tokenise text the same way the FTS engine will: split on whitespace/punctuation,
    // keep only non-empty tokens.
    let fts_tokens: Vec<String> = if text.trim().is_empty() {
        vec![]
    } else {
        text.split(|c: char| !c.is_alphanumeric() && c != '_')
            .filter(|t| !t.is_empty())
            .map(str::to_owned)
            .collect()
    };

    // Build the FTS phrase that will be sent to SQLite FTS5.
    let fts_phrase = if fts_tokens.is_empty() {
        None
    } else if fts_tokens.len() == 1 {
        Some(format!("\"{}\"", fts_tokens[0]))
    } else {
        // Multi-token: FTS5 implicit AND across tokens.
        Some(
            fts_tokens
                .iter()
                .map(|t| format!("\"{t}\""))
                .collect::<Vec<_>>()
                .join(" "),
        )
    };

    // Validate regex separately; non-fatal — we report the error in the result.
    let (regex_valid, regex_error) = if let Some(ref pat) = regex {
        match regex::Regex::new(pat) {
            Ok(_) => (true, None),
            Err(e) => (false, Some(e.to_string())),
        }
    } else {
        (true, None)
    };

    // Determine the search path that query_graph would follow.
    let search_path = match (text.trim().is_empty(), regex.is_some(), semantic, hybrid) {
        (true, true, _, _) => "regex_structural_scan", // text empty, regex only
        (false, false, false, true) => "fts5_vector_hybrid",
        (false, false, true, _) => "fts5_graph_expand",
        (false, true, false, _) => "fts5_regex_filter",
        (false, true, true, _) => "fts5_regex_filter_graph_expand",
        _ => "fts5",
    };

    // Active ranking boosts that will be applied.
    let mut ranking_factors: Vec<&str> = vec!["fts5_bm25"];
    if fuzzy {
        ranking_factors.push("fuzzy_edit_distance_boost");
    }
    if hybrid {
        ranking_factors.push("vector_rrf_merge");
    }
    if semantic {
        ranking_factors.push("graph_neighbor_rerank");
    }

    // Count indexed nodes to help agents understand graph coverage.
    let db_exists = std::path::Path::new(db_path).exists();
    let indexed_node_count: Option<i64> = if db_exists {
        Store::open(db_path)
            .ok()
            .and_then(|s| s.stats().ok())
            .map(|st| st.node_count)
    } else {
        None
    };

    let warnings: Vec<&str> = {
        let mut w = vec![];
        if fts_tokens.len() > 1 {
            w.push(
                "Multi-token text is matched as implicit AND across all tokens; \
                 this often returns zero results. Prefer a single short identifier.",
            );
        }
        if text.contains(' ') && regex.is_none() {
            w.push(
                "Natural-language phrases rarely match FTS5 symbol names. \
                 Use regex for pattern matching or pass a single exact identifier.",
            );
        }
        if !regex_valid {
            w.push("regex pattern is invalid; the query would return an error.");
        }
        w
    };

    let result = serde_json::json!({
        "active_query_mode": search_path,
        "search_path": search_path,
        "input": {
            "text": text,
            "kind": kind,
            "language": language,
            "limit": limit,
            "semantic": semantic,
            "regex": regex,
            "subpath": subpath,
            "fuzzy": fuzzy,
            "hybrid": hybrid,
        },
        "fts_tokens": fts_tokens,
        "fts_phrase": fts_phrase,
        "regex_valid": regex_valid,
        "regex_error": regex_error,
        "ranking_factors": ranking_factors,
        "filters_applied": {
            "kind": kind.is_some(),
            "language": language.is_some(),
            "subpath": subpath.is_some(),
            "fuzzy": fuzzy,
            "hybrid": hybrid,
        },
        "indexed_node_count": indexed_node_count,
        "db_exists": db_exists,
        "warnings": warnings,
    });

    tool_result_value(&result, output_format)
}

/// Map public kind aliases to the internal kind string stored in the graph.
///
/// Qualified names use compact tokens (e.g. `fn`, `struct`) while the `kind`
/// column stores the full NodeKind string (e.g. `function`, `struct`).
/// Agents may supply either form; this function normalises both.
fn resolve_kind_alias(input: &str) -> String {
    match input.to_ascii_lowercase().as_str() {
        "fn" | "func" | "function" => "function",
        "method" | "meth" => "method",
        "class" => "class",
        "struct" | "record" => "struct",
        "interface" | "iface" => "interface",
        "trait" => "trait",
        "enum" => "enum",
        "module" | "mod" => "module",
        "variable" | "var" | "field" => "variable",
        "constant" | "const" => "constant",
        "test" => "test",
        "import" | "use" => "import",
        "package" | "pkg" => "package",
        "file" => "file",
        other => other,
    }
    .to_owned()
}

fn tool_resolve_symbol(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    const DEFAULT_LIMIT: usize = 10;

    let name = str_arg(args, "name")?
        .ok_or_else(|| anyhow::anyhow!("resolve_symbol requires 'name'"))?
        .to_owned();
    let kind_input = str_arg(args, "kind")?.map(str::to_owned);
    let file_filter = str_arg(args, "file")?.map(str::to_owned);
    let language = str_arg(args, "language")?.map(str::to_owned);
    let limit = u64_arg(args, "limit").unwrap_or(DEFAULT_LIMIT as u64) as usize;

    if name.trim().is_empty() {
        anyhow::bail!("resolve_symbol requires non-empty 'name'");
    }

    let store = open_store(db_path)?;

    // If `name` looks like a qualified name (contains `::` suggesting the
    // `<file>::<kind>::<symbol>` structure), route through `resolve_target`
    // which performs exact lookup, alias normalisation (e.g. `::function::` →
    // `::fn::`), and falls back to FTS suggestions on miss.  This exposes
    // `resolve_target` as a first-class public surface without requiring
    // callers to know the canonical token from `query_graph` first.
    if name.contains("::") {
        let target = ContextTarget::QualifiedName {
            qname: name.clone(),
        };
        match resolve_target(&store, &target).context("resolve_symbol qname lookup failed")? {
            ResolvedTarget::Node(node) => {
                #[derive(Serialize)]
                struct ResolvedMatch<'a> {
                    qualified_name: &'a str,
                    name: &'a str,
                    kind: &'a str,
                    file_path: &'a str,
                    language: &'a str,
                    line_start: u32,
                }
                let m = ResolvedMatch {
                    qualified_name: &node.qualified_name,
                    name: &node.name,
                    kind: node.kind.as_str(),
                    file_path: &node.file_path,
                    language: &node.language,
                    line_start: node.line_start,
                };
                // Annotate with canonical QN if an alias was normalised.
                let normalised = normalize_qn_kind_tokens(&name);
                let alias_note = if normalised != name {
                    Some(format!(
                        "Input '{name}' normalised to canonical QN '{normalised}'"
                    ))
                } else {
                    None
                };
                let result = serde_json::json!({
                    "qualified_name": m.qualified_name,
                    "resolved": true,
                    "ambiguous": false,
                    "match_count": 1,
                    "atlas_truncated": false,
                    "matches": [m],
                    "alias_note": alias_note,
                    "suggestions": [{
                        "hint": "Exact match resolved. Pass qualified_name to symbol_neighbors \
                                 or traverse_graph for callers, callees, and relationships.",
                        "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
                    }],
                });
                return tool_result_value(&result, output_format);
            }
            ResolvedTarget::Ambiguous(meta) => {
                let result = serde_json::json!({
                    "qualified_name": meta.candidates.first(),
                    "resolved": false,
                    "ambiguous": true,
                    "match_count": meta.candidates.len(),
                    "atlas_truncated": false,
                    "matches": serde_json::Value::Array(
                        meta.candidates.iter().map(|qn| serde_json::json!({"qualified_name": qn})).collect()
                    ),
                    "suggestions": [{
                        "hint": "Multiple symbols match. Narrow with 'file', 'kind', or 'language'. \
                                 Then pass the exact qualified_name to symbol_neighbors or traverse_graph.",
                        "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
                    }],
                });
                return tool_result_value(&result, output_format);
            }
            ResolvedTarget::NotFound { suggestions } => {
                let result = serde_json::json!({
                    "qualified_name": null,
                    "resolved": false,
                    "ambiguous": false,
                    "match_count": 0,
                    "atlas_truncated": false,
                    "matches": [],
                    "suggestions": [{
                        "hint": format!(
                            "No symbol matched '{}'. Verify canonical QN tokens (e.g. '::fn::' not '::function::'). \
                             Candidates: {:?}. Try query_graph or resolve_symbol with a shorter name.",
                            name, suggestions
                        ),
                        "candidates": suggestions,
                        "next_tools": ["query_graph", "explain_query"]
                    }],
                });
                return tool_result_value(&result, output_format);
            }
            // File-path or special targets are not returned for a QN input.
            ResolvedTarget::File(_) => {}
        }
    }

    // Resolve public kind aliases (e.g. "fn" → "function", "func" → "function").
    let resolved_kind = kind_input.as_deref().map(resolve_kind_alias);

    // Fetch extra candidates to allow post-filtering by file path.
    let fetch_limit = (limit * 4).max(40);
    let query = SearchQuery {
        text: name.clone(),
        kind: resolved_kind.clone(),
        language: language.clone(),
        limit: fetch_limit,
        ..Default::default()
    };
    let results = store
        .search(&query)
        .context("resolve_symbol search failed")?;

    // Apply optional file-path substring filter.
    let filtered: Vec<_> = if let Some(ref file_pat) = file_filter {
        results
            .into_iter()
            .filter(|r| r.node.file_path.contains(file_pat.as_str()))
            .collect()
    } else {
        results
    };

    // Rank: exact name match first (case-insensitive), then preserve FTS score order.
    let name_lower = name.to_ascii_lowercase();
    let mut ranked: Vec<_> = filtered.into_iter().enumerate().collect();
    ranked.sort_by(|(ai, a), (bi, b)| {
        let a_exact = a.node.name.to_ascii_lowercase() == name_lower;
        let b_exact = b.node.name.to_ascii_lowercase() == name_lower;
        b_exact.cmp(&a_exact).then_with(|| ai.cmp(bi))
    });

    let total_before_limit = ranked.len();
    let ranked: Vec<_> = ranked.into_iter().map(|(_, r)| r).take(limit).collect();
    let truncated = total_before_limit > ranked.len();

    let best_qn = ranked.first().map(|r| r.node.qualified_name.as_str());
    let ambiguous = ranked.len() > 1;

    #[derive(Serialize)]
    struct ResolvedMatch<'a> {
        qualified_name: &'a str,
        name: &'a str,
        kind: &'a str,
        file_path: &'a str,
        language: &'a str,
        line_start: u32,
    }

    let matches: Vec<ResolvedMatch<'_>> = ranked
        .iter()
        .map(|r| ResolvedMatch {
            qualified_name: &r.node.qualified_name,
            name: &r.node.name,
            kind: r.node.kind.as_str(),
            file_path: &r.node.file_path,
            language: &r.node.language,
            line_start: r.node.line_start,
        })
        .collect();

    // Build agent-facing suggestions.
    let suggestions: Vec<serde_json::Value> = if best_qn.is_some() {
        if ambiguous {
            vec![serde_json::json!({
                "hint": "Multiple symbols match. Narrow with 'file', 'kind', or 'language'. \
                         Then pass the exact qualified_name to symbol_neighbors or traverse_graph.",
                "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
            })]
        } else {
            vec![serde_json::json!({
                "hint": "Exact match resolved. Pass qualified_name to symbol_neighbors \
                         or traverse_graph for callers, callees, and relationships.",
                "next_tools": ["symbol_neighbors", "traverse_graph", "get_context"]
            })]
        }
    } else {
        vec![serde_json::json!({
            "hint": "No symbol matched. Try query_graph with a regex pattern, \
                     or use explain_query to validate the search input.",
            "next_tools": ["query_graph", "explain_query"]
        })]
    };

    let result = serde_json::json!({
        "qualified_name": best_qn,
        "resolved": best_qn.is_some(),
        "ambiguous": ambiguous,
        "match_count": matches.len(),
        "atlas_truncated": truncated,
        "matches": matches,
        "suggestions": suggestions,
    });

    tool_result_value(&result, output_format)
}

// ---------------------------------------------------------------------------
// MCP10.1 — Analysis tool wrappers
// ---------------------------------------------------------------------------

fn tool_analyze_safety(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let symbol = str_arg(args, "symbol")?
        .ok_or_else(|| anyhow::anyhow!("analyze_safety requires 'symbol'"))?
        .to_owned();

    let store = open_store(db_path)?;
    let engine = ReasoningEngine::new(&store);
    let result = engine
        .score_refactor_safety(&symbol)
        .with_context(|| format!("safety scoring for `{symbol}` failed"))?;

    let payload = serde_json::json!({
        "symbol": result.node.qualified_name,
        "kind": result.node.kind.as_str(),
        "file": result.node.file_path,
        "safety_score": result.safety.score,
        "safety_band": format!("{:?}", result.safety.band),
        "fan_in": result.fan_in,
        "fan_out": result.fan_out,
        "linked_tests": result.linked_test_count,
        "unresolved_edges": result.unresolved_edge_count,
        "reasons": result.safety.reasons,
        "suggested_validations": result.safety.suggested_validations,
        "evidence": result.evidence.iter().map(|e| serde_json::json!({ "key": e.key, "value": e.value })).collect::<Vec<_>>(),
    });
    tool_result_value(&payload, output_format)
}

fn tool_analyze_remove(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let symbols = string_array_arg(args, "symbols")?;
    if symbols.is_empty() {
        return Err(anyhow::anyhow!(
            "analyze_remove requires at least one symbol in 'symbols'"
        ));
    }
    let max_depth = u64_arg(args, "max_depth").unwrap_or(3) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;

    let store = open_store(db_path)?;
    let engine = ReasoningEngine::new(&store);
    let symbol_refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
    let result = engine
        .analyze_removal(&symbol_refs, Some(max_depth), Some(max_nodes))
        .context("analyze_removal failed")?;

    // Compact bounded evidence: cap impacted symbols at 50 for agent output.
    const SYMBOL_CAP: usize = 50;
    let omitted = result.impacted_symbols.len().saturating_sub(SYMBOL_CAP);

    let impacted_preview: Vec<_> = result
        .impacted_symbols
        .iter()
        .take(SYMBOL_CAP)
        .map(|im| {
            serde_json::json!({
                "qn": im.node.qualified_name,
                "kind": im.node.kind.as_str(),
                "file": im.node.file_path,
                "depth": im.depth,
                "impact_class": format!("{:?}", im.impact_class),
            })
        })
        .collect();

    let payload = serde_json::json!({
        "seed_count": result.seed.len(),
        "impacted_symbol_count": result.impacted_symbols.len(),
        "impacted_file_count": result.impacted_files.len(),
        "impacted_test_count": result.impacted_tests.len(),
        "impacted_symbols": impacted_preview,
        "impacted_files": result.impacted_files,
        "omitted_symbol_count": omitted,
        "warnings": result.warnings.iter().map(|w| serde_json::json!({ "message": w.message, "confidence": format!("{:?}", w.confidence) })).collect::<Vec<_>>(),
        "uncertainty_flags": result.uncertainty_flags,
        "evidence": result.evidence.iter().map(|e| serde_json::json!({ "key": e.key, "value": e.value })).collect::<Vec<_>>(),
    });
    tool_result_value(&payload, output_format)
}

fn tool_analyze_dead_code(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let allowlist = string_array_arg(args, "allowlist").unwrap_or_default();
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    let limit = u64_arg(args, "limit").unwrap_or(500) as usize;

    let store = open_store(db_path)?;
    let engine = ReasoningEngine::new(&store);
    let allowlist_refs: Vec<&str> = allowlist.iter().map(String::as_str).collect();
    let candidates = engine
        .detect_dead_code(&allowlist_refs, subpath.as_deref(), Some(limit))
        .context("detect_dead_code failed")?;

    // Cap candidate output at 100 for agent output.
    const CANDIDATE_CAP: usize = 100;
    let omitted = candidates.len().saturating_sub(CANDIDATE_CAP);

    let preview: Vec<_> = candidates
        .iter()
        .take(CANDIDATE_CAP)
        .map(|c| {
            serde_json::json!({
                "qn": c.node.qualified_name,
                "kind": c.node.kind.as_str(),
                "file": c.node.file_path,
                "line": c.node.line_start,
                "certainty": format!("{:?}", c.certainty),
                "reasons": c.reasons,
                "blockers": c.blockers,
            })
        })
        .collect();

    let payload = serde_json::json!({
        "candidate_count": candidates.len(),
        "omitted_count": omitted,
        "candidates": preview,
        "applied_limit": limit,
        "applied_subpath": subpath,
    });
    tool_result_value(&payload, output_format)
}

fn tool_analyze_dependency(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let symbol = str_arg(args, "symbol")?
        .ok_or_else(|| anyhow::anyhow!("analyze_dependency requires 'symbol'"))?
        .to_owned();

    let store = open_store(db_path)?;
    let engine = ReasoningEngine::new(&store);
    let result = engine
        .check_dependency_removal(&symbol)
        .with_context(|| format!("dependency check for `{symbol}` failed"))?;

    // Compact bounded evidence: cap blocking refs at 20.
    const BLOCKER_CAP: usize = 20;
    let omitted = result.blocking_references.len().saturating_sub(BLOCKER_CAP);

    let blocking_preview: Vec<_> = result
        .blocking_references
        .iter()
        .take(BLOCKER_CAP)
        .map(|n| {
            serde_json::json!({
                "qn": n.qualified_name,
                "kind": n.kind.as_str(),
                "file": n.file_path,
            })
        })
        .collect();

    let payload = serde_json::json!({
        "symbol": result.target_qname,
        "removable": result.removable,
        "confidence": format!("{:?}", result.confidence),
        "blocking_reference_count": result.blocking_references.len(),
        "blocking_references": blocking_preview,
        "omitted_blocking_count": omitted,
        "suggested_cleanups": result.suggested_cleanups,
        "uncertainty_flags": result.uncertainty_flags,
        "evidence": result.evidence.iter().map(|e| serde_json::json!({ "key": e.key, "value": e.value })).collect::<Vec<_>>(),
    });
    tool_result_value(&payload, output_format)
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
    fn semantic_empty_result_includes_hint() {
        let fixture = setup_mcp_fixture();
        // Natural-language phrase that won't match any symbol name via FTS.
        let args = serde_json::json!({ "text": "balances tab portfolio asset balance usd notional", "semantic": true, "output_format": "json" });

        let response = call("query_graph", Some(&args), "/ignored", &fixture.db_path)
            .expect("query_graph semantic call");

        // Results should be empty because no symbol name matches the phrase.
        let content = response["content"].as_array().expect("content array");
        assert_eq!(content.len(), 1);
        let text = content[0]["text"].as_str().unwrap_or("");
        // The data payload should be an empty array.
        assert!(text.contains("[]"), "expected empty results, got: {text}");
        // The hint field must be present to guide the LLM.
        assert!(
            response["atlas_hint"].as_str().is_some(),
            "expected atlas_hint when semantic returns empty, got none"
        );
        let hint = response["atlas_hint"].as_str().unwrap();
        assert!(
            hint.contains("FTS found no symbol names"),
            "hint should explain FTS limitation: {hint}"
        );
    }

    #[test]
    fn batch_query_graph_returns_per_query_results() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({
            "queries": [
                { "text": "compute", "output_format": "json" },
                { "text": "handle_request", "output_format": "json" }
            ],
            "output_format": "json"
        });

        let response = call(
            "batch_query_graph",
            Some(&args),
            "/ignored",
            &fixture.db_path,
        )
        .expect("batch_query_graph call");

        assert_eq!(response["atlas_result_kind"], "batch_symbol_search");
        assert_eq!(response["atlas_query_count"], 2);

        // Extract the JSON array from content text.
        let text = response["content"][0]["text"].as_str().unwrap();
        let items: serde_json::Value = serde_json::from_str(text).expect("parse batch result");
        let arr = items.as_array().expect("array");
        assert_eq!(arr.len(), 2);

        // First query result: index 0, text = "compute".
        assert_eq!(arr[0]["query_index"], 0);
        assert_eq!(arr[0]["text"], "compute");
        let first_items = arr[0]["items"].as_array().expect("items array");
        assert!(!first_items.is_empty(), "expected results for 'compute'");
        assert!(
            first_items
                .iter()
                .any(|n| n["qualified_name"] == "src/service.rs::fn::compute")
        );

        // Second query result: index 1, text = "handle_request".
        assert_eq!(arr[1]["query_index"], 1);
        assert_eq!(arr[1]["text"], "handle_request");
        let second_items = arr[1]["items"].as_array().expect("items array");
        assert!(
            !second_items.is_empty(),
            "expected results for 'handle_request'"
        );
    }

    #[test]
    fn batch_query_graph_empty_queries_returns_error() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "queries": [] });
        let result = call(
            "batch_query_graph",
            Some(&args),
            "/ignored",
            &fixture.db_path,
        );
        assert!(result.is_err(), "expected error for empty queries array");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("non-empty") || msg.contains("requires"),
            "error should be descriptive: {msg}"
        );
    }

    #[test]
    fn batch_query_graph_text_phrase_splits_and_queries_each_token() {
        let fixture = setup_mcp_fixture();
        // Pass a phrase — handler splits by whitespace into ["compute", "handle_request"].
        let args = serde_json::json!({
            "text": "compute handle_request",
            "output_format": "json"
        });

        let response = call(
            "batch_query_graph",
            Some(&args),
            "/ignored",
            &fixture.db_path,
        )
        .expect("batch_query_graph with text phrase");

        assert_eq!(response["atlas_result_kind"], "batch_symbol_search");
        assert_eq!(response["atlas_query_count"], 2);

        let text = response["content"][0]["text"].as_str().unwrap();
        let arr: serde_json::Value = serde_json::from_str(text).expect("parse batch result");
        let arr = arr.as_array().expect("array");
        assert_eq!(arr.len(), 2, "one result per token");
        assert_eq!(arr[0]["text"], "compute");
        assert_eq!(arr[1]["text"], "handle_request");
        assert!(
            !arr[0]["items"].as_array().unwrap().is_empty(),
            "compute should have results"
        );
    }

    #[test]
    fn batch_query_graph_over_limit_returns_error() {
        let fixture = setup_mcp_fixture();
        // Build 21 query objects.
        let queries: Vec<serde_json::Value> = (0..21)
            .map(|i| serde_json::json!({ "text": format!("sym{i}") }))
            .collect();
        let args = serde_json::json!({ "queries": queries });
        let result = call(
            "batch_query_graph",
            Some(&args),
            "/ignored",
            &fixture.db_path,
        );
        assert!(result.is_err(), "expected error for >20 queries");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("maximum"),
            "error should mention maximum: {msg}"
        );
    }

    #[test]
    fn batch_query_graph_partial_empty_result_carries_hint() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({
            "queries": [
                // Valid symbol — will find results.
                { "text": "compute" },
                // NL phrase — FTS finds nothing; semantic=true triggers hint.
                { "text": "balances tab portfolio asset usd notional", "semantic": true }
            ],
            "output_format": "json"
        });

        let response = call(
            "batch_query_graph",
            Some(&args),
            "/ignored",
            &fixture.db_path,
        )
        .expect("batch_query_graph call");

        let text = response["content"][0]["text"].as_str().unwrap();
        let items: serde_json::Value = serde_json::from_str(text).expect("parse batch result");
        let arr = items.as_array().expect("array");
        assert_eq!(arr.len(), 2);

        // First query should have results and no hint.
        let first_items = arr[0]["items"].as_array().expect("items");
        assert!(!first_items.is_empty());
        assert!(
            arr[0].get("atlas_hint").is_none(),
            "no hint for successful query"
        );

        // Second query should have empty items and an atlas_hint.
        let second_items = arr[1]["items"].as_array().expect("items");
        assert!(
            second_items.is_empty(),
            "expected empty results for NL phrase"
        );
        let hint = arr[1]["atlas_hint"].as_str().expect("atlas_hint present");
        assert!(
            hint.contains("FTS found no symbol names"),
            "hint should explain FTS limit: {hint}"
        );
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

    // -----------------------------------------------------------------------
    // MCP7 — provenance metadata tests
    // -----------------------------------------------------------------------

    fn assert_provenance(resp: &serde_json::Value, expected_repo: &str, expected_db: &str) {
        let prov = resp
            .get("atlas_provenance")
            .expect("atlas_provenance must be present");
        assert_eq!(
            prov.get("repo_root").and_then(|v| v.as_str()),
            Some(expected_repo),
            "provenance.repo_root mismatch"
        );
        assert_eq!(
            prov.get("db_path").and_then(|v| v.as_str()),
            Some(expected_db),
            "provenance.db_path mismatch"
        );
        assert!(
            prov.get("indexed_file_count")
                .and_then(|v| v.as_i64())
                .is_some(),
            "provenance.indexed_file_count must be an integer"
        );
        // last_indexed_at is allowed to be null when no files indexed.
        assert!(
            prov.get("last_indexed_at").is_some(),
            "provenance.last_indexed_at key must be present"
        );
    }

    #[test]
    fn list_graph_stats_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let resp =
            call("list_graph_stats", None, "/repo", &fixture.db_path).expect("list_graph_stats");
        assert_provenance(&resp, "/repo", &fixture.db_path);
        let prov = &resp["atlas_provenance"];
        assert_eq!(prov["indexed_file_count"].as_i64(), Some(3));
        assert!(prov["last_indexed_at"].as_str().is_some());
    }

    #[test]
    fn query_graph_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "text": "compute" });
        let resp =
            call("query_graph", Some(&args), "/repo", &fixture.db_path).expect("query_graph");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn get_impact_radius_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "files": ["src/service.rs"] });
        let resp = call("get_impact_radius", Some(&args), "/repo", &fixture.db_path)
            .expect("get_impact_radius");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn get_review_context_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "files": ["src/service.rs"] });
        let resp = call("get_review_context", Some(&args), "/repo", &fixture.db_path)
            .expect("get_review_context");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn traverse_graph_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "from_qn": "src/service.rs::fn::compute", "output_format": "json" });
        let resp =
            call("traverse_graph", Some(&args), "/repo", &fixture.db_path).expect("traverse_graph");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn get_context_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "query": "compute", "output_format": "json" });
        let resp =
            call("get_context", Some(&args), "/repo", &fixture.db_path).expect("get_context");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn symbol_neighbors_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args =
            serde_json::json!({ "qname": "src/service.rs::fn::compute", "output_format": "json" });
        let resp = call("symbol_neighbors", Some(&args), "/repo", &fixture.db_path)
            .expect("symbol_neighbors");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn symbol_neighbors_missing_qname_sets_error_code() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({
            "qname": "src/nonexistent.rs::fn::ghost",
            "output_format": "json",
        });
        let resp = call("symbol_neighbors", Some(&args), "/repo", &fixture.db_path)
            .expect("symbol_neighbors should not error for missing symbol");
        assert_eq!(
            resp["atlas_error_code"].as_str(),
            Some("node_not_found"),
            "missing qname must set atlas_error_code=node_not_found"
        );
        assert!(
            resp["atlas_message"].as_str().is_some(),
            "missing qname must include atlas_message"
        );
        let suggestions = resp["atlas_suggestions"]
            .as_array()
            .expect("atlas_suggestions");
        assert!(
            !suggestions.is_empty(),
            "must include suggestions for missing symbol"
        );
    }

    #[test]
    fn cross_file_links_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "file": "src/service.rs", "output_format": "json" });
        let resp = call("cross_file_links", Some(&args), "/repo", &fixture.db_path)
            .expect("cross_file_links");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn concept_clusters_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "files": ["src/service.rs"], "output_format": "json" });
        let resp = call("concept_clusters", Some(&args), "/repo", &fixture.db_path)
            .expect("concept_clusters");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn batch_query_graph_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "text": "compute" });
        let resp = call("batch_query_graph", Some(&args), "/repo", &fixture.db_path)
            .expect("batch_query_graph");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn provenance_indexed_file_count_is_zero_for_empty_db() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("empty.db").to_string_lossy().to_string();
        let _ = Store::open(&db_path).expect("open store");

        let resp = call("list_graph_stats", None, "/repo", &db_path)
            .expect("list_graph_stats on empty db");
        let prov = &resp["atlas_provenance"];
        assert_eq!(
            prov["indexed_file_count"].as_i64(),
            Some(0),
            "empty db must report 0 indexed files"
        );
        assert!(
            prov["last_indexed_at"].is_null(),
            "empty db last_indexed_at must be null"
        );
    }

    // -----------------------------------------------------------------------
    // MCP8 — health and debug tool tests
    // -----------------------------------------------------------------------

    #[test]
    fn status_healthy_repo_returns_ok() {
        let fixture = setup_mcp_fixture();
        let store = Store::open(&fixture.db_path).expect("open");
        store
            .finish_build(
                "/repo",
                atlas_store_sqlite::BuildFinishStats {
                    files_discovered: 3,
                    files_processed: 3,
                    files_failed: 0,
                    nodes_written: 3,
                    edges_written: 2,
                },
            )
            .expect("finish_build");

        let args = serde_json::json!({ "output_format": "json" });
        let resp = call("status", Some(&args), "/repo", &fixture.db_path).expect("status call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(
            v["ok"].as_bool(),
            Some(true),
            "healthy repo ok must be true"
        );
        assert_eq!(
            v["error_code"].as_str(),
            Some("none"),
            "healthy repo must have error_code=none"
        );
        assert!(v["message"].as_str().is_some(), "must have message");
        assert!(
            v["suggestions"].as_array().is_some(),
            "must have suggestions"
        );
        assert_eq!(v["build_state"].as_str(), Some("built"));
        assert!(v["node_count"].as_i64().unwrap_or(0) > 0);
    }

    #[test]
    fn status_missing_db_returns_error_code() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir
            .path()
            .join("no_such_subdir")
            .join("atlas.db")
            .to_string_lossy()
            .to_string();

        let args = serde_json::json!({ "output_format": "json" });
        let resp = call("status", Some(&args), "/repo", &missing).expect("status should not error");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(v["ok"].as_bool(), Some(false));
        assert_eq!(v["error_code"].as_str(), Some("missing_graph_db"));
        assert!(v["message"].as_str().is_some(), "must have message");
        assert!(!v["suggestions"].as_array().expect("suggestions").is_empty());
        assert_eq!(v["db_exists"].as_bool(), Some(false));
    }

    #[test]
    fn status_build_failed_returns_error_code() {
        let fixture = setup_mcp_fixture();
        let store = Store::open(&fixture.db_path).expect("open");
        store.begin_build("/repo").expect("begin_build");
        store
            .fail_build("/repo", "parse error in src/main.rs")
            .expect("fail_build");

        let args = serde_json::json!({ "output_format": "json" });
        let resp = call("status", Some(&args), "/repo", &fixture.db_path).expect("status call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(v["ok"].as_bool(), Some(false));
        assert_eq!(v["error_code"].as_str(), Some("failed_build"));
        assert!(v["message"].as_str().is_some(), "must have message");
        assert_eq!(v["build_state"].as_str(), Some("build_failed"));
    }

    #[test]
    fn status_interrupted_build_returns_category() {
        let fixture = setup_mcp_fixture();
        let store = Store::open(&fixture.db_path).expect("open");
        // begin_build without finish — simulates interrupted build.
        store.begin_build("/repo").expect("begin_build");

        let args = serde_json::json!({ "output_format": "json" });
        let resp = call("status", Some(&args), "/repo", &fixture.db_path).expect("status call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(v["ok"].as_bool(), Some(false));
        assert_eq!(v["error_code"].as_str(), Some("interrupted_build"));
        assert!(v["message"].as_str().is_some(), "must have message");
        assert_eq!(v["build_state"].as_str(), Some("building"));
    }

    #[test]
    fn doctor_returns_checks_array() {
        let fixture = setup_mcp_fixture();
        // Use the tempdir as repo root; git root detection may fail but
        // other checks (db_file, db_open, graph_stats) still run.
        let dir_path = fixture._dir.path().to_string_lossy().to_string();
        let args = serde_json::json!({ "output_format": "json" });

        let resp = call("doctor", Some(&args), &dir_path, &fixture.db_path).expect("doctor call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert!(v.get("ok").is_some(), "doctor must have ok field");
        assert!(
            v["error_code"].as_str().is_some(),
            "doctor must have error_code"
        );
        assert!(v["message"].as_str().is_some(), "doctor must have message");
        assert!(
            v["suggestions"].as_array().is_some(),
            "doctor must have suggestions"
        );
        let checks = v["checks"].as_array().expect("checks must be an array");
        assert!(!checks.is_empty(), "checks must not be empty");
        for item in checks {
            assert!(item.get("check").is_some(), "check item must have check");
            assert!(item.get("ok").is_some(), "check item must have ok");
            assert!(item.get("detail").is_some(), "check item must have detail");
        }
        // db_file check must pass (db exists in fixture).
        let db_item = checks.iter().find(|c| c["check"] == "db_file");
        assert!(db_item.is_some(), "db_file check must be present");
        assert_eq!(
            db_item.unwrap()["ok"].as_bool(),
            Some(true),
            "db_file check must pass when db exists"
        );
    }

    #[test]
    fn doctor_missing_db_fails_db_check() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir
            .path()
            .join("no_such_subdir")
            .join("atlas.db")
            .to_string_lossy()
            .to_string();
        let dir_path = dir.path().to_string_lossy().to_string();
        let args = serde_json::json!({ "output_format": "json" });

        let resp = call("doctor", Some(&args), &dir_path, &missing).expect("doctor must not error");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(
            v["ok"].as_bool(),
            Some(false),
            "doctor must report ok=false"
        );
        assert_eq!(
            v["error_code"].as_str(),
            Some("checks_failed"),
            "doctor must report error_code=checks_failed"
        );
        assert!(v["message"].as_str().is_some(), "doctor must have message");
        let checks = v["checks"].as_array().expect("checks array");
        let db_file_item = checks.iter().find(|c| c["check"] == "db_file");
        assert!(db_file_item.is_some(), "db_file check must be present");
        assert_eq!(
            db_file_item.unwrap()["ok"].as_bool(),
            Some(false),
            "db_file check must fail when db missing"
        );
    }

    #[test]
    fn db_check_healthy_returns_ok() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "output_format": "json" });
        let resp = call("db_check", Some(&args), "/repo", &fixture.db_path).expect("db_check call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(v["ok"].as_bool(), Some(true), "healthy db must be ok");
        assert_eq!(v["error_code"].as_str(), Some("none"));
        assert!(v["message"].as_str().is_some(), "must have message");
        assert!(
            v["suggestions"].as_array().is_some(),
            "must have suggestions"
        );
        let issues = v["integrity_issues"]
            .as_array()
            .expect("integrity_issues array");
        assert_eq!(issues.len(), 0, "healthy db must have 0 integrity issues");
    }

    #[test]
    fn db_check_on_path_in_missing_dir_returns_error() {
        // A path whose parent directory does not exist cannot be opened by SQLite.
        let dir = tempfile::tempdir().expect("tempdir");
        let bad_path = dir
            .path()
            .join("no_such_subdir")
            .join("atlas.db")
            .to_string_lossy()
            .to_string();

        let result = call("db_check", None, "/repo", &bad_path);
        assert!(
            result.is_err(),
            "db_check on unreachable path must return error"
        );
    }

    #[test]
    fn debug_graph_returns_node_counts() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "output_format": "json" });
        let resp =
            call("debug_graph", Some(&args), "/repo", &fixture.db_path).expect("debug_graph call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert!(v["nodes"].as_i64().unwrap_or(0) > 0, "nodes must be > 0");
        assert!(v["edges"].as_i64().unwrap_or(0) > 0, "edges must be > 0");
        assert!(v["files"].as_i64().unwrap_or(0) > 0, "files must be > 0");
        assert!(v.get("edges_by_kind").is_some(), "must have edges_by_kind");
        assert!(
            v.get("top_files_by_node_count").is_some(),
            "must have top_files_by_node_count"
        );
    }

    #[test]
    fn explain_query_describes_fts_path() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "text": "compute", "output_format": "json" });
        let resp = call("explain_query", Some(&args), "/repo", &fixture.db_path)
            .expect("explain_query call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(v["search_path"].as_str(), Some("fts5"));
        let tokens = v["fts_tokens"].as_array().expect("fts_tokens array");
        assert!(
            tokens.iter().any(|t| t.as_str() == Some("compute")),
            "tokens must contain 'compute'"
        );
        assert_eq!(v["fts_phrase"].as_str(), Some("\"compute\""));
        assert_eq!(v["regex_valid"].as_bool(), Some(true));
    }

    #[test]
    fn explain_query_missing_input_returns_error() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "output_format": "json" });
        let result = call("explain_query", Some(&args), "/repo", &fixture.db_path);
        assert!(
            result.is_err(),
            "explain_query with no text and no regex must return error"
        );
    }

    #[test]
    fn explain_query_validates_invalid_regex() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "regex": "[invalid", "output_format": "json" });
        let resp = call("explain_query", Some(&args), "/repo", &fixture.db_path)
            .expect("explain_query should not error on invalid regex");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(
            v["regex_valid"].as_bool(),
            Some(false),
            "invalid regex must report regex_valid=false"
        );
        assert!(
            v["regex_error"].as_str().is_some(),
            "must include regex_error message"
        );
        let warnings = v["warnings"].as_array().expect("warnings array");
        assert!(
            warnings
                .iter()
                .any(|w| w.as_str().is_some_and(|s| s.contains("invalid"))),
            "warnings must mention invalid regex"
        );
    }

    #[test]
    fn explain_query_with_regex_only_uses_structural_scan_path() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "regex": "compute.*", "output_format": "json" });
        let resp = call("explain_query", Some(&args), "/repo", &fixture.db_path)
            .expect("explain_query regex-only call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(v["search_path"].as_str(), Some("regex_structural_scan"));
        assert_eq!(v["regex_valid"].as_bool(), Some(true));
    }

    #[test]
    fn status_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let resp = call("status", None, "/repo", &fixture.db_path).expect("status call");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn doctor_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let dir_path = fixture._dir.path().to_string_lossy().to_string();
        let resp = call("doctor", None, &dir_path, &fixture.db_path).expect("doctor call");
        assert_provenance(&resp, &dir_path, &fixture.db_path);
    }

    #[test]
    fn db_check_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let resp = call("db_check", None, "/repo", &fixture.db_path).expect("db_check call");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn debug_graph_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let resp = call("debug_graph", None, "/repo", &fixture.db_path).expect("debug_graph call");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn explain_query_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "text": "compute", "output_format": "json" });
        let resp =
            call("explain_query", Some(&args), "/repo", &fixture.db_path).expect("explain_query");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    // -----------------------------------------------------------------------
    // MCP3 — resolve_symbol tests
    // -----------------------------------------------------------------------

    #[test]
    fn resolve_symbol_finds_exact_match() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "name": "compute", "output_format": "json" });
        let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
            .expect("resolve_symbol call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(v["resolved"].as_bool(), Some(true));
        assert_eq!(
            v["qualified_name"].as_str(),
            Some("src/service.rs::fn::compute"),
            "must resolve to the compute function qn"
        );
        // match_count may be > 1 because FTS also matches "compute_test"; best match
        // must be the exact name hit ranked first.
        assert!(v["match_count"].as_i64().unwrap_or(0) >= 1);
        let matches = v["matches"].as_array().expect("matches array");
        assert!(!matches.is_empty());
        assert_eq!(matches[0]["kind"].as_str(), Some("function"));
        assert_eq!(matches[0]["file_path"].as_str(), Some("src/service.rs"));
    }

    #[test]
    fn resolve_symbol_missing_name_returns_error() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "output_format": "json" });
        let result = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path);
        assert!(result.is_err(), "resolve_symbol without name must error");
    }

    #[test]
    fn resolve_symbol_empty_name_returns_error() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "name": "", "output_format": "json" });
        let result = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path);
        assert!(result.is_err(), "resolve_symbol with empty name must error");
    }

    #[test]
    fn resolve_symbol_no_match_returns_unresolved() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "name": "nonexistent_symbol_xyz", "output_format": "json" });
        let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
            .expect("resolve_symbol call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(v["resolved"].as_bool(), Some(false));
        assert!(v["qualified_name"].is_null());
        assert_eq!(v["match_count"].as_i64(), Some(0));
        let suggestions = v["suggestions"].as_array().expect("suggestions array");
        assert!(!suggestions.is_empty());
        let hint = suggestions[0]["hint"].as_str().expect("hint string");
        assert!(hint.contains("query_graph") || hint.contains("explain_query"));
    }

    #[test]
    fn resolve_symbol_kind_alias_fn_resolves_to_function() {
        let fixture = setup_mcp_fixture();
        // "fn" is the compact qualified-name token; kind column stores "function".
        let args = serde_json::json!({ "name": "compute", "kind": "fn", "output_format": "json" });
        let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
            .expect("resolve_symbol call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(v["resolved"].as_bool(), Some(true));
        assert_eq!(
            v["qualified_name"].as_str(),
            Some("src/service.rs::fn::compute"),
        );
    }

    #[test]
    fn resolve_symbol_file_filter_narrows_results() {
        let fixture = setup_mcp_fixture();
        // handle_request is in src/api.rs
        let args = serde_json::json!({
            "name": "handle_request",
            "file": "src/api.rs",
            "output_format": "json"
        });
        let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
            .expect("resolve_symbol call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert_eq!(v["resolved"].as_bool(), Some(true));
        assert_eq!(
            v["qualified_name"].as_str(),
            Some("src/api.rs::fn::handle_request"),
        );
        let matches = v["matches"].as_array().expect("matches array");
        for m in matches {
            assert!(
                m["file_path"].as_str().unwrap_or("").contains("src/api.rs"),
                "all matches must be in src/api.rs"
            );
        }
    }

    #[test]
    fn resolve_symbol_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "name": "compute" });
        let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
            .expect("resolve_symbol call");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    #[test]
    fn resolve_symbol_includes_suggestions() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "name": "compute", "output_format": "json" });
        let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
            .expect("resolve_symbol call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        let suggestions = v["suggestions"].as_array().expect("suggestions array");
        assert!(!suggestions.is_empty(), "must include suggestions");
        let next_tools = suggestions[0]["next_tools"].as_array().expect("next_tools");
        assert!(
            next_tools
                .iter()
                .any(|t| t.as_str() == Some("symbol_neighbors")),
            "suggestions must recommend symbol_neighbors"
        );
    }

    #[test]
    fn resolve_symbol_truncation_metadata_present() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "name": "compute", "output_format": "json" });
        let resp = call("resolve_symbol", Some(&args), "/repo", &fixture.db_path)
            .expect("resolve_symbol call");
        // atlas_truncated is a top-level field (not inside content text).
        let text = unwrap_tool_text(resp.clone());
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
        // atlas_truncated is embedded in the JSON payload for resolve_symbol.
        assert!(
            v.get("atlas_truncated").is_some(),
            "resolve_symbol payload must include atlas_truncated"
        );
    }

    #[test]
    fn query_graph_truncation_metadata_present() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "text": "compute" });
        let resp =
            call("query_graph", Some(&args), "/repo", &fixture.db_path).expect("query_graph call");
        // atlas_truncated and atlas_result_count are top-level fields on the response envelope.
        assert!(
            resp.get("atlas_truncated").is_some(),
            "query_graph must include atlas_truncated"
        );
        assert!(
            resp.get("atlas_result_count").is_some(),
            "query_graph must include atlas_result_count"
        );
    }

    // MCP10: subpath filtering
    #[test]
    fn query_graph_subpath_filters_results() {
        let fixture = setup_mcp_fixture();
        // compute lives in src/service.rs; subpath "tests" should exclude it.
        let args = serde_json::json!({
            "text": "compute",
            "subpath": "tests",
            "output_format": "json"
        });
        let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path)
            .expect("query_graph subpath call");
        let text = unwrap_tool_text(resp.clone());
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
        // All returned nodes must have file_path starting with "tests".
        if let Some(arr) = v.as_array() {
            for item in arr {
                let fp = item["file"].as_str().unwrap_or("");
                assert!(
                    fp.starts_with("tests"),
                    "subpath='tests' must restrict results to tests/, got file={fp}"
                );
            }
        }
    }

    // MCP10: fuzzy matching
    #[test]
    fn query_graph_fuzzy_returns_near_miss() {
        let fixture = setup_mcp_fixture();
        // "comput" is a prefix of "compute"; with fuzzy=true the full search path
        // is used (atlas_search::search) which applies ranking boosts.
        // The FTS5 index should return "compute" for the "comput" token because
        // FTS5 matches on token boundaries and "compute" contains "comput" as prefix.
        // If FTS misses it, the relaxed path picks it up via "com*" prefix wildcard.
        let args = serde_json::json!({
            "text": "comput",
            "fuzzy": true,
            "output_format": "json"
        });
        let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path)
            .expect("query_graph fuzzy call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
        let arr = v.as_array().expect("expected array result");
        assert!(
            arr.iter().any(|item| {
                let qn = item["qn"].as_str().unwrap_or("");
                qn.contains("compute")
            }),
            "fuzzy=true must surface symbols matching 'comput' prefix"
        );
    }

    // MCP10: hybrid mode falls back to FTS when no embed backend configured
    #[test]
    fn query_graph_hybrid_falls_back_to_fts() {
        let fixture = setup_mcp_fixture();
        // No ATLAS_EMBED_URL set in test env; hybrid must fall back gracefully.
        let args = serde_json::json!({
            "text": "compute",
            "hybrid": true,
            "output_format": "json"
        });
        let resp = call("query_graph", Some(&args), "/repo", &fixture.db_path)
            .expect("query_graph hybrid call");
        let text = unwrap_tool_text(resp.clone());
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
        // Should still return results via FTS fallback.
        let arr = v.as_array().expect("expected array result");
        assert!(
            arr.iter().any(|item| {
                let qn = item["qn"].as_str().unwrap_or("");
                qn.contains("compute")
            }),
            "hybrid fallback to FTS must still return compute"
        );
        assert_eq!(
            resp["atlas_query_mode"].as_str(),
            Some("fts5_vector_hybrid"),
            "response must report hybrid query mode"
        );
    }

    // MCP10: active_query_mode in response
    #[test]
    fn query_graph_response_includes_query_mode() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "text": "compute", "output_format": "json" });
        let resp =
            call("query_graph", Some(&args), "/repo", &fixture.db_path).expect("query_graph call");
        assert!(
            resp.get("atlas_query_mode").is_some(),
            "query_graph must include atlas_query_mode in response metadata"
        );
        assert_eq!(resp["atlas_query_mode"].as_str(), Some("fts5"));
    }

    // MCP10: explain_query active_query_mode and ranking_factors
    #[test]
    fn explain_query_reports_active_query_mode_and_ranking_factors() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({
            "text": "compute",
            "fuzzy": true,
            "output_format": "json"
        });
        let resp = call("explain_query", Some(&args), "/repo", &fixture.db_path)
            .expect("explain_query call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
        assert_eq!(
            v["active_query_mode"].as_str(),
            Some("fts5"),
            "explain_query must report active_query_mode"
        );
        let factors = v["ranking_factors"]
            .as_array()
            .expect("ranking_factors array");
        assert!(
            factors
                .iter()
                .any(|f| f.as_str() == Some("fuzzy_edit_distance_boost")),
            "fuzzy=true must include fuzzy_edit_distance_boost in ranking_factors"
        );
    }

    // MCP10: explain_query reports subpath in filters_applied
    #[test]
    fn explain_query_reports_subpath_filter() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({
            "text": "compute",
            "subpath": "src/auth",
            "output_format": "json"
        });
        let resp = call("explain_query", Some(&args), "/repo", &fixture.db_path)
            .expect("explain_query call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
        assert_eq!(
            v["filters_applied"]["subpath"].as_bool(),
            Some(true),
            "subpath filter must be reported as active"
        );
        assert_eq!(
            v["input"]["subpath"].as_str(),
            Some("src/auth"),
            "input.subpath must be echoed back"
        );
    }

    // -----------------------------------------------------------------------
    // MCP10.1 — analyze_safety tests
    // -----------------------------------------------------------------------

    #[test]
    fn analyze_safety_returns_score_and_band() {
        let fixture = setup_mcp_fixture();
        // "compute" is in the fixture at src/service.rs::fn::compute.
        let args = serde_json::json!({
            "symbol": "src/service.rs::fn::compute",
            "output_format": "json"
        });
        let resp = call("analyze_safety", Some(&args), "/repo", &fixture.db_path)
            .expect("analyze_safety call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert!(
            v["safety_score"].as_f64().is_some(),
            "must have safety_score"
        );
        assert!(v["safety_band"].as_str().is_some(), "must have safety_band");
        assert!(v["fan_in"].as_i64().is_some(), "must have fan_in");
        assert!(v["fan_out"].as_i64().is_some(), "must have fan_out");
        assert!(
            v["linked_tests"].as_i64().is_some(),
            "must have linked_tests"
        );
        assert!(v["reasons"].as_array().is_some(), "must have reasons array");
        assert!(
            v["suggested_validations"].as_array().is_some(),
            "must have suggested_validations"
        );
        assert!(
            v["evidence"].as_array().is_some(),
            "must have evidence array"
        );
    }

    #[test]
    fn analyze_safety_missing_symbol_returns_error() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "output_format": "json" });
        let result = call("analyze_safety", Some(&args), "/repo", &fixture.db_path);
        assert!(result.is_err(), "analyze_safety without symbol must error");
    }

    #[test]
    fn analyze_safety_unknown_symbol_returns_error() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({
            "symbol": "nonexistent::fn::ghost",
            "output_format": "json"
        });
        let result = call("analyze_safety", Some(&args), "/repo", &fixture.db_path);
        assert!(
            result.is_err(),
            "analyze_safety for unknown symbol must error"
        );
    }

    #[test]
    fn analyze_safety_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "symbol": "src/service.rs::fn::compute" });
        let resp = call("analyze_safety", Some(&args), "/repo", &fixture.db_path)
            .expect("analyze_safety call");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    // -----------------------------------------------------------------------
    // MCP10.1 — analyze_remove tests
    // -----------------------------------------------------------------------

    #[test]
    fn analyze_remove_returns_impact_summary() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({
            "symbols": ["src/service.rs::fn::compute"],
            "output_format": "json"
        });
        let resp = call("analyze_remove", Some(&args), "/repo", &fixture.db_path)
            .expect("analyze_remove call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert!(v["seed_count"].as_i64().is_some(), "must have seed_count");
        assert!(
            v["impacted_symbol_count"].as_i64().is_some(),
            "must have impacted_symbol_count"
        );
        assert!(
            v["impacted_file_count"].as_i64().is_some(),
            "must have impacted_file_count"
        );
        assert!(
            v["impacted_test_count"].as_i64().is_some(),
            "must have impacted_test_count"
        );
        assert!(
            v["impacted_symbols"].as_array().is_some(),
            "must have impacted_symbols"
        );
        assert!(
            v["impacted_files"].as_array().is_some(),
            "must have impacted_files"
        );
        assert!(
            v["omitted_symbol_count"].as_i64().is_some(),
            "must have omitted_symbol_count"
        );
        assert!(
            v["warnings"].as_array().is_some(),
            "must have warnings array"
        );
        assert!(
            v["uncertainty_flags"].as_array().is_some(),
            "must have uncertainty_flags"
        );
        assert!(
            v["evidence"].as_array().is_some(),
            "must have evidence array"
        );
    }

    #[test]
    fn analyze_remove_empty_symbols_returns_error() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "symbols": [], "output_format": "json" });
        let result = call("analyze_remove", Some(&args), "/repo", &fixture.db_path);
        assert!(
            result.is_err(),
            "analyze_remove with empty symbols must error"
        );
    }

    #[test]
    fn analyze_remove_unresolved_seed_returns_warnings() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({
            "symbols": ["nonexistent::fn::ghost"],
            "output_format": "json"
        });
        let resp = call("analyze_remove", Some(&args), "/repo", &fixture.db_path)
            .expect("analyze_remove should not hard-error for unresolved seeds");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
        // Engine returns a warning + uncertainty flag when seeds don't resolve.
        let warnings = v["warnings"].as_array().expect("warnings array");
        let flags = v["uncertainty_flags"]
            .as_array()
            .expect("uncertainty_flags");
        assert!(
            !warnings.is_empty() || !flags.is_empty(),
            "unresolved seed must produce warnings or uncertainty_flags"
        );
    }

    #[test]
    fn analyze_remove_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "symbols": ["src/service.rs::fn::compute"] });
        let resp = call("analyze_remove", Some(&args), "/repo", &fixture.db_path)
            .expect("analyze_remove call");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    // -----------------------------------------------------------------------
    // MCP10.1 — analyze_dead_code tests
    // -----------------------------------------------------------------------

    #[test]
    fn analyze_dead_code_returns_candidate_list() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "output_format": "json" });
        let resp = call("analyze_dead_code", Some(&args), "/repo", &fixture.db_path)
            .expect("analyze_dead_code call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert!(
            v["candidate_count"].as_i64().is_some(),
            "must have candidate_count"
        );
        assert!(
            v["omitted_count"].as_i64().is_some(),
            "must have omitted_count"
        );
        assert!(
            v["candidates"].as_array().is_some(),
            "must have candidates array"
        );
        assert!(
            v["applied_limit"].as_i64().is_some(),
            "must have applied_limit"
        );
    }

    #[test]
    fn analyze_dead_code_subpath_is_echoed() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({
            "subpath": "src",
            "output_format": "json"
        });
        let resp = call("analyze_dead_code", Some(&args), "/repo", &fixture.db_path)
            .expect("analyze_dead_code call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");
        assert_eq!(
            v["applied_subpath"].as_str(),
            Some("src"),
            "applied_subpath must be echoed in response"
        );
    }

    #[test]
    fn analyze_dead_code_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({});
        let resp = call("analyze_dead_code", Some(&args), "/repo", &fixture.db_path)
            .expect("analyze_dead_code call");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    // -----------------------------------------------------------------------
    // MCP10.1 — analyze_dependency tests
    // -----------------------------------------------------------------------

    #[test]
    fn analyze_dependency_returns_removable_verdict() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({
            "symbol": "src/service.rs::fn::compute",
            "output_format": "json"
        });
        let resp = call("analyze_dependency", Some(&args), "/repo", &fixture.db_path)
            .expect("analyze_dependency call");
        let text = unwrap_tool_text(resp);
        let v: serde_json::Value = serde_json::from_str(&text).expect("parse json");

        assert!(
            v["removable"].as_bool().is_some(),
            "must have removable bool"
        );
        assert!(v["confidence"].as_str().is_some(), "must have confidence");
        assert!(
            v["blocking_reference_count"].as_i64().is_some(),
            "must have blocking_reference_count"
        );
        assert!(
            v["blocking_references"].as_array().is_some(),
            "must have blocking_references"
        );
        assert!(
            v["omitted_blocking_count"].as_i64().is_some(),
            "must have omitted_blocking_count"
        );
        assert!(
            v["suggested_cleanups"].as_array().is_some(),
            "must have suggested_cleanups"
        );
        assert!(
            v["uncertainty_flags"].as_array().is_some(),
            "must have uncertainty_flags"
        );
        assert!(
            v["evidence"].as_array().is_some(),
            "must have evidence array"
        );
    }

    #[test]
    fn analyze_dependency_missing_symbol_returns_error() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "output_format": "json" });
        let result = call("analyze_dependency", Some(&args), "/repo", &fixture.db_path);
        assert!(
            result.is_err(),
            "analyze_dependency without symbol must error"
        );
    }

    #[test]
    fn analyze_dependency_includes_provenance() {
        let fixture = setup_mcp_fixture();
        let args = serde_json::json!({ "symbol": "src/service.rs::fn::compute" });
        let resp = call("analyze_dependency", Some(&args), "/repo", &fixture.db_path)
            .expect("analyze_dependency call");
        assert_provenance(&resp, "/repo", &fixture.db_path);
    }

    // -----------------------------------------------------------------------
    // MCP10.1 — tool_list includes all 4 analysis tools
    // -----------------------------------------------------------------------

    #[test]
    fn tool_list_includes_analysis_tools() {
        let list = tool_list();
        let tools = list.get("tools").and_then(|t| t.as_array()).unwrap();
        for name in &[
            "analyze_safety",
            "analyze_remove",
            "analyze_dead_code",
            "analyze_dependency",
        ] {
            assert!(
                tools
                    .iter()
                    .any(|t| t.get("name") == Some(&serde_json::Value::String((*name).to_owned()))),
                "tools/list must include {name}"
            );
        }
    }
}
