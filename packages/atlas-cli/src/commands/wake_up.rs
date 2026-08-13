//! ICM-D2 — `atlas wake-up` command.
//!
//! Builds the bounded session-start recall pack through the shared wake-up
//! builder in `atlas-agent-events` and renders it as JSON or a compact human
//! summary. All list sizes come from `[memory.wake_up]` config (or `--max-items`).

use anyhow::Result;

use atlas_agent_events::{WakeBudget, WakePackBuild, WakePackOptions, build_wake_pack};

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

    let build = build_wake_pack(&WakePackOptions {
        repo_root: &repo,
        graph_db_path: &db_path(cli, &repo),
        frontend: &frontend,
        session_id: session,
        agent_id: None,
        topic: topic.as_deref(),
        budget,
    });

    if cli.json {
        print_json("wake-up", wake_up_json(&build, &repo, &frontend))?;
    } else {
        print_human(&build);
    }
    Ok(())
}

/// Stable JSON output: the pack plus the same summary envelope the MCP tool
/// emits (minus event recording, which stays on the hook/MCP surfaces).
fn wake_up_json(build: &WakePackBuild, repo: &str, frontend: &str) -> serde_json::Value {
    let pack = build.pack.to_json();
    serde_json::json!({
        "tool": "wake-up",
        "repo_root": repo,
        "session_id": pack["session_id"],
        "frontend": frontend,
        "current_focus": pack["current_focus"],
        "recent_decisions": pack["recent_decisions"],
        "critical_memories": pack["critical_memories"],
        "recent_feedback": pack["recent_feedback"],
        "active_memoir_concepts": pack["active_memoir_concepts"],
        "changed_files": pack["changed_files"],
        "graph_readiness": pack["graph_readiness"],
        "retrieval_hints": pack["retrieval_hints"],
        "generated_at": pack["generated_at"],
        "summary": {
            "status": build.session_status,
            "pending_resume": build.pending_resume,
            "event_count": build.event_count,
            "decision_count": list_len(&pack["recent_decisions"]),
            "critical_memory_count": list_len(&pack["critical_memories"]),
            "feedback_count": list_len(&pack["recent_feedback"]),
            "concept_count": list_len(&pack["active_memoir_concepts"]),
            "changed_file_count": list_len(&pack["changed_files"]),
            "retrieval_hint_count": list_len(&pack["retrieval_hints"]),
        },
        "warnings": build.warnings,
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
