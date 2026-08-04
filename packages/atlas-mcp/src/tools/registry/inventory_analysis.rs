//! Base `tools/list` registry entries for the `analysis` tool family.
//!
//! Assembled by `super::base_tool_list_json()`; entry order is
//! irrelevant because descriptors are sorted by name afterwards.

use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::{Value, json};

/// Base registry entry JSON for the analysis tools.
pub(super) fn tools() -> Vec<Value> {
    vec![
        json!({
                "name": "analyze_architecture",
                "description": "Analyze module-level cycles, layer violations, and coupling hotspots. JSON output matches the CLI insights architecture report; JSON output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Cap returned findings after ranking." },
                        "verbose": { "type": "boolean", "description": "Return full report body in JSON output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "analyze_metrics",
                "description": "Analyze graph health metrics, outliers, complexity hotspots, and coupling findings. JSON output matches the CLI insights metrics report; JSON output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Cap returned findings after ranking." },
                        "verbose": { "type": "boolean", "description": "Return full report body in JSON output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "assess_risk",
                "description": "Score deterministic risk for one symbol with factor evidence and ranked findings. JSON output matches the CLI insights risk report; JSON output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Qualified name or resolvable symbol identifier." },
                        "verbose": { "type": "boolean", "description": "Return full report body in JSON output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["symbol"]
                }
        }),
        json!({
                "name": "analyze_patterns",
                "description": "Detect repeated call chains, isolated structures, hubs, bottlenecks, and deep dependency paths. JSON output matches the CLI insights patterns report; JSON output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Cap returned findings after ranking." },
                        "verbose": { "type": "boolean", "description": "Return full report body in JSON output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "find_large_functions",
                "description": "Find large or complex functions repo-wide or within selected files using deterministic LOC and complexity thresholds. JSON output matches the CLI insights report; JSON output stays compact for agent review.",
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
                        "verbose": { "type": "boolean", "description": "Return full report body in JSON output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "find_complex_functions",
                "description": "Find complex functions repo-wide or within selected files using deterministic complexity thresholds. JSON output matches the CLI insights complex-functions report; JSON output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "files": { "type": "array", "items": { "type": "string" }, "description": "Optional repo-relative files to scope the search." },
                        "complexity_threshold": { "type": "integer", "description": "Override cyclomatic complexity threshold." },
                        "cognitive_threshold": { "type": "integer", "description": "Override cognitive complexity threshold." },
                        "nesting_threshold": { "type": "integer", "description": "Override max nesting depth threshold." },
                        "limit": { "type": "integer", "description": "Cap result count after ranking." },
                        "include_tests": { "type": "boolean", "description": "Include test functions and methods." },
                        "verbose": { "type": "boolean", "description": "Return full report body in JSON output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "find_similar_functions",
                "description": "Find semantically similar functions for one callable symbol using name, signature, body-shingle, and neighbor overlap. JSON output matches the CLI similar-functions report; JSON output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "symbol": { "type": "string", "description": "Qualified name or resolvable callable identifier." },
                        "min_score": { "type": "number", "description": "Minimum similarity score to keep (0.0-1.0)." },
                        "limit": { "type": "integer", "description": "Cap returned matches after ranking." },
                        "include_same_file": { "type": "boolean", "description": "Keep same-file matches too." },
                        "verbose": { "type": "boolean", "description": "Return full report body in JSON output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["symbol"]
                }
        }),
        json!({
                "name": "find_duplicates",
                "description": "Find exact-normalized and near-duplicate callable bodies using normalized token shingles. JSON output matches the CLI duplicates report; JSON output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "files": { "type": "array", "items": { "type": "string" }, "description": "Optional repo-relative files to scope the search." },
                        "min_score": { "type": "number", "description": "Minimum duplicate confidence to keep (0.0-1.0)." },
                        "limit": { "type": "integer", "description": "Cap returned duplicate groups after ranking." },
                        "include_tests": { "type": "boolean", "description": "Include test functions and methods." },
                        "suppressions": { "type": "array", "items": { "type": "string" }, "description": "Optional suppressions matched against duplicate group id, normalized summary, file path, or symbol name." },
                        "verbose": { "type": "boolean", "description": "Return full report body in JSON output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "infer_modules",
                "description": "Infer module buckets from package ownership, path layout, and dependency structure. JSON output matches the CLI infer-modules report; JSON output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit": { "type": "integer", "description": "Cap returned findings after ranking." },
                        "verbose": { "type": "boolean", "description": "Return full report body in JSON output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "label_components",
                "description": "Label files and symbols with Atlas component taxonomy such as cli, mcp, parse, review_context, and session_continuity. JSON output matches the CLI label-components report; JSON output stays compact unless verbose=true.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "files": { "type": "array", "items": { "type": "string" }, "description": "Optional repo-relative files to scope file labeling." },
                        "symbols": { "type": "array", "items": { "type": "string" }, "description": "Optional qualified names to scope symbol labeling." },
                        "limit": { "type": "integer", "description": "Cap returned assignments after ranking." },
                        "verbose": { "type": "boolean", "description": "Return full report body in JSON output too." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
        json!({
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
        }),
    ]
}
