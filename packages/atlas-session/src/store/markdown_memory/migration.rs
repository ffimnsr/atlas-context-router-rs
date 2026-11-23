use std::path::{Path, PathBuf};

use atlas_core::{AtlasError, Result};
use atlas_store_markdown::MarkdownStore;
use rusqlite::Connection;
use serde_json::Value as JsonValue;

use super::MemoryLink;
use super::MemoryRecord;
use super::RELATION_SUPERSEDES;
use super::encoding::decode_record;
use super::encoding::encode_record;
use super::encoding::markdown_error;
use super::encoding::relative_path;
use super::index::index_upsert;
use super::index::refresh_supersession_index;

/// Export legacy derived rows that have no markdown file yet. Memory index
/// reconciliation from the markdown tree happens lazily per repo via
/// [`super::markdown_memory_sync::sync_index`].
pub(crate) fn reconcile_on_open(store: &MarkdownStore, conn: &Connection) -> Result<usize> {
    Ok(export_pending(store, conn, false)?.len())
}

// ── Legacy export ─────────────────────────────────────────────────────────────

/// Outcome of `atlas migrate markdown`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownMigrationReport {
    pub dry_run: bool,
    pub written: Vec<String>,
    pub pending: usize,
    pub backup: Option<String>,
    /// Durable records (decisions, feedback, memoir, overview, tasks,
    /// experience) written by the same migration pass.
    pub records_written: usize,
}

/// Number of index rows that still lack a markdown file.
pub(crate) fn pending_export_count(conn: &Connection) -> Result<usize> {
    conn.query_row(
        "SELECT COUNT(*) FROM memory_index WHERE path IS NULL",
        [],
        |row| row.get::<_, i64>(0),
    )
    .map(|count| count.max(0) as usize)
    .map_err(|error| AtlasError::Db(error.to_string()))
}

/// Copy the session database to `session.db.bak` before the first export.
/// Uses SQLite's `VACUUM INTO` so the backup is a consistent snapshot of a WAL
/// database instead of a raw file copy that can miss uncheckpointed frames.
pub(crate) fn write_migration_backup(db_path: &Path) -> Result<String> {
    let backup = db_path.with_extension("db.bak");
    if !backup.exists() {
        let conn = Connection::open(db_path).map_err(|error| AtlasError::Db(error.to_string()))?;
        conn.execute("VACUUM INTO ?1", [backup.to_string_lossy().as_ref()])
            .map_err(|error| AtlasError::Db(format!("create migration backup: {error}")))?;
    }
    Ok(backup.to_string_lossy().into_owned())
}

/// Remove the pre-migration backup and the legacy supersession staging table.
pub(crate) fn finalize_migration(
    db_path: &Path,
    conn: &Connection,
) -> Result<MarkdownMigrationReport> {
    if pending_export_count(conn)? > 0 {
        return Err(AtlasError::Other(
            "cannot finalize: pending memories still need a markdown file; run `atlas migrate markdown` first"
                .to_owned(),
        ));
    }
    conn.execute_batch("DROP TABLE IF EXISTS memory_legacy_supersessions")
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let backup = db_path.with_extension("db.bak");
    let removed = match std::fs::remove_file(&backup) {
        Ok(()) => Some(backup.to_string_lossy().into_owned()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(AtlasError::Other(format!(
                "remove migration backup {}: {error}",
                backup.display()
            )));
        }
    };
    Ok(MarkdownMigrationReport {
        dry_run: false,
        written: Vec::new(),
        pending: 0,
        backup: removed,
        records_written: 0,
    })
}

/// Export index rows that have no markdown file yet (legacy sqlite rows and
/// interrupted imports). Returns the written relative paths.
pub(crate) fn export_pending(
    store: &MarkdownStore,
    conn: &Connection,
    dry_run: bool,
) -> Result<Vec<PathBuf>> {
    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memory_index'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, repo_root, session_id, frontend, scope, topic, title, body,
                    importance, created_at, updated_at, source_id, metadata_json,
                    superseded_by, tags_json, links_json, op_history_json, numeric_value
             FROM memory_index
             WHERE path IS NULL
             ORDER BY id",
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            let mut metadata: JsonValue =
                serde_json::from_str(&row.get::<_, String>(12)?).unwrap_or_default();
            let history_json: String = row.get(16)?;
            let history: JsonValue =
                serde_json::from_str(&history_json).unwrap_or(JsonValue::Array(Vec::new()));
            let numeric: Option<f64> = row.get(17)?;
            if let Some(object) = metadata.as_object_mut() {
                if history.as_array().is_some_and(|items| !items.is_empty()) {
                    object.insert("__op_history".to_owned(), history);
                }
                if let Some(numeric) = numeric {
                    object.insert("__numeric_value".to_owned(), serde_json::json!(numeric));
                }
            }
            Ok(MemoryRecord {
                id: row.get(0)?,
                repo_root: row.get(1)?,
                session_id: row.get(2)?,
                frontend: row.get(3)?,
                scope: row
                    .get::<_, String>(4)?
                    .parse()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                topic: row.get(5)?,
                title: row.get(6)?,
                body: row.get(7)?,
                importance: row
                    .get::<_, String>(8)?
                    .parse()
                    .map_err(|_| rusqlite::Error::InvalidQuery)?,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
                last_accessed_at: row.get(9)?,
                decay_score: 0.0,
                source_id: row.get(11)?,
                metadata,
                tags: serde_json::from_str(&row.get::<_, String>(14)?).unwrap_or_default(),
                links: serde_json::from_str(&row.get::<_, String>(15)?).unwrap_or_default(),
                superseded_by: row.get(13)?,
            })
        })
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;

    let mut written = Vec::new();
    for mut record in rows {
        let mut links = legacy_supersession_links(conn, &record.id)?;
        links.append(&mut record.links);
        links.sort_by(|left, right| {
            left.id
                .cmp(&right.id)
                .then_with(|| left.relation.cmp(&right.relation))
        });
        links.dedup();
        record.links = links;
        let relative = relative_path(&record.topic, &record.id);
        if dry_run {
            written.push(relative);
            continue;
        }
        let encoded = encode_record(&record)?;
        store
            .write_record(&relative, &encoded)
            .map_err(markdown_error)?;
        verify_written(store, &relative, &record)?;
        index_upsert(store, conn, &record, &relative)?;
        written.push(relative);
    }
    if !dry_run && !written.is_empty() {
        let repos = conn
            .prepare("SELECT DISTINCT repo_root FROM memory_index WHERE path IS NOT NULL")
            .ok()
            .and_then(|mut stmt| {
                stmt.query_map([], |row| row.get::<_, String>(0))
                    .ok()
                    .map(|rows| rows.filter_map(std::result::Result::ok).collect::<Vec<_>>())
            })
            .unwrap_or_default();
        for repo_root in repos {
            refresh_supersession_index(conn, &repo_root)?;
        }
    }
    Ok(written)
}

fn legacy_supersession_links(conn: &Connection, new_memory_id: &str) -> Result<Vec<MemoryLink>> {
    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memory_legacy_supersessions'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare("SELECT old_memory_id FROM memory_legacy_supersessions WHERE new_memory_id = ?1")
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([new_memory_id], |row| row.get::<_, String>(0))
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(rows
        .into_iter()
        .map(|id| MemoryLink {
            id,
            relation: RELATION_SUPERSEDES.to_owned(),
        })
        .collect())
}

fn verify_written(store: &MarkdownStore, relative: &Path, expected: &MemoryRecord) -> Result<()> {
    let raw = store.read_record(relative).map_err(markdown_error)?;
    let decoded = decode_record(relative, &raw, &expected.repo_root)?;
    if decoded.id != expected.id
        || decoded.body != expected.body
        || decoded.scope != expected.scope
        || decoded.importance != expected.importance
        || decoded.topic != expected.topic
        || decoded.title != expected.title
        || decoded.tags != expected.tags
    {
        return Err(AtlasError::Other(format!(
            "markdown verification failed for {}",
            relative.display()
        )));
    }
    Ok(())
}
