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

use std::collections::HashMap;

use rusqlite::params;
use serde_json::Value;

use atlas_core::{AtlasError, Result};

use crate::SessionId;

use super::global_memory::update_global_memory_from_events;
use super::types::CurationResult;
use super::util::format_now;
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

    // ── Apply all mutations in a single transaction ───────────────────────────
    let tx = store
        .conn
        .transaction()
        .map_err(|e| AtlasError::Db(e.to_string()))?;

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
    let now = format_now();
    tx.execute(
        "UPDATE session_meta
         SET last_compaction_at = ?2, updated_at = ?2
         WHERE session_id = ?1",
        params![session_id.as_str(), now],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;

    tx.commit().map_err(|e| AtlasError::Db(e.to_string()))?;

    let deleted_count = all_delete.len();
    let events_after = events_before.saturating_sub(deleted_count);

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
