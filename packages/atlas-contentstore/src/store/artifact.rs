use std::collections::HashSet;

use rusqlite::{OptionalExtension, params};
use tracing::debug;

use atlas_core::{AtlasError, Result};

use crate::chunking::chunk_text;

use super::util::{extract_vocab_terms, format_days_ago, format_now};
use super::{ChunkResult, ContentStore, OutputRouting, RoutingStats, SourceMeta, SourceRow};

impl ContentStore {
    /// Route output: if small, return raw; if large, index it and return routing decision.
    pub fn route_output(
        &mut self,
        meta: SourceMeta,
        raw_text: &str,
        content_type: &str,
    ) -> Result<OutputRouting> {
        if raw_text.len() <= self.config.small_output_bytes {
            debug!("content routing: small output, returning raw");
            self.routing_stats.raw_count += 1;
            return Ok(OutputRouting::Raw(raw_text.to_string()));
        }

        let source_id = meta.id.clone();
        self.index_artifact(meta, raw_text, content_type)?;

        if let Some(limit) = self.config.max_db_bytes
            && let Err(e) = self.enforce_size_limit(limit)
        {
            debug!("content store size limit enforcement failed (best-effort): {e}");
        }

        if raw_text.len() <= self.config.preview_threshold_bytes {
            let preview: String = raw_text.chars().take(512).collect();
            debug!("content routing: medium output, returning preview");
            self.routing_stats.preview_count += 1;
            self.routing_stats.avoided_bytes += raw_text.len() as u64;
            Ok(OutputRouting::Preview { source_id, preview })
        } else {
            debug!("content routing: large output, returning pointer");
            self.routing_stats.pointer_count += 1;
            self.routing_stats.avoided_bytes += raw_text.len() as u64;
            Ok(OutputRouting::Pointer { source_id })
        }
    }

    /// Return a snapshot of the in-process routing counters.
    pub fn routing_stats(&self) -> RoutingStats {
        self.routing_stats.clone()
    }

    /// Prune the oldest sources until the content database is under `max_bytes`.
    pub fn enforce_size_limit(&mut self, max_bytes: u64) -> Result<usize> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let page_count: i64 = self
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .map_err(db_err)?;
        let page_size: i64 = self
            .conn
            .query_row("PRAGMA page_size", [], |r| r.get(0))
            .map_err(db_err)?;
        let db_bytes = (page_count * page_size) as u64;

        if db_bytes <= max_bytes {
            return Ok(0);
        }

        let mut removed = 0;
        loop {
            let current_page_count: i64 = self
                .conn
                .query_row("PRAGMA page_count", [], |r| r.get(0))
                .map_err(db_err)?;
            if (current_page_count * page_size) as u64 <= max_bytes {
                break;
            }
            let oldest: Option<String> = self
                .conn
                .query_row(
                    "SELECT id FROM sources ORDER BY created_at ASC LIMIT 1",
                    [],
                    |r| r.get::<_, String>(0),
                )
                .optional()
                .map_err(db_err)?;
            match oldest {
                Some(ref id) => {
                    self.delete_source(id)?;
                    removed += 1;
                }
                None => break,
            }
        }
        Ok(removed)
    }

    /// Index a raw artifact: chunk it and persist to `sources` + `chunks`.
    pub fn index_artifact(
        &mut self,
        meta: SourceMeta,
        raw_text: &str,
        content_type: &str,
    ) -> Result<()> {
        let now = format_now();
        let chunks = chunk_text(&meta.id, raw_text, content_type);

        let tx = self
            .conn
            .transaction()
            .map_err(|e| AtlasError::Db(e.to_string()))?;

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

        let existing_chunk_ids: HashSet<String> = {
            let mut stmt = tx
                .prepare("SELECT chunk_id FROM chunks WHERE source_id = ?1")
                .map_err(|e| AtlasError::Db(e.to_string()))?;
            let mut rows = stmt
                .query(params![meta.id])
                .map_err(|e| AtlasError::Db(e.to_string()))?;
            let mut ids = HashSet::new();
            while let Some(row) = rows.next().map_err(|e| AtlasError::Db(e.to_string()))? {
                let id: String = row.get(0).map_err(|e| AtlasError::Db(e.to_string()))?;
                ids.insert(id);
            }
            ids
        };

        let new_chunk_ids: HashSet<String> =
            chunks.iter().map(|chunk| chunk.chunk_id.clone()).collect();

        let removed_ids: Vec<&String> = existing_chunk_ids
            .iter()
            .filter(|id| !new_chunk_ids.contains(*id))
            .collect();

        for removed_id in &removed_ids {
            let safe_id = meta.id.replace('\'', "''");
            let safe_chunk_id = removed_id.replace('\'', "''");
            let fts_cleanup = format!(
                "INSERT INTO chunks_fts(chunks_fts, rowid, title, content, source_id, content_type)
                 SELECT 'delete', id, title, content, source_id, content_type
                 FROM chunks WHERE source_id = '{safe_id}' AND chunk_id = '{safe_chunk_id}'"
            );
            tx.execute_batch(&fts_cleanup)
                .map_err(|e| AtlasError::Db(e.to_string()))?;
            let trigram_cleanup = format!(
                "INSERT INTO chunks_trigram(chunks_trigram, rowid, title, content, source_id, content_type)
                 SELECT 'delete', id, title, content, source_id, content_type
                 FROM chunks WHERE source_id = '{safe_id}' AND chunk_id = '{safe_chunk_id}'"
            );
            tx.execute_batch(&trigram_cleanup)
                .map_err(|e| AtlasError::Db(e.to_string()))?;
            tx.execute(
                "DELETE FROM chunks WHERE source_id = ?1 AND chunk_id = ?2",
                params![meta.id, removed_id],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        }

        let mut chunks_inserted: i64 = 0;
        let mut chunks_reused: i64 = 0;

        for chunk in &chunks {
            if existing_chunk_ids.contains(&chunk.chunk_id) {
                tx.execute(
                    "UPDATE chunks SET chunk_index = ?1 WHERE source_id = ?2 AND chunk_id = ?3",
                    params![chunk.chunk_index as i64, meta.id, chunk.chunk_id],
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?;
                chunks_reused += 1;
                continue;
            }

            let metadata = serde_json::json!({}).to_string();
            tx.execute(
                "INSERT INTO chunks (source_id, chunk_id, content, content_type, chunk_index, title, metadata_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    meta.id,
                    chunk.chunk_id,
                    chunk.content,
                    chunk.content_type,
                    chunk.chunk_index as i64,
                    chunk.title,
                    metadata,
                    now,
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

            let rowid: i64 = tx
                .query_row(
                    "SELECT id FROM chunks WHERE source_id = ?1 AND chunk_id = ?2",
                    params![meta.id, chunk.chunk_id],
                    |r| r.get(0),
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?;

            tx.execute(
                "INSERT INTO chunks_fts(rowid, title, content, source_id, content_type)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    rowid,
                    chunk.title,
                    chunk.content,
                    meta.id,
                    chunk.content_type
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

            tx.execute(
                "INSERT INTO chunks_trigram(rowid, title, content, source_id, content_type)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    rowid,
                    chunk.title,
                    chunk.content,
                    meta.id,
                    chunk.content_type
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

            chunks_inserted += 1;
        }

        let terms = extract_vocab_terms(raw_text);
        for term in terms {
            tx.execute(
                "INSERT INTO vocabulary (term, doc_freq) VALUES (?1, 1)
                 ON CONFLICT(term) DO UPDATE SET doc_freq = doc_freq + 1",
                params![term],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        }

        tx.commit().map_err(|e| AtlasError::Db(e.to_string()))?;

        if let Some(ref repo_root) = meta.repo_root
            && let Err(e) =
                self.increment_index_counters_with_reuse(repo_root, chunks_inserted, chunks_reused)
        {
            debug!("index counter update failed (best-effort): {e}");
        }

        Ok(())
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

    /// Retrieve all chunks for given source, ordered by chunk_index.
    pub fn get_chunks(&self, source_id: &str) -> Result<Vec<ChunkResult>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT source_id, chunk_id, chunk_index, title, content, content_type
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
                chunk_id: row.get(1).map_err(|e| AtlasError::Db(e.to_string()))?,
                chunk_index: row
                    .get::<_, i64>(2)
                    .map_err(|e| AtlasError::Db(e.to_string()))?
                    as usize,
                title: row.get(3).map_err(|e| AtlasError::Db(e.to_string()))?,
                content: row.get(4).map_err(|e| AtlasError::Db(e.to_string()))?,
                content_type: row.get(5).map_err(|e| AtlasError::Db(e.to_string()))?,
            });
        }
        Ok(out)
    }

    /// Delete a source and its chunks (cascade).
    pub fn delete_source(&mut self, source_id: &str) -> Result<()> {
        let safe_id = source_id.replace('\'', "''");
        let fts_cleanup = format!(
            "INSERT INTO chunks_fts(chunks_fts, rowid, title, content, source_id, content_type)
             SELECT 'delete', id, title, content, source_id, content_type
             FROM chunks WHERE source_id = '{safe_id}'"
        );
        self.conn
            .execute_batch(&fts_cleanup)
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let trigram_cleanup = format!(
            "INSERT INTO chunks_trigram(chunks_trigram, rowid, title, content, source_id, content_type)
             SELECT 'delete', id, title, content, source_id, content_type
             FROM chunks WHERE source_id = '{safe_id}'"
        );
        self.conn
            .execute_batch(&trigram_cleanup)
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        self.conn
            .execute("DELETE FROM sources WHERE id = ?1", params![source_id])
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Cleanup sources older than `keep_days` days.
    pub fn cleanup(&mut self, keep_days: u32) -> Result<usize> {
        let cutoff = format_days_ago(keep_days);
        let count = self
            .conn
            .execute("DELETE FROM sources WHERE created_at < ?1", params![cutoff])
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(count)
    }

    /// Return `(source_count, chunk_count)` for given session or globally.
    pub fn stats(&self, session_id: Option<&str>) -> Result<(usize, usize)> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let (src, chk) = if let Some(sid) = session_id {
            let src: i64 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM sources WHERE session_id = ?1",
                    params![sid],
                    |r| r.get(0),
                )
                .map_err(db_err)?;
            let chk: i64 = self
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM chunks
                     WHERE source_id IN (SELECT id FROM sources WHERE session_id = ?1)",
                    params![sid],
                    |r| r.get(0),
                )
                .map_err(db_err)?;
            (src, chk)
        } else {
            let src: i64 = self
                .conn
                .query_row("SELECT COUNT(*) FROM sources", [], |r| r.get(0))
                .map_err(db_err)?;
            let chk: i64 = self
                .conn
                .query_row("SELECT COUNT(*) FROM chunks", [], |r| r.get(0))
                .map_err(db_err)?;
            (src, chk)
        };
        Ok((src as usize, chk as usize))
    }

    /// Delete all sources belonging to `session_id`.
    pub fn delete_session_sources(&mut self, session_id: &str) -> Result<usize> {
        let safe_sid = session_id.replace('\'', "''");
        let fts_cleanup = format!(
            "INSERT INTO chunks_fts(chunks_fts, rowid, title, content, source_id, content_type)
             SELECT 'delete', c.id, c.title, c.content, c.source_id, c.content_type
             FROM chunks c
             JOIN sources s ON c.source_id = s.id
             WHERE s.session_id = '{safe_sid}'"
        );
        self.conn
            .execute_batch(&fts_cleanup)
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let trigram_cleanup = format!(
            "INSERT INTO chunks_trigram(chunks_trigram, rowid, title, content, source_id, content_type)
             SELECT 'delete', c.id, c.title, c.content, c.source_id, c.content_type
             FROM chunks c
             JOIN sources s ON c.source_id = s.id
             WHERE s.session_id = '{safe_sid}'"
        );
        self.conn
            .execute_batch(&trigram_cleanup)
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let count = self
            .conn
            .execute(
                "DELETE FROM sources WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(count)
    }
}
