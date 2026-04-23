use anyhow::Result;
use serde_json::json;

use crate::cli::{Cli, Command};

use super::{db_path, print_json};

mod actions;
mod metadata;
mod payload;
mod policy;
mod runtime;

#[cfg(test)]
mod tests;

use actions::execute_hook_actions;
use policy::resolve_hook_policy;
use runtime::{hook_frontend, persist_hook_event, read_hook_payload, resolve_hook_repo};

pub fn run_hook(cli: &Cli) -> Result<()> {
    let event = match &cli.command {
        Command::Hook { event } => event.as_str(),
        _ => unreachable!(),
    };

    let repo = resolve_hook_repo(cli)?;
    let graph_db_path = db_path(cli, &repo);
    let payload = read_hook_payload()?;
    let frontend = hook_frontend();
    let policy = resolve_hook_policy(event)?;
    let persisted = persist_hook_event(&repo, &graph_db_path, &frontend, event, payload.clone())?;
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        &frontend,
        policy,
        &persisted,
        &payload,
    );

    if cli.json {
        print_json(
            "hook",
            json!({
                "event": event,
                "frontend": frontend,
                "repo_root": repo,
                "session_id": persisted.session_id.as_str(),
                "pending_resume": persisted.pending_resume,
                "stored": persisted.stored_event_id.is_some(),
                "event_id": persisted.stored_event_id,
                "source_id": persisted.source_id,
                "storage_kind": persisted.storage_kind,
                "snapshot": persisted.snapshot,
                "actions": actions,
            }),
        )?;
    }

    Ok(())
}
