use anyhow::Result;
use serde_json::{Value, json};

use atlas_agent_events::{
    AgentEventRequest, AgentEventResult, AgentEventSource, WakeBudget, WakePackOptions,
    build_wake_pack, record_agent_event, resolve_hook_policy,
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
    let mut payload = read_hook_payload()?;
    let frontend = hook_frontend();

    // ICM-D3: on SessionStart, generate the wake-up pack best-effort and store
    // its success/failure metadata in the recorded session event. Generation
    // never blocks or fails the hook: any error degrades to failure metadata.
    let is_session_start = resolve_hook_policy(event)
        .map(|policy| policy.canonical_event == "session-start")
        .unwrap_or(false);
    if is_session_start {
        let wake_meta = generate_wake_up_metadata(&repo, &graph_db_path, &frontend);
        match payload.as_object_mut() {
            Some(object) => {
                object.insert("wake_up".to_owned(), wake_meta);
            }
            None => payload = json!({ "wake_up": wake_meta }),
        }
    }

    // ICM-G9: best-effort overview refresh after a commit hook or explicit
    // manual trigger. Failures never block the host git/hook flow.
    if let Some(overview_meta) = overview_hook_metadata(&repo, &graph_db_path, &payload) {
        match payload.as_object_mut() {
            Some(object) => {
                object.insert("overview".to_owned(), overview_meta);
            }
            None => payload = json!({ "overview": overview_meta }),
        }
    }

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

/// Build the wake-up pack for a SessionStart hook and reduce it to the compact
/// metadata block stored in the session event (bounded injection: counts and
/// status only, never raw artifact bodies).
fn generate_wake_up_metadata(repo: &str, graph_db_path: &str, frontend: &str) -> Value {
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo)).unwrap_or_default();
    let budget = WakeBudget::from_config(&config);
    let build = build_wake_pack(&WakePackOptions {
        repo_root: repo,
        graph_db_path,
        frontend,
        session_id: None,
        agent_id: None,
        topic: None,
        budget,
    });
    json!({
        "status": build.status,
        "session_status": build.session_status,
        "generated_at": build.pack.generated_at,
        "max_items": budget.max_items,
        "counts": {
            "decisions": build.pack.recent_decisions.len(),
            "memories": build.pack.critical_memories.len(),
            "feedback": build.pack.recent_feedback.len(),
            "concepts": build.pack.active_memoir_concepts.len(),
            "changed_files": build.pack.changed_files.len(),
            "retrieval_hints": build.pack.retrieval_hints.len(),
        },
        "error": null,
    })
}

fn overview_hook_metadata(repo: &str, graph_db_path: &str, payload: &Value) -> Option<Value> {
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo)).unwrap_or_default();
    if !config.overview.enabled {
        return None;
    }
    if !overview_triggered(payload) {
        return None;
    }
    let store = match atlas_store_sqlite::Store::open(graph_db_path) {
        Ok(store) => store,
        Err(error) => {
            return Some(json!({ "status": "failed", "error": error.to_string() }));
        }
    };
    let session_db = atlas_engine::paths::session_db_path(graph_db_path);
    let mut session = match atlas_session::SessionStore::open(&session_db) {
        Ok(session) => session,
        Err(error) => {
            return Some(json!({ "status": "failed", "error": error.to_string() }));
        }
    };
    match atlas_review::overview::refresh_overview_plan(
        repo,
        &store,
        &mut session,
        None,
        config.overview.auto_mark_stale_on_commit,
    ) {
        Ok(plan) => Some(json!({
            "status": "ok",
            "apply": plan.apply,
            "changed_files": plan.changed_files.len(),
            "direct_stale": plan.direct_stale.len(),
            "possibly_stale": plan.possibly_stale.len(),
            "orphaned": plan.orphaned.len(),
            "unchanged": plan.unchanged_count,
        })),
        Err(error) => Some(json!({ "status": "failed", "error": error.to_string() })),
    }
}

fn overview_triggered(payload: &Value) -> bool {
    if payload
        .get("overview")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return true;
    }
    payload
        .get("command")
        .and_then(Value::as_str)
        .is_some_and(|command| command.contains("git commit") || command.contains("git merge"))
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
