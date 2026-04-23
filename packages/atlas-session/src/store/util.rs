use std::path::Path;

use rusqlite::params;
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use atlas_core::{AtlasError, Result};
use atlas_repo::CanonicalRepoPath;
use camino::Utf8Path;

use crate::SessionId;

use super::{SessionEventRow, SessionEventType};

pub(super) fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionEventRow> {
    let event_type: String = row.get(2)?;
    let event_type = event_type.parse().map_err(to_from_sql_error)?;
    Ok(SessionEventRow {
        id: row.get(0)?,
        session_id: SessionId(row.get(1)?),
        event_type,
        priority: row.get(3)?,
        payload_json: row.get(4)?,
        event_hash: row.get(5)?,
        created_at: row.get(6)?,
    })
}

pub(super) fn enforce_event_limit(
    tx: &rusqlite::Transaction<'_>,
    session_id: &str,
    max_events_per_session: usize,
) -> Result<usize> {
    let current_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM session_events WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let current_count = current_count as usize;
    if current_count <= max_events_per_session {
        return Ok(0);
    }

    let overflow = current_count - max_events_per_session;
    tx.execute(
        "DELETE FROM session_events
         WHERE id IN (
             SELECT id
             FROM session_events
             WHERE session_id = ?1
             ORDER BY priority ASC, created_at ASC, id ASC
             LIMIT ?2
         )",
        params![session_id, overflow as i64],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;

    Ok(overflow)
}

pub(super) fn hash_event(
    event_type: &SessionEventType,
    priority: i32,
    payload_json: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(event_type.as_str().as_bytes());
    hasher.update(b"\x00");
    hasher.update(priority.to_string().as_bytes());
    hasher.update(b"\x00");
    hasher.update(payload_json.as_bytes());
    hex_encode(&hasher.finalize())
}

pub(super) fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()),
        Value::Array(values) => {
            let items = values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{items}]")
        }
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(left, _)| *left);
            let items = entries
                .into_iter()
                .map(|(key, value)| {
                    let key = serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key}:{}", canonical_json(value))
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{items}}}")
        }
    }
}

pub(super) fn normalize_event_payload_paths(repo_root: &str, value: &mut Value) {
    match value {
        Value::Array(values) => {
            for nested in values {
                normalize_event_payload_paths(repo_root, nested);
            }
        }
        Value::Object(map) => {
            for (key, nested) in map.iter_mut() {
                if is_repo_path_key(key) {
                    normalize_path_value(repo_root, nested);
                }
                normalize_event_payload_paths(repo_root, nested);
            }
        }
        _ => {}
    }
}

pub(super) fn normalize_repo_path_string(repo_root: &str, candidate: &str) -> Option<String> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Resume snapshots and normalized session payloads persist repo-file
    // references with the same canonical path identity used by graph/content
    // stores so cross-store lookup does not drift.
    CanonicalRepoPath::from_cli_argument(Utf8Path::new(repo_root), Utf8Path::new(trimmed))
        .ok()
        .map(|path| path.as_str().to_owned())
}

fn normalize_path_value(repo_root: &str, value: &mut Value) {
    match value {
        Value::String(text) => {
            if let Some(normalized) = normalize_repo_path_string(repo_root, text) {
                *text = normalized;
            }
        }
        Value::Array(values) => {
            for nested in values {
                normalize_path_value(repo_root, nested);
            }
        }
        _ => {}
    }
}

fn is_repo_path_key(key: &str) -> bool {
    matches!(
        key,
        "file"
            | "filePath"
            | "file_path"
            | "files"
            | "path"
            | "paths"
            | "changed_files"
            | "changedFiles"
            | "target_file"
            | "targetPath"
    )
}

pub(super) fn format_now() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub(super) fn format_days_ago(days: u32) -> String {
    let ts = OffsetDateTime::now_utc() - time::Duration::days(days as i64);
    ts.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub(super) fn format_seconds_ago(secs: u64) -> String {
    let ts = OffsetDateTime::now_utc() - time::Duration::seconds(secs as i64);
    ts.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

pub(super) fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(super) fn path_to_str(path: &Path) -> Result<&str> {
    path.to_str()
        .ok_or_else(|| AtlasError::Other(format!("non-utf8 path: {}", path.display())))
}

pub(super) fn to_from_sql_error(error: AtlasError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

pub(super) fn is_corruption_error(err: &AtlasError) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("malformed")
        || msg.contains("not a database")
        || msg.contains("disk image is malformed")
        || msg.contains("database disk image")
        || msg.contains("file is not a database")
}

pub(super) fn quarantine_db(path: &str) {
    let qpath = format!("{path}.quarantine");
    if let Err(e) = std::fs::rename(path, &qpath) {
        tracing::warn!(
            path = path,
            quarantine = %qpath,
            err = %e,
            "session DB quarantine rename failed"
        );
    } else {
        tracing::warn!(
            path = path,
            quarantine = %qpath,
            "corrupt session DB quarantined; a fresh store will be created on next open"
        );
    }
}
