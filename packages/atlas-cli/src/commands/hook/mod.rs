use anyhow::Result;
use serde_json::{Value, json};

use atlas_agent_events::{
    AgentEventRequest, AgentEventResult, AgentEventSource, record_agent_event,
};

use crate::cli::{Cli, Command};

use super::{db_path, print_json};

mod runtime;

#[cfg(test)]
mod tests;

use runtime::{hook_frontend, read_hook_payload, resolve_hook_repo};

pub fn run_hook(cli: &Cli) -> Result<()> {
    let event = match &cli.command {
        Command::Hook { event } => event.as_str(),
        _ => unreachable!(),
    };

    let repo = resolve_hook_repo(cli)?;
    let graph_db_path = db_path(cli, &repo);
    let payload = read_hook_payload()?;
    let frontend = hook_frontend();
    let result = record_agent_event(AgentEventRequest {
        repo_root: repo.clone(),
        graph_db_path,
        frontend,
        event: event.to_owned(),
        session_id: None,
        agent_id: None,
        payload,
        source: AgentEventSource::Hook,
    })?;

    if cli.json {
        print_json("hook", hook_result_json(&repo, result))?;
    }

    Ok(())
}

/// Build the stable `atlas hook --json` output shape.
///
/// Field names and presence are part of the hook output contract; keep this
/// helper in sync with pre-refactor output so hook consumers never break.
pub(crate) fn hook_result_json(repo: &str, result: AgentEventResult) -> Value {
    json!({
        "event": result.event,
        "frontend": result.frontend,
        "repo_root": repo,
        "session_id": result.session_id,
        "pending_resume": result.pending_resume,
        "stored": result.stored,
        "event_id": result.event_id,
        "source_id": result.source_id,
        "storage_kind": result.storage_kind,
        "snapshot": result.snapshot,
        "actions": result.actions,
    })
}
