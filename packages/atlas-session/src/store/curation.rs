//! CM10 — Memory curation: event compaction, merge, decay, dedup, and promotion.
//!
//! Applies deterministic compaction rules to a session's event ledger:
//!
//! - **Decay** stale low-value events (FILE_READ, GRAPH_BUILD/UPDATE, CONTEXT_REQUEST).
//! - **Merge** repeated actions of the same type into the most-recent representative.
//! - **Deduplicate** reasoning outputs by source_id.
//! - **Promote** high-value events (DECISION, USER_INTENT, REASONING_RESULT) to a higher
//!   priority so they survive future eviction cycles.
//!
//! All mutations run inside a single transaction. `last_compaction_at` is updated
//! on session_meta whenever at least one event is removed or promoted.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use rusqlite::params;
use serde_json::Value;

use atlas_core::{AtlasError, Result};

use crate::SessionId;

use super::global_memory::update_global_memory_from_events;
use super::memory_ops::{MemoryOp, apply_memory_op_on};
use super::types::CurationResult;
use super::util::{canonical_json, format_now, hash_event};
use super::{SessionEventRow, SessionEventType, SessionStore};

// ── Tuning constants ─────────────────────────────────────────────────────────

/// Max FILE_READ events to keep per unique file path.
const MAX_FILE_READ_PER_PATH: usize = 3;

/// Max GRAPH_BUILD / GRAPH_UPDATE events to retain across the whole session.
const MAX_GRAPH_STATE_EVENTS: usize = 1;

/// Max COMMAND_RUN events to retain per unique command text.
const MAX_COMMAND_RUNS_PER_CMD: usize = 3;

/// Max CONTEXT_REQUEST events to retain per unique target name / query hint.
const MAX_CONTEXT_REQUEST_PER_TARGET: usize = 2;

/// Priority floor to which high-value events are promoted.
const PROMOTION_PRIORITY: i32 = 90;

/// Event types considered high-value and eligible for priority promotion.
const PROMOTION_TYPES: &[&str] = &["DECISION", "USER_INTENT", "REASONING_RESULT"];

/// Event types that must never enter a compressed group or deletion path.
const COMPRESSION_PROTECTED_TYPES: &[SessionEventType] =
    &[SessionEventType::Decision, SessionEventType::UserIntent];

/// Priority of the deterministic SESSION_SUMMARY rollup event.
const SUMMARY_PRIORITY: i32 = 85;

// ── Public entry point ────────────────────────────────────────────────────────

/// Compact and curate events for `session_id`.
///
/// Returns a [`CurationResult`] describing what was removed / promoted.
/// Produces no error when no session exists yet — returns a zero-change result.
pub(super) fn compact_session_events(
    store: &mut SessionStore,
    session_id: &SessionId,
) -> Result<CurationResult> {
    let events = store.list_events(session_id)?;
    let events_before = events.len();

    // Collect event IDs to delete, grouped by curation category.
    let mut decay_ids: Vec<i64> = Vec::new();
    let mut merge_ids: Vec<i64> = Vec::new();
    let mut dedup_ids: Vec<i64> = Vec::new();

    // ── 1. Decay: keep only MAX_FILE_READ_PER_PATH per file ──────────────────
    {
        let mut by_path: HashMap<String, Vec<&SessionEventRow>> = HashMap::new();
        for ev in events
            .iter()
            .filter(|e| e.event_type == SessionEventType::FileRead)
        {
            let path = extract_str_field(&ev.payload_json, &["path", "file", "file_path"])
                .unwrap_or_else(|| "_unknown_".to_string());
            by_path.entry(path).or_default().push(ev);
        }
        for mut group in by_path.into_values() {
            group.sort_unstable_by(|a, b| b.created_at.cmp(&a.created_at));
            for old in group.into_iter().skip(MAX_FILE_READ_PER_PATH) {
                decay_ids.push(old.id);
            }
        }
    }

    // ── 2. Decay: keep only MAX_GRAPH_STATE_EVENTS GRAPH_BUILD / GRAPH_UPDATE ─
    {
        let mut graph_evs: Vec<&SessionEventRow> = events
            .iter()
            .filter(|e| {
                matches!(
                    e.event_type,
                    SessionEventType::GraphBuild | SessionEventType::GraphUpdate
                )
            })
            .collect();
        graph_evs.sort_unstable_by(|a, b| b.created_at.cmp(&a.created_at));
        for old in graph_evs.into_iter().skip(MAX_GRAPH_STATE_EVENTS) {
            decay_ids.push(old.id);
        }
    }

    // ── 3. Merge: keep MAX_COMMAND_RUNS_PER_CMD per unique command text ───────
    {
        let mut by_cmd: HashMap<String, Vec<&SessionEventRow>> = HashMap::new();
        for ev in events
            .iter()
            .filter(|e| e.event_type == SessionEventType::CommandRun)
        {
            let cmd = extract_str_field(&ev.payload_json, &["command"])
                .unwrap_or_else(|| "_unknown_".to_string());
            by_cmd.entry(cmd).or_default().push(ev);
        }
        for mut group in by_cmd.into_values() {
            if group.len() > MAX_COMMAND_RUNS_PER_CMD {
                group.sort_unstable_by(|a, b| b.created_at.cmp(&a.created_at));
                for old in group.into_iter().skip(MAX_COMMAND_RUNS_PER_CMD) {
                    merge_ids.push(old.id);
                }
            }
        }
    }

    // ── 4. Deduplicate: REASONING_RESULT by source_id (keep latest) ──────────
    {
        let mut by_source: HashMap<String, Vec<&SessionEventRow>> = HashMap::new();
        for ev in events
            .iter()
            .filter(|e| e.event_type == SessionEventType::ReasoningResult)
        {
            let src = extract_str_field(&ev.payload_json, &["source_id"])
                .unwrap_or_else(|| "_none_".to_string());
            by_source.entry(src).or_default().push(ev);
        }
        for mut group in by_source.into_values() {
            if group.len() > 1 {
                group.sort_unstable_by(|a, b| b.created_at.cmp(&a.created_at));
                for dup in group.into_iter().skip(1) {
                    dedup_ids.push(dup.id);
                }
            }
        }
    }

    // ── 5. Decay: keep MAX_CONTEXT_REQUEST_PER_TARGET per unique target ───────
    {
        let mut by_target: HashMap<String, Vec<&SessionEventRow>> = HashMap::new();
        for ev in events
            .iter()
            .filter(|e| e.event_type == SessionEventType::ContextRequest)
        {
            let target =
                extract_str_field(&ev.payload_json, &["name", "symbol", "query_hint", "file"])
                    .unwrap_or_else(|| "_unknown_".to_string());
            by_target.entry(target).or_default().push(ev);
        }
        for mut group in by_target.into_values() {
            if group.len() > MAX_CONTEXT_REQUEST_PER_TARGET {
                group.sort_unstable_by(|a, b| b.created_at.cmp(&a.created_at));
                for old in group.into_iter().skip(MAX_CONTEXT_REQUEST_PER_TARGET) {
                    decay_ids.push(old.id);
                }
            }
        }
    }

    // Dedup the delete lists (an id might appear in multiple steps).
    let mut all_delete: Vec<i64> = decay_ids
        .iter()
        .chain(merge_ids.iter())
        .chain(dedup_ids.iter())
        .copied()
        .collect();
    all_delete.sort_unstable();
    all_delete.dedup();

    // VM2.3: protected event types never enter compression or deletion.
    let protected_ids = events
        .iter()
        .filter(|event| COMPRESSION_PROTECTED_TYPES.contains(&event.event_type))
        .map(|event| event.id)
        .collect::<BTreeSet<_>>();
    all_delete.retain(|id| !protected_ids.contains(id));

    // Collect IDs eligible for priority promotion.
    let promote_ids: Vec<i64> = events
        .iter()
        .filter(|e| {
            PROMOTION_TYPES.contains(&e.event_type.as_str())
                && e.priority < PROMOTION_PRIORITY
                && !all_delete.contains(&e.id)
        })
        .map(|e| e.id)
        .collect();

    let merged_count = merge_ids.len();
    let dedup_count = dedup_ids.len();
    let decay_count = {
        // decay_ids may overlap with merge/dedup; count only unique decay ids
        let mut unique: Vec<i64> = decay_ids.clone();
        unique.sort_unstable();
        unique.dedup();
        // Subtract anything also in merge or dedup lists
        unique
            .iter()
            .filter(|id| !merge_ids.contains(id) && !dedup_ids.contains(id))
            .count()
    };
    let promoted_count = promote_ids.len();

    // VM2.3: summarize every evicted row before deletion so topic timelines
    // keep a deterministic rollup instead of vanishing outright.
    let now = format_now();
    let evicted_rows = all_delete
        .iter()
        .filter_map(|id| events.iter().find(|event| event.id == *id))
        .collect::<Vec<_>>();
    let summary_payload = compress_event_group(&evicted_rows, &now);
    let evicted_topics = summary_payload
        .as_ref()
        .and_then(|payload| payload.get("topics"))
        .and_then(Value::as_array)
        .map(|topics| {
            topics
                .iter()
                .filter_map(|topic| topic.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // ── Apply all mutations in a single transaction ──────────────────────────
    let repo_root = store
        .get_session_meta(session_id)?
        .map(|meta| meta.repo_root);
    let tx = store
        .conn
        .transaction()
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    if let Some(payload) = &summary_payload {
        let payload_json = canonical_json(payload);
        let event_hash = hash_event(
            &SessionEventType::SessionSummary,
            SUMMARY_PRIORITY,
            &payload_json,
        );
        tx.execute(
            "INSERT INTO session_events
                (session_id, event_type, priority, payload_json, event_hash, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                session_id.as_str(),
                SessionEventType::SessionSummary.as_str(),
                SUMMARY_PRIORITY,
                payload_json,
                event_hash,
                now,
            ],
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        // VM2.3: stamp the deterministic TimeCompress rollup on the active
        // memory entity for each evicted topic. Missing entities are skipped.
        if let Some(repo_root) = repo_root.as_deref() {
            let op = MemoryOp::TimeCompress {
                window_label: "session_compaction".to_owned(),
            };
            for topic in &evicted_topics {
                let memory_id = tx
                    .query_row(
                        "SELECT id FROM memories
                         WHERE repo_root = ?1 AND topic = ?2 AND superseded_by IS NULL
                         ORDER BY updated_at DESC, id ASC LIMIT 1",
                        params![repo_root, topic],
                        |row| row.get::<_, String>(0),
                    )
                    .ok();
                if let Some(memory_id) = memory_id {
                    apply_memory_op_on(&tx, repo_root, &memory_id, &op, &now)?;
                }
            }
        }
    }

    for id in &all_delete {
        tx.execute("DELETE FROM session_events WHERE id = ?1", params![id])
            .map_err(|e| AtlasError::Db(e.to_string()))?;
    }

    for id in &promote_ids {
        tx.execute(
            "UPDATE session_events SET priority = ?2 WHERE id = ?1 AND priority < ?2",
            params![id, PROMOTION_PRIORITY],
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    }

    // Always stamp last_compaction_at — caller invoked compaction explicitly.
    tx.execute(
        "UPDATE session_meta
         SET last_compaction_at = ?2, updated_at = ?2
         WHERE session_id = ?1",
        params![session_id.as_str(), now],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;

    tx.commit().map_err(|e| AtlasError::Db(e.to_string()))?;

    let deleted_count = all_delete.len();
    let summary_written = usize::from(summary_payload.is_some());
    let events_after = events_before
        .saturating_sub(deleted_count)
        .saturating_add(summary_written);

    // CM11: update global memory from all events (before eviction list is applied).
    // Best-effort — failure must not abort compaction.
    if let Ok(Some(meta)) = store.get_session_meta(session_id) {
        let _ = update_global_memory_from_events(&store.conn, &meta.repo_root, &events, &now);
    }

    Ok(CurationResult {
        events_before,
        events_after,
        merged_count,
        decayed_count: decay_count,
        deduplicated_count: dedup_count,
        promoted_count,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Aggregate evicted rows per topic, file, and command into one deterministic
/// `SESSION_SUMMARY` payload (VM2.3).
///
/// `DECISION` and `USER_INTENT` rows are always excluded. Returns `None` when
/// nothing compressible was evicted so repeated compactions stay idempotent.
fn compress_event_group(events: &[&SessionEventRow], now: &str) -> Option<Value> {
    let mut rows = events
        .iter()
        .copied()
        .filter(|event| !COMPRESSION_PROTECTED_TYPES.contains(&event.event_type))
        .collect::<Vec<_>>();
    if rows.is_empty() {
        return None;
    }
    rows.sort_by_key(|event| (event.created_at.clone(), event.id));

    let mut total_reads = 0usize;
    let mut command_runs = 0usize;
    let mut failure_count = 0usize;
    let mut topics = BTreeSet::new();
    let mut files = BTreeMap::<String, usize>::new();
    let mut commands = BTreeMap::<String, usize>::new();
    let mut source_ids = BTreeSet::new();
    let mut event_ids = Vec::new();

    for event in &rows {
        match event.event_type {
            SessionEventType::FileRead => total_reads += 1,
            SessionEventType::CommandRun => command_runs += 1,
            SessionEventType::CommandFail | SessionEventType::Error => failure_count += 1,
            _ => {}
        }
        if let Some(file) = extract_str_field(&event.payload_json, &["path", "file", "file_path"]) {
            *files.entry(file).or_insert(0) += 1;
        }
        if let Some(command) = extract_str_field(&event.payload_json, &["command"]) {
            *commands.entry(command).or_insert(0) += 1;
        }
        if let Some(topic) = extract_str_field(&event.payload_json, &["topic"]) {
            topics.insert(topic);
        }
        if let Some(source_id) = extract_str_field(&event.payload_json, &["source_id"]) {
            source_ids.insert(source_id);
        }
        event_ids.push(event.id.to_string());
    }

    let first_seen = rows.first().map(|event| event.created_at.clone());
    let last_seen = rows.last().map(|event| event.created_at.clone());
    let topics = topics.into_iter().collect::<Vec<_>>();

    Some(serde_json::json!({
        "tool": "compact_session_events",
        "window_label": "session_compaction",
        "compressed_at": now,
        "evicted_event_count": rows.len(),
        "total_reads": total_reads,
        "command_runs": command_runs,
        "failure_count": failure_count,
        "first_seen": first_seen,
        "last_seen": last_seen,
        "topics": topics,
        "files": files,
        "commands": commands,
        "source_ids": source_ids.into_iter().collect::<Vec<_>>(),
        "event_ids": event_ids,
    }))
}

/// Try to extract a string from the first matching key in `payload_json`.
fn extract_str_field(payload_json: &str, keys: &[&str]) -> Option<String> {
    let val: Value = serde_json::from_str(payload_json).ok()?;
    for key in keys {
        if let Some(s) = val.get(*key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}
