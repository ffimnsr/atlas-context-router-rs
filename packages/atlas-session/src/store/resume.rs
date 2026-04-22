use atlas_core::{AtlasError, Result};

use crate::SessionId;

use super::{ResumeSnapshot, SessionEventType, SessionStore};

pub(super) fn build_resume_snapshot(
    store: &mut SessionStore,
    session_id: &SessionId,
) -> Result<ResumeSnapshot> {
    let meta = store
        .get_session_meta(session_id)?
        .ok_or_else(|| AtlasError::Other(format!("session {} not found", session_id)))?;

    let events = store.list_events(session_id)?;
    let event_count = events.len() as i64;

    let mut last_user_intent: Option<String> = None;
    let mut recent_commands: Vec<serde_json::Value> = Vec::new();
    let mut changed_files: Vec<String> = Vec::new();
    let mut impacted_symbols: Vec<String> = Vec::new();
    let mut unresolved_errors: Vec<serde_json::Value> = Vec::new();
    let mut recent_reasoning: Vec<serde_json::Value> = Vec::new();
    let mut saved_artifact_refs: Vec<String> = Vec::new();
    let mut recent_decisions: Vec<serde_json::Value> = Vec::new();
    let mut active_rules: Vec<serde_json::Value> = Vec::new();
    let mut graph_state: Option<serde_json::Value> = None;
    let mut retrieval_hints: Vec<serde_json::Value> = Vec::new();

    for event in &events {
        let payload: serde_json::Value =
            serde_json::from_str(&event.payload_json).unwrap_or(serde_json::Value::Null);

        match event.event_type {
            SessionEventType::UserIntent => {
                if let Some(intent) = payload.get("intent").and_then(|v| v.as_str()) {
                    last_user_intent = Some(intent.to_string());
                }
            }
            SessionEventType::CommandRun => {
                let entry = serde_json::json!({
                    "command": payload.get("command"),
                    "status": payload.get("status"),
                    "at": event.created_at,
                });
                recent_commands.push(entry);
                if recent_commands.len() > 10 {
                    recent_commands.remove(0);
                }
            }
            SessionEventType::CommandFail | SessionEventType::Error => {
                let entry = serde_json::json!({
                    "command": payload.get("command").or_else(|| payload.get("tool")),
                    "error": payload.get("error").or_else(|| payload.get("extra")),
                    "at": event.created_at,
                });
                unresolved_errors.push(entry);
                if unresolved_errors.len() > 5 {
                    unresolved_errors.remove(0);
                }
            }
            SessionEventType::ReviewContext | SessionEventType::ImpactAnalysis => {
                if let Some(files) = payload.get("files").and_then(|v| v.as_array()) {
                    for f in files {
                        if let Some(s) = f.as_str() {
                            let owned = s.to_string();
                            if !changed_files.contains(&owned) {
                                changed_files.push(owned);
                            }
                        }
                    }
                    if changed_files.len() > 20 {
                        changed_files.truncate(20);
                    }
                }
                if let Some(syms) = payload.get("impacted_symbols").and_then(|v| v.as_array()) {
                    for sym in syms {
                        if let Some(s) = sym.as_str() {
                            let owned = s.to_string();
                            if !impacted_symbols.contains(&owned) {
                                impacted_symbols.push(owned);
                            }
                        }
                    }
                    if impacted_symbols.len() > 30 {
                        impacted_symbols.truncate(30);
                    }
                }
            }
            SessionEventType::ReasoningResult => {
                if let Some(src) = payload.get("source_id").and_then(|v| v.as_str()) {
                    let owned = src.to_string();
                    if !saved_artifact_refs.contains(&owned) {
                        saved_artifact_refs.push(owned.clone());
                    }
                    retrieval_hints.push(serde_json::json!({
                        "kind": "saved_artifact",
                        "source_id": owned,
                        "summary": payload.get("summary"),
                    }));
                }
                let entry = serde_json::json!({
                    "summary": payload.get("summary"),
                    "source_id": payload.get("source_id"),
                    "at": event.created_at,
                });
                recent_reasoning.push(entry);
                if recent_reasoning.len() > 5 {
                    recent_reasoning.remove(0);
                }
            }
            SessionEventType::ContextRequest => {
                if let Some(qh) = payload.get("query_hint").and_then(|v| v.as_str()) {
                    retrieval_hints.push(serde_json::json!({
                        "kind": "context_query",
                        "query": qh,
                    }));
                    if retrieval_hints.len() > 15 {
                        retrieval_hints.remove(0);
                    }
                }
            }
            SessionEventType::Decision => {
                let entry = serde_json::json!({
                    "summary": payload.get("summary"),
                    "rationale": payload.get("rationale"),
                    "at": event.created_at,
                });
                recent_decisions.push(entry);
                if recent_decisions.len() > 10 {
                    recent_decisions.remove(0);
                }
            }
            SessionEventType::RuleInstruction => {
                let label = payload
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let entry = serde_json::json!({
                    "label": label,
                    "rule": payload.get("rule"),
                    "source": payload.get("source"),
                    "at": event.created_at,
                });
                if let Some(pos) = active_rules
                    .iter()
                    .position(|r| r.get("label").and_then(|v| v.as_str()) == Some(label.as_str()))
                {
                    active_rules[pos] = entry;
                } else {
                    active_rules.push(entry);
                    if active_rules.len() > 10 {
                        active_rules.remove(0);
                    }
                }
            }
            SessionEventType::GraphBuild | SessionEventType::GraphUpdate => {
                graph_state = Some(payload.clone());
            }
            _ => {}
        }
    }

    let snapshot = serde_json::json!({
        "session_id": session_id.as_str(),
        "repo_root": meta.repo_root,
        "worktree_id": meta.worktree_id,
        "frontend": meta.frontend,
        "session_created_at": meta.created_at,
        "last_user_intent": last_user_intent,
        "recent_commands": recent_commands,
        "changed_files": changed_files,
        "impacted_symbols": impacted_symbols,
        "unresolved_errors": unresolved_errors,
        "recent_reasoning": recent_reasoning,
        "saved_artifact_refs": saved_artifact_refs,
        "recent_decisions": recent_decisions,
        "active_rules": active_rules,
        "graph_state": graph_state,
        "retrieval_hints": retrieval_hints,
        "event_count": event_count,
    });

    let snapshot_str = serde_json::to_string(&snapshot)
        .map_err(|e| AtlasError::Other(format!("cannot serialise resume snapshot: {e}")))?;

    let snapshot_str = if snapshot_str.len() > store.config.max_snapshot_bytes {
        trim_snapshot_to_limit(snapshot, store.config.max_snapshot_bytes)?
    } else {
        snapshot_str
    };

    store.put_resume_snapshot(session_id, &snapshot_str, event_count, false)?;

    store
        .get_resume_snapshot(session_id)?
        .ok_or_else(|| AtlasError::Other("resume snapshot was not persisted".to_string()))
}

fn trim_snapshot_to_limit(mut snapshot: serde_json::Value, max_bytes: usize) -> Result<String> {
    let trimable_arrays = [
        "impacted_symbols",
        "changed_files",
        "retrieval_hints",
        "recent_commands",
        "unresolved_errors",
        "recent_reasoning",
        "recent_decisions",
        "active_rules",
        "saved_artifact_refs",
    ];

    for key in &trimable_arrays {
        let serialised = serde_json::to_string(&snapshot)
            .map_err(|e| AtlasError::Other(format!("cannot serialise snapshot: {e}")))?;
        if serialised.len() <= max_bytes {
            return Ok(serialised);
        }
        if let Some(arr) = snapshot.get(key).and_then(|v| v.as_array()).cloned() {
            let mut arr = arr;
            while arr.len() > 1 {
                arr.truncate(arr.len() / 2);
                if let Some(obj) = snapshot.as_object_mut() {
                    obj.insert((*key).to_string(), serde_json::Value::Array(arr.clone()));
                }
                let s = serde_json::to_string(&snapshot).unwrap_or_default();
                if s.len() <= max_bytes {
                    return Ok(s);
                }
            }
            if let Some(obj) = snapshot.as_object_mut() {
                obj.insert((*key).to_string(), serde_json::Value::Array(vec![]));
            }
        }
    }

    snapshot.as_object_mut().map(|m| m.remove("graph_state"));

    let serialised = serde_json::to_string(&snapshot)
        .map_err(|e| AtlasError::Other(format!("cannot serialise snapshot: {e}")))?;
    if serialised.len() > max_bytes {
        return Err(AtlasError::Other(format!(
            "resume snapshot {} bytes exceeds limit {} bytes even after trimming",
            serialised.len(),
            max_bytes
        )));
    }
    Ok(serialised)
}
