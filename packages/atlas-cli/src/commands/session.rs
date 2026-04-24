use anyhow::{Context, Result};

use atlas_session::{GlobalAccessEntry, GlobalWorkflowPattern};

use crate::cli::{Cli, Command, SessionCommand};

use super::{print_json, resolve_repo};

pub fn run_session(cli: &Cli) -> Result<()> {
    use atlas_session::{SessionId, SessionStore};
    use std::path::Path;

    let repo = resolve_repo(cli)?;

    let sub = match &cli.command {
        Command::Session { subcommand } => subcommand,
        _ => unreachable!(),
    };

    // Derive the stable session id for this repo + CLI frontend.
    let session_id = SessionId::derive(&repo, "", "cli");

    // Open (or create) the session store; most sub-commands need it.
    let open_store = || -> Result<SessionStore> {
        SessionStore::open_in_repo(Path::new(&repo))
            .with_context(|| format!("cannot open session store in {repo}"))
    };

    match sub {
        SessionCommand::Start => {
            let mut store = open_store()?;
            store
                .upsert_session_meta(session_id.clone(), &repo, "cli", None)
                .context("cannot register session")?;

            // Check whether a pending (unconsumed) resume snapshot exists.
            let pending = store.get_resume_snapshot(&session_id)?;
            let has_resume = pending.as_ref().map(|s| !s.consumed).unwrap_or(false);

            if cli.json {
                print_json(
                    "session.start",
                    serde_json::json!({
                        "session_id": session_id.as_str(),
                        "repo_root": repo,
                        "has_resume": has_resume,
                    }),
                )?;
            } else {
                println!("Session started: {}", session_id.as_str());
                println!("Repo           : {repo}");
                if has_resume {
                    println!("Resume snapshot: available (run `atlas session resume` to load)");
                }
            }
        }

        SessionCommand::Status => {
            let store = open_store()?;
            let meta = store.get_session_meta(&session_id)?;
            let events = store.list_events(&session_id)?;
            let snapshot = store.get_resume_snapshot(&session_id)?;

            // CM11: best-effort global memory; empty if no data yet.
            let frequent_symbols: Vec<GlobalAccessEntry> =
                store.get_frequent_symbols(&repo, 10).unwrap_or_default();
            let frequent_files: Vec<GlobalAccessEntry> =
                store.get_frequent_files(&repo, 10).unwrap_or_default();
            let recurring_workflows: Vec<GlobalWorkflowPattern> =
                store.get_recurring_workflows(&repo, 5).unwrap_or_default();

            if cli.json {
                let snapshot_info = snapshot.as_ref().map(|s| {
                    serde_json::json!({
                        "event_count": s.event_count,
                        "consumed": s.consumed,
                        "created_at": s.created_at,
                        "updated_at": s.updated_at,
                    })
                });
                let symbols_json: Vec<_> = frequent_symbols
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "value": e.value,
                            "access_count": e.access_count,
                            "last_accessed": e.last_accessed,
                        })
                    })
                    .collect();
                let files_json: Vec<_> = frequent_files
                    .iter()
                    .map(|e| {
                        serde_json::json!({
                            "value": e.value,
                            "access_count": e.access_count,
                            "last_accessed": e.last_accessed,
                        })
                    })
                    .collect();
                let workflows_json: Vec<_> = recurring_workflows
                    .iter()
                    .map(|w| {
                        serde_json::json!({
                            "pattern": w.pattern,
                            "occurrence_count": w.occurrence_count,
                            "last_seen": w.last_seen,
                        })
                    })
                    .collect();
                print_json(
                    "session.status",
                    serde_json::json!({
                        "session_id": session_id.as_str(),
                        "exists": meta.is_some(),
                        "meta": meta.as_ref().map(|m| serde_json::json!({
                            "repo_root": m.repo_root,
                            "frontend": m.frontend,
                            "worktree_id": m.worktree_id,
                            "created_at": m.created_at,
                            "updated_at": m.updated_at,
                            "last_resume_at": m.last_resume_at,
                            "last_compaction_at": m.last_compaction_at,
                        })),
                        "event_count": events.len(),
                        "resume_snapshot": snapshot_info,
                        "global_memory": {
                            "frequent_symbols": symbols_json,
                            "frequent_files": files_json,
                            "recurring_workflows": workflows_json,
                        },
                    }),
                )?;
            } else {
                match &meta {
                    None => println!("No active session (run `atlas session start`)"),
                    Some(m) => {
                        println!("Session   : {}", session_id.as_str());
                        println!("Repo      : {}", m.repo_root);
                        println!("Frontend  : {}", m.frontend);
                        println!("Created   : {}", m.created_at);
                        println!("Updated   : {}", m.updated_at);
                        println!("Events    : {}", events.len());
                        if let Some(ca) = &m.last_compaction_at {
                            println!("Compacted : {ca}");
                        }
                        match &snapshot {
                            None => println!("Snapshot  : none"),
                            Some(s) => {
                                let state = if s.consumed { "consumed" } else { "pending" };
                                println!("Snapshot  : {state} ({} events)", s.event_count);
                            }
                        }
                        // CM11: show global memory summary if data exists.
                        if !frequent_symbols.is_empty() {
                            println!("\nFrequent symbols ({}):", frequent_symbols.len());
                            for e in &frequent_symbols {
                                println!("  {} (accessed {} times)", e.value, e.access_count);
                            }
                        }
                        if !frequent_files.is_empty() {
                            println!("\nFrequent files ({}):", frequent_files.len());
                            for e in &frequent_files {
                                println!("  {} (accessed {} times)", e.value, e.access_count);
                            }
                        }
                        if !recurring_workflows.is_empty() {
                            println!("\nRecurring workflows ({}):", recurring_workflows.len());
                            for w in &recurring_workflows {
                                let pat = w.pattern.join(" → ");
                                println!("  [{pat}] ({} times)", w.occurrence_count);
                            }
                        }
                    }
                }
            }
        }

        SessionCommand::Resume => {
            let mut store = open_store()?;
            let snapshot = store.get_resume_snapshot(&session_id)?;
            match snapshot {
                None => {
                    if cli.json {
                        print_json(
                            "session.resume",
                            serde_json::json!({
                                "session_id": session_id.as_str(),
                                "snapshot": null,
                            }),
                        )?;
                    } else {
                        println!("No resume snapshot for session {}", session_id.as_str());
                        println!("Build one with `atlas session start` after active work.");
                    }
                }
                Some(s) => {
                    // Mark consumed so next `session start` knows it was loaded.
                    store.mark_resume_consumed(&session_id, true)?;

                    let inner: serde_json::Value =
                        serde_json::from_str(&s.snapshot).unwrap_or(serde_json::Value::Null);

                    if cli.json {
                        print_json(
                            "session.resume",
                            serde_json::json!({
                                "session_id": session_id.as_str(),
                                "consumed": true,
                                "snapshot": inner,
                            }),
                        )?;
                    } else {
                        println!("=== Resume snapshot: {} ===", session_id.as_str());
                        println!("{}", serde_json::to_string_pretty(&inner)?);
                        println!("(snapshot marked consumed)");
                    }
                }
            }
        }

        SessionCommand::Clear => {
            let mut store = open_store()?;
            let deleted = store.delete_session(&session_id)?;
            if cli.json {
                print_json(
                    "session.clear",
                    serde_json::json!({
                        "session_id": session_id.as_str(),
                        "deleted": deleted,
                    }),
                )?;
            } else if deleted {
                println!("Session {} cleared.", session_id.as_str());
            } else {
                println!("No session found for this repo.");
            }
        }

        SessionCommand::List => {
            let store = open_store()?;
            let sessions = store.list_sessions()?;
            if cli.json {
                let rows: Vec<serde_json::Value> = sessions
                    .iter()
                    .map(|m| {
                        serde_json::json!({
                            "session_id": m.session_id.as_str(),
                            "repo_root": m.repo_root,
                            "frontend": m.frontend,
                            "worktree_id": m.worktree_id,
                            "created_at": m.created_at,
                            "updated_at": m.updated_at,
                            "last_resume_at": m.last_resume_at,
                        })
                    })
                    .collect();
                print_json("session.list", serde_json::json!({ "sessions": rows }))?;
            } else if sessions.is_empty() {
                println!("No sessions.");
            } else {
                println!(
                    "{:<20} {:<12} {:<14} REPO",
                    "UPDATED", "FRONTEND", "SESSION_ID"
                );
                for s in &sessions {
                    let updated = s.updated_at.get(..19).unwrap_or(s.updated_at.as_str());
                    let id_short = s
                        .session_id
                        .as_str()
                        .get(..12)
                        .unwrap_or(s.session_id.as_str());
                    println!(
                        "{:<20} {:<12} {:<14} {}",
                        updated, s.frontend, id_short, s.repo_root
                    );
                }
            }
        }
        SessionCommand::Decisions {
            query,
            current_session,
            limit,
        } => {
            let store = open_store()?;
            let session_filter = current_session.then_some(session_id.as_str());
            let hits = store.search_decisions(&repo, query, session_filter, *limit)?;

            if cli.json {
                print_json(
                    "session.decisions",
                    serde_json::json!({
                        "query": query,
                        "session_id": session_filter,
                        "results": hits,
                        "total": hits.len(),
                    }),
                )?;
            } else if hits.is_empty() {
                println!("No prior decisions matched '{query}'.");
            } else {
                println!("Decision matches ({}):", hits.len());
                for hit in &hits {
                    let conclusion = hit
                        .decision
                        .conclusion
                        .as_deref()
                        .unwrap_or("no conclusion recorded");
                    println!(
                        "  [{:.1}] {} -> {}",
                        hit.relevance_score, hit.decision.summary, conclusion
                    );
                    if let Some(rationale) = hit.decision.rationale.as_deref() {
                        println!("    rationale: {rationale}");
                    }
                    if !hit.decision.source_ids.is_empty() {
                        println!("    artifacts: {}", hit.decision.source_ids.join(", "));
                    }
                }
            }
        }
        SessionCommand::Compact => {
            let mut store = open_store()?;
            let result = store
                .compact_session(&session_id)
                .with_context(|| "cannot compact session events")?;

            if cli.json {
                print_json(
                    "session.compact",
                    serde_json::json!({
                        "session_id": session_id.as_str(),
                        "events_before": result.events_before,
                        "events_after": result.events_after,
                        "merged": result.merged_count,
                        "decayed": result.decayed_count,
                        "deduplicated": result.deduplicated_count,
                        "promoted": result.promoted_count,
                    }),
                )?;
            } else if result.events_before == result.events_after && result.promoted_count == 0 {
                println!("Session already compact ({} events).", result.events_before);
            } else {
                println!(
                    "Session compacted: {} → {} events",
                    result.events_before, result.events_after
                );
                if result.decayed_count > 0 {
                    println!("  Decayed    : {}", result.decayed_count);
                }
                if result.merged_count > 0 {
                    println!("  Merged     : {}", result.merged_count);
                }
                if result.deduplicated_count > 0 {
                    println!("  Deduplicated: {}", result.deduplicated_count);
                }
                if result.promoted_count > 0 {
                    println!("  Promoted   : {}", result.promoted_count);
                }
            }
        }
    }

    Ok(())
}
