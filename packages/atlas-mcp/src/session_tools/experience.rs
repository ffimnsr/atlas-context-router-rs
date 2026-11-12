//! VR4 — MCP `experience_inspect` tool.

use anyhow::Result;
use atlas_adapters::derive_session_db_path;
use atlas_session::{ExperienceDirection, SessionStore};
use serde_json::{Value, json};

use crate::output::OutputFormat;
use crate::tool_result::tool_result_value as build_tool_result_value;

pub fn tool_experience_inspect(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let uri = args
        .and_then(|a| a.get("uri"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing required argument: uri"))?;
    let direction: ExperienceDirection = args
        .and_then(|a| a.get("direction"))
        .and_then(Value::as_str)
        .unwrap_or("both")
        .parse()?;
    let limit = args
        .and_then(|a| a.get("limit"))
        .and_then(Value::as_u64)
        .unwrap_or(20)
        .clamp(1, 100) as usize;

    let session_db = derive_session_db_path(db_path);
    if let Some(parent) = std::path::Path::new(&session_db).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let store = SessionStore::open(&session_db)?;
    let edges = store.inspect_experience_edges(repo_root, uri, direction, limit)?;

    build_tool_result_value(
        &json!({
            "tool": "experience_inspect",
            "repo_root": repo_root,
            "uri": uri,
            "direction": direction,
            "edges": edges,
            "summary": {
                "edge_count": edges.len(),
                "limit": limit,
            },
            "warnings": [],
        }),
        output_format,
    )
}
