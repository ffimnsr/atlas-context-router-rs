//! Typed input-schema arm for the `graph` tool family.
//!
//! Dispatched by `super::typed_input_schema_for()`; schema structs
//! come from `super::schemas`.

use super::*;
use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::Value;

pub(super) fn typed_input_schema_for(name: &str) -> Option<Value> {
    match name {
        "query_graph" => Some(typed_schema_with_descriptions::<QueryGraphArgsSchema>(&[
            (
                "properties/text",
                "Query text. Supported grammar: plain identifier ('compute'), exact qualified name ('src/service.rs::fn::compute'), 'who calls <symbol>', 'what breaks <symbol>', or 'tests for <symbol>'. Natural-language-only descriptions without a concrete identifier are rejected.",
            ),
            (
                "properties/kind",
                "Filter by node kind (e.g. 'function', 'struct')",
            ),
            (
                "properties/language",
                "Filter by language (e.g. 'rust', 'python')",
            ),
            ("properties/limit", "Maximum results to return (default 20)"),
            (
                "properties/semantic",
                "Graph-neighbour expansion on top of FTS: re-ranks initial FTS hits using graph edges (default false). NOT vector/embedding search — still requires FTS to find at least one initial symbol-name hit. If FTS returns nothing (e.g. text was a phrase not a symbol name), semantic expansion also returns nothing. Use regex instead for pattern-based fallback.",
            ),
            (
                "properties/expand",
                "Expand results through graph edges after ranking (default false). Subsumed by semantic=true; setting both is redundant.",
            ),
            (
                "properties/expand_hops",
                "Max edge hops when expand=true (default 1)",
            ),
            (
                "properties/regex",
                "Regex pattern matched against name and qualified_name via SQL UDF. Empty string is treated like omitted. Three modes: (1) regex-only structural scan when text is empty — filters every node in the DB; (2) text+regex: FTS5 runs first then the UDF post-filters its candidates inside SQLite; (3) invalid pattern returns an error with details. Supports regex crate alternation syntax (e.g. 'handle|HANDLE|Handle_'). Must be valid regex crate syntax.",
            ),
            (
                "properties/subpath",
                "Restrict results to nodes whose file_path starts with this prefix (e.g. 'src/auth', 'packages/atlas-core'). Filtering happens in SQL before ranking.",
            ),
            (
                "properties/fuzzy",
                "Enable fuzzy (edit-distance) typo recovery for near-miss symbol names (default false). Uses relaxed candidate expansion plus stronger code-symbol ranking so close symbol typos outrank weaker docs/config matches.",
            ),
            (
                "properties/hybrid",
                "Enable hybrid FTS + vector retrieval with Reciprocal Rank Fusion (default false). Requires search.embedding.url in .atlas/config.toml; falls back to FTS-only when no embedding backend is configured.",
            ),
            (
                "properties/include_files",
                "Include file nodes in the result set (default false). Leave disabled for symbol-centric search; enable when a file-level hit is useful.",
            ),
            (
                "properties/repo_scope",
                "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
            ),
            (
                "properties/repo_scope/properties/kind",
                "Repo scope kind: current, repo_id, or all.",
            ),
            (
                "properties/repo_scope/properties/repo_id",
                "Required when kind='repo_id'. Must be registered and enabled.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "batch_query_graph" => Some(typed_schema_with_descriptions::<BatchQueryGraphArgsSchema>(
            &[
                (
                    "properties/items",
                    "Array of query objects (1-20). Each object accepts same fields as query_graph: text, kind, language, limit, semantic, expand, expand_hops, regex, subpath, fuzzy, hybrid, include_files.",
                ),
                (
                    "properties/items/items/properties/text",
                    "Symbol name or identifier to search (e.g. 'BalancesTab'). Required unless 'regex' is set.",
                ),
                (
                    "properties/items/items/properties/kind",
                    "Filter by node kind (e.g. 'function', 'struct')",
                ),
                (
                    "properties/items/items/properties/language",
                    "Filter by language (e.g. 'rust', 'typescript')",
                ),
                (
                    "properties/items/items/properties/limit",
                    "Maximum results for this query (default 20)",
                ),
                (
                    "properties/items/items/properties/semantic",
                    "Graph-neighbour expansion on top of FTS (default false). Requires FTS to find at least one hit first.",
                ),
                (
                    "properties/items/items/properties/expand",
                    "Expand results through graph edges after ranking (default false)",
                ),
                (
                    "properties/items/items/properties/expand_hops",
                    "Max edge hops when expand=true (default 1)",
                ),
                (
                    "properties/items/items/properties/regex",
                    "Regex pattern matched against name and qualified_name via SQL UDF. Must be valid regex crate syntax.",
                ),
                (
                    "properties/items/items/properties/subpath",
                    "Restrict results to nodes whose file_path starts with this prefix.",
                ),
                (
                    "properties/items/items/properties/fuzzy",
                    "Enable fuzzy typo recovery (default false).",
                ),
                (
                    "properties/items/items/properties/hybrid",
                    "Enable hybrid FTS + vector retrieval (default false). Requires search.embedding.url in .atlas/config.toml.",
                ),
                (
                    "properties/items/items/properties/include_files",
                    "Include file nodes in the result set (default false).",
                ),
                (
                    "properties/repo_scope",
                    "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
                ),
                (
                    "properties/repo_scope/properties/kind",
                    "Repo scope kind: current, repo_id, or all.",
                ),
                (
                    "properties/repo_scope/properties/repo_id",
                    "Required when kind='repo_id'. Must be registered and enabled.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "traverse_graph" => Some(typed_schema_with_descriptions::<TraverseGraphArgsSchema>(
            &[
                (
                    "properties/from_qn",
                    "Qualified name of the starting node (e.g. 'src/lib.rs::fn::my_func')",
                ),
                ("properties/max_depth", "Traversal depth limit (default 3)"),
                (
                    "properties/max_nodes",
                    "Maximum nodes to return (default 100)",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "symbol_neighbors" => Some(typed_schema_with_descriptions::<SymbolNeighborsArgsSchema>(
            &[
                (
                    "properties/qname",
                    "Fully-qualified name of the symbol (e.g. 'src/lib.rs::fn::my_func').",
                ),
                (
                    "properties/limit",
                    "Maximum nodes to return per relationship kind (default 10).",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "cross_file_links" => Some(typed_schema_with_descriptions::<CrossFileLinksArgsSchema>(
            &[
                (
                    "properties/file",
                    "Repo-relative file path to analyse (e.g. 'src/auth.rs').",
                ),
                ("properties/limit", "Maximum links to return (default 20)."),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "concept_clusters" => Some(typed_schema_with_descriptions::<ConceptClustersArgsSchema>(
            &[
                (
                    "properties/files",
                    "Seed file paths (repo-relative) to cluster around.",
                ),
                (
                    "properties/limit",
                    "Maximum clusters to return (default 10).",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "explain_query" => Some(typed_schema_with_descriptions::<ExplainQueryArgsSchema>(&[
            (
                "properties/text",
                "Symbol name or identifier — same as query_graph 'text'.",
            ),
            (
                "properties/kind",
                "Node kind filter (e.g. 'function', 'struct') — same as query_graph 'kind'.",
            ),
            (
                "properties/language",
                "Language filter (e.g. 'rust') — same as query_graph 'language'.",
            ),
            ("properties/limit", "Result limit (default 20)."),
            (
                "properties/semantic",
                "Whether semantic expansion would be applied (default false).",
            ),
            (
                "properties/regex",
                "Regex pattern — validated and explained. Regex-only (text empty): structural scan. text+regex: FTS5 first then UDF post-filter. Invalid pattern: error with details.",
            ),
            (
                "properties/subpath",
                "File-path prefix filter — same as query_graph 'subpath'.",
            ),
            (
                "properties/fuzzy",
                "Whether fuzzy name-matching boost would be active (default false).",
            ),
            (
                "properties/hybrid",
                "Whether hybrid FTS + vector retrieval would be used (default false). Requires search.embedding.url in .atlas/config.toml.",
            ),
            (
                "properties/include_files",
                "Whether file nodes would be included in the result set (default false).",
            ),
            (
                "properties/repo_scope",
                "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
            ),
            (
                "properties/repo_scope/properties/kind",
                "Repo scope kind: current, repo_id, or all.",
            ),
            (
                "properties/repo_scope/properties/repo_id",
                "Required when kind='repo_id'. Must be registered and enabled.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "resolve_symbol" => Some(typed_schema_with_descriptions::<ResolveSymbolArgsSchema>(
            &[
                (
                    "properties/name",
                    "Symbol identifier or exact qualified name to resolve (e.g. 'LoadIdentityMessages', 'compute', 'src/service.rs::fn::compute').",
                ),
                (
                    "properties/kind",
                    "Optional kind filter. Accepts public aliases: 'function'/'fn'/'func', 'method', 'class', 'struct'/'record', 'interface'/'iface', 'trait', 'enum', 'module'/'mod', 'variable'/'var', 'constant'/'const', 'test', 'import', 'package'/'pkg', 'file'.",
                ),
                (
                    "properties/file",
                    "Optional file path filter. Only returns matches whose file_path contains this string (e.g. 'internal/requestctx/context.go' or 'src/').",
                ),
                (
                    "properties/language",
                    "Optional language filter (e.g. 'rust', 'go', 'typescript').",
                ),
                (
                    "properties/limit",
                    "Maximum matches to return (default 10).",
                ),
                (
                    "properties/repo_scope",
                    "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
                ),
                (
                    "properties/repo_scope/properties/kind",
                    "Repo scope kind: current, repo_id, or all.",
                ),
                (
                    "properties/repo_scope/properties/repo_id",
                    "Required when kind='repo_id'. Must be registered and enabled.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        _ => None,
    }
}
