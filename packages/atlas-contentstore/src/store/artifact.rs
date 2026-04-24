use std::collections::HashSet;

use rusqlite::{OptionalExtension, params};
use tracing::{debug, warn};

use atlas_core::{AtlasError, Result};

use crate::chunking::chunk_text;

use super::util::{extract_vocab_terms, format_days_ago, format_now};
use super::{
    ChunkResult, ContentStore, IndexRunStats, OutputRouting, OversizedPolicy, RoutingStats,
    SourceMeta, SourceRow,
};

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

    /// Return a snapshot of the in-process run stats (chunk throughput, batch flushes, etc.).
    pub fn run_stats(&self) -> IndexRunStats {
        self.run_stats.clone()
    }

    /// Reset run-level counters. Call before starting a new indexing run.
    pub fn reset_run_stats(&mut self) {
        self.run_stats = IndexRunStats::default();
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
    ///
    /// Enforces `max_chunks_per_file` and `max_chunks_per_index_run` guardrails via
    /// the configured [`OversizedPolicy`]. Chunks are flushed in batches of
    /// `retrieval_batch_size`; each flush increments [`IndexRunStats::batch_flush_count`].
    pub fn index_artifact(
        &mut self,
        meta: SourceMeta,
        raw_text: &str,
        content_type: &str,
    ) -> Result<()> {
        let now = format_now();
        let mut chunks = chunk_text(&meta.id, raw_text, content_type);

        // --- per-file chunk cap ---
        let max_per_file = self.config.max_chunks_per_file;
        if chunks.len() > max_per_file {
            match self.config.oversized_policy {
                OversizedPolicy::FailFast => {
                    return Err(AtlasError::ChunkCapExceeded(format!(
                        "source '{}' produced {} chunks, max_chunks_per_file={}",
                        meta.id,
                        chunks.len(),
                        max_per_file
                    )));
                }
                OversizedPolicy::PartialWithWarning => {
                    warn!(
                        source_id = %meta.id,
                        chunk_count = chunks.len(),
                        cap = max_per_file,
                        "chunk cap per file exceeded; truncating to cap (partial_with_warning)"
                    );
                    chunks.truncate(max_per_file);
                }
                OversizedPolicy::SkipFile => {
                    warn!(
                        source_id = %meta.id,
                        chunk_count = chunks.len(),
                        cap = max_per_file,
                        "chunk cap per file exceeded; skipping file (skip_file)"
                    );
                    return Ok(());
                }
            }
        }

        // --- per-run chunk cap ---
        let max_per_run = self.config.max_chunks_per_index_run;
        let remaining_run_capacity =
            max_per_run.saturating_sub(self.run_stats.chunks_this_run as usize);
        if chunks.len() > remaining_run_capacity {
            match self.config.oversized_policy {
                OversizedPolicy::FailFast => {
                    return Err(AtlasError::ChunkCapExceeded(format!(
                        "source '{}' would exceed max_chunks_per_index_run={} \
                         (run total so far={}, this file={})",
                        meta.id,
                        max_per_run,
                        self.run_stats.chunks_this_run,
                        chunks.len(),
                    )));
                }
                OversizedPolicy::PartialWithWarning => {
                    warn!(
                        source_id = %meta.id,
                        chunk_count = chunks.len(),
                        remaining_cap = remaining_run_capacity,
                        run_total = self.run_stats.chunks_this_run,
                        "run chunk cap would be exceeded; truncating this file's chunks (partial_with_warning)"
                    );
                    chunks.truncate(remaining_run_capacity);
                }
                OversizedPolicy::SkipFile => {
                    warn!(
                        source_id = %meta.id,
                        chunk_count = chunks.len(),
                        remaining_cap = remaining_run_capacity,
                        run_total = self.run_stats.chunks_this_run,
                        "run chunk cap would be exceeded; skipping file (skip_file)"
                    );
                    return Ok(());
                }
            }
        }

        // --- update run stats: buffering ---
        let buffered_bytes: u64 = chunks.iter().map(|c| c.content.len() as u64).sum();
        self.run_stats.buffered_chunk_count += chunks.len() as u64;
        self.run_stats.buffered_bytes += buffered_bytes;
        // staged_vector_bytes tracks bytes staged for embedding (all chunk bytes queued here)
        self.run_stats.staged_vector_bytes += buffered_bytes;

        debug!(
            source_id = %meta.id,
            chunk_count = chunks.len(),
            buffered_bytes,
            "indexing artifact"
        );

        // --- upsert source row and remove stale chunks (single transaction) ---
        let existing_chunk_ids: HashSet<String> = {
            let tx = self
                .conn
                .transaction()
                .map_err(|e| AtlasError::Db(e.to_string()))?;

            tx.execute(
                "INSERT OR REPLACE INTO sources (
                     id, session_id, source_type, label, repo_root, identity_kind, identity_value, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    meta.id,
                    meta.session_id,
                    meta.source_type,
                    meta.label,
                    meta.repo_root,
                    meta.identity_kind,
                    meta.identity_value,
                    now,
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

            let existing: HashSet<String> = {
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
                chunks.iter().map(|c| c.chunk_id.clone()).collect();
            let removed_ids: Vec<&String> = existing
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

            tx.commit().map_err(|e| AtlasError::Db(e.to_string()))?;
            existing
        };

        // --- flush chunk batches ---
        let batch_size = self.config.retrieval_batch_size.max(1);
        let mut chunks_inserted: i64 = 0;
        let mut chunks_reused: i64 = 0;

        for batch in chunks.chunks(batch_size) {
            let tx = self
                .conn
                .transaction()
                .map_err(|e| AtlasError::Db(e.to_string()))?;

            for chunk in batch {
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

            tx.commit().map_err(|e| AtlasError::Db(e.to_string()))?;
            self.run_stats.batch_flush_count += 1;

            debug!(
                source_id = %meta.id,
                batch_chunks = batch.len(),
                batch_flush_count = self.run_stats.batch_flush_count,
                "flushed chunk batch"
            );
        }

        // --- vocabulary terms ---
        let tx = self
            .conn
            .transaction()
            .map_err(|e| AtlasError::Db(e.to_string()))?;
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

        // --- update per-run counters ---
        self.run_stats.chunks_this_run += chunks_inserted as u64;

        if let Some(ref repo_root) = meta.repo_root
            && let Err(e) =
                self.increment_index_counters_with_reuse(repo_root, chunks_inserted, chunks_reused)
        {
            debug!("index counter update failed (best-effort): {e}");
        }

        if let Some(limit) = self.config.max_db_bytes
            && let Err(e) = self.enforce_size_limit(limit)
        {
            debug!("content store size limit enforcement failed (best-effort): {e}");
        }

        Ok(())
    }

    /// Retrieve source metadata by id.
    pub fn get_source(&self, source_id: &str) -> Result<Option<SourceRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, session_id, source_type, label, repo_root, identity_kind, identity_value, created_at
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
                identity_kind: row.get(5).map_err(|e| AtlasError::Db(e.to_string()))?,
                identity_value: row.get(6).map_err(|e| AtlasError::Db(e.to_string()))?,
                created_at: row.get(7).map_err(|e| AtlasError::Db(e.to_string()))?,
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
