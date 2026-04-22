use anyhow::{Context, Result};
use atlas_core::model::ContextIntent;
use atlas_store_sqlite::{GraphBuildState, Store};
use serde::Serialize;

use crate::output::{OutputFormat, render_serializable};

pub(super) const DEFAULT_OUTPUT_DESCRIPTION: &str =
    "Response body format: 'toon' (default) or 'json'";

pub(super) fn parse_mcp_intent(s: &str) -> ContextIntent {
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

pub(super) fn str_arg<'a>(
    args: Option<&'a serde_json::Value>,
    key: &str,
) -> Result<Option<&'a str>> {
    Ok(args.and_then(|a| a.get(key)).and_then(|v| v.as_str()))
}

pub(super) fn u64_arg(args: Option<&serde_json::Value>, key: &str) -> Option<u64> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_u64())
}

pub(super) fn bool_arg(args: Option<&serde_json::Value>, key: &str) -> Option<bool> {
    args.and_then(|a| a.get(key)).and_then(|v| v.as_bool())
}

pub(super) fn string_array_arg(args: Option<&serde_json::Value>, key: &str) -> Result<Vec<String>> {
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

pub(super) fn open_store(db_path: &str) -> Result<Store> {
    Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))
}

pub(super) fn failure_category(
    db_exists: bool,
    db_open_ok: bool,
    build_state: Option<&GraphBuildState>,
) -> &'static str {
    if !db_exists {
        return "missing_graph_db";
    }
    if !db_open_ok {
        return "corrupt_or_inconsistent_graph_rows";
    }
    match build_state {
        Some(GraphBuildState::Building) => "interrupted_build",
        Some(GraphBuildState::BuildFailed) => "failed_build",
        _ => "none",
    }
}

pub(super) fn error_message(error_code: &str) -> &'static str {
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

pub(super) fn error_suggestions(error_code: &str) -> &'static [&'static str] {
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

pub(super) fn resolve_kind_alias(input: &str) -> String {
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

pub(super) fn tool_result_value<T: Serialize>(
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
