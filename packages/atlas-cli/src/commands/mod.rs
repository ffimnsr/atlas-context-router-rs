mod changes;
mod context_cmd;
mod graph;
mod graph_objects;
mod init_wizard;
mod maintenance;
mod platform;
mod query;
mod reasoning;
mod session;

pub use changes::{run_detect_changes, run_explain_change, run_impact, run_review_context};
pub use context_cmd::{run_context, run_shell};
pub use graph::{run_build, run_init, run_status, run_update, run_watch};
pub use graph_objects::{run_communities, run_flows};
pub use maintenance::{run_db_check, run_debug_graph, run_doctor};
pub use platform::{run_completions, run_install, run_serve};
pub use query::{run_embed, run_explain_query, run_query};
pub use reasoning::{run_analyze, run_refactor};
pub use session::run_session;

use std::io;
use std::io::IsTerminal;

use anyhow::{Context, Result};
use atlas_core::model::ChangeType;
use atlas_repo::DiffTarget;
use atlas_store_sqlite::Store;

use crate::cli::Cli;

pub(crate) const MACHINE_SCHEMA_VERSION: &str = "atlas_cli.v1";

pub(crate) fn json_envelope(command: &str, data: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "schema_version": MACHINE_SCHEMA_VERSION,
        "command": command,
        "data": data,
    })
}

pub(crate) fn print_json(command: &str, data: serde_json::Value) -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&json_envelope(command, data))?
    );
    Ok(())
}

pub(crate) fn detect_changes_target(base: &Option<String>, staged: bool) -> DiffTarget {
    if staged {
        DiffTarget::Staged
    } else if let Some(base_ref) = base {
        DiffTarget::BaseRef(base_ref.clone())
    } else {
        DiffTarget::WorkingTree
    }
}

pub(crate) fn change_tag(change_type: ChangeType) -> &'static str {
    match change_type {
        ChangeType::Added => "A",
        ChangeType::Modified => "M",
        ChangeType::Deleted => "D",
        ChangeType::Renamed => "R",
        ChangeType::Copied => "C",
    }
}

pub(crate) fn augment_changes_with_node_counts(
    changes: &[atlas_core::model::ChangedFile],
    store: Option<&Store>,
) -> Vec<serde_json::Value> {
    changes
        .iter()
        .map(|cf| {
            let node_count = store
                .and_then(|s| s.nodes_by_file(&cf.path).ok())
                .map(|ns| ns.len());
            let mut value = serde_json::to_value(cf).unwrap_or_default();
            if let Some(count) = node_count {
                value["node_count"] = serde_json::json!(count);
            }
            value
        })
        .collect()
}

pub(crate) fn colorize(text: &str, ansi: &str) -> String {
    if io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none() {
        format!("\x1b[{ansi}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

pub(crate) fn query_display_path(node: &atlas_core::Node) -> String {
    node.extra_json
        .as_object()
        .and_then(|extra| {
            extra
                .get("owner_manifest_path")
                .or_else(|| extra.get("workspace_manifest_path"))
        })
        .and_then(|value| value.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| node.file_path.clone())
}

pub(crate) fn resolve_repo(cli: &Cli) -> Result<String> {
    if let Some(r) = &cli.repo {
        return Ok(r.clone());
    }
    Ok(std::env::current_dir()
        .context("cannot determine cwd")?
        .to_string_lossy()
        .into_owned())
}

pub(crate) fn db_path(cli: &Cli, repo: &str) -> String {
    if let Some(p) = &cli.db {
        return p.clone();
    }
    atlas_engine::paths::default_db_path(repo)
}
