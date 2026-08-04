//! Typed input-schema arm for the `analysis` tool family.
//!
//! Dispatched by `super::typed_input_schema_for()`; schema structs
//! come from `super::schemas`.

use super::*;
use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::Value;

pub(super) fn typed_input_schema_for(name: &str) -> Option<Value> {
    match name {
        "analyze_safety" => Some(typed_schema_with_descriptions::<AnalyzeSafetyArgsSchema>(
            &[
                (
                    "properties/symbol",
                    "Fully-qualified symbol name (e.g. 'src/auth.rs::fn::verify_token').",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "analyze_remove" => Some(typed_schema_with_descriptions::<AnalyzeRemoveArgsSchema>(
            &[
                (
                    "properties/symbols",
                    "Fully-qualified symbol names to remove.",
                ),
                ("properties/max_depth", "Traversal depth limit (default 3)."),
                (
                    "properties/max_nodes",
                    "Maximum impacted nodes to return (default 200).",
                ),
                (
                    "properties/max_files",
                    "Maximum impacted files to include in the response (default 20). Raises omitted_file_count when truncated.",
                ),
                (
                    "properties/max_edges",
                    "Maximum relevant edges to include in the response (default 50). Raises omitted_edge_count when truncated.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "analyze_dead_code" => Some(typed_schema_with_descriptions::<AnalyzeDeadCodeArgsSchema>(
            &[
                (
                    "properties/allowlist",
                    "Qualified names to exclude from dead-code candidates even when they have no inbound edges.",
                ),
                (
                    "properties/subpath",
                    "Restrict scan to nodes whose file_path starts with this prefix (e.g. 'src/internal').",
                ),
                (
                    "properties/limit",
                    "Maximum candidates to return (default 50).",
                ),
                (
                    "properties/summary",
                    "Return only the candidate count, not the full list. Useful for quick health checks.",
                ),
                (
                    "properties/exclude_kind",
                    "Node kinds to exclude from results (e.g. ['constant', 'variable']). Accepted values: function, method, struct, enum, trait, interface, class, constant, variable.",
                ),
                (
                    "properties/code_only",
                    "Restrict to code symbols only (default true). Non-code nodes (files, packages, docs) are always excluded in the current implementation.",
                ),
                (
                    "properties/max_files",
                    "Reserved for future per-candidate file-list truncation. No effect in current implementation.",
                ),
                (
                    "properties/max_edges",
                    "Reserved for future per-candidate edge-list truncation. No effect in current implementation.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "analyze_dependency" => Some(
            typed_schema_with_descriptions::<AnalyzeDependencyArgsSchema>(&[
                (
                    "properties/symbol",
                    "Fully-qualified symbol name to check (e.g. 'src/lib.rs::fn::legacy_parse').",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ]),
        ),
        "find_large_functions" => Some(typed_schema_with_descriptions::<
            FindLargeFunctionsArgsSchema,
        >(&[
            (
                "properties/files",
                "Optional repo-relative files to scope the search.",
            ),
            ("properties/threshold", "Override LOC threshold."),
            (
                "properties/complexity_threshold",
                "Override cyclomatic complexity threshold.",
            ),
            (
                "properties/cognitive_threshold",
                "Override cognitive complexity threshold.",
            ),
            (
                "properties/nesting_threshold",
                "Override max nesting depth threshold.",
            ),
            (
                "properties/mode",
                "One of 'large', 'complex', or 'large-or-complex'.",
            ),
            ("properties/limit", "Cap result count after ranking."),
            (
                "properties/include_tests",
                "Include test functions and methods.",
            ),
            (
                "properties/verbose",
                "Return full report body in JSON output too.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "find_complex_functions" => Some(typed_schema_with_descriptions::<
            FindComplexFunctionsArgsSchema,
        >(&[
            (
                "properties/files",
                "Optional repo-relative files to scope the search.",
            ),
            (
                "properties/complexity_threshold",
                "Override cyclomatic complexity threshold.",
            ),
            (
                "properties/cognitive_threshold",
                "Override cognitive complexity threshold.",
            ),
            (
                "properties/nesting_threshold",
                "Override max nesting depth threshold.",
            ),
            ("properties/limit", "Cap result count after ranking."),
            (
                "properties/include_tests",
                "Include test functions and methods.",
            ),
            (
                "properties/verbose",
                "Return full report body in JSON output too.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "find_similar_functions" => Some(typed_schema_with_descriptions::<
            FindSimilarFunctionsArgsSchema,
        >(&[
            (
                "properties/symbol",
                "Qualified name or resolvable callable identifier.",
            ),
            (
                "properties/min_score",
                "Minimum similarity score to keep (0.0-1.0).",
            ),
            ("properties/limit", "Cap returned matches after ranking."),
            (
                "properties/include_same_file",
                "Keep same-file matches too.",
            ),
            (
                "properties/verbose",
                "Return full report body in JSON output too.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "find_duplicates" => Some(typed_schema_with_descriptions::<FindDuplicatesArgsSchema>(
            &[
                (
                    "properties/files",
                    "Optional repo-relative files to scope the search.",
                ),
                (
                    "properties/min_score",
                    "Minimum duplicate confidence to keep (0.0-1.0).",
                ),
                (
                    "properties/limit",
                    "Cap returned duplicate groups after ranking.",
                ),
                (
                    "properties/include_tests",
                    "Include test functions and methods.",
                ),
                (
                    "properties/suppressions",
                    "Optional suppressions matched against duplicate group id, normalized summary, file path, or symbol name.",
                ),
                (
                    "properties/verbose",
                    "Return full report body in JSON output too.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "infer_modules" => Some(typed_schema_with_descriptions::<InferModulesArgsSchema>(&[
            ("properties/limit", "Cap returned findings after ranking."),
            (
                "properties/verbose",
                "Return full report body in JSON output too.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "label_components" => Some(typed_schema_with_descriptions::<LabelComponentsArgsSchema>(
            &[
                (
                    "properties/files",
                    "Optional repo-relative files to scope file labeling.",
                ),
                (
                    "properties/symbols",
                    "Optional qualified names to scope symbol labeling.",
                ),
                (
                    "properties/limit",
                    "Cap returned assignments after ranking.",
                ),
                (
                    "properties/verbose",
                    "Return full report body in JSON output too.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        _ => None,
    }
}
