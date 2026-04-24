//! CM11 — Global memory layer: cross-session access tracking and workflow detection.
//!
//! Provides three operations on the persistent global memory tables introduced
//! in migration 002:
//!
//! - **Symbol / file access recording**: `upsert_symbol_access`, `upsert_file_access`.
//! - **Workflow pattern detection**: `upsert_workflow_pattern`.
//! - **Recall queries**: `get_frequent_symbols`, `get_frequent_files`,
//!   `get_recurring_workflows`, `find_relevant_sessions`.
//!
//! Extraction helpers consume a slice of `SessionEventRow` values (e.g. the full
//! event list of a session being compacted) and call the upsert functions above
//! in a single transaction.

use std::collections::HashMap;

use rusqlite::{Connection, params};
use serde_json::Value;
use sha2::{Digest, Sha256};

use atlas_core::{AtlasError, Result};

use super::types::{
    GlobalAccessEntry, GlobalWorkflowPattern, SessionEventRow, SessionEventType, SessionMeta,
};

// ── Minimum window size for workflow sequences ────────────────────────────────

/// Sliding-window length used when extracting command sequences.
const WORKFLOW_WINDOW: usize = 3;

/// Minimum occurrences before a workflow is considered "recurring".
/// We store all detected patterns and let callers decide the threshold via `limit`.
const _WORKFLOW_MIN_OCCURRENCES: u64 = 1;

// ── ID derivation ─────────────────────────────────────────────────────────────

fn make_id(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for p in parts {
        hasher.update(p.as_bytes());
        hasher.update(b":");
    }
    format!("{:x}", hasher.finalize())
}

// ── Write operations ──────────────────────────────────────────────────────────

/// Increment-or-insert a symbol access for `(repo_root, symbol_qn)`.
pub(super) fn upsert_symbol_access(
    conn: &Connection,
    repo_root: &str,
    symbol_qn: &str,
    now: &str,
) -> Result<()> {
    let id = make_id(&[repo_root, symbol_qn]);
    conn.execute(
        "INSERT INTO global_symbol_access
             (id, repo_root, symbol_qn, access_count, last_accessed, first_accessed)
         VALUES (?1, ?2, ?3, 1, ?4, ?4)
         ON CONFLICT(id) DO UPDATE SET
             access_count  = access_count + 1,
             last_accessed = excluded.last_accessed",
        params![id, repo_root, symbol_qn, now],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(())
}

/// Increment-or-insert a file access for `(repo_root, file_path)`.
pub(super) fn upsert_file_access(
    conn: &Connection,
    repo_root: &str,
    file_path: &str,
    now: &str,
) -> Result<()> {
    let id = make_id(&[repo_root, file_path]);
    conn.execute(
        "INSERT INTO global_file_access
             (id, repo_root, file_path, access_count, last_accessed, first_accessed)
         VALUES (?1, ?2, ?3, 1, ?4, ?4)
         ON CONFLICT(id) DO UPDATE SET
             access_count  = access_count + 1,
             last_accessed = excluded.last_accessed",
        params![id, repo_root, file_path, now],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(())
}

/// Increment-or-insert a workflow pattern for `(repo_root, pattern)`.
///
/// `pattern` should be a short, ordered slice of command strings or event tokens.
pub(super) fn upsert_workflow_pattern(
    conn: &Connection,
    repo_root: &str,
    pattern: &[String],
    now: &str,
) -> Result<()> {
    let pattern_json =
        serde_json::to_string(pattern).map_err(|e| AtlasError::Other(e.to_string()))?;
    let id = make_id(&[repo_root, &pattern_json]);
    conn.execute(
        "INSERT INTO global_workflow_patterns
             (id, repo_root, pattern_json, occurrence_count, last_seen, first_seen)
         VALUES (?1, ?2, ?3, 1, ?4, ?4)
         ON CONFLICT(id) DO UPDATE SET
             occurrence_count = occurrence_count + 1,
             last_seen        = excluded.last_seen",
        params![id, repo_root, pattern_json, now],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(())
}

// ── Read operations ───────────────────────────────────────────────────────────

/// Return the `limit` most-accessed symbols for `repo_root`.
pub(super) fn get_frequent_symbols(
    conn: &Connection,
    repo_root: &str,
    limit: u32,
) -> Result<Vec<GlobalAccessEntry>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, repo_root, symbol_qn, access_count, last_accessed, first_accessed
             FROM   global_symbol_access
             WHERE  repo_root = ?1
             ORDER  BY access_count DESC, last_accessed DESC
             LIMIT  ?2",
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let rows = stmt
        .query_map(params![repo_root, limit], |row| {
            Ok(GlobalAccessEntry {
                id: row.get(0)?,
                repo_root: row.get(1)?,
                value: row.get(2)?,
                access_count: row.get::<_, i64>(3)? as u64,
                last_accessed: row.get(4)?,
                first_accessed: row.get(5)?,
            })
        })
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| AtlasError::Db(e.to_string()))
}

/// Return the `limit` most-accessed files for `repo_root`.
pub(super) fn get_frequent_files(
    conn: &Connection,
    repo_root: &str,
    limit: u32,
) -> Result<Vec<GlobalAccessEntry>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, repo_root, file_path, access_count, last_accessed, first_accessed
             FROM   global_file_access
             WHERE  repo_root = ?1
             ORDER  BY access_count DESC, last_accessed DESC
             LIMIT  ?2",
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let rows = stmt
        .query_map(params![repo_root, limit], |row| {
            Ok(GlobalAccessEntry {
                id: row.get(0)?,
                repo_root: row.get(1)?,
                value: row.get(2)?,
                access_count: row.get::<_, i64>(3)? as u64,
                last_accessed: row.get(4)?,
                first_accessed: row.get(5)?,
            })
        })
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| AtlasError::Db(e.to_string()))
}

/// Return the `limit` most-frequent workflow patterns for `repo_root`.
pub(super) fn get_recurring_workflows(
    conn: &Connection,
    repo_root: &str,
    limit: u32,
) -> Result<Vec<GlobalWorkflowPattern>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, repo_root, pattern_json, occurrence_count, last_seen, first_seen
             FROM   global_workflow_patterns
             WHERE  repo_root = ?1
             ORDER  BY occurrence_count DESC, last_seen DESC
             LIMIT  ?2",
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let rows = stmt
        .query_map(params![repo_root, limit], |row| {
            let id: String = row.get(0)?;
            let repo: String = row.get(1)?;
            let pattern_json: String = row.get(2)?;
            let count: i64 = row.get(3)?;
            let last_seen: String = row.get(4)?;
            let first_seen: String = row.get(5)?;
            Ok((id, repo, pattern_json, count, last_seen, first_seen))
        })
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let mut out = Vec::new();
    for row in rows {
        let (id, repo, pattern_json, count, last_seen, first_seen) =
            row.map_err(|e| AtlasError::Db(e.to_string()))?;
        let pattern: Vec<String> =
            serde_json::from_str(&pattern_json).map_err(|e| AtlasError::Other(e.to_string()))?;
        out.push(GlobalWorkflowPattern {
            id,
            repo_root: repo,
            pattern,
            occurrence_count: count as u64,
            last_seen,
            first_seen,
        });
    }
    Ok(out)
}

/// Find sessions that share frequently-accessed symbols or files with the given focus lists.
///
/// Returns sessions ordered by overlap score (most overlapping first), up to `limit`.
/// Overlap score = number of matching symbol + file access entries from `focus_symbols`
/// and `focus_files` that also appear in the session's global memory contribution.
///
/// This is a best-effort heuristic: it correlates global_symbol_access /
/// global_file_access entries back to sessions via the `session_events` JSON payloads.
pub(super) fn find_relevant_sessions(
    conn: &Connection,
    repo_root: &str,
    focus_symbols: &[String],
    focus_files: &[String],
    limit: u32,
) -> Result<Vec<SessionMeta>> {
    if focus_symbols.is_empty() && focus_files.is_empty() {
        return Ok(Vec::new());
    }

    // Build a set of session IDs that have touched any of the focus symbols/files.
    let mut session_scores: HashMap<String, u32> = HashMap::new();

    // Match symbol events.
    for symbol in focus_symbols {
        let like = format!("%{}%", symbol.replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT session_id FROM session_events
                 WHERE event_type = 'CONTEXT_REQUEST'
                   AND payload_json LIKE ?1 ESCAPE '\\'",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let ids = stmt
            .query_map(params![like], |row| row.get::<_, String>(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        for id in ids {
            let id = id.map_err(|e| AtlasError::Db(e.to_string()))?;
            *session_scores.entry(id).or_default() += 1;
        }
    }

    // Match file events.
    for file in focus_files {
        let like = format!("%{}%", file.replace('%', "\\%").replace('_', "\\_"));
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT session_id FROM session_events
                 WHERE event_type IN ('FILE_READ','FILE_WRITE')
                   AND payload_json LIKE ?1 ESCAPE '\\'",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let ids = stmt
            .query_map(params![like], |row| row.get::<_, String>(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        for id in ids {
            let id = id.map_err(|e| AtlasError::Db(e.to_string()))?;
            *session_scores.entry(id).or_default() += 1;
        }
    }

    if session_scores.is_empty() {
        return Ok(Vec::new());
    }

    // Sort by score descending and take top `limit`.
    let mut ranked: Vec<(String, u32)> = session_scores.into_iter().collect();
    ranked.sort_unstable_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    ranked.truncate(limit as usize);

    // Fetch session_meta rows for the winning IDs.
    let ids: Vec<String> = ranked.into_iter().map(|(id, _)| id).collect();
    let mut out = Vec::with_capacity(ids.len());
    for id in &ids {
        let mut stmt = conn
            .prepare(
                "SELECT session_id, repo_root, frontend, worktree_id,
                        created_at, updated_at, last_resume_at, last_compaction_at
                 FROM   session_meta
                 WHERE  session_id = ?1 AND repo_root = ?2",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let row = stmt.query_row(params![id, repo_root], |row| {
            use crate::SessionId;
            let sid_str: String = row.get(0)?;
            Ok(SessionMeta {
                session_id: SessionId(sid_str),
                repo_root: row.get(1)?,
                frontend: row.get(2)?,
                worktree_id: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                last_resume_at: row.get(6)?,
                last_compaction_at: row.get(7)?,
            })
        });
        match row {
            Ok(meta) => out.push(meta),
            Err(rusqlite::Error::QueryReturnedNoRows) => {} // session deleted / different repo
            Err(e) => return Err(AtlasError::Db(e.to_string())),
        }
    }
    Ok(out)
}

// ── Bulk extraction helper ────────────────────────────────────────────────────

/// Extract symbols, files, and workflow patterns from `events` and upsert them
/// into the global memory tables.
///
/// Called by `compact_session` after curation to ensure the global memory
/// reflects all events that have ever been seen for the session, even if those
/// events are later evicted.
pub(super) fn update_global_memory_from_events(
    conn: &Connection,
    repo_root: &str,
    events: &[SessionEventRow],
    now: &str,
) -> Result<()> {
    // Extract symbols from CONTEXT_REQUEST events.
    for ev in events
        .iter()
        .filter(|e| e.event_type == SessionEventType::ContextRequest)
    {
        if let Some(sym) = extract_str_field(&ev.payload_json, &["name", "symbol", "target"])
            && !sym.is_empty()
        {
            upsert_symbol_access(conn, repo_root, &sym, now)?;
        }
    }

    // Extract file paths from FILE_READ / FILE_WRITE events.
    for ev in events.iter().filter(|e| {
        matches!(
            e.event_type,
            SessionEventType::FileRead | SessionEventType::FileWrite
        )
    }) {
        if let Some(path) = extract_str_field(&ev.payload_json, &["path", "file_path", "file"])
            && !path.is_empty()
        {
            upsert_file_access(conn, repo_root, &path, now)?;
        }
    }

    // Detect workflow patterns: sliding windows of COMMAND_RUN commands.
    let cmds: Vec<String> = events
        .iter()
        .filter(|e| e.event_type == SessionEventType::CommandRun)
        .filter_map(|e| extract_str_field(&e.payload_json, &["command"]))
        .filter(|c| !c.is_empty())
        .collect();

    if cmds.len() >= WORKFLOW_WINDOW {
        for window in cmds.windows(WORKFLOW_WINDOW) {
            upsert_workflow_pattern(conn, repo_root, window, now)?;
        }
    }

    Ok(())
}

// ── Payload field extractor ───────────────────────────────────────────────────

fn extract_str_field(payload_json: &str, keys: &[&str]) -> Option<String> {
    let val: Value = serde_json::from_str(payload_json).ok()?;
    for key in keys {
        if let Some(s) = val.get(*key).and_then(|v| v.as_str()) {
            return Some(s.to_string());
        }
    }
    None
}
