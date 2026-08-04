//! Base `tools/list` registry entries for the `graph` tool family.
//!
//! Assembled by `super::base_tool_list_json()`; entry order is
//! irrelevant because descriptors are sorted by name afterwards.

use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::{Value, json};

/// Base registry entry JSON for the graph tools.
pub(super) fn tools() -> Vec<Value> {
    vec![
        json!({
                "name": "query_graph",
                "description": "Full-text search the code graph by symbol name, qualified name, or supported intent phrase. Supported `text` grammar: plain identifier (`compute`), exact qualified name (`src/service.rs::fn::compute`), `who calls <symbol>`, `what breaks <symbol>`, and `tests for <symbol>`. Returns stable object `structuredContent` with `query`, `matches`, `summary`, `truncated`, and `warnings`; set include_files=true when file-level hits are also useful. It does not return caller/callee usage edges. Empty `regex` is treated like omitted; truly empty `text`+`regex` requests return a self-correcting retry example instead of a bare validation failure. Natural-language-only descriptions without a concrete identifier are rejected with retry guidance. Follow up with symbol_neighbors, traverse_graph, or get_context when you need relationships.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text":     { "type": "string",  "description": "Query text. Supported grammar: plain identifier ('compute'), exact qualified name ('src/service.rs::fn::compute'), 'who calls <symbol>', 'what breaks <symbol>', or 'tests for <symbol>'. Natural-language-only descriptions without a concrete identifier are rejected." },
                        "kind":     { "type": "string",  "description": "Filter by node kind (e.g. 'function', 'struct')" },
                        "language": { "type": "string",  "description": "Filter by language (e.g. 'rust', 'python')" },
                        "limit":    { "type": "integer", "description": "Maximum results to return (default 20)" },
                        "semantic": { "type": "boolean", "description": "Graph-neighbour expansion on top of FTS: re-ranks initial FTS hits using graph edges (default false). NOT vector/embedding search — still requires FTS to find at least one initial symbol-name hit. If FTS returns nothing (e.g. text was a phrase not a symbol name), semantic expansion also returns nothing. Use regex instead for pattern-based fallback." },
                        "expand":   { "type": "boolean", "description": "Expand results through graph edges after ranking (default false). Subsumed by semantic=true; setting both is redundant." },
                        "expand_hops": { "type": "integer", "description": "Max edge hops when expand=true (default 1)" },
                        "regex":    { "type": "string",  "description": "Regex pattern matched against name and qualified_name via SQL UDF. Empty string is treated like omitted. Three modes: (1) regex-only structural scan when text is empty — filters every node in the DB; (2) text+regex: FTS5 runs first then the UDF post-filters its candidates inside SQLite; (3) invalid pattern returns an error with details. Supports regex crate alternation syntax (e.g. 'handle|HANDLE|Handle_'). Must be valid regex crate syntax." },
                        "subpath":  { "type": "string",  "description": "Restrict results to nodes whose file_path starts with this prefix (e.g. 'src/auth', 'packages/atlas-core'). Filtering happens in SQL before ranking." },
                        "fuzzy":    { "type": "boolean", "description": "Enable fuzzy (edit-distance) typo recovery for near-miss symbol names (default false). Uses relaxed candidate expansion plus stronger code-symbol ranking so close symbol typos outrank weaker docs/config matches." },
                        "hybrid":   { "type": "boolean", "description": "Enable hybrid FTS + vector retrieval with Reciprocal Rank Fusion (default false). Requires search.embedding.url in .atlas/config.toml; falls back to FTS-only when no embedding backend is configured." },
                        "include_files": { "type": "boolean", "description": "Include file nodes in the result set (default false). Leave disabled for symbol-centric search; enable when a file-level hit is useful." },
                        "repo_scope": {
                            "type": "object",
                            "description": "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Repo scope kind: current, repo_id, or all." },
                                "repo_id": { "type": "string", "description": "Required when kind='repo_id'. Must be registered and enabled." }
                            },
                            "required": ["kind"]
                        },

                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "batch_query_graph",
                "description": "Run multiple query_graph searches in a single round-trip. Canonical input is top-level `items`: an explicit array of query_graph-shaped objects. Returns stable object `structuredContent` with normalized `items`, per-item `results`, `summary`, and `warnings`. Each item uses same symbol-name FTS as query_graph — pass short exact identifiers, not natural-language phrases. File nodes remain opt-in per item via include_files=true. Max 20 items per call.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "items": {
                            "type": "array",
                            "description": "Array of query objects (1-20). Each object accepts same fields as query_graph: text, kind, language, limit, semantic, expand, expand_hops, regex, subpath, fuzzy, hybrid, include_files.",
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
                                    "fuzzy":       { "type": "boolean", "description": "Enable fuzzy typo recovery (default false)." },
                                    "hybrid":      { "type": "boolean", "description": "Enable hybrid FTS + vector retrieval (default false). Requires search.embedding.url in .atlas/config.toml." },
                                    "include_files": { "type": "boolean", "description": "Include file nodes in the result set (default false)." }
                                },
                                "required": []
                            }
                        },

                        "repo_scope": {
                            "type": "object",
                            "description": "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Repo scope kind: current, repo_id, or all." },
                                "repo_id": { "type": "string", "description": "Required when kind='repo_id'. Must be registered and enabled." }
                            },
                            "required": ["kind"]
                        },

                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
                "name": "resolve_symbol",
                "description": "Resolve a symbol name or exact qualified_name to its canonical graph symbol. Useful before relationship tools when you want one exact target from either a plain identifier (`compute`) or a fully qualified name (`src/service.rs::fn::compute`). Eliminates the manual workflow of query_graph → copy qualified_name → call symbol_neighbors. Returns the best match, an ambiguity list when multiple symbols match, and follow-up suggestions. Accepts public kind aliases (e.g. 'function'/'fn', 'struct'/'record') that are mapped to the compact tokens used in qualified names.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name":          { "type": "string",  "description": "Symbol identifier or exact qualified name to resolve (e.g. 'LoadIdentityMessages', 'compute', 'src/service.rs::fn::compute')." },
                        "kind":          { "type": "string",  "description": "Optional kind filter. Accepts public aliases: 'function'/'fn'/'func', 'method', 'class', 'struct'/'record', 'interface'/'iface', 'trait', 'enum', 'module'/'mod', 'variable'/'var', 'constant'/'const', 'test', 'import', 'package'/'pkg', 'file'." },
                        "file":          { "type": "string",  "description": "Optional file path filter. Only returns matches whose file_path contains this string (e.g. 'internal/requestctx/context.go' or 'src/')." },
                        "language":      { "type": "string",  "description": "Optional language filter (e.g. 'rust', 'go', 'typescript')." },
                        "limit":         { "type": "integer", "description": "Maximum matches to return (default 10)." },
                        "repo_scope": {
                            "type": "object",
                            "description": "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Repo scope kind: current, repo_id, or all." },
                                "repo_id": { "type": "string", "description": "Required when kind='repo_id'. Must be registered and enabled." }
                            },
                            "required": ["kind"]
                        },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["name"]
                }
        }),
    ]
}
