//! CM7 — MCP session-continuity and saved-context tools.
//!
//! Implements the six new MCP tools that expose session identity, resume
//! snapshots, and content-store search/save/purge to agents.  Session event
//! emission for the four existing continuity tools is also handled here via
//! `emit_session_event_best_effort`.
//!
//! Design constraints (from Core Design Rules):
//! - Never store saved context in the graph database.
//! - Never block the primary tool result on session persistence failure.
//! - Return previews / pointers instead of raw large blobs.
//! - Restore context through retrieval, not transcript replay.
//!
//! Module root: per-family tool implementations live in `session`,
//! `saved_context`, `artifact_io`, and `memory`; shared helpers stay here.
//! Tool entry points are re-exported so `crate::session_tools::*` paths stay
//! stable.

mod artifact_io;
mod memory;
mod saved_context;
mod session;

#[cfg(test)]
mod memory_tests;
#[cfg(test)]
mod read_tests;
#[cfg(test)]
mod saved_context_tests;
#[cfg(test)]
mod session_tests;
#[cfg(test)]
mod test_util;

use anyhow::Result;
use atlas_adapters::derive_session_db_path;
use atlas_core::{BudgetPolicy, BudgetReport};
use atlas_session::{SessionId, SessionStore};
use serde::Serialize;
use serde_json::Value;

use crate::output::OutputFormat;
use crate::tool_result::tool_result_value as build_tool_result_value;
use crate::tools::shared::resolve_repo_scope_selection;

pub use artifact_io::{
    tool_cross_session_search, tool_purge_saved_context, tool_read_saved_context,
};
pub use memory::{tool_get_global_memory, tool_memory_recall, tool_memory_store};
pub(crate) use saved_context::{
    decision_hits_json, record_mcp_decision_best_effort, search_decisions_best_effort,
};
pub use saved_context::{
    tool_get_context_stats, tool_save_context_artifact, tool_search_decisions,
    tool_search_saved_context,
};
pub use session::{
    emit_session_event_best_effort, tool_compact_session, tool_get_session_status,
    tool_resume_session,
};

/// Derive the MCP session id for a given repo root.
///
/// Uses `worktree_id = ""` and `frontend = "mcp"` as stable anchors.
fn mcp_session_id(repo_root: &str) -> SessionId {
    SessionId::derive(repo_root, "", "mcp")
}

fn open_session_store_best_effort(db_path: &str) -> Option<SessionStore> {
    let session_db = derive_session_db_path(db_path);
    SessionStore::open(&session_db).ok()
}

fn normalize_repo_roots(mut repo_roots: Vec<String>) -> Vec<String> {
    repo_roots.sort();
    repo_roots.dedup();
    repo_roots.retain(|root| !root.trim().is_empty());
    repo_roots
}

fn resolve_requested_repo_roots(
    tool_name: &str,
    args: Option<&Value>,
    repo_root: &str,
) -> std::result::Result<(Vec<String>, Vec<String>), Box<crate::tool_result::ToolErrorPayload>> {
    let scope = resolve_repo_scope_selection(tool_name, args, repo_root)?;
    let repo_roots = scope
        .selection
        .as_ref()
        .map(|selection| {
            selection
                .registrations
                .iter()
                .map(|entry| entry.root.to_string())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![repo_root.to_owned()]);
    Ok((
        normalize_repo_roots(repo_roots),
        scope.deprecated_input_fields,
    ))
}

fn resolve_session_id(args: Option<&Value>, repo_root: &str) -> SessionId {
    if let Some(sid) = args
        .and_then(|a| a.get("session_id"))
        .and_then(|v| v.as_str())
    {
        SessionId(sid.to_string())
    } else {
        mcp_session_id(repo_root)
    }
}

fn load_budget_policy(repo_root: &str) -> Result<BudgetPolicy> {
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    config.budget_policy()
}

fn inject_budget_metadata(response: &mut Value, budget: &BudgetReport) {
    response["budget_status"] = serde_json::json!(budget.budget_status);
    response["budget_hit"] = serde_json::json!(budget.budget_hit);
    response["budget_name"] = serde_json::json!(&budget.budget_name);
    response["budget_limit"] = serde_json::json!(budget.budget_limit);
    response["budget_observed"] = serde_json::json!(budget.budget_observed);
    response["partial"] = serde_json::json!(budget.partial);
    response["safe_to_answer"] = serde_json::json!(budget.safe_to_answer);
}

/// Wrap structured output in MCP tool-result envelope.
pub(crate) fn tool_result_value<T: Serialize>(
    value: &T,
    output_format: OutputFormat,
) -> Result<Value> {
    build_tool_result_value(value, output_format)
}
