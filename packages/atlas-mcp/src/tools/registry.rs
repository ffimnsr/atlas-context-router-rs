use super::shared::DEFAULT_OUTPUT_DESCRIPTION;
use crate::descriptors::{
    IconDescriptor, ToolAnnotations, ToolDescriptor, ToolRegistry, descriptor_meta,
    ensure_schema_2020_12, human_title, normalized_tool_output_schema, validate_descriptor_name,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToolResultContract {
    StableObject,
    TextOnly,
    MixedNeedsRedesign,
}

impl ToolResultContract {
    fn label(self) -> &'static str {
        match self {
            Self::StableObject => "stable-object",
            Self::TextOnly => "text-only",
            Self::MixedNeedsRedesign => "mixed-needs-redesign",
        }
    }

    fn output_schema_note(self) -> &'static str {
        match self {
            Self::StableObject => "exact structuredContent schema",
            Self::TextOnly | Self::MixedNeedsRedesign => "none",
        }
    }

    fn guidance(self) -> &'static str {
        match self {
            Self::StableObject => {
                "Returns object structuredContent in JSON mode; outputSchema validates that object."
            }
            Self::TextOnly => {
                "Do not rely on structuredContent; consume text content or resource links only."
            }
            Self::MixedNeedsRedesign => {
                "Current output may vary by mode or payload shape; no outputSchema advertised yet."
            }
        }
    }
}

pub(crate) fn tool_result_contract(name: &str) -> ToolResultContract {
    match name {
        "list_graph_stats"
        | "tool_list"
        | "tool_search"
        | "tool_help"
        | "broker_status"
        | "get_context_stats"
        | "man"
        | "detect_changes"
        | "get_impact_radius"
        | "get_review_context"
        | "get_minimal_context"
        | "explain_change"
        | "traverse_graph"
        | "get_context"
        | "build_or_update_graph"
        | "postprocess_graph"
        | "status"
        | "doctor"
        | "db_check"
        | "debug_graph"
        | "explain_query"
        | "analyze_architecture"
        | "analyze_metrics"
        | "assess_risk"
        | "analyze_patterns"
        | "find_large_functions"
        | "find_complex_functions"
        | "get_session_status"
        | "compact_session"
        | "resume_session"
        | "read_saved_context"
        | "save_context_artifact"
        | "purge_saved_context"
        | "get_global_memory"
        | "symbol_neighbors"
        | "cross_file_links"
        | "concept_clusters"
        | "analyze_safety"
        | "analyze_remove"
        | "analyze_dead_code"
        | "analyze_dependency"
        | "resolve_symbol"
        | "search_files"
        | "search_content"
        | "read_file_excerpt"
        | "get_docs_section"
        | "read_file_around_match"
        | "search_templates"
        | "search_text_assets" => ToolResultContract::StableObject,
        "query_graph"
        | "batch_query_graph"
        | "search_saved_context"
        | "search_decisions"
        | "cross_session_search" => ToolResultContract::TextOnly,
        _ => ToolResultContract::MixedNeedsRedesign,
    }
}

pub fn tool_list_markdown() -> String {
    let mut markdown = String::from(
        "# MCP Tools\n\nThis file is generated from `atlas_mcp::tool_list()`. Do not edit by hand.\n\nResult contract legend:\n- `stable-object`: JSON mode returns object `structuredContent`; `outputSchema` validates that object.\n- `text-only`: consume MCP `content`; no `outputSchema` advertised.\n- `mixed-needs-redesign`: output not yet normalized to one deterministic object contract; no `outputSchema` advertised.\n\n| Tool | Result contract | Output schema | Description |\n|------|-----------------|---------------|-------------|\n",
    );

    for tool in tool_list()["tools"].as_array().expect("tools array") {
        let name = tool["name"].as_str().expect("tool name");
        let description = tool["description"].as_str().expect("tool description");
        let contract = tool_result_contract(name);
        markdown.push_str("| `");
        markdown.push_str(name);
        markdown.push_str("` | `");
        markdown.push_str(contract.label());
        markdown.push_str("` | ");
        markdown.push_str(contract.output_schema_note());
        markdown.push_str(" | ");
        markdown.push_str(&escape_markdown_table_cell(description));
        markdown.push_str(" |\n");
    }

    markdown
}

fn escape_markdown_table_cell(text: &str) -> String {
    text.replace('\n', " ").replace('|', "\\|")
}

/// Return the MCP `tools/list` response body.
fn base_tool_list_json() -> Value {
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
                "name": "tool_list",
                "description": "List visible exported MCP tools in compact runtime inventory form. Use this instead of hardcoding tool tables in agent instructions; pair with tool_search and tool_help for discovery.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "category": { "type": "string", "description": "Optional exact category filter: graph, content, analysis, health, memory, maintenance, or introspection." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "tool_search",
                "description": "Search visible exported MCP tools by name, title, or description without executing them. Ranks matches with explicit lexical score factors and typo-tolerant fuzzy name matching, and returns suggestions when no strong direct match exists.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Short tool-name fragment or capability phrase to search, such as 'query', 'review', 'context', or 'docs'. Exact/prefix/contains matches rank highest; fuzzy name matching tolerates small typos." },
                        "limit": { "type": "integer", "description": "Maximum matches to return (default 10, max 50)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "tool_help",
                "description": "Return runtime manual documentation for one visible exported MCP tool by exact name. Shorthand for man with namespace='mcp'.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string", "description": "Exact exported MCP tool name to document. Case-sensitive." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["name"]
                }
            },
            {
                "name": "man",
                "description": "Return authoritative runtime manual documentation for one visible exported MCP tool without executing that target tool. Requires namespace='mcp' and exact case-sensitive tool_name lookup from the live registry.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "namespace": { "type": "string", "description": "Manual namespace. Must be exactly 'mcp'." },
                        "tool_name": { "type": "string", "description": "Exact exported MCP tool name to document. Case-sensitive." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["namespace", "tool_name"]
                }
            },
            {
                "name": "query_graph",
                "description": "Full-text search the code graph by symbol name or identifier. Returns a compact, ranked list of matching symbols by default; set include_files=true when file-level hits are also useful. It does not return caller/callee usage edges. Empty `regex` is treated like omitted; truly empty `text`+`regex` requests return a self-correcting retry example instead of a bare validation failure. IMPORTANT: text is matched against indexed symbol names and qualified names (identifiers like 'BalancesTab', 'useFilteredBalances'), NOT against natural language — use short exact symbol names, not descriptive phrases. Follow up with symbol_neighbors, traverse_graph, or get_context when you need relationships.",
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
                        "regex":    { "type": "string",  "description": "Regex pattern matched against name and qualified_name via SQL UDF. Empty string is treated like omitted. Three modes: (1) regex-only structural scan when text is empty — filters every node in the DB; (2) text+regex: FTS5 runs first then the UDF post-filters its candidates inside SQLite; (3) invalid pattern returns an error with details. Supports regex crate alternation syntax (e.g. 'handle|HANDLE|Handle_'). Must be valid regex crate syntax." },
                        "subpath":  { "type": "string",  "description": "Restrict results to nodes whose file_path starts with this prefix (e.g. 'src/auth', 'packages/atlas-core'). Filtering happens in SQL before ranking." },
                        "fuzzy":    { "type": "boolean", "description": "Enable fuzzy (edit-distance) typo recovery for near-miss symbol names (default false). Uses relaxed candidate expansion plus stronger code-symbol ranking so close symbol typos outrank weaker docs/config matches." },
                        "hybrid":   { "type": "boolean", "description": "Enable hybrid FTS + vector retrieval with Reciprocal Rank Fusion (default false). Requires search.embedding.url in .atlas/config.toml; falls back to FTS-only when no embedding backend is configured." },
                        "include_files": { "type": "boolean", "description": "Include file nodes in the result set (default false). Leave disabled for symbol-centric search; enable when a file-level hit is useful." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "batch_query_graph",
                "description": "Run multiple query_graph searches in a single round-trip. Provide EITHER 'text' (a space- or comma-separated list of symbol names that is auto-split into one query per token, e.g. 'BalancesTab, compute, handleRequest') OR 'queries' (an explicit array of query objects). Returns an array of per-query results. Each token/query uses the same symbol-name FTS as query_graph — pass short exact identifiers, not natural-language phrases. File nodes remain opt-in per query via include_files=true. Max 20 tokens/queries per call.",
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
                                    "fuzzy":       { "type": "boolean", "description": "Enable fuzzy typo recovery (default false)." },
                                    "hybrid":      { "type": "boolean", "description": "Enable hybrid FTS + vector retrieval (default false). Requires search.embedding.url in .atlas/config.toml." },
                                    "include_files": { "type": "boolean", "description": "Include file nodes in the result set (default false)." }
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
                "description": "Compute nodes and files affected when the given files change. Returns compact, capped results. Change-source conflicts return structured retry guidance instead of a bare ambiguity error.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "mode":      { "type": "string",  "description": "Optional change-source mode: files, base, staged, or working_tree. When set, provide only that mode's required fields and omit other mode families." },
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
                "description": "Assemble review context for the given files: changed symbols, impacted neighbors, critical edges, and risk summary. Agent-optimized output. Change-source conflicts return structured retry guidance instead of a bare ambiguity error.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "mode": { "type": "string", "description": "Optional change-source mode: files, base, staged, or working_tree. When set, provide only that mode's required fields and omit other mode families." },
                        "files": { "type": "array", "items": { "type": "string" }, "description": "Repo-relative changed file paths. Alternative to base/staged/working_tree." },
                        "base": { "type": "string", "description": "Base git ref (e.g. 'origin/main'). Infers changed files from git diff when files not provided." },
                        "staged": { "type": "boolean", "description": "Diff staged changes only (default false). Mutually exclusive with files/base/working_tree." },
                        "working_tree": { "type": "boolean", "description": "Diff working-tree changes only. Mutually exclusive with files/base/staged." },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit (default 3)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes to consider (default 200)" },
                        "token_budget": { "type": "integer", "description": "Maximum tokens to include in the result. Overrides the default policy limit for this call only. Cannot exceed the policy ceiling." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "detect_changes",
                "description": "List files changed since a base git ref, with per-file node counts from the graph. Change-source conflicts return structured retry guidance instead of a bare ambiguity error.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "mode":   { "type": "string",  "description": "Optional change-source mode: base, staged, or working_tree. When set, provide only that mode's required fields and omit other mode families." },
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
                "name": "postprocess_graph",
                "description": "Run explicit derived-analytics postprocessing after build/update without reparsing source files. Supports full or changed-only mode, optional single-stage execution, and dry-run lifecycle preview.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "changed_only": { "type": "boolean", "description": "Restrict postprocess to files currently changed in the working tree when stage dependencies allow." },
                        "stage": { "type": "string", "description": "Optional stage name: flows, communities, architecture_metrics, query_hints, or large_function_summaries." },
                        "dry_run": { "type": "boolean", "description": "Compute the stage summary without recording lifecycle state." },
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
                "description": "Auto-detect changed files from git, then return a compact review bundle: changed symbols, immediate impact, risk flags. Lower token overhead than get_review_context. Change-source conflicts return structured retry guidance instead of a bare ambiguity error.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "mode":      { "type": "string",  "description": "Optional change-source mode: base, staged, or working_tree. When set, provide only that mode's required fields and omit other mode families." },
                        "base":      { "type": "string",  "description": "Base git ref (e.g. 'origin/main'). Omit to diff working tree." },
                        "staged":    { "type": "boolean", "description": "Diff staged changes only (default false)" },
                        "working_tree": { "type": "boolean", "description": "Diff working-tree changes only. Mutually exclusive with base/staged." },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit (default 2)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes (default 50)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "explain_change",
                "description": "Advanced impact analysis for a set of changed files: risk level, changed-symbol breakdown by change kind (api/signature/internal), boundary violations, test coverage gaps, and a compact summary. Deterministic, LLM-free. Change-source conflicts return structured retry guidance instead of a bare ambiguity error.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "mode":      { "type": "string",  "description": "Optional change-source mode: files, base, staged, or working_tree. When set, provide only that mode's required fields and omit other mode families." },
                        "files":     { "type": "array", "items": { "type": "string" }, "description": "Repo-relative changed file paths. Required unless using base/staged/working_tree." },
                        "base":      { "type": "string",  "description": "Base git ref (e.g. 'origin/main'). Infers changed files from git diff when files not provided." },
                        "staged":    { "type": "boolean", "description": "Diff staged changes only (default false). Used when inferring files from git." },
                        "working_tree": { "type": "boolean", "description": "Diff working-tree changes only. Mutually exclusive with files/base/staged." },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit for impact (default 5)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes (default 200)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "get_context",
                "description": "Build bounded context around a symbol, file, or change-set. Provide EITHER 'query' (a symbol name, qualified name, or structured intent phrase like 'who calls MyFunc') OR 'file' (a repo-relative path) OR 'files' (a list of changed paths). Returns ranked nodes, edges, files, and truncation/ambiguity metadata. IMPORTANT: 'query' is matched against indexed symbol names — it does NOT accept natural-language descriptions. Use short exact identifiers or intent phrases. When changed files include docs, config, templates, SQL, or prompts, pass those paths in 'files' to merge graph and content assets under one bounded selection, ranking, and truncation policy.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":     { "type": "string",  "description": "Symbol name, qualified name, or intent phrase. Examples: 'AuthService' (symbol), 'src/lib.rs::fn::foo' (qualified name), 'who calls handle_request' (usage lookup), 'what breaks MyFunc' (impact). Do NOT pass natural-language descriptions — they will not match any graph nodes. Alternative to file/files." },
                        "file":      { "type": "string",  "description": "Repo-relative file path target (file intent). Alternative to query/files." },
                        "files":     { "type": "array", "items": { "type": "string" }, "description": "Changed file paths for review/impact context. Alternative to query/file." },
                        "intent":    { "type": "string",  "description": "Override intent: symbol, file, review, impact, usage_lookup, refactor_safety, dead_code_check, rename_preview, dependency_removal. Inferred when omitted." },
                        "max_nodes": { "type": "integer", "description": "Maximum nodes to include (default 100)." },
                        "max_edges": { "type": "integer", "description": "Maximum edges to include (default 100)." },
                        "max_files": { "type": "integer", "description": "Maximum files to include in result. Omit for no cap. Reduces token use when the change-set is large." },
                        "max_depth": { "type": "integer", "description": "Traversal depth in graph hops (default 2)." },
                        "code_spans": { "type": "boolean", "description": "Include line-range spans for each selected file node (default false). Adds token cost; useful when you need precise edit coordinates." },
                        "tests":     { "type": "boolean", "description": "Include test nodes in context (default false). Enable when reviewing test coverage or debugging test failures." },
                        "imports":   { "type": "boolean", "description": "Include import edges and nodes (default true). Set false to reduce noise when only callers/callees matter." },
                        "neighbors": { "type": "boolean", "description": "Include containment-sibling nodes — functions/types in the same parent scope (default false)." },
                        "semantic":  { "type": "boolean", "description": "Run graph-aware semantic search to resolve the best-matching qualified name before building context (default false). Useful when the symbol name is ambiguous or approximate." },
                        "include_saved_context": { "type": "boolean", "description": "When true, also query the content store for saved artifacts relevant to this request and include them in the result (default false)." },
                        "session_id": { "type": "string",  "description": "Restrict saved-context retrieval to artifacts from this session and apply a same-session relevance boost." },
                        "agent_id": { "type": "string",  "description": "Restrict saved-context retrieval to one agent memory partition." },
                        "merge_agent_partitions": { "type": "boolean", "description": "Intentionally merge context across all agent partitions instead of filtering to one partition." },
                        "token_budget": { "type": "integer", "description": "Maximum tokens to include in the result. Overrides the default policy limit for this call only. Cannot exceed the policy ceiling. Use to enforce tighter context budgets from the caller side." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "analyze_architecture",
                "description": "Analyze module-level cycles, layer violations, and coupling hotspots. JSON output matches the CLI insights architecture report; default toon output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Cap returned findings after ranking." },
                        "verbose": { "type": "boolean", "description": "Return full report body in toon output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "analyze_metrics",
                "description": "Analyze graph health metrics, outliers, complexity hotspots, and coupling findings. JSON output matches the CLI insights metrics report; default toon output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Cap returned findings after ranking." },
                        "verbose": { "type": "boolean", "description": "Return full report body in toon output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "assess_risk",
                "description": "Score deterministic risk for one symbol with factor evidence and ranked findings. JSON output matches the CLI insights risk report; default toon output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Qualified name or resolvable symbol identifier." },
                        "verbose": { "type": "boolean", "description": "Return full report body in toon output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["symbol"]
                }
            },
            {
                "name": "analyze_patterns",
                "description": "Detect repeated call chains, isolated structures, hubs, bottlenecks, and deep dependency paths. JSON output matches the CLI insights patterns report; default toon output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Cap returned findings after ranking." },
                        "verbose": { "type": "boolean", "description": "Return full report body in toon output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "find_large_functions",
                "description": "Find large or complex functions repo-wide or within selected files using deterministic LOC and complexity thresholds. JSON output matches the CLI insights report; default toon output stays compact for agent review.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "files": { "type": "array", "items": { "type": "string" }, "description": "Optional repo-relative files to scope the search." },
                        "threshold": { "type": "integer", "description": "Override LOC threshold." },
                        "complexity_threshold": { "type": "integer", "description": "Override cyclomatic complexity threshold." },
                        "cognitive_threshold": { "type": "integer", "description": "Override cognitive complexity threshold." },
                        "nesting_threshold": { "type": "integer", "description": "Override max nesting depth threshold." },
                        "mode": { "type": "string", "description": "One of 'large', 'complex', or 'large-or-complex'." },
                        "limit": { "type": "integer", "description": "Cap result count after ranking." },
                        "include_tests": { "type": "boolean", "description": "Include test functions and methods." },
                        "verbose": { "type": "boolean", "description": "Return full report body in toon output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "find_complex_functions",
                "description": "Find complex functions repo-wide or within selected files using deterministic complexity thresholds. JSON output matches the CLI insights complex-functions report; default toon output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "files": { "type": "array", "items": { "type": "string" }, "description": "Optional repo-relative files to scope the search." },
                        "complexity_threshold": { "type": "integer", "description": "Override cyclomatic complexity threshold." },
                        "cognitive_threshold": { "type": "integer", "description": "Override cognitive complexity threshold." },
                        "nesting_threshold": { "type": "integer", "description": "Override max nesting depth threshold." },
                        "limit": { "type": "integer", "description": "Cap result count after ranking." },
                        "include_tests": { "type": "boolean", "description": "Include test functions and methods." },
                        "verbose": { "type": "boolean", "description": "Return full report body in toon output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "get_session_status",
                "description": "Return the status of the current MCP session: identity, event count, last compaction time, and whether a resume snapshot exists.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id":    { "type": "string",  "description": "Explicit session id. Omit to use the derived id for the current repo." },
                        "agent_id":      { "type": "string",  "description": "Restrict status to one agent memory partition." },
                        "merge_agent_partitions": { "type": "boolean", "description": "Intentionally merge status across all agent partitions." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "compact_session",
                "description": "Compact and curate the session event ledger. Removes stale low-value events, merges repeated actions, deduplicates reasoning outputs, and promotes high-value events to survive future eviction. Returns curation stats. Safe to call repeatedly.",
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
                        "agent_id":       { "type": "string",  "description": "Restrict resume output to one agent memory partition." },
                        "merge_agent_partitions": { "type": "boolean", "description": "Intentionally merge resume output across all agent partitions." },
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
                        "agent_id":     { "type": "string",  "description": "Restrict search to artifacts from one agent memory partition." },
                        "merge_agent_partitions": { "type": "boolean", "description": "Intentionally merge saved-context search across all agent partitions." },
                        "source_type":  { "type": "string",  "description": "Filter by source type (e.g. 'review_context', 'mcp_artifact')." },
                        "limit":        { "type": "integer", "description": "Maximum results to return (default 10)." },
                        "output_format":{ "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "search_decisions",
                "description": "Search persisted decision memory for prior conclusions, linked evidence, and artifact references.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":        { "type": "string",  "description": "Search query text." },
                        "session_id":   { "type": "string",  "description": "Restrict search to one session. Omit for repo-wide decision recall." },
                        "limit":        { "type": "integer", "description": "Maximum decisions to return (default 10)." },
                        "output_format":{ "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "read_saved_context",
                "description": "Retrieve the full content of a saved artifact by source_id. Supports paging via chunk_offset and max_bytes for large artifacts. Enforces session and repository scoping so cross-session reads are blocked.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "source_id":     { "type": "string",  "description": "The source_id returned by save_context_artifact or search_saved_context." },
                        "session_id":    { "type": "string",  "description": "Optional: restrict access to artifacts owned by this session. Omit to skip session scoping." },
                        "agent_id":      { "type": "string",  "description": "Optional: restrict access to artifacts owned by this agent partition." },
                        "merge_agent_partitions": { "type": "boolean", "description": "When true, allow reads across agent partitions intentionally after repo/session checks pass." },
                        "chunk_offset":  { "type": "integer", "description": "0-based chunk index to start reading from (default 0). Use next_chunk_offset from a prior truncated response for paging." },
                        "max_bytes":     { "type": "integer", "description": "Byte cap on returned content (default 65536). When content exceeds this the response sets truncated=true and includes next_chunk_offset." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["source_id"]
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
                        "agent_id":     { "type": "string",  "description": "Associate artifact with this agent memory partition." },
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
                        "agent_id":      { "type": "string",  "description": "Restrict storage statistics to one agent memory partition." },
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
                        "agent_id":      { "type": "string",  "description": "Restrict session deletion to one agent memory partition." },
                        "keep_days":     { "type": "integer", "description": "For age-based cleanup: keep sources newer than this many days (default 30)." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "cross_session_search",
                "description": "CM11: Search saved context artifacts across all sessions for this repo. Use this for cross-session recall when the relevant content may have been saved in a prior session.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":         { "type": "string",  "description": "Full-text or semantic search query." },
                        "source_type":   { "type": "string",  "description": "Optional filter: restrict to a specific source_type (e.g. 'mcp_artifact')." },
                        "agent_id":      { "type": "string",  "description": "Restrict cross-session search to one agent memory partition." },
                        "merge_agent_partitions": { "type": "boolean", "description": "Intentionally merge cross-session search across all agent partitions." },
                        "limit":         { "type": "integer", "description": "Maximum results to return (default 10)." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "get_global_memory",
                "description": "CM11: Return the cross-session global memory summary for this repo: frequently-accessed symbols and files, and recurring workflow patterns. Optionally provide focus_symbols and focus_files to also find past sessions most relevant to the current work context.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit":          { "type": "integer", "description": "Maximum entries to return per category (default 10)." },
                        "focus_symbols":  { "type": "array", "items": { "type": "string" }, "description": "Symbol qualified names from the current context used to find related past sessions." },
                        "focus_files":    { "type": "array", "items": { "type": "string" }, "description": "File paths from the current context used to find related past sessions." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
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
                "description": "Discover files by name or path glob. Use as a graph/content companion lookup for non-code assets — docs, config, SQL, Markdown, templates — after graph tools have surfaced structural context. Empty or omitted `subpath` means repo-root scope. Do not use before graph resolution for symbol questions. For symbol/relationship questions use query_graph instead.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "pattern":        { "type": "string",  "description": "Glob pattern matched against file names and repo-relative paths (e.g. '*.sql', '**/*.toml', 'config/*')." },
                        "globs":          { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters: only files whose repo-relative path matches at least one of these globs are considered." },
                        "exclude_globs":  { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters: files matching any of these globs are skipped (e.g. ['**/generated/**', '**/*.min.js'])." },
                        "subpath":        { "type": "string",  "description": "Scope the walk to a repo sub-directory (e.g. 'packages/api'). Empty or omitted value means repo root." },
                        "case_sensitive": { "type": "boolean", "description": "Match pattern case-sensitively (default false)." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["pattern"]
                }
            },
            {
                "name": "search_content",
                "description": "Search file contents by literal string or regex. Use as a graph/content companion lookup when changed symbols depend on non-code text — config keys, SQL queries, prompt content, error messages, comments. Generated and vendored files are excluded by default. Empty or omitted `subpath` means repo-root scope. Do not use before graph resolution for symbol questions; use as companion after graph tools surface relevant context. For symbol/relationship questions use query_graph instead.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":              { "type": "string",  "description": "Text to search for. Literal string by default; set is_regex=true for regex patterns. Invalid regex stays strict and returns an error; for literal metacharacters prefer is_regex=false or escape them, e.g. 'Command::Context|Context \\{' ." },
                        "globs":              { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters: only files matching at least one glob are searched." },
                        "exclude_globs":      { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters: files matching any of these globs are skipped." },
                        "exclude_generated":  { "type": "boolean", "description": "Skip generated/vendor files (node_modules, dist, *.min.js, etc.). Default true." },
                        "is_regex":           { "type": "boolean", "description": "Treat query as a regex pattern (default false). Literal queries are case-insensitive by default. Invalid regex does not fall back to literal search." },
                        "context_lines":      { "type": "integer", "description": "Lines of context to include before and after each match (default 0)." },
                        "rich_snippets":      { "type": "boolean", "description": "When true, also return grouped per-match snippets with before/match/after context lines. Default false to keep payloads compact." },
                        "snippet_context_lines": { "type": "integer", "description": "Context lines per grouped rich snippet (default: max(context_lines, 2) when rich_snippets=true)." },
                        "max_results":        { "type": "integer", "description": "Maximum match lines to return (default 50)." },
                        "subpath":            { "type": "string",  "description": "Scope the walk to a repo sub-directory (e.g. 'services/auth'). Empty or omitted value means repo root." },
                        "output_format":      { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
            },
            {
                "name": "read_file_excerpt",
                "description": "Read bounded file content from a repo-relative path using either explicit line range(s) or a single line with surrounding context. Wrapper-emitted absent-equivalent selector fields like `0`, `[]`, and `null` are ignored when exactly one selector family is materially set. Use this when you already know the file path and need precise excerpts instead of content search.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file": { "type": "string", "description": "Repo-relative file path to read." },
                        "start_line": { "type": "integer", "description": "1-based inclusive start line for single-range mode. Requires end_line." },
                        "end_line": { "type": "integer", "description": "1-based inclusive end line for single-range mode. Requires start_line." },
                        "line": { "type": "integer", "description": "1-based line number for line-with-context mode." },
                        "before": { "type": "integer", "description": "Context lines before `line` (default 0). Only valid with line." },
                        "after": { "type": "integer", "description": "Context lines after `line` (default 0). Only valid with line." },
                        "line_ranges": {
                            "type": "array",
                            "description": "Explicit list of line ranges. Mutually exclusive with start_line/end_line and line.",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "start_line": { "type": "integer", "description": "1-based inclusive start line." },
                                    "end_line": { "type": "integer", "description": "1-based inclusive end line." }
                                },
                                "required": ["start_line", "end_line"]
                            }
                        },
                        "max_lines": { "type": "integer", "description": "Maximum excerpt lines to return across all ranges (default 200, clamped by policy)." },
                        "repo_root": { "type": "string", "description": "Optional repo-root assertion. When provided, Atlas fails if it does not match current repo identity." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["file"]
                }
            },
            {
                "name": "get_docs_section",
                "description": "Resolve a Markdown section from a repo-relative documentation file using either a heading path/slug or a line number. Returns the section excerpt with heading metadata and file hash.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file": { "type": "string", "description": "Repo-relative Markdown file path to read." },
                        "heading": { "type": "string", "description": "Heading path, slug, or title to resolve. Mutually exclusive with line." },
                        "line": { "type": "integer", "description": "1-based line number to resolve to the containing Markdown section. Mutually exclusive with heading." },
                        "max_bytes": { "type": "integer", "description": "Maximum bytes of section content to emit before truncating (default 16384)." },
                        "repo_root": { "type": "string", "description": "Optional repo-root assertion. When provided, Atlas fails if it does not match current repo identity." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["file"]
                }
            },
            {
                "name": "read_file_around_match",
                "description": "Read grouped snippets around literal or regex matches inside one repo-relative file. Use this when the file path is known and you need nearby context around matched lines.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "file": { "type": "string", "description": "Repo-relative file path to search." },
                        "query": { "type": "string", "description": "Literal string or regex pattern to match within the file." },
                        "is_regex": { "type": "boolean", "description": "Treat query as regex (default false)." },
                        "case_sensitive": { "type": "boolean", "description": "When false, literal matching is case-insensitive by default; regex matching is case-sensitive by default." },
                        "before": { "type": "integer", "description": "Context lines before each match window (default 2)." },
                        "after": { "type": "integer", "description": "Context lines after each match window (default 2)." },
                        "max_matches": { "type": "integer", "description": "Maximum matched lines to consider before truncating (default 20, clamped by policy)." },
                        "max_lines": { "type": "integer", "description": "Maximum lines to emit across returned snippets (default 200, clamped by policy)." },
                        "repo_root": { "type": "string", "description": "Optional repo-root assertion. When provided, Atlas fails if it does not match current repo identity." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["file", "query"]
                }
            },
            {
                "name": "search_templates",
                "description": "Discover template files (HTML, Jinja2, Handlebars, Tera, Mako, Mustache, Twig, Liquid, ERB, HAML, Pug) by extension. Use as a graph/content companion lookup when changed files or graph evidence suggests a dependency on template behavior. Empty or omitted `subpath` means repo-root scope. Narrows by `kind` when you know the template engine. Prefer this over search_files for template-specific discovery. For symbol/relationship questions use query_graph instead.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "kind":           { "type": "string",  "description": "Template engine: html, jinja, handlebars, tera, mako, mustache, twig, liquid, erb, haml, pug. Omit to search all template types." },
                        "globs":          { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters." },
                        "exclude_globs":  { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters." },
                        "subpath":        { "type": "string",  "description": "Scope the walk to a repo sub-directory. Empty or omitted value means repo root." },
                        "case_sensitive": { "type": "boolean", "description": "Match case-sensitively (default false)." },
                        "max_results":    { "type": "integer", "description": "Maximum files to return (default 100)." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "search_text_assets",
                "description": "Discover SQL, config (TOML/YAML/INI), environment (.env), and prompt files. Use as a graph/content companion lookup when changed files include SQL, config, or prompt assets, or when graph evidence suggests a non-code dependency. Empty or omitted `subpath` means repo-root scope. Use `kind` to narrow to a specific asset type. These files are not indexed as graph symbols; use query_graph for symbol/relationship questions.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "kind":           { "type": "string",  "description": "Asset type: sql, config, env, prompt. Omit to search all text asset types." },
                        "globs":          { "type": "array", "items": { "type": "string" }, "description": "Optional include-path filters." },
                        "exclude_globs":  { "type": "array", "items": { "type": "string" }, "description": "Optional exclusion filters." },
                        "subpath":        { "type": "string",  "description": "Scope the walk to a repo sub-directory. Empty or omitted value means repo root." },
                        "case_sensitive": { "type": "boolean", "description": "Match case-sensitively (default false)." },
                        "max_results":    { "type": "integer", "description": "Maximum files to return (default 100)." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
            },
            {
                "name": "broker_status",
                "description": "Return a lightweight health/ready check for the MCP broker process itself. Reports process uptime, PID, server version, and configured worker threads. Does NOT check graph readiness — use `status` or `doctor` for graph health. Useful for liveness probes and connectivity verification independent of graph state.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
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
                        "hybrid":        { "type": "boolean", "description": "Whether hybrid FTS + vector retrieval would be used (default false). Requires search.embedding.url in .atlas/config.toml." },
                        "include_files": { "type": "boolean", "description": "Whether file nodes would be included in the result set (default false)." },
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
                        "max_files":     { "type": "integer", "description": "Maximum impacted files to include in the response (default 20). Raises omitted_file_count when truncated." },
                        "max_edges":     { "type": "integer", "description": "Maximum relevant edges to include in the response (default 50). Raises omitted_edge_count when truncated." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["symbols"]
                }
            },
            {
                "name": "analyze_dead_code",
                "description": "Detect dead-code candidates: private/unexported code symbols (functions, methods, structs/types, traits, enums, interfaces, constants, variables) with no inbound semantic edges, not in the entrypoint allowlist, and not tests. Returns candidates with certainty tiers and blocker flags. Defaults to code symbols only.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "allowlist":     { "type": "array",   "items": { "type": "string" }, "description": "Qualified names to exclude from dead-code candidates even when they have no inbound edges." },
                        "subpath":       { "type": "string",  "description": "Restrict scan to nodes whose file_path starts with this prefix (e.g. 'src/internal')." },
                        "limit":         { "type": "integer", "description": "Maximum candidates to return (default 50)." },
                        "summary":       { "type": "boolean", "description": "Return only the candidate count, not the full list. Useful for quick health checks." },
                        "exclude_kind":  { "type": "array",   "items": { "type": "string" }, "description": "Node kinds to exclude from results (e.g. ['constant', 'variable']). Accepted values: function, method, struct, enum, trait, interface, class, constant, variable." },
                        "code_only":     { "type": "boolean", "description": "Restrict to code symbols only (default true). Non-code nodes (files, packages, docs) are always excluded in the current implementation." },
                        "max_files":     { "type": "integer", "description": "Reserved for future per-candidate file-list truncation. No effect in current implementation." },
                        "max_edges":     { "type": "integer", "description": "Reserved for future per-candidate edge-list truncation. No effect in current implementation." },
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

#[derive(Deserialize)]
struct ToolDescriptorSeed {
    name: String,
    description: String,
    #[serde(rename = "inputSchema")]
    input_schema: Value,
}

pub fn tool_descriptors() -> Vec<ToolDescriptor> {
    let tools_value = base_tool_list_json()["tools"].clone();
    let seeds: Vec<ToolDescriptorSeed> =
        serde_json::from_value(tools_value).expect("base tool registry json must be valid");
    seeds.into_iter().map(build_tool_descriptor).collect()
}

pub(crate) fn tool_descriptor_by_name(name: &str) -> Option<ToolDescriptor> {
    tool_descriptors()
        .into_iter()
        .find(|tool| tool.name == name)
}

#[cfg(test)]
pub fn tool_input_schema_by_name(name: &str) -> Option<Value> {
    tool_descriptors()
        .into_iter()
        .find(|tool| tool.name == name)
        .map(|tool| tool.input_schema)
}

pub fn tool_list() -> Value {
    serde_json::to_value(ToolRegistry {
        tools: tool_descriptors(),
    })
    .expect("tool registry serialization")
}

fn build_tool_descriptor(seed: ToolDescriptorSeed) -> ToolDescriptor {
    validate_descriptor_name(&seed.name).expect("tool name must satisfy MCP guidance");
    let category = tool_category(&seed.name);
    let contract = tool_result_contract(&seed.name);
    let mut meta = descriptor_meta("tool", category);
    meta["atlas:resultContract"] = serde_json::json!(contract.label());
    meta["atlas:resultContractGuidance"] = serde_json::json!(contract.guidance());
    ToolDescriptor {
        title: human_title(&seed.name),
        output_schema: tool_output_schema_for(&seed.name),
        annotations: tool_annotations(&seed.name),
        icons: tool_icons(category),
        meta,
        name: seed.name,
        description: seed.description,
        input_schema: ensure_schema_2020_12(seed.input_schema),
    }
}

fn tool_output_schema_for(name: &str) -> Option<Value> {
    match name {
        "list_graph_stats" => Some(list_graph_stats_output_schema()),
        "tool_list" => Some(tool_list_output_schema()),
        "tool_search" => Some(tool_search_output_schema()),
        "tool_help" => Some(man_output_schema()),
        "broker_status" => Some(broker_status_output_schema()),
        "build_or_update_graph" => Some(build_or_update_graph_output_schema()),
        "postprocess_graph" => Some(postprocess_graph_output_schema()),
        "status" => Some(status_output_schema()),
        "doctor" => Some(doctor_output_schema()),
        "db_check" => Some(db_check_output_schema()),
        "debug_graph" => Some(debug_graph_output_schema()),
        "explain_query" => Some(explain_query_output_schema()),
        "analyze_architecture" => Some(insight_report_output_schema()),
        "analyze_metrics" => Some(insight_report_output_schema()),
        "assess_risk" => Some(insight_report_output_schema()),
        "analyze_patterns" => Some(insight_report_output_schema()),
        "find_large_functions" => Some(large_function_report_output_schema()),
        "find_complex_functions" => Some(large_function_report_output_schema()),
        "detect_changes" => Some(detect_changes_output_schema()),
        "get_impact_radius" => Some(get_impact_radius_output_schema()),
        "get_review_context" => Some(get_review_context_output_schema()),
        "get_minimal_context" => Some(get_minimal_context_output_schema()),
        "explain_change" => Some(explain_change_output_schema()),
        "traverse_graph" => Some(traverse_graph_output_schema()),
        "get_context" => Some(get_context_output_schema()),
        "get_session_status" => Some(get_session_status_output_schema()),
        "compact_session" => Some(compact_session_output_schema()),
        "resume_session" => Some(resume_session_output_schema()),
        "read_saved_context" => Some(read_saved_context_output_schema()),
        "save_context_artifact" => Some(save_context_artifact_output_schema()),
        "get_context_stats" => Some(get_context_stats_output_schema()),
        "purge_saved_context" => Some(purge_saved_context_output_schema()),
        "get_global_memory" => Some(get_global_memory_output_schema()),
        "symbol_neighbors" => Some(symbol_neighbors_output_schema()),
        "cross_file_links" => Some(cross_file_links_output_schema()),
        "concept_clusters" => Some(concept_clusters_output_schema()),
        "analyze_safety" => Some(analyze_safety_output_schema()),
        "analyze_remove" => Some(analyze_remove_output_schema()),
        "analyze_dead_code" => Some(analyze_dead_code_output_schema()),
        "analyze_dependency" => Some(analyze_dependency_output_schema()),
        "resolve_symbol" => Some(resolve_symbol_output_schema()),
        "search_files" => Some(search_files_output_schema()),
        "search_content" => Some(search_content_output_schema()),
        "read_file_excerpt" => Some(read_file_excerpt_output_schema()),
        "get_docs_section" => Some(get_docs_section_output_schema()),
        "read_file_around_match" => Some(read_file_around_match_output_schema()),
        "search_templates" => Some(search_templates_output_schema()),
        "search_text_assets" => Some(search_text_assets_output_schema()),
        "man" => Some(man_output_schema()),
        _ => None,
    }
}

fn list_graph_stats_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "file_count": { "type": "integer" },
            "node_count": { "type": "integer" },
            "edge_count": { "type": "integer" },
            "nodes_by_kind": {
                "type": "array",
                "items": {
                    "type": "array",
                    "prefixItems": [
                        { "type": "string" },
                        { "type": "integer" }
                    ],
                    "minItems": 2,
                    "maxItems": 2
                }
            },
            "languages": {
                "type": "array",
                "items": { "type": "string" }
            },
            "last_indexed_at": {
                "type": ["string", "null"]
            }
        }),
        &[
            "file_count",
            "node_count",
            "edge_count",
            "nodes_by_kind",
            "languages",
            "last_indexed_at",
        ],
        None,
    )
}

fn tool_list_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "total_tools": { "type": "integer" },
            "returned_tools": { "type": "integer" },
            "applied_category": { "type": ["string", "null"] },
            "tools": {
                "type": "array",
                "items": { "$ref": "#/$defs/tool_inventory_entry" }
            },
            "guidance": { "$ref": "#/$defs/tool_inventory_guidance" }
        }),
        &[
            "total_tools",
            "returned_tools",
            "applied_category",
            "tools",
            "guidance",
        ],
        Some(serde_json::json!({
            "tool_inventory_entry": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "name": { "type": "string" },
                    "title": { "type": "string" },
                    "description": { "type": "string" },
                    "category": { "type": "string" },
                    "result_contract": { "type": "string" },
                    "read_only": { "type": "boolean" },
                    "state_changing": { "type": "boolean" },
                    "destructive": { "type": "boolean" }
                },
                "required": [
                    "name",
                    "title",
                    "description",
                    "category",
                    "result_contract",
                    "read_only",
                    "state_changing",
                    "destructive"
                ]
            },
            "tool_inventory_guidance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "list": { "type": "string" },
                    "search": { "type": "string" },
                    "help": { "type": "string" }
                },
                "required": ["list", "search", "help"]
            }
        })),
    )
}

fn tool_search_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": { "type": "string" },
            "total_matches": { "type": "integer" },
            "returned_matches": { "type": "integer" },
            "matches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "name": { "type": "string" },
                        "title": { "type": "string" },
                        "description": { "type": "string" },
                        "category": { "type": "string" },
                        "result_contract": { "type": "string" },
                        "score": { "type": "integer" },
                        "match_reasons": {
                            "type": "array",
                            "items": { "type": "string" }
                        },
                        "score_factors": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "additionalProperties": false,
                                "properties": {
                                    "factor": { "type": "string" },
                                    "contribution": { "type": "integer" },
                                    "detail": { "type": ["string", "null"] }
                                },
                                "required": ["factor", "contribution", "detail"]
                            }
                        }
                    },
                    "required": [
                        "name",
                        "title",
                        "description",
                        "category",
                        "result_contract",
                        "score",
                        "match_reasons",
                        "score_factors"
                    ]
                }
            },
            "suggestions": {
                "type": "array",
                "items": { "type": "string" }
            },
            "guidance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "list": { "type": "string" },
                    "search": { "type": "string" },
                    "help": { "type": "string" }
                },
                "required": ["list", "search", "help"]
            }
        }),
        &[
            "query",
            "total_matches",
            "returned_matches",
            "matches",
            "suggestions",
            "guidance",
        ],
        None,
    )
}

fn broker_status_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "ok": { "type": "boolean" },
            "pid": { "type": "integer" },
            "version": { "type": "string" },
            "uptime_secs": { "type": "integer" },
            "worker_threads_configured": { "type": "integer" },
            "repo_root": { "type": "string" },
            "db_path": { "type": "string" }
        }),
        &[
            "ok",
            "pid",
            "version",
            "uptime_secs",
            "worker_threads_configured",
            "repo_root",
            "db_path",
        ],
        None,
    )
}

fn build_stage_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string" },
            "status": { "type": "string" },
            "item_count": { "type": "integer" },
            "details": { "type": "object" }
        },
        "required": ["name", "status", "item_count", "details"]
    })
}

fn postprocess_stage_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "stage": { "type": "string" },
            "status": { "type": "string" },
            "mode": { "type": "string" },
            "affected_file_count": { "type": "integer" },
            "item_count": { "type": "integer" },
            "elapsed_ms": { "type": "integer" },
            "error_code": { "type": ["string", "null"] },
            "message": { "type": ["string", "null"] },
            "details": { "type": "object" }
        },
        "required": [
            "stage",
            "status",
            "mode",
            "affected_file_count",
            "item_count",
            "elapsed_ms",
            "error_code",
            "message",
            "details"
        ]
    })
}

fn orphan_node_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": { "type": "string" },
            "qualified_name": { "type": "string" },
            "file_path": { "type": "string" },
            "line_start": { "type": "integer" }
        },
        "required": ["kind", "qualified_name", "file_path", "line_start"]
    })
}

fn dangling_edge_diagnostic_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": { "type": "integer" },
            "kind": { "type": "string" },
            "source_qn": { "type": "string" },
            "target_qn": { "type": "string" },
            "missing_side": { "type": "string" }
        },
        "required": ["id", "kind", "source_qn", "target_qn", "missing_side"]
    })
}

fn doctor_check_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "name": { "type": "string" },
            "status": { "type": "string" },
            "message": { "type": "string" },
            "details": { "type": "object" },
            "fix_hint": { "type": ["string", "null"] }
        },
        "required": ["name", "status", "message", "details", "fix_hint"]
    })
}

fn debug_top_file_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string" },
            "node_count": { "type": "integer" }
        },
        "required": ["path", "node_count"]
    })
}

fn explain_query_input_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "text": { "type": "string" },
            "kind": { "type": ["string", "null"] },
            "language": { "type": ["string", "null"] },
            "limit": { "type": "integer" },
            "semantic": { "type": "boolean" },
            "expand": { "type": "boolean" },
            "expand_hops": { "type": "integer" },
            "regex": { "type": ["string", "null"] },
            "subpath": { "type": ["string", "null"] },
            "fuzzy": { "type": "boolean" },
            "hybrid": { "type": "boolean" },
            "include_files": { "type": "boolean" }
        },
        "required": [
            "text",
            "kind",
            "language",
            "limit",
            "semantic",
            "expand",
            "expand_hops",
            "regex",
            "subpath",
            "fuzzy",
            "hybrid",
            "include_files"
        ]
    })
}

fn explain_query_filters_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": { "type": "boolean" },
            "language": { "type": "boolean" },
            "subpath": { "type": "boolean" },
            "fuzzy": { "type": "boolean" },
            "hybrid": { "type": "boolean" },
            "semantic": { "type": "boolean" },
            "expand": { "type": "boolean" },
            "include_files": { "type": "boolean" }
        },
        "required": ["kind", "language", "subpath", "fuzzy", "hybrid", "semantic", "expand", "include_files"]
    })
}

fn backend_capabilities_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "lexical_fts": { "type": "boolean" },
            "dense_vector": { "type": "boolean" },
            "hybrid_lexical_vector": { "type": "boolean" },
            "sparse_bm25_native": { "type": "boolean" },
            "metadata_filtering": { "type": "boolean" }
        },
        "required": ["lexical_fts", "dense_vector", "hybrid_lexical_vector", "sparse_bm25_native", "metadata_filtering"]
    })
}

fn explain_query_match_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "score": { "type": "number" },
            "kind": { "type": "string" },
            "qualified_name": { "type": "string" },
            "file_path": { "type": "string" },
            "line_start": { "type": "integer" },
            "language": { "type": "string" },
            "ranking_evidence": { "type": "object" }
        },
        "required": ["score", "kind", "qualified_name", "file_path", "line_start", "language"]
    })
}

fn compact_node_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "qn": { "type": "string" },
            "kind": { "type": "string" },
            "file": { "type": "string" },
            "line": { "type": "integer" },
            "parent": { "type": "string" },
            "sig": { "type": "string" },
            "lang": { "type": "string" }
        },
        "required": ["qn", "kind", "file", "line", "lang"]
    })
}

fn compact_edge_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "from": { "type": "string" },
            "to": { "type": "string" },
            "kind": { "type": "string" }
        },
        "required": ["from", "to", "kind"]
    })
}

fn line_range_schema() -> Value {
    serde_json::json!({
        "type": "array",
        "prefixItems": [{ "type": "integer" }, { "type": "integer" }],
        "minItems": 2,
        "maxItems": 2
    })
}

fn packaged_selected_node_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "reason": { "type": "string" },
            "distance": { "type": "integer" },
            "context_ranking_evidence": { "type": "object" },
            "qn": { "type": "string" },
            "kind": { "type": "string" },
            "file": { "type": "string" },
            "line": { "type": "integer" },
            "parent": { "type": "string" },
            "sig": { "type": "string" },
            "lang": { "type": "string" }
        },
        "required": ["reason", "distance", "qn", "kind", "file", "line", "lang"]
    })
}

fn packaged_selected_edge_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "reason": { "type": "string" },
            "context_ranking_evidence": { "type": "object" },
            "from": { "type": "string" },
            "to": { "type": "string" },
            "kind": { "type": "string" }
        },
        "required": ["reason", "from", "to", "kind"]
    })
}

fn packaged_selected_file_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string" },
            "reason": { "type": "string" },
            "line_ranges": { "type": "array", "items": { "$ref": "#/$defs/line_range" } }
        },
        "required": ["path", "reason"]
    })
}

fn change_source_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "mode": { "type": "string" },
            "resolved_files": { "type": "array", "items": { "type": "string" } },
            "deleted_files": { "type": "array", "items": { "type": "string" } },
            "base": { "type": ["string", "null"] },
            "staged": { "type": "boolean" },
            "working_tree": { "type": "boolean" }
        },
        "required": ["mode", "resolved_files", "deleted_files", "base", "staged", "working_tree"]
    })
}

fn seed_budget_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "seed_kind": { "type": "string" },
            "requested_seed_count": { "type": "integer" },
            "accepted_seed_count": { "type": "integer" },
            "omitted_seed_count": { "type": "integer" },
            "budget_hit": { "type": "boolean" },
            "partial": { "type": "boolean" },
            "safe_to_answer": { "type": "boolean" },
            "suggested_narrower_query": { "type": "string" }
        },
        "required": [
            "seed_kind",
            "requested_seed_count",
            "accepted_seed_count",
            "omitted_seed_count",
            "budget_hit",
            "partial",
            "safe_to_answer"
        ]
    })
}

fn traversal_budget_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "requested_depth": { "type": "integer" },
            "accepted_depth": { "type": "integer" },
            "requested_node_budget": { "type": "integer" },
            "accepted_node_budget": { "type": "integer" },
            "requested_edge_budget": { "type": "integer" },
            "accepted_edge_budget": { "type": "integer" },
            "emitted_node_count": { "type": "integer" },
            "emitted_edge_count": { "type": "integer" },
            "omitted_edge_count": { "type": "integer" },
            "budget_hit": { "type": "boolean" },
            "suggested_narrower_query": { "type": "string" }
        },
        "required": [
            "requested_depth",
            "accepted_depth",
            "requested_node_budget",
            "accepted_node_budget",
            "requested_edge_budget",
            "accepted_edge_budget",
            "emitted_node_count",
            "emitted_edge_count",
            "omitted_edge_count",
            "budget_hit"
        ]
    })
}

fn context_source_mix_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "source_kind": { "type": "string" },
            "items_included": { "type": "integer" },
            "items_dropped": { "type": "integer" },
            "tokens_used": { "type": "integer" }
        },
        "required": ["source_kind", "items_included", "items_dropped", "tokens_used"]
    })
}

fn payload_truncation_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "bytes_requested": { "type": "integer" },
            "bytes_emitted": { "type": "integer" },
            "tokens_estimated": { "type": "integer" },
            "token_budget_applied": { "type": "integer" },
            "omitted_node_count": { "type": "integer" },
            "omitted_file_count": { "type": "integer" },
            "omitted_source_count": { "type": "integer" },
            "omitted_byte_count": { "type": "integer" },
            "continuation_hint": { "type": "string" },
            "source_mix": { "type": "array", "items": { "$ref": "#/$defs/context_source_mix" } }
        },
        "required": [
            "bytes_requested",
            "bytes_emitted",
            "tokens_estimated",
            "omitted_node_count",
            "omitted_file_count",
            "omitted_source_count",
            "omitted_byte_count"
        ]
    })
}

fn packaged_saved_source_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "source_id": { "type": "string" },
            "label": { "type": "string" },
            "source_type": { "type": "string" },
            "session_id": { "type": "string" },
            "agent_id": { "type": "string" },
            "preview": { "type": "string" },
            "retrieval_hint": { "type": "string" },
            "relevance_score": { "type": "number" },
            "context_ranking_evidence": { "type": "object" }
        },
        "required": ["source_id", "label", "source_type", "preview", "retrieval_hint", "relevance_score"]
    })
}

fn artifact_saved_context_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "artifact_kind": { "type": "string" },
            "source_id": { "type": "string" },
            "label": { "type": "string" },
            "source_type": { "type": "string" },
            "session_id": { "type": "string" },
            "agent_id": { "type": "string" },
            "preview": { "type": "string" },
            "retrieval_hint": { "type": "string" },
            "relevance_score": { "type": "number" },
            "context_ranking_evidence": { "type": "object" }
        },
        "required": [
            "artifact_kind",
            "source_id",
            "label",
            "source_type",
            "preview",
            "retrieval_hint",
            "relevance_score"
        ]
    })
}

fn ambiguity_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "query": { "type": ["string", "null"] },
            "candidates": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["query", "candidates"]
    })
}

fn ranked_symbol_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "qn": { "type": "string" },
            "reason": { "type": "string" },
            "distance": { "type": "integer" }
        },
        "required": ["qn", "reason", "distance"]
    })
}

fn ranked_edge_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "from": { "type": "string" },
            "to": { "type": "string" },
            "kind": { "type": "string" }
        },
        "required": ["from", "to", "kind"]
    })
}

fn ranked_file_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string" },
            "reason": { "type": "string" }
        },
        "required": ["path", "reason"]
    })
}

fn explain_changed_by_kind_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "api_change": { "type": "integer" },
            "signature_change": { "type": "integer" },
            "internal_change": { "type": "integer" }
        },
        "required": ["api_change", "signature_change", "internal_change"]
    })
}

fn explain_changed_symbol_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "qn": { "type": "string" },
            "kind": { "type": "string" },
            "file": { "type": "string" },
            "line": { "type": "integer" },
            "change_kind": { "type": "string" },
            "lang": { "type": "string" },
            "sig": { "type": "string" }
        },
        "required": ["qn", "kind", "file", "line", "change_kind", "lang"]
    })
}

fn explain_boundary_violation_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "kind": { "type": "string" },
            "description": { "type": "string" },
            "nodes": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["kind", "description", "nodes"]
    })
}

fn explain_diff_counts_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "added": { "type": "integer" },
            "modified": { "type": "integer" },
            "deleted": { "type": "integer" },
            "renamed": { "type": "integer" },
            "copied": { "type": "integer" }
        },
        "required": ["added", "modified", "deleted", "renamed", "copied"]
    })
}

fn explain_diff_file_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "path": { "type": "string" },
            "change_type": { "type": "string" },
            "old_path": { "type": "string" },
            "changed_symbol_count": { "type": "integer" },
            "impacted_symbol_count": { "type": "integer" }
        },
        "required": ["path", "change_type", "changed_symbol_count", "impacted_symbol_count"]
    })
}

fn explain_diff_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "counts": { "$ref": "#/$defs/explain_diff_counts" },
            "files": { "type": "array", "items": { "$ref": "#/$defs/explain_diff_file" } }
        },
        "required": ["counts", "files"]
    })
}

fn workflow_focus_node_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "qualified_name": { "type": "string" },
            "kind": { "type": "string" },
            "file_path": { "type": "string" },
            "relevance_score": { "type": "number" },
            "selection_reason": { "type": "string" }
        },
        "required": ["qualified_name", "kind", "file_path", "relevance_score", "selection_reason"]
    })
}

fn workflow_component_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "label": { "type": "string" },
            "kind": { "type": "string" },
            "changed_node_count": { "type": "integer" },
            "impacted_node_count": { "type": "integer" },
            "file_count": { "type": "integer" },
            "summary": { "type": "string" }
        },
        "required": ["label", "kind", "changed_node_count", "impacted_node_count", "file_count", "summary"]
    })
}

fn workflow_call_chain_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "summary": { "type": "string" },
            "steps": { "type": "array", "items": { "type": "string" } },
            "edge_kinds": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["summary", "steps", "edge_kinds"]
    })
}

fn noise_reduction_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "retained_nodes": { "type": "integer" },
            "retained_edges": { "type": "integer" },
            "retained_files": { "type": "integer" },
            "dropped_nodes": { "type": "integer" },
            "dropped_edges": { "type": "integer" },
            "dropped_files": { "type": "integer" },
            "rules_applied": { "type": "array", "items": { "type": "string" } }
        },
        "required": [
            "retained_nodes",
            "retained_edges",
            "retained_files",
            "dropped_nodes",
            "dropped_edges",
            "dropped_files",
            "rules_applied"
        ]
    })
}

fn explain_test_impact_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "affected_test_count": { "type": "integer" },
            "uncovered_symbol_count": { "type": "integer" },
            "uncovered_symbols": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["affected_test_count", "uncovered_symbol_count", "uncovered_symbols"]
    })
}

fn coverage_gap_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "symbol": { "type": "string" }
        },
        "required": ["symbol"]
    })
}

fn review_risk_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "intent": { "type": "string" },
            "node_count": { "type": "integer" },
            "edge_count": { "type": "integer" },
            "file_count": { "type": "integer" },
            "truncated": { "type": "boolean" },
            "nodes_dropped": { "type": "integer" },
            "edges_dropped": { "type": "integer" },
            "files_dropped": { "type": "integer" },
            "ambiguity_present": { "type": "boolean" }
        },
        "required": [
            "intent",
            "node_count",
            "edge_count",
            "file_count",
            "truncated",
            "nodes_dropped",
            "edges_dropped",
            "files_dropped",
            "ambiguity_present"
        ]
    })
}

fn traverse_edge_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "from": { "type": "string" },
            "to": { "type": "string" },
            "kind": { "type": "string" },
            "direction": { "type": "string" }
        },
        "required": ["from", "to", "kind", "direction"]
    })
}

fn insight_severity_schema() -> Value {
    serde_json::json!({
        "type": "string",
        "enum": ["info", "low", "medium", "high"]
    })
}

fn confidence_tier_schema() -> Value {
    serde_json::json!({
        "type": "string",
        "enum": ["low", "medium", "high"]
    })
}

fn insight_line_range_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "start_line": { "type": "integer" },
            "end_line": { "type": "integer" }
        },
        "required": ["start_line", "end_line"]
    })
}

fn insight_evidence_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "file_path": { "type": "string" },
            "qualified_name": { "type": "string" },
            "node_kind": { "type": "string" },
            "edge_kind": { "type": "string" },
            "line_range": { "$ref": "#/$defs/insight_line_range" },
            "confidence_tier": { "$ref": "#/$defs/confidence_tier" }
        }
    })
}

fn insight_finding_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": { "type": "string" },
            "title": { "type": "string" },
            "severity": { "$ref": "#/$defs/insight_severity" },
            "category": { "type": "string" },
            "message": { "type": "string" },
            "evidence": { "type": "array", "items": { "$ref": "#/$defs/insight_evidence" } },
            "ranking_reason": { "type": "string" },
            "details": true,
            "score": { "type": "number" }
        },
        "required": ["id", "title", "severity", "category", "message", "ranking_reason", "score"]
    })
}

fn insight_summary_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "total_findings": { "type": "integer" },
            "highest_severity": { "$ref": "#/$defs/insight_severity" },
            "generated_at": { "type": "string" }
        },
        "required": ["total_findings", "generated_at"]
    })
}

fn graph_freshness_warning_schema() -> Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "stale": { "type": "boolean" },
            "changed_files": { "type": "array", "items": { "type": "string" } },
            "stale_result_files": { "type": "array", "items": { "type": "string" } },
            "warning": { "type": "string" },
            "suggested_recovery": { "type": "array", "items": { "type": "string" } }
        },
        "required": ["stale", "changed_files", "stale_result_files", "warning", "suggested_recovery"]
    })
}

fn insight_report_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "summary": { "$ref": "#/$defs/insight_summary" },
            "findings": { "type": "array", "items": { "$ref": "#/$defs/insight_finding" } },
            "atlas_provenance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "indexed_file_count": { "type": "integer" },
                    "last_indexed_at": { "type": ["string", "null"] }
                },
                "required": ["indexed_file_count"]
            },
            "atlas_freshness": { "$ref": "#/$defs/graph_freshness_warning" }
        }),
        &["summary", "findings", "atlas_provenance"],
        Some(serde_json::json!({
            "insight_severity": insight_severity_schema(),
            "confidence_tier": confidence_tier_schema(),
            "insight_line_range": insight_line_range_schema(),
            "insight_evidence": insight_evidence_schema(),
            "insight_finding": insight_finding_schema(),
            "insight_summary": insight_summary_schema(),
            "graph_freshness_warning": graph_freshness_warning_schema()
        })),
    )
}

fn large_function_report_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "mode": { "type": "string", "enum": ["large", "complex", "large-or-complex"] },
            "summary": { "$ref": "#/$defs/insight_summary" },
            "findings": { "type": "array", "items": { "$ref": "#/$defs/insight_finding" } },
            "atlas_provenance": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "indexed_file_count": { "type": "integer" },
                    "last_indexed_at": { "type": ["string", "null"] }
                },
                "required": ["indexed_file_count"]
            },
            "atlas_freshness": { "$ref": "#/$defs/graph_freshness_warning" }
        }),
        &["mode", "summary", "findings", "atlas_provenance"],
        Some(serde_json::json!({
            "insight_severity": insight_severity_schema(),
            "confidence_tier": confidence_tier_schema(),
            "insight_line_range": insight_line_range_schema(),
            "insight_evidence": insight_evidence_schema(),
            "insight_finding": insight_finding_schema(),
            "insight_summary": insight_summary_schema(),
            "graph_freshness_warning": graph_freshness_warning_schema()
        })),
    )
}

fn build_or_update_graph_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "mode": { "type": "string" },
            "status": { "type": "string" },
            "source": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "target_kind": { "type": "string" },
                    "base_ref": { "type": ["string", "null"] },
                    "staged": { "type": "boolean" }
                },
                "required": ["target_kind", "base_ref", "staged"]
            },
            "files_scanned": { "type": "integer" },
            "files_changed": { "type": "integer" },
            "files_parsed": { "type": "integer" },
            "files_deleted": { "type": "integer" },
            "files_renamed": { "type": "integer" },
            "files_skipped_unsupported": { "type": "integer" },
            "files_skipped_unchanged": { "type": "integer" },
            "parse_error_count": { "type": "integer" },
            "chunk_upsert_failure_count": { "type": "integer" },
            "call_target_reconcile_failure_count": { "type": "integer" },
            "nodes_written": { "type": "integer" },
            "edges_written": { "type": "integer" },
            "duration_ms": { "type": "integer" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "stages": { "type": "array", "items": { "$ref": "#/$defs/build_stage" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "budget_status": { "type": "string" },
                    "budget_hit": { "type": "boolean" },
                    "partial": { "type": "boolean" },
                    "safe_to_answer": { "type": "boolean" },
                    "budget_counters": { "type": "object" }
                },
                "required": ["budget_status", "budget_hit", "partial", "safe_to_answer", "budget_counters"]
            },
            "build_status": { "type": ["object", "null"] },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "mode",
            "status",
            "files_scanned",
            "files_changed",
            "nodes_written",
            "edges_written",
            "duration_ms",
            "stages",
            "warnings",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "build_stage": build_stage_schema(),
        })),
    )
}

fn postprocess_graph_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "mode": { "type": "string" },
            "scope": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "changed_only": { "type": "boolean" },
                    "stage_filter": { "type": ["string", "null"] },
                    "changed_file_count": { "type": "integer" },
                    "changed_files": { "type": "array", "items": { "type": "string" } }
                },
                "required": ["changed_only", "stage_filter", "changed_file_count", "changed_files"]
            },
            "dry_run": { "type": "boolean" },
            "planned_stages": { "type": "array", "items": { "$ref": "#/$defs/postprocess_stage" } },
            "executed_stages": { "type": "array", "items": { "$ref": "#/$defs/postprocess_stage" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "ok": { "type": "boolean" },
                    "noop": { "type": "boolean" },
                    "noop_reason": { "type": ["string", "null"] },
                    "error_code": { "type": "string" },
                    "error_code_docs": { "type": "string" },
                    "message": { "type": "string" },
                    "suggestions": { "type": "array", "items": { "type": "string" } },
                    "graph_built": { "type": "boolean" },
                    "state": { "type": "string" },
                    "started_at_ms": { "type": "integer" },
                    "finished_at_ms": { "type": "integer" },
                    "duration_ms": { "type": "integer" },
                    "stage_count": { "type": "integer" },
                    "supported_stage_count": { "type": "integer" }
                },
                "required": ["ok", "noop", "noop_reason", "error_code", "error_code_docs", "message", "suggestions", "graph_built", "state", "started_at_ms", "finished_at_ms", "duration_ms", "stage_count", "supported_stage_count"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "mode",
            "scope",
            "dry_run",
            "planned_stages",
            "executed_stages",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "postprocess_stage": postprocess_stage_schema(),
        })),
    )
}

fn status_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "graph_state": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "graph_built": { "type": "boolean" },
                    "build_state": { "type": ["string", "null"] },
                    "build_last_error": { "type": ["string", "null"] },
                    "build_budget_stop_reason": { "type": ["string", "null"] },
                    "stale_index": { "type": "boolean" },
                    "pending_graph_change_count": { "type": "integer" },
                    "pending_graph_changes": { "type": "array", "items": { "type": "string" } },
                    "execution_state": { "type": "string" },
                    "connection_mode": { "type": "string" },
                    "read_pool_active": { "type": "boolean" }
                },
                "required": ["graph_built", "build_state", "build_last_error", "build_budget_stop_reason", "stale_index", "pending_graph_change_count", "pending_graph_changes", "execution_state", "connection_mode", "read_pool_active"]
            },
            "db_state": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string" },
                    "exists": { "type": "boolean" },
                    "open_ok": { "type": "boolean" },
                    "open_error": { "type": ["string", "null"] },
                    "query_error": { "type": ["string", "null"] },
                    "build_status": { "type": ["object", "null"] }
                },
                "required": ["path", "exists", "open_ok", "open_error", "query_error", "build_status"]
            },
            "indexed_file_count": { "type": "integer" },
            "node_count": { "type": "integer" },
            "edge_count": { "type": "integer" },
            "last_indexed_at": { "type": ["string", "null"] },
            "failure_category": { "type": "string" },
            "ready": { "type": "boolean" },
            "safe_for_symbol_lookup": { "type": "boolean" },
            "safe_for_analysis": { "type": "boolean" },
            "retrieval_index": { "type": "object" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "message": { "type": "string" },
                    "suggestions": { "type": "array", "items": { "type": "string" } },
                    "error_code_docs": { "type": "string" }
                },
                "required": ["message", "suggestions", "error_code_docs"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "graph_state",
            "db_state",
            "indexed_file_count",
            "node_count",
            "edge_count",
            "last_indexed_at",
            "failure_category",
            "atlas_provenance",
        ],
        None,
    )
}

fn doctor_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "overall_status": { "type": "string" },
            "checks": { "type": "array", "items": { "$ref": "#/$defs/doctor_check" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "total_count": { "type": "integer" },
                    "pass_count": { "type": "integer" },
                    "fail_count": { "type": "integer" },
                    "message": { "type": "string" },
                    "suggestions": { "type": "array", "items": { "type": "string" } },
                    "error_code_docs": { "type": "string" }
                },
                "required": ["total_count", "pass_count", "fail_count", "message", "suggestions", "error_code_docs"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "overall_status",
            "checks",
            "summary",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "doctor_check": doctor_check_schema(),
        })),
    )
}

fn db_check_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "ok": { "type": "boolean" },
            "integrity": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "ok": { "type": "boolean" },
                    "issues": { "type": "array", "items": { "type": "string" } },
                    "issue_count": { "type": "integer" }
                },
                "required": ["ok", "issues", "issue_count"]
            },
            "orphan_nodes": { "type": "array", "items": { "$ref": "#/$defs/orphan_node" } },
            "dangling_edges": { "type": "array", "items": { "$ref": "#/$defs/dangling_edge_diagnostic" } },
            "noncanonical_path_rows": { "type": "array", "items": { "type": "string" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "ok": { "type": "boolean" },
                    "failure_category": { "type": "string" },
                    "message": { "type": "string" },
                    "suggestions": { "type": "array", "items": { "type": "string" } },
                    "error_code_docs": { "type": "string" },
                    "orphan_node_count": { "type": "integer" },
                    "dangling_edge_count": { "type": "integer" },
                    "noncanonical_path_row_count": { "type": "integer" }
                },
                "required": ["ok", "failure_category", "message", "suggestions", "error_code_docs", "orphan_node_count", "dangling_edge_count", "noncanonical_path_row_count"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "db_path": { "type": "string" },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "ok",
            "integrity",
            "orphan_nodes",
            "dangling_edges",
            "noncanonical_path_rows",
            "summary",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "orphan_node": orphan_node_schema(),
            "dangling_edge_diagnostic": dangling_edge_diagnostic_schema(),
        })),
    )
}

fn debug_graph_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "node_counts_by_kind": { "type": "array", "items": { "type": "array" } },
            "edge_counts_by_kind": { "type": "array", "items": { "type": "array" } },
            "top_files": { "type": "array", "items": { "$ref": "#/$defs/debug_top_file" } },
            "orphan_nodes": { "type": "array", "items": { "$ref": "#/$defs/orphan_node" } },
            "dangling_edges": { "type": "array", "items": { "$ref": "#/$defs/dangling_edge_diagnostic" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "node_count": { "type": "integer" },
                    "edge_count": { "type": "integer" },
                    "file_count": { "type": "integer" },
                    "top_file_count": { "type": "integer" },
                    "orphan_node_count": { "type": "integer" },
                    "dangling_edge_count": { "type": "integer" }
                },
                "required": ["node_count", "edge_count", "file_count", "top_file_count", "orphan_node_count", "dangling_edge_count"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "node_counts_by_kind",
            "edge_counts_by_kind",
            "top_files",
            "orphan_nodes",
            "dangling_edges",
            "summary",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "debug_top_file": debug_top_file_schema(),
            "orphan_node": orphan_node_schema(),
            "dangling_edge_diagnostic": dangling_edge_diagnostic_schema(),
        })),
    )
}

fn explain_query_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "input": { "$ref": "#/$defs/explain_query_input" },
            "normalized_query": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "active_query_mode": { "type": "string" },
                    "search_path": { "type": "string" },
                    "indexed_node_count": { "type": ["integer", "null"] },
                    "db_exists": { "type": "boolean" },
                    "ranking_factors": { "type": "array", "items": { "type": "string" } },
                    "filters_applied": { "$ref": "#/$defs/explain_query_filters" },
                    "active_capabilities": { "$ref": "#/$defs/backend_capabilities" }
                },
                "required": ["active_query_mode", "search_path", "indexed_node_count", "db_exists", "ranking_factors", "filters_applied", "active_capabilities"]
            },
            "tokenization": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "fts_tokens": { "type": "array", "items": { "type": "string" } },
                    "fts_phrase": { "type": ["string", "null"] }
                },
                "required": ["fts_tokens", "fts_phrase"]
            },
            "fts_plan": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "enabled": { "type": "boolean" },
                    "phrase": { "type": ["string", "null"] },
                    "token_count": { "type": "integer" },
                    "limit": { "type": "integer" },
                    "semantic": { "type": "boolean" },
                    "expand": { "type": "boolean" },
                    "include_files": { "type": "boolean" }
                },
                "required": ["enabled", "phrase", "token_count", "limit", "semantic", "expand", "include_files"]
            },
            "regex_plan": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "enabled": { "type": "boolean" },
                    "pattern": { "type": ["string", "null"] },
                    "valid": { "type": "boolean" },
                    "error": { "type": ["string", "null"] }
                },
                "required": ["enabled", "pattern", "valid", "error"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "latency_ms": { "type": ["integer", "null"] },
            "result_count": { "type": "integer" },
            "matches": { "type": "array", "items": { "$ref": "#/$defs/explain_query_match" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "input",
            "normalized_query",
            "tokenization",
            "fts_plan",
            "regex_plan",
            "warnings",
            "matches",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "explain_query_input": explain_query_input_schema(),
            "explain_query_filters": explain_query_filters_schema(),
            "backend_capabilities": backend_capabilities_schema(),
            "explain_query_match": explain_query_match_schema(),
        })),
    )
}

fn detect_changes_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "mode": { "type": "string" },
            "base_ref": { "type": ["string", "null"] },
            "files": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "path": { "type": "string" },
                        "change_type": { "type": "string" },
                        "old_path": { "type": ["string", "null"] },
                        "node_count": { "type": ["integer", "null"] },
                        "language": { "type": ["string", "null"] },
                        "is_added": { "type": "boolean" },
                        "is_modified": { "type": "boolean" },
                        "is_deleted": { "type": "boolean" },
                        "is_renamed": { "type": "boolean" },
                        "is_copied": { "type": "boolean" }
                    },
                    "required": [
                        "path",
                        "change_type",
                        "is_added",
                        "is_modified",
                        "is_deleted",
                        "is_renamed",
                        "is_copied"
                    ]
                }
            },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "changed_file_count": { "type": "integer" },
                    "resolved_file_count": { "type": "integer" },
                    "deleted_file_count": { "type": "integer" },
                    "added_file_count": { "type": "integer" },
                    "modified_file_count": { "type": "integer" },
                    "renamed_file_count": { "type": "integer" },
                    "copied_file_count": { "type": "integer" },
                    "files_with_graph_nodes": { "type": "integer" }
                },
                "required": [
                    "changed_file_count",
                    "resolved_file_count",
                    "deleted_file_count",
                    "added_file_count",
                    "modified_file_count",
                    "renamed_file_count",
                    "copied_file_count",
                    "files_with_graph_nodes"
                ]
            },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "mode",
            "base_ref",
            "files",
            "summary",
            "atlas_provenance",
        ],
        None,
    )
}

fn get_impact_radius_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "seed_files": { "type": "array", "items": { "type": "string" } },
            "changed_symbols": { "type": "array", "items": { "$ref": "#/$defs/compact_node" } },
            "impacted_symbols": { "type": "array", "items": { "$ref": "#/$defs/compact_node" } },
            "impacted_files": { "type": "array", "items": { "type": "string" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "changed_file_count": { "type": "integer" },
                    "changed_symbol_count": { "type": "integer" },
                    "impacted_symbol_count": { "type": "integer" },
                    "impacted_file_count": { "type": "integer" },
                    "relevant_edge_count": { "type": "integer" },
                    "seed_budget_count": { "type": "integer" },
                    "traversal_budget_applied": { "type": "boolean" }
                },
                "required": [
                    "changed_file_count",
                    "changed_symbol_count",
                    "impacted_symbol_count",
                    "impacted_file_count",
                    "relevant_edge_count",
                    "seed_budget_count",
                    "traversal_budget_applied"
                ]
            },
            "truncated": { "type": "boolean" },
            "relevant_edges": { "type": "array", "items": { "$ref": "#/$defs/compact_edge" } },
            "seed_budgets": { "type": "array", "items": { "$ref": "#/$defs/seed_budget" } },
            "traversal_budget": {
                "oneOf": [
                    { "$ref": "#/$defs/traversal_budget" },
                    { "type": "null" }
                ]
            },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "seed_files",
            "changed_symbols",
            "impacted_symbols",
            "impacted_files",
            "summary",
            "truncated",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "compact_node": compact_node_schema(),
            "compact_edge": compact_edge_schema(),
            "seed_budget": seed_budget_schema(),
            "traversal_budget": traversal_budget_schema(),
        })),
    )
}

fn get_review_context_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "changed_files": { "type": "array", "items": { "type": "string" } },
            "changed_symbols": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_node" } },
            "neighbors": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_node" } },
            "critical_edges": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_edge" } },
            "risk_summary": { "$ref": "#/$defs/review_risk_summary" },
            "artifacts": { "type": "array", "items": { "$ref": "#/$defs/artifact_saved_context" } },
            "intent": { "type": "string" },
            "node_count": { "type": "integer" },
            "nodes": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_node" } },
            "edge_count": { "type": "integer" },
            "edges": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_edge" } },
            "file_count": { "type": "integer" },
            "files": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_file" } },
            "truncated": { "type": "boolean" },
            "nodes_dropped": { "type": "integer" },
            "edges_dropped": { "type": "integer" },
            "files_dropped": { "type": "integer" },
            "ambiguity_query": { "type": ["string", "null"] },
            "ambiguity_candidates": { "type": "array", "items": { "type": "string" } },
            "seed_budgets": { "type": "array", "items": { "$ref": "#/$defs/seed_budget" } },
            "traversal_budget": {
                "oneOf": [
                    { "$ref": "#/$defs/traversal_budget" },
                    { "type": "null" }
                ]
            },
            "payload_truncation": {
                "oneOf": [
                    { "$ref": "#/$defs/payload_truncation" },
                    { "type": "null" }
                ]
            },
            "source_mix": { "type": "array", "items": { "$ref": "#/$defs/context_source_mix" } },
            "token_budget_applied": { "type": ["integer", "null"] },
            "budget_status": { "type": "string" },
            "linked_decisions": { "type": "array", "items": { "type": "object" } },
            "decision_lookup_query": { "type": "string" },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "changed_files",
            "changed_symbols",
            "neighbors",
            "critical_edges",
            "risk_summary",
            "artifacts",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "line_range": line_range_schema(),
            "packaged_selected_node": packaged_selected_node_schema(),
            "packaged_selected_edge": packaged_selected_edge_schema(),
            "packaged_selected_file": packaged_selected_file_schema(),
            "packaged_saved_source": packaged_saved_source_schema(),
            "artifact_saved_context": artifact_saved_context_schema(),
            "seed_budget": seed_budget_schema(),
            "traversal_budget": traversal_budget_schema(),
            "context_source_mix": context_source_mix_schema(),
            "payload_truncation": payload_truncation_schema(),
            "review_risk_summary": review_risk_summary_schema(),
        })),
    )
}

fn get_minimal_context_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "change_source": { "$ref": "#/$defs/change_source" },
            "changed_symbols": { "type": "array", "items": { "$ref": "#/$defs/compact_node" } },
            "immediate_impact": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "impacted_symbols": { "type": "array", "items": { "$ref": "#/$defs/compact_node" } },
                    "impacted_files": { "type": "array", "items": { "type": "string" } },
                    "relevant_edges": { "type": "array", "items": { "$ref": "#/$defs/compact_edge" } }
                },
                "required": ["impacted_symbols", "impacted_files", "relevant_edges"]
            },
            "risk_flags": { "type": "array", "items": { "type": "string" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "changed_file_count": { "type": "integer" },
                    "deleted_file_count": { "type": "integer" },
                    "changed_symbol_count": { "type": "integer" },
                    "impacted_symbol_count": { "type": "integer" },
                    "impacted_file_count": { "type": "integer" },
                    "truncated": { "type": "boolean" }
                },
                "required": [
                    "changed_file_count",
                    "deleted_file_count",
                    "changed_symbol_count",
                    "impacted_symbol_count",
                    "impacted_file_count",
                    "truncated"
                ]
            },

            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "change_source",
            "changed_symbols",
            "immediate_impact",
            "risk_flags",
            "summary",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "change_source": change_source_schema(),
            "compact_node": compact_node_schema(),
            "compact_edge": compact_edge_schema(),
            "seed_budget": seed_budget_schema(),
            "traversal_budget": traversal_budget_schema(),
        })),
    )
}

fn explain_change_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "changed_files": { "type": "array", "items": { "$ref": "#/$defs/explain_diff_file" } },
            "change_kinds": { "$ref": "#/$defs/explain_changed_by_kind" },
            "risk_level": { "type": "string" },
            "boundary_violations": { "type": "array", "items": { "$ref": "#/$defs/explain_boundary_violation" } },
            "coverage_gaps": { "type": "array", "items": { "$ref": "#/$defs/coverage_gap" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "text": { "type": "string" },
                    "changed_file_count": { "type": "integer" },
                    "changed_symbol_count": { "type": "integer" },
                    "impacted_file_count": { "type": "integer" },
                    "impacted_node_count": { "type": "integer" }
                },
                "required": [
                    "text",
                    "changed_file_count",
                    "changed_symbol_count",
                    "impacted_file_count",
                    "impacted_node_count"
                ]
            },
            "diff_summary": { "$ref": "#/$defs/explain_diff_summary" },
            "changed_symbols": { "type": "array", "items": { "$ref": "#/$defs/explain_changed_symbol" } },
            "high_impact_nodes": { "type": "array", "items": { "$ref": "#/$defs/workflow_focus_node" } },
            "impacted_components": { "type": "array", "items": { "$ref": "#/$defs/workflow_component" } },
            "call_chains": { "type": "array", "items": { "$ref": "#/$defs/workflow_call_chain" } },
            "ripple_effects": { "type": "array", "items": { "type": "string" } },
            "test_impact": { "$ref": "#/$defs/explain_test_impact" },
            "noise_reduction": { "$ref": "#/$defs/noise_reduction_summary" },
            "budget_status": { "type": "string" },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "changed_files",
            "change_kinds",
            "risk_level",
            "boundary_violations",
            "coverage_gaps",
            "summary",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "explain_changed_by_kind": explain_changed_by_kind_schema(),
            "explain_changed_symbol": explain_changed_symbol_schema(),
            "explain_boundary_violation": explain_boundary_violation_schema(),
            "explain_diff_counts": explain_diff_counts_schema(),
            "explain_diff_file": explain_diff_file_schema(),
            "explain_diff_summary": explain_diff_summary_schema(),
            "workflow_focus_node": workflow_focus_node_schema(),
            "workflow_component": workflow_component_schema(),
            "workflow_call_chain": workflow_call_chain_schema(),
            "noise_reduction_summary": noise_reduction_summary_schema(),
            "explain_test_impact": explain_test_impact_schema(),
            "coverage_gap": coverage_gap_schema(),
        })),
    )
}

fn traverse_graph_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "root_symbol": { "type": "string" },
            "direction": { "type": "string" },
            "depth": { "type": "integer" },
            "nodes": { "type": "array", "items": { "$ref": "#/$defs/compact_node" } },
            "edges": { "type": "array", "items": { "$ref": "#/$defs/traverse_edge" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "changed_symbol_count": { "type": "integer" },
                    "impacted_symbol_count": { "type": "integer" },
                    "impacted_file_count": { "type": "integer" },
                    "relevant_edge_count": { "type": "integer" }
                },
                "required": [
                    "changed_symbol_count",
                    "impacted_symbol_count",
                    "impacted_file_count",
                    "relevant_edge_count"
                ]
            },
            "truncated": { "type": "boolean" },
            "impacted_files": { "type": "array", "items": { "type": "string" } },
            "seed_budgets": { "type": "array", "items": { "$ref": "#/$defs/seed_budget" } },
            "traversal_budget": {
                "oneOf": [
                    { "$ref": "#/$defs/traversal_budget" },
                    { "type": "null" }
                ]
            },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "root_symbol",
            "direction",
            "depth",
            "nodes",
            "edges",
            "summary",
            "truncated",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "compact_node": compact_node_schema(),
            "compact_edge": compact_edge_schema(),
            "seed_budget": seed_budget_schema(),
            "traversal_budget": traversal_budget_schema(),
            "traverse_edge": traverse_edge_schema(),
        })),
    )
}

fn get_context_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "mode": { "type": "string" },
            "query": { "type": ["string", "null"] },
            "file": { "type": ["string", "null"] },
            "files": { "type": "array", "items": { "type": "string" } },
            "ranked_symbols": { "type": "array", "items": { "$ref": "#/$defs/ranked_symbol_summary" } },
            "ranked_edges": { "type": "array", "items": { "$ref": "#/$defs/ranked_edge_summary" } },
            "ranked_files": { "type": "array", "items": { "$ref": "#/$defs/ranked_file_summary" } },
            "assets": { "type": "array", "items": { "$ref": "#/$defs/artifact_saved_context" } },
            "ambiguity": { "$ref": "#/$defs/ambiguity" },
            "intent": { "type": "string" },
            "node_count": { "type": "integer" },
            "nodes": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_node" } },
            "edge_count": { "type": "integer" },
            "edges": { "type": "array", "items": { "$ref": "#/$defs/packaged_selected_edge" } },
            "file_count": { "type": "integer" },
            "files_dropped": { "type": "integer" },
            "truncated": { "type": "boolean" },
            "nodes_dropped": { "type": "integer" },
            "edges_dropped": { "type": "integer" },
            "seed_budgets": { "type": "array", "items": { "$ref": "#/$defs/seed_budget" } },
            "traversal_budget": {
                "oneOf": [
                    { "$ref": "#/$defs/traversal_budget" },
                    { "type": "null" }
                ]
            },
            "payload_truncation": {
                "oneOf": [
                    { "$ref": "#/$defs/payload_truncation" },
                    { "type": "null" }
                ]
            },
            "source_mix": { "type": "array", "items": { "$ref": "#/$defs/context_source_mix" } },
            "token_budget_applied": { "type": ["integer", "null"] },
            "budget_status": { "type": "string" },
            "linked_decisions": { "type": "array", "items": { "type": "object" } },
            "decision_lookup_query": { "type": "string" },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "mode",
            "query",
            "file",
            "files",
            "ranked_symbols",
            "ranked_edges",
            "ranked_files",
            "assets",
            "ambiguity",
            "truncated",
            "atlas_provenance",
        ],
        Some(serde_json::json!({
            "line_range": line_range_schema(),
            "ranked_symbol_summary": ranked_symbol_summary_schema(),
            "ranked_edge_summary": ranked_edge_summary_schema(),
            "ranked_file_summary": ranked_file_summary_schema(),
            "artifact_saved_context": artifact_saved_context_schema(),
            "ambiguity": ambiguity_schema(),
            "packaged_selected_node": packaged_selected_node_schema(),
            "packaged_selected_edge": packaged_selected_edge_schema(),
            "packaged_saved_source": packaged_saved_source_schema(),
            "seed_budget": seed_budget_schema(),
            "traversal_budget": traversal_budget_schema(),
            "context_source_mix": context_source_mix_schema(),
            "payload_truncation": payload_truncation_schema(),
        })),
    )
}

fn man_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "requested_namespace": { "type": "string" },
            "requested_tool_name": { "type": "string" },
            "resolved_tool_name": { "type": "string" },
            "description": { "type": "string" },
            "tool_structure": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "purpose": { "type": "string" },
                    "operation_name": { "type": "string" },
                    "request_shape": { "type": "string" },
                    "response_shape": { "type": "string" },
                    "result_contract": { "type": "string" },
                    "annotations": {
                        "type": "object",
                        "additionalProperties": false,
                        "properties": {
                            "read_only": { "type": "boolean" },
                            "state_changing": { "type": "boolean" },
                            "destructive": { "type": "boolean" }
                        },
                        "required": ["read_only", "state_changing", "destructive"]
                    }
                },
                "required": [
                    "purpose",
                    "operation_name",
                    "request_shape",
                    "response_shape",
                    "result_contract",
                    "annotations"
                ]
            },
            "input_args": {
                "type": "array",
                "items": { "$ref": "#/$defs/manual_field" }
            },
            "output_response": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "response_shape": { "type": "string" },
                    "structured_content_available": { "type": "boolean" },
                    "response_fields": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/manual_field" }
                    },
                    "metadata_fields": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/manual_field" }
                    },
                    "error_payload_fields": {
                        "type": "array",
                        "items": { "$ref": "#/$defs/manual_field" }
                    }
                },
                "required": [
                    "response_shape",
                    "structured_content_available",
                    "response_fields",
                    "metadata_fields",
                    "error_payload_fields"
                ]
            },
            "usage": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "cli": { "type": "string" },
                    "mcp_manual_tool_call": { "type": "string" },
                    "target_tool_call_examples": {
                        "type": "array",
                        "items": { "type": "string" }
                    }
                },
                "required": ["cli", "mcp_manual_tool_call", "target_tool_call_examples"]
            },
            "error_cases": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "code": { "type": "string" },
                        "when": { "type": "string" },
                        "behavior": { "type": "string" }
                    },
                    "required": ["code", "when", "behavior"]
                }
            },
            "truncation": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "description_truncated": { "type": "boolean" },
                    "usage_examples_truncated": { "type": "boolean" }
                },
                "required": ["description_truncated", "usage_examples_truncated"]
            }
        }),
        &[
            "requested_namespace",
            "requested_tool_name",
            "resolved_tool_name",
            "description",
            "tool_structure",
            "input_args",
            "output_response",
            "usage",
            "error_cases",
            "truncation",
        ],
        Some(serde_json::json!({
            "manual_field": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "name": { "type": "string" },
                    "field_type": { "type": "string" },
                    "required": { "type": "boolean" },
                    "default_value": { "type": ["string", "null"] },
                    "enum_values": {
                        "type": "array",
                        "items": { "type": "string" }
                    },
                    "description": { "type": "string" }
                },
                "required": [
                    "name",
                    "field_type",
                    "required",
                    "default_value",
                    "enum_values",
                    "description"
                ]
            }
        })),
    )
}

fn get_session_status_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "session_id": { "type": "string" },
            "agent_id": { "type": ["string", "null"] },
            "merged_agent_view": { "type": "boolean" },
            "status": { "type": "string" },
            "repo_root": { "type": ["string", "null"] },
            "frontend": { "type": ["string", "null"] },
            "worktree_id": { "type": ["string", "null"] },
            "created_at": { "type": ["string", "null"] },
            "updated_at": { "type": ["string", "null"] },
            "last_resume_at": { "type": ["string", "null"] },
            "last_compaction_at": { "type": ["string", "null"] },
            "event_count": { "type": "integer" },
            "resume_snapshot_exists": { "type": "boolean" },
            "snapshot_consumed": { "type": ["boolean", "null"] },
            "agent_partitions": { "type": "array", "items": { "type": "object" } },
            "delegated_tasks": { "type": "array", "items": { "type": "object" } },
            "agent_responsibilities": { "type": "array", "items": { "type": "object" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "status": { "type": "string" },
                    "has_session": { "type": "boolean" },
                    "event_count": { "type": "integer" },
                    "partition_count": { "type": "integer" },
                    "delegated_task_count": { "type": "integer" },
                    "responsibility_count": { "type": "integer" },
                    "resume_snapshot_exists": { "type": "boolean" }
                },
                "required": ["status", "has_session", "event_count", "partition_count", "delegated_task_count", "responsibility_count", "resume_snapshot_exists"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "session_id",
            "event_count",
            "resume_snapshot_exists",
            "last_compaction_at",
            "repo_root",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn compact_session_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "session_id": { "type": "string" },
            "before_counts": { "type": "object", "additionalProperties": false, "properties": { "events": { "type": "integer" } }, "required": ["events"] },
            "after_counts": { "type": "object", "additionalProperties": false, "properties": { "events": { "type": "integer" } }, "required": ["events"] },
            "promoted_events": { "type": "integer" },
            "removed_events": { "type": "integer" },
            "merged_groups": { "type": "integer" },
            "decayed_events": { "type": "integer" },
            "deduplicated_events": { "type": "integer" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "status": { "type": "string" },
                    "no_op": { "type": "boolean" },
                    "events_before": { "type": "integer" },
                    "events_after": { "type": "integer" },
                    "events_removed": { "type": "integer" }
                },
                "required": ["status", "no_op", "events_before", "events_after", "events_removed"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "session_id",
            "before_counts",
            "after_counts",
            "promoted_events",
            "removed_events",
            "merged_groups",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn resume_session_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "session_id": { "type": "string" },
            "agent_id": { "type": ["string", "null"] },
            "merged_agent_view": { "type": "boolean" },
            "snapshot_status": { "type": "string" },
            "snapshot": { "type": "object" },
            "event_count": { "type": "integer" },
            "consumed": { "type": "boolean" },
            "created_at": { "type": ["string", "null"] },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "event_count": { "type": "integer" },
                    "merged_agent_view": { "type": "boolean" },
                    "snapshot_consumed": { "type": "boolean" }
                },
                "required": ["event_count", "merged_agent_view", "snapshot_consumed"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "session_id",
            "snapshot_status",
            "snapshot",
            "consumed",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn read_saved_context_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "found": { "type": "boolean" },
            "access_status": { "type": "string" },
            "source_id": { "type": "string" },
            "content": { "type": ["string", "null"] },
            "content_format": { "type": ["string", "null"] },
            "chunk_offset": { "type": "integer" },
            "next_chunk_offset": { "type": ["integer", "null"] },
            "truncated": { "type": "boolean" },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "status": { "type": "string" },
                    "byte_count": { "type": "integer" },
                    "chunk_count": { "type": "integer" },
                    "returned_chunk_count": { "type": "integer" }
                },
                "required": ["status", "byte_count", "chunk_count", "returned_chunk_count"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "artifact_kind": { "type": "string" },
            "identity_kind": { "type": "string" },
            "identity_value": { "type": "string" },
            "created_at": { "type": "string" },
            "session_id": { "type": ["string", "null"] },
            "agent_id": { "type": ["string", "null"] },
            "merged_agent_view": { "type": "boolean" },
            "label": { "type": "string" },
            "byte_count": { "type": "integer" },
            "chunk_count": { "type": "integer" },
            "last_included_chunk": { "type": ["integer", "null"] },
            "last_included_chunk_id": { "type": ["string", "null"] },
            "returned_chunk_ids": { "type": "array", "items": { "type": "string" } },
            "next_chunk_id": { "type": ["string", "null"] },
            "continuation_hint": { "type": ["string", "null"] },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "source_id",
            "content",
            "content_format",
            "chunk_offset",
            "next_chunk_offset",
            "truncated",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn save_context_artifact_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "storage_mode": { "type": "string" },
            "source_id": { "type": ["string", "null"] },
            "label": { "type": "string" },
            "source_type": { "type": "string" },
            "agent_id": { "type": ["string", "null"] },
            "preview": { "type": ["string", "null"] },
            "inline_content": { "type": ["string", "null"] },
            "content_size_bytes": { "type": "integer" },
            "chunk_count": { "type": "integer" },
            "resource_link": { "type": ["object", "null"] },
            "retrieval_hint": { "type": ["string", "null"] },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "session_id": { "type": "string" },
                    "stored": { "type": "boolean" },
                    "inline": { "type": "boolean" },
                    "content_type": { "type": "string" }
                },
                "required": ["session_id", "stored", "inline", "content_type"]
            },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "storage_mode",
            "source_id",
            "preview",
            "content_size_bytes",
            "chunk_count",
            "resource_link",
            "summary",
            "atlas_provenance",
        ],
        None,
    )
}

fn purge_saved_context_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "mode": { "type": "string" },
            "session_id": { "type": ["string", "null"] },
            "agent_id": { "type": ["string", "null"] },
            "cutoff_days": { "type": "integer" },
            "deleted_sources": { "type": "integer" },
            "deleted_chunks": { "type": "integer" },
            "deleted_bridge_files": { "type": "integer" },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "mode",
            "session_id",
            "cutoff_days",
            "deleted_sources",
            "deleted_chunks",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn get_global_memory_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "repo_root": { "type": "string" },
            "focus": { "type": ["object", "null"] },
            "frequent_symbols": { "type": "array", "items": { "type": "object" } },
            "frequent_files": { "type": "array", "items": { "type": "object" } },
            "workflow_patterns": { "type": "array", "items": { "type": "object" } },
            "relevant_sessions": { "type": "array", "items": { "type": "object" } },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "frequent_symbol_count": { "type": "integer" },
                    "frequent_file_count": { "type": "integer" },
                    "workflow_pattern_count": { "type": "integer" },
                    "relevant_session_count": { "type": "integer" }
                },
                "required": ["frequent_symbol_count", "frequent_file_count", "workflow_pattern_count", "relevant_session_count"]
            },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" }
        }),
        &[
            "tool",
            "repo_root",
            "frequent_symbols",
            "frequent_files",
            "workflow_patterns",
            "relevant_sessions",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn symbol_neighbors_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "symbol": { "type": "object" },
            "callers": { "type": "array", "items": { "type": "object" } },
            "callees": { "type": "array", "items": { "type": "object" } },
            "call_sites": { "type": "array", "items": { "type": "object" } },
            "tests": { "type": "array", "items": { "type": "object" } },
            "siblings": { "type": "array", "items": { "type": "object" } },
            "imports": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "symbol",
            "callers",
            "callees",
            "call_sites",
            "tests",
            "siblings",
            "imports",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn cross_file_links_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "source_file": { "type": "string" },
            "linked_files": { "type": "array", "items": { "type": "object" } },
            "coupling_metric": { "type": "object" },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "source_file",
            "linked_files",
            "coupling_metric",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn concept_clusters_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "seed_files": { "type": "array", "items": { "type": "string" } },
            "clusters": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "seed_files",
            "clusters",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn analyze_safety_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "symbol": { "type": "object" },
            "fan_in": { "type": "integer" },
            "fan_out": { "type": "integer" },
            "test_adjacency": { "type": "object" },
            "cross_module_callers": { "type": "integer" },
            "safety_score": { "type": "number" },
            "safety_band": { "type": "string" },
            "suggested_validations": { "type": "array", "items": { "type": "string" } },
            "factor_evidence": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "symbol",
            "fan_in",
            "fan_out",
            "test_adjacency",
            "cross_module_callers",
            "safety_score",
            "safety_band",
            "suggested_validations",
            "factor_evidence",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn analyze_remove_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "symbols": { "type": "array", "items": { "type": "object" } },
            "definite_impacts": { "type": "array", "items": { "type": "object" } },
            "probable_impacts": { "type": "array", "items": { "type": "object" } },
            "weak_impacts": { "type": "array", "items": { "type": "object" } },
            "tests": { "type": "array", "items": { "type": "object" } },
            "uncertainty_flags": { "type": "array", "items": { "type": "string" } },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "object" } },
            "evidence": { "type": "array", "items": { "type": "object" } },
            "impacted_files": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "symbols",
            "definite_impacts",
            "probable_impacts",
            "weak_impacts",
            "tests",
            "uncertainty_flags",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn analyze_dead_code_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "scope": { "type": "object" },
            "candidates": { "type": "array", "items": { "type": "object" } },
            "blockers": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "scope",
            "candidates",
            "blockers",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn analyze_dependency_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "symbol": { "type": "string" },
            "removable": { "type": "boolean" },
            "blocking_references": { "type": "array", "items": { "type": "object" } },
            "confidence_tier": { "type": "string" },
            "suggested_cleanups": { "type": "array", "items": { "type": "string" } },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "evidence": { "type": "array", "items": { "type": "object" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "symbol",
            "removable",
            "blocking_references",
            "confidence_tier",
            "suggested_cleanups",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn resolve_symbol_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": { "type": "object" },
            "best_match": { "type": ["object", "null"] },
            "ambiguity": { "type": "object" },
            "suggestions": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "query",
            "best_match",
            "ambiguity",
            "suggestions",
            "summary",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn search_files_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "pattern": { "type": "string" },
                    "globs": { "type": "array", "items": { "type": "string" } },
                    "exclude_globs": { "type": "array", "items": { "type": "string" } },
                    "case_sensitive": { "type": "boolean" }
                },
                "required": ["pattern", "globs", "exclude_globs", "case_sensitive"]
            },
            "subpath": { "type": ["string", "null"] },
            "matches": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "path": { "type": "string" },
                        "file_name": { "type": "string" },
                        "extension": { "type": ["string", "null"] }
                    },
                    "required": ["path", "file_name", "extension"]
                }
            },
            "summary": {
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "returned_count": { "type": "integer" },
                    "result_limit": { "type": "integer" },
                    "scope": { "type": "string", "enum": ["repo_root", "subpath"] },
                    "has_matches": { "type": "boolean" }
                },
                "required": ["returned_count", "result_limit", "scope", "has_matches"]
            },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "query",
            "subpath",
            "matches",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn search_content_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "query": { "type": "object" },
            "mode": { "type": "string", "enum": ["literal", "regex"] },
            "subpath": { "type": ["string", "null"] },
            "matches": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "query",
            "mode",
            "subpath",
            "matches",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn read_file_excerpt_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "file": { "type": "string" },
            "selection_mode": { "type": "string" },
            "ranges": { "type": "array", "items": { "type": "object" } },
            "snippets": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "mode": { "type": "string" },
            "total_lines": { "type": "integer" },
            "excerpts": { "type": "array", "items": { "type": "object" } },
            "excerpt_count": { "type": "integer" },
            "atlas_result_kind": { "type": "string" },
            "atlas_hint": { "type": ["string", "null"] },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "file",
            "selection_mode",
            "ranges",
            "snippets",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn get_docs_section_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "file": { "type": "string" },
            "selector_mode": { "type": "string" },
            "heading": { "type": ["object", "null"] },
            "slug": { "type": ["string", "null"] },
            "line_start": { "type": ["integer", "null"] },
            "line_end": { "type": ["integer", "null"] },
            "content": { "type": ["string", "null"] },
            "file_hash": { "type": ["string", "null"] },
            "resolved": { "type": "boolean" },
            "query": { "type": ["string", "null"] },
            "candidates": { "type": "array", "items": { "type": "object" } },
            "lines": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "heading_path": { "type": ["string", "null"] },
            "heading_level": { "type": ["integer", "null"] },
            "start_line": { "type": ["integer", "null"] },
            "end_line": { "type": ["integer", "null"] },
            "atlas_result_kind": { "type": "string" },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "file",
            "selector_mode",
            "heading",
            "slug",
            "line_start",
            "line_end",
            "content",
            "file_hash",
            "resolved",
            "candidates",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn read_file_around_match_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "file": { "type": "string" },
            "match_mode": { "type": "string", "enum": ["literal", "regex"] },
            "query": { "type": "string" },
            "before": { "type": "integer" },
            "after": { "type": "integer" },
            "matches": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "is_regex": { "type": "boolean" },
            "case_sensitive": { "type": "boolean" },
            "total_matches": { "type": "integer" },
            "returned_matches": { "type": "integer" },
            "snippet_count": { "type": "integer" },
            "snippets": { "type": "array", "items": { "type": "object" } },
            "atlas_result_kind": { "type": "string" },
            "atlas_hint": { "type": ["string", "null"] },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "file",
            "match_mode",
            "query",
            "before",
            "after",
            "matches",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn search_templates_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "kind": { "type": ["string", "null"] },
            "subpath": { "type": ["string", "null"] },
            "matches": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "kind",
            "subpath",
            "matches",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn search_text_assets_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "kind": { "type": ["string", "null"] },
            "subpath": { "type": ["string", "null"] },
            "matches": { "type": "array", "items": { "type": "object" } },
            "summary": { "type": "object" },
            "truncated": { "type": "boolean" },
            "warnings": { "type": "array", "items": { "type": "string" } },
            "atlas_provenance": { "type": "object" },
            "atlas_freshness": { "type": "object" }
        }),
        &[
            "tool",
            "kind",
            "subpath",
            "matches",
            "summary",
            "truncated",
            "warnings",
            "atlas_provenance",
        ],
        None,
    )
}

fn get_context_stats_output_schema() -> Value {
    normalized_tool_output_schema(
        serde_json::json!({
            "session_id": { "type": "string" },
            "agent_id": { "type": ["string", "null"] },
            "event_count": { "type": "integer" },
            "source_count": { "type": "integer" },
            "chunk_count": { "type": "integer" },
            "bridge_file_count": { "type": "integer" },
            "content_db_path": { "type": "string" },
            "session_db_path": { "type": "string" },
            "bridge_dir_path": { "type": "string" },
            "retrieval_index": {
                "type": ["object", "null"],
                "additionalProperties": false,
                "properties": {
                    "state": { "type": "string" },
                    "files_discovered": { "type": "integer" },
                    "files_indexed": { "type": "integer" },
                    "chunks_written": { "type": "integer" },
                    "chunks_reused": { "type": "integer" },
                    "last_indexed_at": { "type": ["string", "null"] },
                    "last_error": { "type": ["string", "null"] },
                    "updated_at": { "type": "string" },
                    "searchable": { "type": "boolean" }
                },
                "required": [
                    "state",
                    "files_discovered",
                    "files_indexed",
                    "chunks_written",
                    "chunks_reused",
                    "last_indexed_at",
                    "last_error",
                    "updated_at",
                    "searchable"
                ]
            }
        }),
        &[
            "session_id",
            "agent_id",
            "event_count",
            "source_count",
            "chunk_count",
            "bridge_file_count",
            "content_db_path",
            "session_db_path",
            "bridge_dir_path",
            "retrieval_index",
        ],
        None,
    )
}

fn tool_annotations(name: &str) -> ToolAnnotations {
    let destructive = matches!(name, "purge_saved_context");
    let state_changing = matches!(
        name,
        "build_or_update_graph" | "postprocess_graph" | "compact_session" | "purge_saved_context"
    );
    ToolAnnotations {
        read_only_hint: !state_changing,
        state_changing_hint: state_changing,
        destructive_hint: destructive,
    }
}

fn tool_category(name: &str) -> &'static str {
    match name {
        "build_or_update_graph" | "postprocess_graph" => "maintenance",
        "compact_session"
        | "purge_saved_context"
        | "resume_session"
        | "save_context_artifact"
        | "read_saved_context"
        | "search_saved_context"
        | "search_decisions"
        | "get_context_stats"
        | "get_session_status"
        | "cross_session_search"
        | "get_global_memory" => "memory",
        "tool_list" | "tool_search" | "tool_help" | "man" => "introspection",
        "status" | "doctor" | "db_check" | "debug_graph" | "broker_status" => "health",
        name if name.starts_with("analyze_")
            || name.starts_with("assess_")
            || name.starts_with("find_") =>
        {
            "analysis"
        }
        name if name.starts_with("search_") || name.starts_with("read_") => "content",
        _ => "graph",
    }
}

fn tool_icons(_category: &str) -> Vec<IconDescriptor> {
    Vec::new()
}

#[cfg(test)]
const ALLOWED_TOOL_DESCRIPTOR_FIELDS: &[&str] = &[
    "name",
    "title",
    "description",
    "inputSchema",
    "outputSchema",
    "annotations",
    "icons",
    "_meta",
];

#[cfg(test)]
mod tests {
    use super::{
        ToolResultContract, tool_descriptors, tool_input_schema_by_name, tool_list,
        tool_list_markdown, tool_result_contract,
    };
    use crate::descriptors::JSON_SCHEMA_2020_12_URI;
    use jsonschema::{Draft, JSONSchema};
    use serde_json::json;

    fn compile_schema(schema: &serde_json::Value) {
        JSONSchema::options()
            .with_draft(Draft::Draft202012)
            .compile(schema)
            .expect("valid 2020-12 schema");
    }

    #[test]
    fn every_tool_name_title_and_annotations_are_present() {
        for tool in tool_descriptors() {
            assert!(
                !tool.title.trim().is_empty(),
                "missing title for {}",
                tool.name
            );
            if tool.annotations.state_changing_hint {
                assert!(
                    !tool.annotations.read_only_hint,
                    "state-changing tool marked read-only: {}",
                    tool.name
                );
            }
            if tool.annotations.destructive_hint {
                assert!(
                    tool.annotations.state_changing_hint,
                    "destructive tool must be state-changing: {}",
                    tool.name
                );
            }
        }
    }

    #[test]
    fn tool_registry_schemas_validate_as_2020_12() {
        for tool in tool_descriptors() {
            assert_eq!(tool.input_schema["$schema"], json!(JSON_SCHEMA_2020_12_URI));
            compile_schema(&tool.input_schema);
            if let Some(output_schema) = tool.output_schema.as_ref() {
                assert_eq!(output_schema["$schema"], json!(JSON_SCHEMA_2020_12_URI));
                compile_schema(output_schema);
            }
        }
    }

    #[test]
    fn every_tool_has_inventory_contract_and_matching_schema_policy() {
        for tool in tool_descriptors() {
            match tool_result_contract(&tool.name) {
                ToolResultContract::StableObject => {
                    assert!(
                        tool.output_schema.is_some(),
                        "{} must advertise outputSchema",
                        tool.name
                    );
                }
                ToolResultContract::TextOnly | ToolResultContract::MixedNeedsRedesign => {
                    assert!(
                        tool.output_schema.is_none(),
                        "{} must omit outputSchema",
                        tool.name
                    );
                }
            }
            assert_eq!(
                tool.meta["atlas:resultContract"],
                json!(tool_result_contract(&tool.name).label())
            );
        }
    }

    #[test]
    fn schema_builder_output_matches_registry_entries() {
        for tool in tool_descriptors() {
            let built = tool_input_schema_by_name(&tool.name).expect("schema by name");
            assert_eq!(
                built, tool.input_schema,
                "input schema mismatch for {}",
                tool.name
            );
        }
    }

    #[test]
    fn tool_list_serializes_typed_descriptors() {
        let value = tool_list();
        let tools = value["tools"].as_array().expect("tools array");
        assert!(tools.iter().all(|tool| tool.get("title").is_some()));
        assert!(tools.iter().all(|tool| tool.get("annotations").is_some()));
        assert!(tools.iter().all(|tool| tool.get("icons").is_none()));
        assert!(tools.iter().any(|tool| tool.get("outputSchema").is_some()));
        assert!(tools.iter().any(|tool| tool.get("outputSchema").is_none()));
        assert!(
            tools
                .iter()
                .all(|tool| tool.pointer("/_meta/atlas:resultContract").is_some())
        );
    }

    #[test]
    fn tool_list_markdown_documents_result_contract_inventory() {
        let markdown = tool_list_markdown();
        assert!(markdown.contains("| Tool | Result contract | Output schema | Description |"));
        assert!(markdown.contains("`stable-object`"));
        assert!(markdown.contains("`mixed-needs-redesign`"));
        assert!(
            markdown.contains("`broker_status` | `stable-object` | exact structuredContent schema")
        );
    }
}

#[cfg(test)]
mod schema_contract_tests {
    use super::tool_list;
    use crate::descriptors::JSON_SCHEMA_2020_12_URI;
    use jsonschema::{Draft, JSONSchema};
    use std::collections::BTreeSet;

    #[test]
    fn tools_list_serializes_only_mcp_supported_descriptor_fields() {
        let tools = tool_list()["tools"]
            .as_array()
            .expect("tools array")
            .clone();
        let allowed = super::ALLOWED_TOOL_DESCRIPTOR_FIELDS
            .iter()
            .copied()
            .collect::<BTreeSet<_>>();

        for tool in tools {
            let keys = tool
                .as_object()
                .expect("tool descriptor object")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            assert!(
                keys.is_subset(&allowed),
                "descriptor keys not allowed: {:?}",
                keys.difference(&allowed).copied().collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn tools_list_emitted_schemas_compile_under_json_schema_2020_12() {
        for tool in tool_list()["tools"].as_array().expect("tools array") {
            let input_schema = tool.get("inputSchema").expect("input schema");
            assert_eq!(
                input_schema["$schema"],
                serde_json::json!(JSON_SCHEMA_2020_12_URI)
            );
            JSONSchema::options()
                .with_draft(Draft::Draft202012)
                .compile(input_schema)
                .expect("input schema compiles");

            if let Some(output_schema) = tool.get("outputSchema") {
                assert_eq!(
                    output_schema["$schema"],
                    serde_json::json!(JSON_SCHEMA_2020_12_URI)
                );
                JSONSchema::options()
                    .with_draft(Draft::Draft202012)
                    .compile(output_schema)
                    .expect("output schema compiles");
            }
        }
    }

    #[test]
    fn r2_output_schemas_expose_nested_defs_for_normalized_payloads() {
        let registry = tool_list();
        let tools = registry["tools"].as_array().expect("tools array");

        let by_name = |name: &str| {
            tools
                .iter()
                .find(|tool| tool.get("name") == Some(&serde_json::json!(name)))
                .expect("tool present")
        };

        let impact = by_name("get_impact_radius");
        assert_eq!(
            impact["outputSchema"]["properties"]["changed_symbols"]["items"]["$ref"],
            serde_json::json!("#/$defs/compact_node")
        );
        assert!(impact["outputSchema"].get("$defs").is_some());

        let review = by_name("get_review_context");
        assert_eq!(
            review["outputSchema"]["properties"]["risk_summary"]["$ref"],
            serde_json::json!("#/$defs/review_risk_summary")
        );
        assert!(review["outputSchema"].get("$defs").is_some());

        let context = by_name("get_context");
        assert_eq!(
            context["outputSchema"]["properties"]["ranked_symbols"]["items"]["$ref"],
            serde_json::json!("#/$defs/ranked_symbol_summary")
        );
        assert_eq!(
            context["outputSchema"]["properties"]["ambiguity"]["$ref"],
            serde_json::json!("#/$defs/ambiguity")
        );
    }
}
