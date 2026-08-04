//! Base `tools/list` registry entries for the `context` tool family.
//!
//! Assembled by `super::base_tool_list_json()`; entry order is
//! irrelevant because descriptors are sorted by name afterwards.

use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::{Value, json};

/// Base registry entry JSON for the context tools.
pub(super) fn tools() -> Vec<Value> {
    vec![
        json!({
                "name": "get_impact_radius",
                "description": "Compute nodes and files affected when the given files change. Returns compact, capped results. Change-source conflicts return structured retry guidance instead of a bare ambiguity error.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "change_source": {
                            "type": "object",
                            "description": "Change-source object. Use { kind: 'files', files: ['src/lib.rs'] }, { kind: 'base', base: 'origin/main' }, { kind: 'staged' }, or { kind: 'working_tree' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Change-source kind: files, base, staged, or working_tree." },
                                "files": { "type": "array", "items": { "type": "string" }, "description": "Required when kind='files'. Non-empty repo-relative file path list." },
                                "base": { "type": "string", "description": "Required when kind='base'. Base git ref such as 'origin/main'." }
                            },
                            "required": ["kind"]
                        },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit (default 5)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes to return (default 200)" },
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
                "name": "get_review_context",
                "description": "Assemble review context for the given files: changed symbols, impacted neighbors, critical edges, and risk summary. Agent-optimized output. Change-source conflicts return structured retry guidance instead of a bare ambiguity error.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "change_source": {
                            "type": "object",
                            "description": "Change-source object. Use { kind: 'files', files: ['src/lib.rs'] }, { kind: 'base', base: 'origin/main' }, { kind: 'staged' }, or { kind: 'working_tree' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Change-source kind: files, base, staged, or working_tree." },
                                "files": { "type": "array", "items": { "type": "string" }, "description": "Required when kind='files'. Non-empty repo-relative file path list." },
                                "base": { "type": "string", "description": "Required when kind='base'. Base git ref such as 'origin/main'." }
                            },
                            "required": ["kind"]
                        },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit (default 3)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes to consider (default 200)" },
                        "token_budget": { "type": "integer", "description": "Maximum tokens to include in the result. Overrides the default policy limit for this call only. Cannot exceed the policy ceiling." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "detect_changes",
                "description": "List files changed since a base git ref, with per-file node counts from the graph. Use `change_source={ kind: ... }` with `base`, `staged`, or `working_tree`.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "change_source": {
                            "type": "object",
                            "description": "Change-source object. Use { kind: 'base', base: 'origin/main' }, { kind: 'staged' }, or { kind: 'working_tree' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Change-source kind: base, staged, or working_tree." },
                                "base": { "type": "string", "description": "Required when kind='base'. Base git ref such as 'origin/main'." }
                            },
                            "required": ["kind"]
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
                "name": "build_or_update_graph",
                "description": "Scan, parse, and persist the code graph. Canonical input is `operation={ kind: 'build' }` for full scan or `operation={ kind: 'update', change_source: ... }` for incremental update.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "operation": {
                            "type": "object",
                            "description": "Operation object. Use { kind: 'build' } or { kind: 'update', change_source: { kind: 'working_tree'|'staged'|'base'|'files', ... } }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Operation kind: build or update." },
                                "change_source": {
                                    "type": "object",
                                    "description": "Required when kind='update'. Use { kind: 'working_tree' }, { kind: 'staged' }, { kind: 'base', base: 'origin/main' }, or { kind: 'files', files: ['src/lib.rs'] }.",
                                    "properties": {
                                        "kind": { "type": "string", "description": "Change-source kind: working_tree, staged, base, or files." },
                                        "base": { "type": "string", "description": "Required when kind='base'. Base git ref such as 'origin/main'." },
                                        "files": { "type": "array", "items": { "type": "string" }, "description": "Required when kind='files'. Non-empty repo-relative file path list." }
                                    },
                                    "required": ["kind"]
                                }
                            },
                            "required": ["kind"]
                        },

                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
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
        }),
        json!({
                "name": "get_minimal_context",
                "description": "Auto-detect changed files from git, then return a compact review bundle: changed symbols, immediate impact, risk flags. Lower token overhead than get_review_context.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "change_source": {
                            "type": "object",
                            "description": "Change-source object. Use { kind: 'base', base: 'origin/main' }, { kind: 'staged' }, or { kind: 'working_tree' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Change-source kind: base, staged, or working_tree." },
                                "base": { "type": "string", "description": "Required when kind='base'. Base git ref such as 'origin/main'." }
                            },
                            "required": ["kind"]
                        },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit (default 2)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes (default 50)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "explain_change",
                "description": "Advanced impact analysis for a set of changed files: risk level, changed-symbol breakdown by change kind (api/signature/internal), boundary violations, test coverage gaps, and a compact summary. Deterministic, LLM-free.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "change_source": {
                            "type": "object",
                            "description": "Change-source object. Use { kind: 'files', files: ['src/lib.rs'] }, { kind: 'base', base: 'origin/main' }, { kind: 'staged' }, or { kind: 'working_tree' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Change-source kind: files, base, staged, or working_tree." },
                                "files": { "type": "array", "items": { "type": "string" }, "description": "Required when kind='files'. Non-empty repo-relative file path list." },
                                "base": { "type": "string", "description": "Required when kind='base'. Base git ref such as 'origin/main'." }
                            },
                            "required": ["kind"]
                        },
                        "max_depth": { "type": "integer", "description": "Traversal depth limit for impact (default 5)" },
                        "max_nodes": { "type": "integer", "description": "Maximum impacted nodes (default 200)" },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "get_context",
                "description": "Build bounded context around a symbol, file, or change-set. Canonical input is `target={ kind: 'query'|'file'|'files', ... }`. Supported query grammar: plain identifier (`compute`), exact qualified name (`src/service.rs::fn::compute`), `who calls <symbol>`, `what breaks <symbol>`, and `tests for <symbol>`. Natural-language-only descriptions without a concrete identifier are rejected with retry guidance. When changed files include docs, config, templates, SQL, or prompts, pass those paths in files target to merge graph and content assets under one bounded selection, ranking, and truncation policy.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "target": {
                            "type": "object",
                            "description": "Target object. Use { kind: 'query', query: 'handle_request' }, { kind: 'file', file: 'src/lib.rs' }, or { kind: 'files', files: ['src/lib.rs'] }. Query grammar supports plain identifiers, exact qualified names, 'who calls <symbol>', 'what breaks <symbol>', and 'tests for <symbol>'.",
                            "properties": {
                                "kind": { "type": "string", "description": "Target kind: query, file, or files." },
                                "query": { "type": "string", "description": "Required when kind='query'. Supported grammar: plain identifier, exact qualified name, 'who calls <symbol>', 'what breaks <symbol>', or 'tests for <symbol>'. Natural-language-only descriptions without a concrete identifier are rejected." },
                                "file": { "type": "string", "description": "Required when kind='file'. Repo-relative file path." },
                                "files": { "type": "array", "items": { "type": "string" }, "description": "Required when kind='files'. Non-empty repo-relative file path list." }
                            },
                            "required": ["kind"]
                        },

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
        }),
    ]
}
