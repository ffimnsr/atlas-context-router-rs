use atlas_core::{AtlasError, Result};

use crate::SessionId;

use super::util::normalize_repo_path_string;
use super::{ResumeSnapshot, SessionEventRow, SessionEventType, SessionStore};

fn push_unique_string(values: &mut Vec<String>, candidate: String, limit: usize) {
    if values.iter().any(|existing| existing == &candidate) {
        return;
    }
    values.push(candidate);
    if values.len() > limit {
        values.remove(0);
    }
}

fn push_unique_value(
    values: &mut Vec<serde_json::Value>,
    candidate: serde_json::Value,
    limit: usize,
) {
    if values.iter().any(|existing| existing == &candidate) {
        return;
    }
    values.push(candidate);
    if values.len() > limit {
        values.remove(0);
    }
}

fn collect_hook_saved_artifact_refs(
    payload: &serde_json::Value,
    saved_artifact_refs: &mut Vec<String>,
) {
    for key in ["source_id", "saved_artifact_refs"] {
        match payload.get(key) {
            Some(serde_json::Value::String(source_id)) => {
                push_unique_string(saved_artifact_refs, source_id.clone(), 30);
            }
            Some(serde_json::Value::Array(values)) => {
                for value in values {
                    if let Some(source_id) = value.as_str() {
                        push_unique_string(saved_artifact_refs, source_id.to_owned(), 30);
                    }
                }
            }
            _ => {}
        }
    }
}

fn collect_hook_retrieval_hints(
    payload: &serde_json::Value,
    retrieval_hints: &mut Vec<serde_json::Value>,
) {
    if let Some(serde_json::Value::Array(values)) = payload.get("retrieval_hints") {
        for value in values {
            push_unique_value(retrieval_hints, value.clone(), 15);
        }
    }
}

pub(super) fn build_resume_snapshot(
    store: &mut SessionStore,
    session_id: &SessionId,
) -> Result<ResumeSnapshot> {
    let events = store.list_events(session_id)?;
    let event_count = events.len() as i64;
    let snapshot_str = build_resume_snapshot_json(store, session_id, events, None, false)?;

    store.put_resume_snapshot(session_id, &snapshot_str, event_count, false)?;

    store
        .get_resume_snapshot(session_id)?
        .ok_or_else(|| AtlasError::Other("resume snapshot was not persisted".to_string()))
}

pub(super) fn build_resume_snapshot_view(
    store: &SessionStore,
    session_id: &SessionId,
    events: Vec<SessionEventRow>,
    agent_id: Option<&str>,
    merge_agent_partitions: bool,
) -> Result<serde_json::Value> {
    let snapshot =
        build_resume_snapshot_json(store, session_id, events, agent_id, merge_agent_partitions)?;
    serde_json::from_str(&snapshot)
        .map_err(|error| AtlasError::Other(format!("cannot parse resume snapshot view: {error}")))
}

fn build_resume_snapshot_json(
    store: &SessionStore,
    session_id: &SessionId,
    events: Vec<SessionEventRow>,
    agent_id: Option<&str>,
    merge_agent_partitions: bool,
) -> Result<String> {
    let meta = store
        .get_session_meta(session_id)?
        .ok_or_else(|| AtlasError::Other(format!("session {} not found", session_id)))?;

    let agent_summary = store.summarize_agent_memory_from_events(
        events.clone(),
        agent_id,
        merge_agent_partitions,
    )?;
    let filtered_events = filter_events_for_scope(events, agent_id, merge_agent_partitions);
    let event_count = filtered_events.len() as i64;

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

    for event in &filtered_events {
        let payload: serde_json::Value =
            serde_json::from_str(&event.payload_json).unwrap_or(serde_json::Value::Null);

        collect_hook_saved_artifact_refs(&payload, &mut saved_artifact_refs);
        if let Some(hook_metadata) = payload.get("hook_metadata") {
            collect_hook_saved_artifact_refs(hook_metadata, &mut saved_artifact_refs);
            collect_hook_retrieval_hints(hook_metadata, &mut retrieval_hints);
        }
        collect_hook_retrieval_hints(&payload, &mut retrieval_hints);

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
                            let owned = normalize_repo_path_string(&meta.repo_root, s)
                                .unwrap_or_else(|| s.to_string());
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
        "agent_id": agent_id,
        "merged_agent_view": merge_agent_partitions || agent_id.is_none(),
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
        "agent_partitions": agent_summary.partitions,
        "delegated_tasks": agent_summary.delegated_tasks,
        "agent_responsibilities": agent_summary.responsibilities,
        "event_count": event_count,
    });

    let snapshot_str = serde_json::to_string(&snapshot)
        .map_err(|e| AtlasError::Other(format!("cannot serialise resume snapshot: {e}")))?;

    let snapshot_str = if snapshot_str.len() > store.config.max_snapshot_bytes {
        trim_snapshot_to_limit(snapshot, store.config.max_snapshot_bytes)?
    } else {
        snapshot_str
    };

    Ok(snapshot_str)
}

fn filter_events_for_scope(
    events: Vec<SessionEventRow>,
    agent_id: Option<&str>,
    merge_agent_partitions: bool,
) -> Vec<SessionEventRow> {
    if merge_agent_partitions || agent_id.is_none() {
        return events;
    }
    events
        .into_iter()
        .filter(|event| {
            let payload: serde_json::Value =
                serde_json::from_str(&event.payload_json).unwrap_or(serde_json::Value::Null);
            payload_agent_id(&payload).as_deref() == agent_id
        })
        .collect()
}

fn payload_agent_id(payload: &serde_json::Value) -> Option<String> {
    for key in [
        "agent_id",
        "agentId",
        "agent_name",
        "agentName",
        "subagent_id",
        "subagentId",
        "subagent_name",
        "subagentName",
    ] {
        if let Some(text) = find_first_string(payload, key) {
            return Some(text);
        }
    }
    None
}

fn find_first_string(value: &serde_json::Value, key: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(candidate) = map.get(key).and_then(serde_json::Value::as_str) {
                let trimmed = candidate.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_owned());
                }
            }
            for nested in map.values() {
                if let Some(found) = find_first_string(nested, key) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(values) => values
            .iter()
            .find_map(|nested| find_first_string(nested, key)),
        _ => None,
    }
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
        "agent_partitions",
        "delegated_tasks",
        "agent_responsibilities",
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

    for key in ["last_user_intent", "worktree_id", "session_created_at"] {
        let serialised = serde_json::to_string(&snapshot)
            .map_err(|e| AtlasError::Other(format!("cannot serialise snapshot: {e}")))?;
        if serialised.len() <= max_bytes {
            return Ok(serialised);
        }
        if let Some(obj) = snapshot.as_object_mut() {
            obj.insert(key.to_string(), serde_json::Value::Null);
        }
    }

    let serialised = serde_json::to_string(&snapshot)
        .map_err(|e| AtlasError::Other(format!("cannot serialise snapshot: {e}")))?;
    if serialised.len() > max_bytes {
        let minimal = minimal_snapshot(&snapshot);
        let minimal_serialised = serde_json::to_string(&minimal)
            .map_err(|e| AtlasError::Other(format!("cannot serialise minimal snapshot: {e}")))?;
        if minimal_serialised.len() > max_bytes {
            return Err(AtlasError::Other(format!(
                "resume snapshot {} bytes exceeds limit {} bytes even after trimming",
                minimal_serialised.len(),
                max_bytes
            )));
        }
        return Ok(minimal_serialised);
    }
    Ok(serialised)
}

fn minimal_snapshot(snapshot: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "session_id": snapshot.get("session_id").cloned().unwrap_or(serde_json::Value::Null),
        "agent_id": snapshot.get("agent_id").cloned().unwrap_or(serde_json::Value::Null),
        "merged_agent_view": snapshot
            .get("merged_agent_view")
            .cloned()
            .unwrap_or(serde_json::Value::Bool(true)),
        "repo_root": snapshot.get("repo_root").cloned().unwrap_or(serde_json::Value::Null),
        "frontend": snapshot.get("frontend").cloned().unwrap_or(serde_json::Value::Null),
        "event_count": snapshot.get("event_count").cloned().unwrap_or(serde_json::Value::from(0)),
        "truncated": true,
    })
}
