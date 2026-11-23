use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use atlas_core::{AtlasError, Result};
use atlas_store_markdown::MarkdownStore;
use rusqlite::Connection;
use rusqlite::params;
use serde_json::Value as JsonValue;

use super::MemoryLink;
use super::MemoryRecord;
use super::RELATION_SUPERSEDES;
use super::encoding::public_metadata;

// ── Derived index ─────────────────────────────────────────────────────────────

/// Insert or replace one record in the derived index. The FTS triggers keep
/// `memory_index_fts` in sync.
pub(crate) fn index_upsert(
    store: &MarkdownStore,
    conn: &Connection,
    record: &MemoryRecord,
    relative: &Path,
) -> Result<()> {
    let (file_mtime_ns, file_size) = file_identity(store, relative);
    let metadata_json = serde_json::to_string(&public_metadata(&record.metadata))
        .map_err(|error| AtlasError::Other(format!("serialize memory metadata: {error}")))?;
    let tags_json = serde_json::to_string(&record.tags)
        .map_err(|error| AtlasError::Other(format!("serialize memory tags: {error}")))?;
    let links_json = serde_json::to_string(&record.links)
        .map_err(|error| AtlasError::Other(format!("serialize memory links: {error}")))?;
    let history = record
        .metadata
        .get("__op_history")
        .cloned()
        .unwrap_or_else(|| JsonValue::Array(Vec::new()));
    let history_json = serde_json::to_string(&history)
        .map_err(|error| AtlasError::Other(format!("serialize memory history: {error}")))?;
    let numeric = record
        .metadata
        .get("__numeric_value")
        .and_then(JsonValue::as_f64);
    conn.execute(
        "INSERT INTO memory_index
            (id, repo_root, session_id, frontend, scope, topic, title, body, importance,
             created_at, updated_at, source_id, metadata_json, superseded_by, tags_json,
             links_json, op_history_json, numeric_value, path, file_mtime_ns, file_size)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
         ON CONFLICT(id) DO UPDATE SET
             repo_root = excluded.repo_root,
             session_id = excluded.session_id,
             frontend = excluded.frontend,
             scope = excluded.scope,
             topic = excluded.topic,
             title = excluded.title,
             body = excluded.body,
             importance = excluded.importance,
             created_at = excluded.created_at,
             updated_at = excluded.updated_at,
             source_id = excluded.source_id,
             metadata_json = excluded.metadata_json,
             superseded_by = excluded.superseded_by,
             tags_json = excluded.tags_json,
             links_json = excluded.links_json,
             op_history_json = excluded.op_history_json,
             numeric_value = excluded.numeric_value,
             path = excluded.path,
             file_mtime_ns = excluded.file_mtime_ns,
             file_size = excluded.file_size",
        params![
            record.id,
            record.repo_root,
            record.session_id,
            record.frontend,
            record.scope.as_str(),
            record.topic,
            record.title,
            record.body,
            record.importance.as_str(),
            record.created_at,
            record.updated_at,
            record.source_id,
            metadata_json,
            record.superseded_by,
            tags_json,
            links_json,
            history_json,
            numeric,
            relative.to_string_lossy(),
            file_mtime_ns,
            file_size,
        ],
    )
    .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(())
}

/// `(mtime_ns, size)` for one truth file. Missing files store `NULL` identity so
/// the next reconciliation treats the row as changed.
fn file_identity(store: &MarkdownStore, relative: &Path) -> (Option<i64>, Option<i64>) {
    let Ok(metadata) = std::fs::metadata(store.absolute(relative)) else {
        return (None, None);
    };
    let mtime = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|value| i64::try_from(value.as_nanos()).ok());
    (mtime, i64::try_from(metadata.len()).ok())
}

/// Remove one record from the derived index (and its access/embedding rows).
pub(crate) fn index_delete(conn: &Connection, memory_id: &str) -> Result<()> {
    conn.execute("DELETE FROM memory_index WHERE id = ?1", [memory_id])
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    conn.execute(
        "DELETE FROM memory_access WHERE memory_id = ?1",
        [memory_id],
    )
    .map_err(|error| AtlasError::Db(error.to_string()))?;
    conn.execute(
        "DELETE FROM memory_embeddings WHERE memory_id = ?1",
        [memory_id],
    )
    .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(())
}

/// Recompute `superseded_by` for a repo from indexed `supersedes` links.
pub(crate) fn refresh_supersession_index(conn: &Connection, repo_root: &str) -> Result<()> {
    let mut stmt = conn
        .prepare("SELECT id, links_json FROM memory_index WHERE repo_root = ?1")
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([repo_root], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;

    let mut by_target: BTreeMap<String, String> = BTreeMap::new();
    for (id, links_json) in &rows {
        let links: Vec<MemoryLink> = serde_json::from_str(links_json).unwrap_or_default();
        for link in links
            .into_iter()
            .filter(|link| link.relation == RELATION_SUPERSEDES)
        {
            by_target
                .entry(link.id)
                .and_modify(|existing| {
                    if id < existing {
                        *existing = id.clone();
                    }
                })
                .or_insert_with(|| id.clone());
        }
    }

    for (id, _) in rows {
        let superseded_by = by_target.get(&id);
        conn.execute(
            "UPDATE memory_index SET superseded_by = ?1 WHERE id = ?2",
            params![superseded_by, id],
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    }
    Ok(())
}

/// Ensure the derived index mirrors the markdown tree for a repo. One stat per
/// file detects hand edits, checkouts, new files, and deletions without reading
/// unchanged bodies.
pub(crate) fn ensure_index(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
) -> Result<()> {
    super::markdown_memory_sync::sync_index(store, conn, repo_root)?;
    Ok(())
}

pub(crate) fn indexed_ids(conn: &Connection, repo_root: &str) -> Result<BTreeSet<String>> {
    let mut stmt = conn
        .prepare("SELECT id FROM memory_index WHERE repo_root = ?1")
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([repo_root], |row| row.get::<_, String>(0))
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(rows.into_iter().collect())
}

pub(crate) fn record_belongs_to_repo(conn: &Connection, memory_id: &str, repo_root: &str) -> bool {
    conn.query_row(
        "SELECT COUNT(*) FROM memory_index WHERE id = ?1 AND repo_root = ?2",
        params![memory_id, repo_root],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
        > 0
}

/// Record one recall access burst in the derived access index. Batched into a
/// single transaction so a recall never issues one write per returned memory.
pub(crate) fn record_recall_access(
    conn: &Connection,
    repo_root: &str,
    memory_ids: &[String],
    now: &str,
) -> Result<()> {
    if memory_ids.is_empty() {
        return Ok(());
    }
    let tx = conn
        .unchecked_transaction()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    for memory_id in memory_ids {
        tx.execute(
            "INSERT INTO memory_access (memory_id, repo_root, last_accessed_at, access_count, decay_score)
             VALUES (?1, ?2, ?3, 1, 0)
             ON CONFLICT(memory_id) DO UPDATE SET
                 last_accessed_at = excluded.last_accessed_at,
                 access_count = memory_access.access_count + 1",
            params![memory_id, repo_root, now],
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    }
    tx.commit()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(())
}

pub(crate) fn apply_access_index(
    conn: &Connection,
    repo_root: &str,
    records: &mut [MemoryRecord],
) -> Result<()> {
    let mut stmt = conn
        .prepare(
            "SELECT memory_id, last_accessed_at, decay_score
             FROM memory_access WHERE repo_root = ?1",
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([repo_root], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let access = rows
        .into_iter()
        .map(|(id, last, decay)| (id, (last, decay)))
        .collect::<BTreeMap<_, _>>();
    for record in records {
        if let Some((last, decay)) = access.get(&record.id) {
            record.last_accessed_at = last.clone();
            record.decay_score = *decay;
        }
    }
    Ok(())
}
