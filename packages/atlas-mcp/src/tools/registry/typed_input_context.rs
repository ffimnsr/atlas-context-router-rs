//! Typed input-schema arm for the `context` tool family.
//!
//! Dispatched by `super::typed_input_schema_for()`; schema structs
//! come from `super::schemas`.

use super::*;
use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::Value;

pub(super) fn typed_input_schema_for(name: &str) -> Option<Value> {
    match name {
        "get_impact_radius" => Some(typed_schema_with_descriptions::<GetImpactRadiusArgsSchema>(
            &[
                (
                    "properties/change_source",
                    "Change-source object. Use { kind: 'files', files: ['src/lib.rs'] }, { kind: 'base', base: 'origin/main' }, { kind: 'staged' }, or { kind: 'working_tree' }.",
                ),
                (
                    "properties/change_source/properties/kind",
                    "Change-source kind: files, base, staged, or working_tree.",
                ),
                (
                    "properties/change_source/properties/files",
                    "Required when kind='files'. Non-empty repo-relative file path list.",
                ),
                (
                    "properties/change_source/properties/base",
                    "Required when kind='base'. Base git ref such as 'origin/main'.",
                ),
                ("properties/max_depth", "Traversal depth limit (default 5)"),
                (
                    "properties/max_nodes",
                    "Maximum impacted nodes to return (default 200)",
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
        "get_review_context" => Some(
            typed_schema_with_descriptions::<GetReviewContextArgsSchema>(&[
                (
                    "properties/change_source",
                    "Change-source object. Use { kind: 'files', files: ['src/lib.rs'] }, { kind: 'base', base: 'origin/main' }, { kind: 'staged' }, or { kind: 'working_tree' }.",
                ),
                (
                    "properties/change_source/properties/kind",
                    "Change-source kind: files, base, staged, or working_tree.",
                ),
                (
                    "properties/change_source/properties/files",
                    "Required when kind='files'. Non-empty repo-relative file path list.",
                ),
                (
                    "properties/change_source/properties/base",
                    "Required when kind='base'. Base git ref such as 'origin/main'.",
                ),
                ("properties/max_depth", "Traversal depth limit (default 3)"),
                (
                    "properties/max_nodes",
                    "Maximum impacted nodes to consider (default 200)",
                ),
                (
                    "properties/token_budget",
                    "Maximum tokens to include in the result. Overrides the default policy limit for this call only. Cannot exceed the policy ceiling.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ]),
        ),
        "detect_changes" => Some(typed_schema_with_descriptions::<DetectChangesArgsSchema>(
            &[
                (
                    "properties/change_source",
                    "Change-source object. Use { kind: 'base', base: 'origin/main' }, { kind: 'staged' }, or { kind: 'working_tree' }.",
                ),
                (
                    "properties/change_source/properties/kind",
                    "Change-source kind: base, staged, or working_tree.",
                ),
                (
                    "properties/change_source/properties/base",
                    "Required when kind='base'. Base git ref such as 'origin/main'.",
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
        "build_or_update_graph" => Some(typed_schema_with_descriptions::<
            BuildOrUpdateGraphArgsSchema,
        >(&[
            (
                "properties/operation",
                "Operation object. Use { kind: 'build' } or { kind: 'update', change_source: { kind: 'working_tree'|'staged'|'base'|'files', ... } }.",
            ),
            (
                "properties/operation/properties/kind",
                "Operation kind: build or update.",
            ),
            (
                "properties/operation/properties/change_source",
                "Required when kind='update'. Use { kind: 'working_tree' }, { kind: 'staged' }, { kind: 'base', base: 'origin/main' }, or { kind: 'files', files: ['src/lib.rs'] }.",
            ),
            (
                "properties/operation/properties/change_source/properties/kind",
                "Change-source kind: working_tree, staged, base, or files.",
            ),
            (
                "properties/operation/properties/change_source/properties/base",
                "Required when kind='base'. Base git ref such as 'origin/main'.",
            ),
            (
                "properties/operation/properties/change_source/properties/files",
                "Required when kind='files'. Non-empty repo-relative file path list.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "postprocess_graph" => Some(
            typed_schema_with_descriptions::<PostprocessGraphArgsSchema>(&[
                (
                    "properties/changed_only",
                    "Restrict postprocess to files currently changed in the working tree when stage dependencies allow.",
                ),
                (
                    "properties/stage",
                    "Optional stage name: flows, communities, architecture_metrics, query_hints, or large_function_summaries.",
                ),
                (
                    "properties/dry_run",
                    "Compute the stage summary without recording lifecycle state.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ]),
        ),
        "get_minimal_context" => Some(
            typed_schema_with_descriptions::<GetMinimalContextArgsSchema>(&[
                (
                    "properties/change_source",
                    "Change-source object. Use { kind: 'base', base: 'origin/main' }, { kind: 'staged' }, or { kind: 'working_tree' }.",
                ),
                (
                    "properties/change_source/properties/kind",
                    "Change-source kind: base, staged, or working_tree.",
                ),
                (
                    "properties/change_source/properties/base",
                    "Required when kind='base'. Base git ref such as 'origin/main'.",
                ),
                ("properties/max_depth", "Traversal depth limit (default 2)"),
                (
                    "properties/max_nodes",
                    "Maximum impacted nodes (default 50)",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ]),
        ),
        "explain_change" => Some(typed_schema_with_descriptions::<ExplainChangeArgsSchema>(
            &[
                (
                    "properties/change_source",
                    "Change-source object. Use { kind: 'files', files: ['src/lib.rs'] }, { kind: 'base', base: 'origin/main' }, { kind: 'staged' }, or { kind: 'working_tree' }.",
                ),
                (
                    "properties/change_source/properties/kind",
                    "Change-source kind: files, base, staged, or working_tree.",
                ),
                (
                    "properties/change_source/properties/files",
                    "Required when kind='files'. Non-empty repo-relative file path list.",
                ),
                (
                    "properties/change_source/properties/base",
                    "Required when kind='base'. Base git ref such as 'origin/main'.",
                ),
                (
                    "properties/max_depth",
                    "Traversal depth limit for impact (default 5)",
                ),
                (
                    "properties/max_nodes",
                    "Maximum impacted nodes (default 200)",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "get_context" => Some(typed_schema_with_descriptions::<GetContextArgsSchema>(&[
            (
                "properties/target",
                "Target object. Use { kind: 'query', query: 'handle_request' }, { kind: 'file', file: 'src/lib.rs' }, or { kind: 'files', files: ['src/lib.rs'] }. Query grammar supports plain identifiers, exact qualified names, 'who calls <symbol>', 'what breaks <symbol>', and 'tests for <symbol>'.",
            ),
            (
                "properties/target/properties/kind",
                "Target kind: query, file, or files.",
            ),
            (
                "properties/target/properties/query",
                "Required when kind='query'. Supported grammar: plain identifier, exact qualified name, 'who calls <symbol>', 'what breaks <symbol>', or 'tests for <symbol>'. Natural-language-only descriptions without a concrete identifier are rejected.",
            ),
            (
                "properties/target/properties/file",
                "Required when kind='file'. Repo-relative file path.",
            ),
            (
                "properties/target/properties/files",
                "Required when kind='files'. Non-empty repo-relative file path list.",
            ),
            (
                "properties/intent",
                "Override intent: symbol, file, review, impact, usage_lookup, refactor_safety, dead_code_check, rename_preview, dependency_removal. Inferred when omitted.",
            ),
            (
                "properties/max_nodes",
                "Maximum nodes to include (default 100).",
            ),
            (
                "properties/max_edges",
                "Maximum edges to include (default 100).",
            ),
            (
                "properties/max_files",
                "Maximum files to include in result. Omit for no cap. Reduces token use when the change-set is large.",
            ),
            (
                "properties/max_depth",
                "Traversal depth in graph hops (default 2).",
            ),
            (
                "properties/code_spans",
                "Include line-range spans for each selected file node (default false). Adds token cost; useful when you need precise edit coordinates.",
            ),
            (
                "properties/tests",
                "Include test nodes in context (default false). Enable when reviewing test coverage or debugging test failures.",
            ),
            (
                "properties/imports",
                "Include import edges and nodes (default true). Set false to reduce noise when only callers/callees matter.",
            ),
            (
                "properties/neighbors",
                "Include containment-sibling nodes — functions/types in the same parent scope (default false).",
            ),
            (
                "properties/semantic",
                "Run graph-aware semantic search to resolve the best-matching qualified name before building context (default false). Useful when the symbol name is ambiguous or approximate.",
            ),
            (
                "properties/include_saved_context",
                "When true, also query the content store for saved artifacts relevant to this request and include them in the result (default false).",
            ),
            (
                "properties/session_id",
                "Restrict saved-context retrieval to artifacts from this session and apply a same-session relevance boost.",
            ),
            (
                "properties/agent_id",
                "Restrict saved-context retrieval to one agent memory partition.",
            ),
            (
                "properties/merge_agent_partitions",
                "Intentionally merge context across all agent partitions instead of filtering to one partition.",
            ),
            (
                "properties/token_budget",
                "Maximum tokens to include in the result. Overrides the default policy limit for this call only. Cannot exceed the policy ceiling. Use to enforce tighter context budgets from the caller side.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        _ => None,
    }
}
