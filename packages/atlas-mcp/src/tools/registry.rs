use super::shared::DEFAULT_OUTPUT_DESCRIPTION;

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
                        "files":     { "type": "array",   "items": { "type": "string" }, "description": "Repo-relative changed file paths. Alternative to base/staged/working_tree." },
                        "base":      { "type": "string",  "description": "Base git ref (e.g. 'origin/main'). Infers changed files from git diff when files not provided." },
                        "staged":    { "type": "boolean", "description": "Diff staged changes only (default false). Mutually exclusive with files/base/working_tree." },
                        "working_tree": { "type": "boolean", "description": "Diff working-tree changes only. Mutually exclusive with files/base/staged." },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit (default 5)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes to return (default 200)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "get_review_context",
                "description": "Assemble review context for the given files: changed symbols, impacted neighbors, critical edges, and risk summary. Agent-optimized output.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "files": { "type": "array", "items": { "type": "string" }, "description": "Repo-relative changed file paths. Alternative to base/staged/working_tree." },
                        "base": { "type": "string", "description": "Base git ref (e.g. 'origin/main'). Infers changed files from git diff when files not provided." },
                        "staged": { "type": "boolean", "description": "Diff staged changes only (default false). Mutually exclusive with files/base/working_tree." },
                        "working_tree": { "type": "boolean", "description": "Diff working-tree changes only. Mutually exclusive with files/base/staged." },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit (default 3)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes to consider (default 200)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
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
                        "working_tree": { "type": "boolean", "description": "Diff working-tree changes only. Mutually exclusive with base/staged." },
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
                        "source_type":  { "type": "string",  "description": "Category tag (e.g. 'review_context', 'mcp_artifact'). Default: 'mcp_artifact'." },
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
