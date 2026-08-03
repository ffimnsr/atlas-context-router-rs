//! ICM-A — Shared memory model storage layer.
//!
//! The `memories` table lives in the continuity-owned session database, next
//! to decision memory and global memory. This module owns validation, schema
//! checks, and CRUD used by CLI and MCP memory surfaces so the two cannot
//! drift on record shape, defaults, or validation.

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use atlas_core::{AtlasError, Clock, Result, SystemClock, format_rfc3339};

use super::types::{
    MemoryDeleteResult, MemoryListFilter, MemoryRecord, MemorySearchHit, MemoryViewer, NewMemory,
};
use super::util::{hex_encode, to_from_sql_error};

pub(super) const MEMORIES_TABLE: &str = "memories";

/// Exact column set of the `memories` table (migration 007).
pub(super) const MEMORY_COLUMNS: &[&str] = &[
    "id",
    "repo_root",
    "session_id",
    "frontend",
    "scope",
    "topic",
    "title",
    "body",
    "importance",
    "created_at",
    "updated_at",
    "last_accessed_at",
    "decay_score",
    "source_id",
    "metadata_json",
];

/// Exact index set of the `memories` table (migration 007).
pub(super) const MEMORY_INDEXES: &[&str] = &[
    "idx_memories_repo_topic",
    "idx_memories_repo_importance",
    "idx_memories_repo_scope",
    "idx_memories_repo_session",
    "idx_memories_repo_accessed",
];

// ── IDs and timestamps ────────────────────────────────────────────────────────

/// Stable per-record id: hex SHA-256 of `repo_root`, `body`, and a creation
/// nanosecond nonce. The nonce keeps identical texts stored in the same second
/// distinct without adding a dependency for random ids.
pub(super) fn derive_memory_id(repo_root: &str, body: &str, created_at_nanos: i128) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_root.as_bytes());
    hasher.update(b"\x00");
    hasher.update(body.as_bytes());
    hasher.update(b"\x00");
    hasher.update(created_at_nanos.to_string().as_bytes());
    hex_encode(&hasher.finalize())
}

/// RFC 3339 timestamp normalized to second precision so lexicographic order
/// equals chronological order (`format_rfc3339` emits subseconds when nonzero,
/// which breaks string ordering across rows).
pub(super) fn format_memory_now() -> String {
    format_memory_timestamp(SystemClock.now_utc())
}

fn format_memory_timestamp(ts: OffsetDateTime) -> String {
    format_rfc3339(
        ts.replace_nanosecond(0)
            .expect("0 nanoseconds is always valid"),
    )
}

// ── Schema validation ─────────────────────────────────────────────────────────

/// Returns schema issues for the `memories` table; empty when healthy.
///
/// Used by `atlas db check` (CLI and MCP) to validate the memory schema.
pub(super) fn memory_schema_issues(conn: &Connection) -> Vec<String> {
    let mut issues = Vec::new();

    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memories'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        issues.push(format!("missing table: {MEMORIES_TABLE}"));
        return issues;
    }

    let present_columns = conn
        .prepare("PRAGMA table_info('memories')")
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(1))
                .ok()
                .map(|rows| {
                    rows.filter_map(std::result::Result::ok)
                        .collect::<Vec<String>>()
                })
        })
        .unwrap_or_default();
    for column in MEMORY_COLUMNS {
        if !present_columns.iter().any(|present| present == column) {
            issues.push(format!("missing column: memories.{column}"));
        }
    }

    for index in MEMORY_INDEXES {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                [index],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if exists == 0 {
            issues.push(format!("missing index: {index}"));
        }
    }

    issues
}

// ── Write ─────────────────────────────────────────────────────────────────────

/// Validate and persist a new memory, deriving its id and timestamps.
pub(super) fn store_memory(conn: &Connection, input: &NewMemory) -> Result<MemoryRecord> {
    input.validate()?;
    let id = derive_memory_id(
        &input.repo_root,
        &input.body,
        SystemClock.now_utc().unix_timestamp_nanos(),
    );
    store_memory_at(conn, input, &format_memory_now(), &id)
}

/// Raw insert used by [`store_memory`] and deterministic tests.
pub(super) fn store_memory_at(
    conn: &Connection,
    input: &NewMemory,
    now: &str,
    id: &str,
) -> Result<MemoryRecord> {
    input.validate()?;
    let metadata_json = serde_json::to_string(&input.metadata)?;
    conn.execute(
        "INSERT INTO memories
            (id, repo_root, session_id, frontend, scope, topic, title, body, importance,
             created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?10, 0, ?11, ?12)",
        params![
            id,
            input.repo_root,
            input.session_id,
            input.frontend,
            input.scope.as_str(),
            input.topic,
            input.title,
            input.body,
            input.importance.as_str(),
            now,
            input.source_id,
            metadata_json,
        ],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(MemoryRecord {
        id: id.to_owned(),
        repo_root: input.repo_root.clone(),
        session_id: input.session_id.clone(),
        frontend: input.frontend.clone(),
        scope: input.scope,
        topic: input.topic.clone(),
        title: input.title.clone(),
        body: input.body.clone(),
        importance: input.importance,
        created_at: now.to_owned(),
        updated_at: now.to_owned(),
        last_accessed_at: now.to_owned(),
        decay_score: 0.0,
        source_id: input.source_id.clone(),
        metadata: input.metadata.clone(),
    })
}

// ── Recall and list ───────────────────────────────────────────────────────────

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn append_filter_clauses(sql: &mut String, params: &mut Vec<String>, filter: &MemoryListFilter) {
    if let Some(topic) = &filter.topic {
        sql.push_str(" AND topic = ? COLLATE NOCASE");
        params.push(topic.clone());
    }
    if let Some(importance) = filter.importance {
        sql.push_str(" AND importance = ?");
        params.push(importance.as_str().to_owned());
    }
    if let Some(scope) = filter.scope {
        sql.push_str(" AND scope = ?");
        params.push(scope.as_str().to_owned());
    }
    if let Some(older_than) = &filter.older_than {
        sql.push_str(" AND updated_at < ?");
        params.push(older_than.clone());
    }
    if let Some(newer_than) = &filter.newer_than {
        sql.push_str(" AND updated_at > ?");
        params.push(newer_than.clone());
    }
}

fn query_memories(
    conn: &Connection,
    select_sql: &str,
    params: &[String],
) -> Result<Vec<MemoryRecord>> {
    let mut stmt = conn
        .prepare(select_sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs = params.iter().map(String::as_str).collect::<Vec<_>>();
    stmt.query_map(rusqlite::params_from_iter(param_refs), row_to_memory)
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))
}

/// Lexical recall: exact topic matches rank above topic/title contains
/// matches, which rank above body-only matches; ties break on importance,
/// then recency. Visibility (ICM-A3) is enforced for the given viewer:
/// `session`-scoped memories require the same session id, `frontend`-scoped
/// memories require the same frontend. `shared_only` narrows results to
/// `project` + `global` and bypasses viewer-based conditions.
pub(super) fn recall_memories(
    conn: &Connection,
    repo_root: &str,
    query: &str,
    filter: &MemoryListFilter,
    shared_only: bool,
    viewer: &MemoryViewer,
    limit: usize,
) -> Result<Vec<MemorySearchHit>> {
    let like = format!("%{}%", escape_like(query));
    let mut sql = String::from(
        "SELECT id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json,
                CASE WHEN topic = ?2 THEN 0
                     WHEN topic LIKE ?3 ESCAPE '\\' OR title LIKE ?3 ESCAPE '\\' THEN 1
                     ELSE 2 END AS match_tier,
                CASE importance WHEN 'critical' THEN 0 WHEN 'high' THEN 1
                                WHEN 'normal' THEN 2 ELSE 3 END AS importance_rank
         FROM memories
         WHERE repo_root = ?1
           AND (topic LIKE ?3 ESCAPE '\\' OR title LIKE ?3 ESCAPE '\\'
                OR body LIKE ?3 ESCAPE '\\')",
    );
    let mut params = vec![repo_root.to_owned(), query.to_owned(), like];
    append_filter_clauses(&mut sql, &mut params, filter);
    if shared_only {
        sql.push_str(" AND scope IN ('project', 'global')");
    } else {
        sql.push_str(
            " AND (scope IN ('project', 'global')
                  OR (scope = 'session' AND session_id = ?)
                  OR (scope = 'frontend' AND frontend = ?))",
        );
        params.push(viewer.session_id.clone());
        params.push(viewer.frontend.clone());
    }
    sql.push_str(
        " ORDER BY match_tier ASC, importance_rank ASC, updated_at DESC, created_at DESC
         LIMIT ?",
    );
    params.push(limit.to_string());

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs = params.iter().map(String::as_str).collect::<Vec<_>>();
    stmt.query_map(rusqlite::params_from_iter(param_refs), |row| {
        let relevance_score: i32 = row.get(15)?;
        Ok(MemorySearchHit {
            memory: row_to_memory(row)?,
            relevance_score,
        })
    })
    .map_err(|e| AtlasError::Db(e.to_string()))?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(|e| AtlasError::Db(e.to_string()))
}

/// List memories for a repo, filtered and sorted by `updated_at DESC`.
pub(super) fn list_memories(
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
) -> Result<Vec<MemoryRecord>> {
    let mut sql = String::from(
        "SELECT id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json
         FROM memories
         WHERE repo_root = ?1",
    );
    let mut params = vec![repo_root.to_owned()];
    append_filter_clauses(&mut sql, &mut params, filter);
    sql.push_str(" ORDER BY updated_at DESC, created_at DESC, id");
    query_memories(conn, &sql, &params)
}

// ── Delete ────────────────────────────────────────────────────────────────────

/// Delete a memory by exact id within a repo. Dry-run only reports whether
/// the row exists. Linked saved-context artifacts are never touched here.
pub(super) fn delete_memory(
    conn: &Connection,
    repo_root: &str,
    memory_id: &str,
    dry_run: bool,
) -> Result<MemoryDeleteResult> {
    let found = conn
        .query_row(
            "SELECT 1 FROM memories WHERE id = ?1 AND repo_root = ?2",
            params![memory_id, repo_root],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .is_some();

    let deleted = if found && !dry_run {
        let removed = conn
            .execute(
                "DELETE FROM memories WHERE id = ?1 AND repo_root = ?2",
                params![memory_id, repo_root],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        removed > 0
    } else {
        false
    };

    Ok(MemoryDeleteResult {
        memory_id: memory_id.to_owned(),
        found,
        deleted,
        dry_run,
    })
}

// ── Row mapping ───────────────────────────────────────────────────────────────

/// Maps a `memories` row (column order from migration 007) to [`MemoryRecord`].
pub(super) fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
    let scope: String = row.get(4)?;
    let importance: String = row.get(8)?;
    Ok(MemoryRecord {
        id: row.get(0)?,
        repo_root: row.get(1)?,
        session_id: row.get(2)?,
        frontend: row.get(3)?,
        scope: scope.parse().map_err(to_from_sql_error)?,
        topic: row.get(5)?,
        title: row.get(6)?,
        body: row.get(7)?,
        importance: importance.parse().map_err(to_from_sql_error)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        last_accessed_at: row.get(11)?,
        decay_score: row.get(12)?,
        source_id: row.get(13)?,
        metadata: serde_json::from_str(&row.get::<_, String>(14)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                14,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_ids_are_deterministic_and_distinct() {
        let a = derive_memory_id("/repo", "same body", 1_700_000_000_000_000_000);
        let b = derive_memory_id("/repo", "same body", 1_700_000_000_000_000_000);
        let c = derive_memory_id("/repo", "same body", 1_700_000_000_000_000_001);
        let d = derive_memory_id("/repo", "other body", 1_700_000_000_000_000_000);
        assert_eq!(a, b);
        assert_ne!(a, c, "nonce must separate same-second writes");
        assert_ne!(a, d, "body must be part of the id");
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn memory_timestamps_are_second_precision() {
        let now = OffsetDateTime::from_unix_timestamp_nanos(1_700_000_000_123_456_789)
            .expect("valid nanos");
        assert_eq!(
            format_memory_timestamp(now),
            "2023-11-14T22:13:20Z",
            "subseconds must be dropped so string order equals time order"
        );
    }
}
