//! Shared memory-surface helpers for the markdown-truth backend.
//!
//! Durable records live in `.atlas/memories/**` (see [`super::markdown_memory`]).
//! This module keeps the pieces other store modules still need: id derivation,
//! timestamp formatting, age math, SQL filter fragments for derived-index
//! queries, row mapping for the `memories` compatibility view, and derived
//! schema validation.

use rusqlite::Connection;
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use atlas_core::{Clock, SystemClock, format_rfc3339};
use atlas_store_markdown::MarkdownStore;

use super::markdown_memory;
use super::types::{MemoryListFilter, MemoryRecord};
use super::util::{hex_encode, to_from_sql_error};

pub(super) const MEMORY_INDEX_TABLE: &str = "memory_index";

/// Exact column set of the derived `memory_index` table (migration 017).
pub(super) const MEMORY_INDEX_COLUMNS: &[&str] = &[
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
    "source_id",
    "metadata_json",
    "superseded_by",
    "tags_json",
    "links_json",
    "op_history_json",
    "numeric_value",
    "path",
    "file_mtime_ns",
    "file_size",
];

/// Index set of the derived `memory_index` table.
pub(super) const MEMORY_INDEX_INDEXES: &[&str] = &[
    "idx_memory_index_repo_topic",
    "idx_memory_index_repo_importance",
    "idx_memory_index_repo_scope",
    "idx_memory_index_repo_session",
    "idx_memory_index_repo_updated",
    "idx_memory_index_repo_source",
    "idx_memory_index_repo_active",
    "idx_memory_index_superseded",
];

/// Exact column set of the derived `memory_access` table.
pub(super) const MEMORY_ACCESS_COLUMNS: &[&str] = &[
    "memory_id",
    "repo_root",
    "last_accessed_at",
    "access_count",
    "decay_score",
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
/// equals chronological order.
pub(super) fn format_memory_now() -> String {
    format_memory_timestamp(SystemClock.now_utc())
}

fn format_memory_timestamp(ts: OffsetDateTime) -> String {
    format_rfc3339(
        ts.replace_nanosecond(0)
            .expect("0 nanoseconds is always valid"),
    )
}

/// Parse a second-precision RFC 3339 timestamp stored on memory records.
fn parse_memory_timestamp(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
}

/// Whole-day age of a memory record relative to `now`, anchored on `updated_at`.
pub(super) fn age_days(updated_at: &str, now: OffsetDateTime) -> f64 {
    let Some(ts) = parse_memory_timestamp(updated_at) else {
        return 0.0;
    };
    let seconds = now.unix_timestamp() - ts.unix_timestamp();
    if seconds <= 0 {
        0.0
    } else {
        seconds as f64 / 86_400.0
    }
}

// ── Derived SQL helpers ───────────────────────────────────────────────────────

/// Append filter predicates shared by derived-index queries. Callers alias the
/// index table as `prefix` and push params in the same order.
pub(super) fn append_filter_clauses(
    sql: &mut String,
    params: &mut Vec<String>,
    filter: &MemoryListFilter,
    prefix: &str,
) {
    if let Some(topic) = &filter.topic {
        sql.push_str(&format!(" AND {prefix}topic = ? COLLATE NOCASE"));
        params.push(topic.clone());
    }
    if let Some(importance) = filter.importance {
        sql.push_str(&format!(" AND {prefix}importance = ?"));
        params.push(importance.as_str().to_owned());
    }
    if let Some(scope) = filter.scope {
        sql.push_str(&format!(" AND {prefix}scope = ?"));
        params.push(scope.as_str().to_owned());
    }
    if let Some(older_than) = &filter.older_than {
        sql.push_str(&format!(" AND {prefix}updated_at < ?"));
        params.push(older_than.clone());
    }
    if let Some(newer_than) = &filter.newer_than {
        sql.push_str(&format!(" AND {prefix}updated_at > ?"));
        params.push(newer_than.clone());
    }
    for tag in &filter.tags {
        sql.push_str(&format!(
            " AND EXISTS (SELECT 1 FROM json_each({prefix}tags_json) WHERE lower(json_each.value) = lower(?))"
        ));
        params.push(tag.clone());
    }
}

/// Maps a `memories` view row (column order from migration 017) to
/// [`MemoryRecord`].
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
        tags: Vec::new(),
        links: Vec::new(),
        metadata: serde_json::from_str(&row.get::<_, String>(14)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                14,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        superseded_by: row.get(15)?,
    })
}

// ── Derived schema validation ─────────────────────────────────────────────────

/// Returns issues for the derived memory index plus markdown-tree integrity;
/// empty when healthy. Used by `atlas db check` (CLI and MCP).
pub(super) fn memory_schema_issues(conn: &Connection, store: &MarkdownStore) -> Vec<String> {
    let mut issues = Vec::new();

    let view_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'view' AND name = 'memories'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if view_exists == 0 {
        issues.push("missing view: memories".to_owned());
    }

    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [MEMORY_INDEX_TABLE],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        issues.push(format!("missing table: {MEMORY_INDEX_TABLE}"));
        return issues;
    }

    for column in MEMORY_INDEX_COLUMNS {
        if !column_exists(conn, MEMORY_INDEX_TABLE, column) {
            issues.push(format!("missing column: {MEMORY_INDEX_TABLE}.{column}"));
        }
    }
    for index in MEMORY_INDEX_INDEXES {
        if !index_exists(conn, index) {
            issues.push(format!("missing index: {index}"));
        }
    }

    for column in MEMORY_ACCESS_COLUMNS {
        if !column_exists(conn, "memory_access", column) {
            issues.push(format!("missing column: memory_access.{column}"));
        }
    }

    let fts_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memory_index_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if fts_exists == 0 {
        issues.push("missing table: memory_index_fts".to_owned());
    } else {
        let index_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_index", [], |row| row.get(0))
            .unwrap_or(0);
        let fts_count: i64 = conn
            .query_row("SELECT COUNT(*) FROM memory_index_fts_docsize", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        if index_count != fts_count {
            issues.push(format!(
                "memory_index_fts is out of sync with memory_index: {fts_count} indexed rows for {index_count} records"
            ));
        }
    }

    issues.extend(markdown_memory::tree_integrity(store, ""));
    issues.extend(super::markdown_records::integrity_issues(store));
    issues.extend(super::memory_ops::schema_issues(conn));
    issues.extend(super::memory_embeddings::schema_issues(conn));
    issues
}

fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    conn.prepare(&format!("PRAGMA table_info('{table}')"))
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(1))
                .ok()
                .map(|rows| {
                    rows.filter_map(std::result::Result::ok)
                        .any(|present| present == column)
                })
        })
        .unwrap_or(false)
}

fn index_exists(conn: &Connection, index: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
        [index],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
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
