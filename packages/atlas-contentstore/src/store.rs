//! SQLite-backed content store for Atlas artifact persistence.
//!
//! Stores large command outputs, tool results, and context payloads so they
//! can be retrieved by session or keyword search without growing the prompt
//! context window.
//!
//! Uses `.atlas/context.db`, kept strictly separate from the graph database.

use rusqlite::{Connection, OpenFlags, params};
use time::OffsetDateTime;
use tracing::{debug, info};

use atlas_core::{AtlasError, Result};

use crate::chunking::chunk_text;
use crate::migrations::MIGRATIONS;

// Size thresholds for compression routing.
const SMALL_OUTPUT_BYTES: usize = 512;
const PREVIEW_THRESHOLD_BYTES: usize = 4096;

/// Metadata describing an artifact being stored.
#[derive(Debug, Clone)]
pub struct SourceMeta {
    /// Caller-assigned stable id (e.g. a UUID or content hash).
    pub id: String,
    /// Session this artifact belongs to (optional).
    pub session_id: Option<String>,
    /// Category label: `"review_context"`, `"impact_result"`, `"command_output"`, etc.
    pub source_type: String,
    /// Human-readable label for display and retrieval.
    pub label: String,
    /// Repo root at time of indexing (optional, for scoped queries).
    pub repo_root: Option<String>,
}

/// Filters for content search.
#[derive(Debug, Default, Clone)]
pub struct SearchFilters {
    pub session_id: Option<String>,
    pub source_type: Option<String>,
    pub repo_root: Option<String>,
}

/// A single chunk result returned from search.
#[derive(Debug, Clone)]
pub struct ChunkResult {
    pub source_id: String,
    pub chunk_index: usize,
    pub title: Option<String>,
    pub content: String,
    pub content_type: String,
}

/// Routing decision for an artifact based on size.
#[derive(Debug, Clone, PartialEq)]
pub enum OutputRouting {
    /// Small enough to return directly; not indexed.
    Raw(String),
    /// Indexed; preview (first N chars) returned inline.
    Preview { source_id: String, preview: String },
    /// Indexed; only a pointer (source_id) returned.
    Pointer { source_id: String },
}

/// SQLite-backed content store.
pub struct ContentStore {
    conn: Connection,
}

impl ContentStore {
    /// Open (or create) the content store database at `path`.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        let store = Self { conn };
        store.apply_pragmas()?;
        Ok(store)
    }

    fn apply_pragmas(&self) -> Result<()> {
        // Some pragmas return rows in rusqlite; drain them to avoid "Execute returned results".
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        for sql in &[
            "PRAGMA journal_mode=WAL",
            "PRAGMA synchronous=NORMAL",
            "PRAGMA foreign_keys=ON",
            "PRAGMA busy_timeout=5000",
        ] {
            let mut stmt = self.conn.prepare(sql).map_err(db_err)?;
            let mut rows = stmt.query([]).map_err(db_err)?;
            while rows.next().map_err(db_err)?.is_some() {}
        }
        Ok(())
    }

    /// Apply any pending schema migrations.
    pub fn migrate(&mut self) -> Result<()> {
        // Ensure schema_version table exists before reading from it.
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_version (version INTEGER NOT NULL);",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        // Insert initial version if table is empty.
        self.conn
            .execute(
                "INSERT OR IGNORE INTO schema_version (version) VALUES (0)",
                [],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let current: i32 = self
            .conn
            .query_row("SELECT version FROM schema_version LIMIT 1", [], |r| r.get(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        for m in MIGRATIONS {
            if m.version > current {
                info!("applying content store migration v{}", m.version);
                self.conn
                    .execute_batch(m.sql)
                    .map_err(|e| AtlasError::Db(format!("migration {}: {e}", m.version)))?;
                self.conn
                    .execute("UPDATE schema_version SET version = ?1", [m.version])
                    .map_err(|e| AtlasError::Db(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Route output: if small, return raw; if large, index it and return routing decision.
    ///
    /// `content_type` should be `"text/plain"`, `"text/markdown"`, or
    /// `"application/json"`.
    pub fn route_output(
        &mut self,
        meta: SourceMeta,
        raw_text: &str,
        content_type: &str,
    ) -> Result<OutputRouting> {
        if raw_text.len() <= SMALL_OUTPUT_BYTES {
            debug!("content routing: small output, returning raw");
            return Ok(OutputRouting::Raw(raw_text.to_string()));
        }

        let source_id = meta.id.clone();
        self.index_artifact(meta, raw_text, content_type)?;

        if raw_text.len() <= PREVIEW_THRESHOLD_BYTES {
            let preview: String = raw_text.chars().take(512).collect();
            debug!("content routing: medium output, returning preview");
            Ok(OutputRouting::Preview { source_id, preview })
        } else {
            debug!("content routing: large output, returning pointer");
            Ok(OutputRouting::Pointer { source_id })
        }
    }

    /// Index a raw artifact: chunk it and persist to `sources` + `chunks`.
    pub fn index_artifact(
        &mut self,
        meta: SourceMeta,
        raw_text: &str,
        content_type: &str,
    ) -> Result<()> {
        let now = format_now();
        let chunks = chunk_text(raw_text, content_type);

        let tx = self.conn.transaction().map_err(|e| AtlasError::Db(e.to_string()))?;

        tx.execute(
            "INSERT OR REPLACE INTO sources (id, session_id, source_type, label, repo_root, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                meta.id,
                meta.session_id,
                meta.source_type,
                meta.label,
                meta.repo_root,
                now,
            ],
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        // Delete any pre-existing chunks for this source (idempotent re-index).
        // Also remove from FTS before deleting rows.
        {
            let fts_cleanup = format!(
                "INSERT INTO chunks_fts(chunks_fts, rowid, title, content, source_id, content_type)
                 SELECT 'delete', id, title, content, source_id, content_type
                 FROM chunks WHERE source_id = '{}'",
                meta.id.replace('\'', "''")
            );
            tx.execute_batch(&fts_cleanup)
                .map_err(|e| AtlasError::Db(e.to_string()))?;
        }
        tx.execute("DELETE FROM chunks WHERE source_id = ?1", params![meta.id])
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        for chunk in &chunks {
            let metadata = serde_json::json!({}).to_string();
            tx.execute(
                "INSERT INTO chunks (source_id, content, content_type, chunk_index, title, metadata_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    meta.id,
                    chunk.content,
                    chunk.content_type,
                    chunk.chunk_index as i64,
                    chunk.title,
                    metadata,
                    now,
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

            // Retrieve the rowid of the just-inserted chunk and update FTS.
            let rowid: i64 = tx
                .query_row(
                    "SELECT id FROM chunks WHERE source_id = ?1 AND chunk_index = ?2",
                    params![meta.id, chunk.chunk_index as i64],
                    |r| r.get(0),
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?;

            tx.execute(
                "INSERT INTO chunks_fts(rowid, title, content, source_id, content_type)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![rowid, chunk.title, chunk.content, meta.id, chunk.content_type],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        }

        tx.commit().map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Keyword search over indexed chunks using FTS5.
    pub fn search(&self, query: &str, filters: &SearchFilters) -> Result<Vec<ChunkResult>> {
        let fts_query = fts5_escape(query);

        // Build the JOIN with optional filters.
        let mut where_parts = vec!["chunks_fts MATCH ?1".to_string()];
        let mut extra_params: Vec<String> = Vec::new();

        if let Some(ref sid) = filters.session_id {
            extra_params.push(sid.clone());
            where_parts.push(format!("s.session_id = ?{}", extra_params.len() + 1));
        }
        if let Some(ref st) = filters.source_type {
            extra_params.push(st.clone());
            where_parts.push(format!("s.source_type = ?{}", extra_params.len() + 1));
        }
        if let Some(ref rr) = filters.repo_root {
            extra_params.push(rr.clone());
            where_parts.push(format!("s.repo_root = ?{}", extra_params.len() + 1));
        }

        let sql = format!(
            "SELECT c.source_id, c.chunk_index, c.title, c.content, c.content_type
             FROM chunks_fts
             JOIN chunks c ON chunks_fts.rowid = c.id
             JOIN sources s ON c.source_id = s.id
             WHERE {}
             ORDER BY rank
             LIMIT 50",
            where_parts.join(" AND ")
        );

        let mut stmt = self.conn.prepare(&sql).map_err(|e| AtlasError::Db(e.to_string()))?;

        // Bind positional params.
        let mut rows = stmt
            .query(rusqlite::params_from_iter(
                std::iter::once(fts_query).chain(extra_params),
            ))
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().map_err(|e| AtlasError::Db(e.to_string()))? {
            results.push(ChunkResult {
                source_id: row.get(0).map_err(|e| AtlasError::Db(e.to_string()))?,
                chunk_index: row.get::<_, i64>(1).map_err(|e| AtlasError::Db(e.to_string()))? as usize,
                title: row.get(2).map_err(|e| AtlasError::Db(e.to_string()))?,
                content: row.get(3).map_err(|e| AtlasError::Db(e.to_string()))?,
                content_type: row.get(4).map_err(|e| AtlasError::Db(e.to_string()))?,
            });
        }
        Ok(results)
    }

    /// Retrieve source metadata by id.
    pub fn get_source(&self, source_id: &str) -> Result<Option<SourceRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, session_id, source_type, label, repo_root, created_at
                 FROM sources WHERE id = ?1",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut rows = stmt
            .query(params![source_id])
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        if let Some(row) = rows.next().map_err(|e| AtlasError::Db(e.to_string()))? {
            Ok(Some(SourceRow {
                id: row.get(0).map_err(|e| AtlasError::Db(e.to_string()))?,
                session_id: row.get(1).map_err(|e| AtlasError::Db(e.to_string()))?,
                source_type: row.get(2).map_err(|e| AtlasError::Db(e.to_string()))?,
                label: row.get(3).map_err(|e| AtlasError::Db(e.to_string()))?,
                repo_root: row.get(4).map_err(|e| AtlasError::Db(e.to_string()))?,
                created_at: row.get(5).map_err(|e| AtlasError::Db(e.to_string()))?,
            }))
        } else {
            Ok(None)
        }
    }

    /// Retrieve all chunks for a given source, ordered by chunk_index.
    pub fn get_chunks(&self, source_id: &str) -> Result<Vec<ChunkResult>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT source_id, chunk_index, title, content, content_type
                 FROM chunks WHERE source_id = ?1
                 ORDER BY chunk_index",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut rows = stmt
            .query(params![source_id])
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|e| AtlasError::Db(e.to_string()))? {
            out.push(ChunkResult {
                source_id: row.get(0).map_err(|e| AtlasError::Db(e.to_string()))?,
                chunk_index: row.get::<_, i64>(1).map_err(|e| AtlasError::Db(e.to_string()))? as usize,
                title: row.get(2).map_err(|e| AtlasError::Db(e.to_string()))?,
                content: row.get(3).map_err(|e| AtlasError::Db(e.to_string()))?,
                content_type: row.get(4).map_err(|e| AtlasError::Db(e.to_string()))?,
            });
        }
        Ok(out)
    }

    /// Delete a source and its chunks (cascade).
    pub fn delete_source(&mut self, source_id: &str) -> Result<()> {
        // Remove FTS rows before deleting chunks.
        let fts_cleanup = format!(
            "INSERT INTO chunks_fts(chunks_fts, rowid, title, content, source_id, content_type)
             SELECT 'delete', id, title, content, source_id, content_type
             FROM chunks WHERE source_id = '{}'",
            source_id.replace('\'', "''")
        );
        self.conn
            .execute_batch(&fts_cleanup)
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        self.conn
            .execute("DELETE FROM sources WHERE id = ?1", params![source_id])
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Cleanup sources older than `keep_days` days (retention policy).
    pub fn cleanup(&mut self, keep_days: u32) -> Result<usize> {
        let cutoff = format_days_ago(keep_days);
        let count = self
            .conn
            .execute(
                "DELETE FROM sources WHERE created_at < ?1",
                params![cutoff],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(count)
    }
}

/// Retrieved source row from the store.
#[derive(Debug, Clone)]
pub struct SourceRow {
    pub id: String,
    pub session_id: Option<String>,
    pub source_type: String,
    pub label: String,
    pub repo_root: Option<String>,
    pub created_at: String,
}

fn format_now() -> String {
    OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn format_days_ago(days: u32) -> String {
    let duration = time::Duration::days(days as i64);
    let ts = OffsetDateTime::now_utc() - duration;
    ts.format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".to_string())
}

fn fts5_escape(input: &str) -> String {
    let has_special = input
        .chars()
        .any(|c| matches!(c, '"' | '(' | ')' | '^' | '-' | '*'));
    if has_special {
        format!("\"{}\"", input.replace('"', "\"\""))
    } else {
        input.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn open_store() -> ContentStore {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_str().unwrap().to_string();
        // keep file alive via leak — acceptable in tests
        std::mem::forget(file);
        let mut store = ContentStore::open(&path).unwrap();
        store.migrate().unwrap();
        store
    }

    fn meta(id: &str) -> SourceMeta {
        SourceMeta {
            id: id.to_string(),
            session_id: Some("sess1".into()),
            source_type: "review_context".into(),
            label: "test artifact".into(),
            repo_root: Some("/repo".into()),
        }
    }

    #[test]
    fn index_and_retrieve_by_source_id() {
        let mut store = open_store();
        store.index_artifact(meta("src-1"), "hello world", "text/plain").unwrap();
        let src = store.get_source("src-1").unwrap();
        assert!(src.is_some());
        let chunks = store.get_chunks("src-1").unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn delete_source_removes_chunks() {
        let mut store = open_store();
        store.index_artifact(meta("src-2"), "some content here", "text/plain").unwrap();
        store.delete_source("src-2").unwrap();
        let src = store.get_source("src-2").unwrap();
        assert!(src.is_none());
        let chunks = store.get_chunks("src-2").unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn routing_small_returns_raw() {
        let mut store = open_store();
        let routing = store
            .route_output(meta("src-3"), "tiny output", "text/plain")
            .unwrap();
        assert!(matches!(routing, OutputRouting::Raw(_)));
    }

    #[test]
    fn routing_large_returns_pointer() {
        let mut store = open_store();
        let big = "word ".repeat(2000);
        let routing = store
            .route_output(meta("src-4"), &big, "text/plain")
            .unwrap();
        assert!(matches!(routing, OutputRouting::Pointer { .. }));
    }

    #[test]
    fn search_returns_indexed_chunk() {
        let mut store = open_store();
        store
            .index_artifact(meta("src-5"), "the quick brown fox", "text/plain")
            .unwrap();
        let results = store
            .search("quick", &SearchFilters::default())
            .unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("quick"));
    }

    #[test]
    fn idempotent_reindex_replaces_chunks() {
        let mut store = open_store();
        store.index_artifact(meta("src-6"), "version one content here", "text/plain").unwrap();
        let before = store.get_chunks("src-6").unwrap().len();
        store.index_artifact(meta("src-6"), "version two different content entirely", "text/plain").unwrap();
        let after = store.get_chunks("src-6").unwrap();
        // Chunks from v1 should not stack; v2 content must appear.
        assert!(after.iter().any(|c| c.content.contains("version two")));
        // chunk count should not double.
        assert!(after.len() <= before + 5);
    }
}
