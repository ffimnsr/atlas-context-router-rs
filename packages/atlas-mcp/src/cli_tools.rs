//! Stable non-transport entry points for `atlas-cli` parity commands.
//!
//! These wrappers call the exact tool implementations the MCP server
//! dispatches, so a CLI command and its MCP counterpart cannot drift into
//! different argument contracts or payload shapes.

use anyhow::Result;
use serde_json::Value;

use crate::output::OutputFormat;

fn json_format() -> OutputFormat {
    OutputFormat::Json
}

/// `search_files` — file-path discovery by glob pattern.
pub fn search_files(args: &Value, repo_root: &str) -> Result<Value> {
    crate::discovery::tool_search_files(Some(args), repo_root, json_format())
}

/// `search_content` — content search by literal string or regex.
pub fn search_content(args: &Value, repo_root: &str) -> Result<Value> {
    crate::discovery::tool_search_content(Some(args), repo_root, json_format())
}

/// `read_file_excerpt` — bounded file reads by line range or context.
pub fn read_file_excerpt(args: &Value, repo_root: &str) -> Result<Value> {
    crate::discovery::tool_read_file_excerpt(Some(args), repo_root, json_format())
}

/// `read_file_around_match` — grouped snippets around one file's matches.
pub fn read_file_around_match(args: &Value, repo_root: &str) -> Result<Value> {
    crate::discovery::tool_read_file_around_match(Some(args), repo_root, json_format())
}

/// `search_templates` — template file discovery by engine kind.
pub fn search_templates(args: &Value, repo_root: &str) -> Result<Value> {
    crate::discovery::tool_search_templates(Some(args), repo_root, json_format())
}

/// `search_text_assets` — SQL, config, env, and prompt file discovery.
pub fn search_text_assets(args: &Value, repo_root: &str) -> Result<Value> {
    crate::discovery::tool_search_text_assets(Some(args), repo_root, json_format())
}

/// `symbol_neighbors` — immediate callers, callees, tests, and siblings.
pub fn symbol_neighbors(args: &Value, repo_root: &str, db_path: &str) -> Result<Value> {
    crate::tools::tool_symbol_neighbors(Some(args), repo_root, db_path, json_format())
}

/// `cross_file_links` — files coupled through shared symbol references.
pub fn cross_file_links(args: &Value, repo_root: &str, db_path: &str) -> Result<Value> {
    crate::tools::tool_cross_file_links(Some(args), repo_root, db_path, json_format())
}

/// `concept_clusters` — related file groups around seed files.
pub fn concept_clusters(args: &Value, repo_root: &str, db_path: &str) -> Result<Value> {
    crate::tools::tool_concept_clusters(Some(args), repo_root, db_path, json_format())
}

// ---------------------------------------------------------------------------
// Session continuity / saved-context surfaces
// ---------------------------------------------------------------------------

/// `save_context_artifact` — persist a large payload and return its pointer.
pub fn save_context_artifact(args: &Value, repo_root: &str, db_path: &str) -> Result<Value> {
    crate::session_tools::tool_save_context_artifact(Some(args), repo_root, db_path, json_format())
}

/// `read_saved_context` — read a saved artifact by `source_id`.
pub fn read_saved_context(args: &Value, repo_root: &str, db_path: &str) -> Result<Value> {
    crate::session_tools::tool_read_saved_context(Some(args), repo_root, db_path, json_format())
}

/// `search_saved_context` — search saved artifacts.
pub fn search_saved_context(args: &Value, repo_root: &str, db_path: &str) -> Result<Value> {
    crate::session_tools::tool_search_saved_context(Some(args), repo_root, db_path, json_format())
}

/// `get_context_stats` — saved-context storage footprint for a session.
pub fn get_context_stats(args: &Value, repo_root: &str, db_path: &str) -> Result<Value> {
    crate::session_tools::tool_get_context_stats(Some(args), repo_root, db_path, json_format())
}

/// `compact_session` — curate the session event ledger.
pub fn compact_session(args: &Value, repo_root: &str, db_path: &str) -> Result<Value> {
    crate::session_tools::tool_compact_session(Some(args), repo_root, db_path, json_format())
}

/// `get_global_memory` — cross-session global memory summary.
pub fn get_global_memory(args: &Value, repo_root: &str, db_path: &str) -> Result<Value> {
    crate::session_tools::tool_get_global_memory(Some(args), repo_root, db_path, json_format())
}

/// `search_decisions` — decision memory search.
pub fn search_decisions(args: &Value, repo_root: &str, db_path: &str) -> Result<Value> {
    crate::session_tools::tool_search_decisions(Some(args), repo_root, db_path, json_format())
}
