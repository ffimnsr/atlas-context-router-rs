//! ICM-D2 — `atlas wake-up` command.
//!
//! Builds the bounded session-start recall pack through the shared wake-up
//! builder in `atlas-agent-events` and renders it as JSON or a compact human
//! summary. All list sizes come from `[memory.wake_up]` config (or `--max-items`).

use anyhow::Result;
use serde_json::json;

use atlas_agent_events::{
    AgentEventRequest, AgentEventSource, WakeBudget, WakePackBuild, WakePackOptions,
    build_wake_pack, record_agent_event,
};

use crate::cli::{Cli, Command};

use super::{db_path, print_json, resolve_repo};

pub fn run_wake_up(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;

    let (topic, session, frontend, max_items) = match &cli.command {
        Command::WakeUp {
            topic,
            session,
            frontend,
            max_items,
        } => (topic.clone(), session.clone(), frontend.clone(), *max_items),
        _ => unreachable!(),
    };
    let frontend = frontend.unwrap_or_else(|| "cli".to_owned());

    let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
    let budget = WakeBudget::from_config(&config);
    let budget = match max_items {
        Some(max_items) => budget.with_max_items(max_items),
        None => budget,
    };
    let max_items_value = budget.max_items;

    let build = build_wake_pack(&WakePackOptions {
        repo_root: &repo,
        graph_db_path: &db_path(cli, &repo),
        frontend: &frontend,
        session_id: session.clone(),
        agent_id: None,
        topic: topic.as_deref(),
        budget,
    });

    // Record the session-start event through the shared event service. MCP
    // `wake_up` does the same, so the CLI must not silently skip lifecycle
    // capture or the two surfaces diverge.
    let pack = build.pack.to_json();
    let mut warnings = build.warnings.clone();
    let wake_status = if build.session_status == "unavailable" {
        "degraded"
    } else {
        "ok"
    };
    let event_recorded = match record_agent_event(AgentEventRequest {
        repo_root: repo.clone(),
        graph_db_path: db_path(cli, &repo),
        frontend: frontend.clone(),
        event: "session-start".to_owned(),
        session_id: Some(pack["session_id"].as_str().unwrap_or_default().to_owned()),
        agent_id: None,
        payload: json!({
            "tool": "wake_up",
            "topic": topic,
            "wake_up": { "status": wake_status, "max_items": max_items_value },
        }),
        source: AgentEventSource::CliFallback,
    }) {
        Ok(result) => json!({
            "event": result.canonical_event,
            "stored": result.stored,
            "event_id": result.event_id,
            "pending_resume": result.pending_resume,
        }),
        Err(error) => {
            warnings.push(format!("wake-up session event recording failed: {error}"));
            json!({ "stored": false, "error": error.to_string() })
        }
    };

    if cli.json {
        print_json(
            "wake-up",
            wake_up_json(
                &build,
                &repo,
                &frontend,
                wake_status,
                &event_recorded,
                &warnings,
            ),
        )?;
    } else {
        print_human(&build);
        if !event_recorded["stored"].as_bool().unwrap_or(false) {
            println!("Warning   : session-start event was not recorded");
        }
    }
    Ok(())
}

/// Stable JSON output: the pack plus the same summary envelope the MCP tool
/// emits.
fn wake_up_json(
    build: &WakePackBuild,
    repo: &str,
    frontend: &str,
    wake_status: &str,
    event_recorded: &serde_json::Value,
    warnings: &[String],
) -> serde_json::Value {
    let pack = build.pack.to_json();
    serde_json::json!({
        "tool": "wake-up",
        "repo_root": repo,
        "session_id": pack["session_id"],
        "frontend": frontend,
        "agent_id": serde_json::Value::Null,
        "current_focus": pack["current_focus"],
        "recent_decisions": pack["recent_decisions"],
        "critical_memories": pack["critical_memories"],
        "recent_feedback": pack["recent_feedback"],
        "active_memoir_concepts": pack["active_memoir_concepts"],
        "changed_files": pack["changed_files"],
        "recent_runtime_failures": pack["recent_runtime_failures"],
        "graph_readiness": pack["graph_readiness"],
        "retrieval_hints": pack["retrieval_hints"],
        "generated_at": pack["generated_at"],
        "event_recorded": event_recorded,
        "summary": {
            "status": build.session_status,
            "pending_resume": build.pending_resume,
            "event_count": build.event_count,
            "decision_count": list_len(&pack["recent_decisions"]),
            "critical_memory_count": list_len(&pack["critical_memories"]),
            "feedback_count": list_len(&pack["recent_feedback"]),
            "concept_count": list_len(&pack["active_memoir_concepts"]),
            "changed_file_count": list_len(&pack["changed_files"]),
            "runtime_failure_count": list_len(&pack["recent_runtime_failures"]),
            "retrieval_hint_count": list_len(&pack["retrieval_hints"]),
            "recorded": wake_status,
        },
        "warnings": warnings,
    })
}

fn list_len(value: &serde_json::Value) -> usize {
    value.as_array().map(|items| items.len()).unwrap_or(0)
}

fn print_human(build: &WakePackBuild) {
    let pack = &build.pack;
    println!("Wake-up pack for {} [{}]", pack.repo_root, pack.frontend);
    println!(
        "Session   : {} (status: {}, {} event(s), pending_resume: {})",
        pack.session_id, build.session_status, build.event_count, build.pending_resume
    );
    match &pack.current_focus.intent {
        Some(intent) => println!("Focus     : {intent}"),
        None => println!("Focus     : (none)"),
    }
    println!("Decisions : {}", pack.recent_decisions.len());
    println!(
        "Memories  : {} (critical/topic + artifact refs)",
        pack.critical_memories.len()
    );
    println!("Feedback  : {}", pack.recent_feedback.len());
    println!("Concepts  : {}", pack.active_memoir_concepts.join(", "));
    if !pack.changed_files.is_empty() {
        println!("Changed   : {}", pack.changed_files.join(", "));
    }
    println!(
        "Graph     : {} (built: {}, stale: {})",
        pack.graph_readiness["execution_state"]
            .as_str()
            .unwrap_or("unknown"),
        pack.graph_readiness["graph_built"],
        pack.graph_readiness["stale_index"]
    );
    for warning in &build.warnings {
        println!("Warning   : {warning}");
    }
    if pack.recent_feedback.is_empty() && pack.critical_memories.is_empty() {
        println!(
            "No memories or feedback yet; run atlas memory store and atlas feedback record to seed recall."
        );
    }
}
