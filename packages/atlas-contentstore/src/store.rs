//! SQLite-backed content store for Atlas artifact persistence.
//!
//! Stores large command outputs, tool results, and context payloads so they
//! can be retrieved by session or keyword search without growing the prompt
//! context window.
//!
//! Uses `.atlas/context.db`, kept strictly separate from the graph database.
//!
//! # Routing thresholds (configurable via `ContentStoreConfig`)
//!
//! | Range                              | Action                                |
//! |------------------------------------|---------------------------------------|
//! | `<= small_output_bytes` (512 B)    | Return raw output; do not index.      |
//! | `<= preview_threshold_bytes` (4 KB)| Index + return first 512 chars inline.|
//! | `> preview_threshold_bytes`        | Index + return source_id pointer only.|
//!
//! All three thresholds are configurable so callers with different context
//! budgets can tune them without recompiling.

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use tracing::{debug, info};

use atlas_core::{AtlasError, Result};

use crate::chunking::chunk_text;
use crate::migrations::MIGRATIONS;

// ── Default size thresholds ────────────────────────────────────────────────
/// Below this → raw pass-through, no indexing.
const DEFAULT_SMALL_OUTPUT_BYTES: usize = 512;
/// Below this → index + return preview; above → pointer only.
const DEFAULT_PREVIEW_THRESHOLD_BYTES: usize = 4096;
/// Minimum FTS result count before trigram fallback is attempted.
const DEFAULT_FALLBACK_MIN_RESULTS: usize = 3;
/// RRF constant k (typical value 60 from the original paper).
const RRF_K: f64 = 60.0;

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

/// Configurable size thresholds for compression routing.
///
/// All sizes are in bytes. The documented defaults are:
/// - `small_output_bytes` = 512 B — below this, output is returned raw.
/// - `preview_threshold_bytes` = 4096 B — below this, a 512-char preview is returned.
/// - `fallback_min_results` = 3 — below this many FTS results, trigram fallback fires.
/// - `max_db_bytes` = `None` — no size limit unless set.
#[derive(Debug, Clone)]
pub struct ContentStoreConfig {
    /// Outputs at or below this size are returned raw without indexing.
    pub small_output_bytes: usize,
    /// Outputs above this size return only a pointer (source_id) rather than a preview.
    pub preview_threshold_bytes: usize,
    /// Minimum number of FTS hits before `search_with_fallback` skips trigram search.
    pub fallback_min_results: usize,
    /// When set, oldest sources are pruned after each index operation to keep the
    /// content database below this approximate byte limit.  `None` disables enforcement.
    pub max_db_bytes: Option<u64>,
}

impl Default for ContentStoreConfig {
    fn default() -> Self {
        Self {
            small_output_bytes: DEFAULT_SMALL_OUTPUT_BYTES,
            preview_threshold_bytes: DEFAULT_PREVIEW_THRESHOLD_BYTES,
            fallback_min_results: DEFAULT_FALLBACK_MIN_RESULTS,
            max_db_bytes: None,
        }
    }
}

/// In-process counters tracking how `route_output` has dispatched artifacts.
///
/// Counts and byte totals accumulate over the lifetime of the `ContentStore`
/// instance and are not persisted to SQLite.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoutingStats {
    /// Number of outputs returned raw (not indexed).
    pub raw_count: u64,
    /// Number of outputs indexed and returned with a preview.
    pub preview_count: u64,
    /// Number of outputs indexed and returned as a pointer only.
    pub pointer_count: u64,
    /// Total bytes of output that were routed as preview or pointer (i.e. kept
    /// out of the prompt context window).
    pub avoided_bytes: u64,
}

/// SQLite-backed content store.
pub struct ContentStore {
    conn: Connection,
    config: ContentStoreConfig,
    routing_stats: RoutingStats,
}

impl ContentStore {
    /// Open (or create) the content store database at `path` with default config.
    pub fn open(path: &str) -> Result<Self> {
        Self::open_with_config(path, ContentStoreConfig::default())
    }

    /// Open (or create) the content store database at `path` with custom config.
    pub fn open_with_config(path: &str, config: ContentStoreConfig) -> Result<Self> {
        match Self::try_open(path, config.clone()) {
            Ok(store) => Ok(store),
            Err(e) => {
                if is_corruption_error(&e) {
                    quarantine_db(path);
                }
                Err(e)
            }
        }
    }

    fn try_open(path: &str, config: ContentStoreConfig) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        let store = Self {
            conn,
            config,
            routing_stats: RoutingStats::default(),
        };
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
        // Bootstrap metadata table so version tracking matches graph/session stores.
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS metadata (
                     key   TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let current: i32 = self
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap_or(0);

        for m in MIGRATIONS {
            if m.version > current {
                info!("applying content store migration v{}", m.version);
                self.conn
                    .execute_batch(m.sql)
                    .map_err(|e| AtlasError::Db(format!("migration {}: {e}", m.version)))?;
                self.conn
                    .execute(
                        "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', ?1)",
                        params![m.version.to_string()],
                    )
                    .map_err(|e| AtlasError::Db(e.to_string()))?;
            }
        }
        Ok(())
    }

    /// Route output: if small, return raw; if large, index it and return routing decision.
    ///
    /// Routing counters in `routing_stats()` are incremented on each call.
    /// If `max_db_bytes` is configured, old sources are pruned after indexing.
    ///
    /// `content_type` should be `"text/plain"`, `"text/markdown"`, or
    /// `"application/json"`.
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

        // Enforce size limit after each index operation (best-effort: errors logged, not propagated).
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
    ///
    /// Estimates DB size via `page_count * page_size` SQLite pragmas.
    /// Best-effort: returns immediately if the estimate cannot be read.
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

        // Delete oldest sources one by one until under the limit.
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
    /// Also populates `chunks_fts`, `chunks_trigram`, and `vocabulary`.
    pub fn index_artifact(
        &mut self,
        meta: SourceMeta,
        raw_text: &str,
        content_type: &str,
    ) -> Result<()> {
        let now = format_now();
        let chunks = chunk_text(raw_text, content_type);

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

        // Delete any pre-existing chunks for this source (idempotent re-index).
        // Remove from both FTS tables before deleting rows.
        let safe_id = meta.id.replace('\'', "''");
        {
            let fts_cleanup = format!(
                "INSERT INTO chunks_fts(chunks_fts, rowid, title, content, source_id, content_type)
                 SELECT 'delete', id, title, content, source_id, content_type
                 FROM chunks WHERE source_id = '{safe_id}'"
            );
            tx.execute_batch(&fts_cleanup)
                .map_err(|e| AtlasError::Db(e.to_string()))?;
        }
        {
            let trigram_cleanup = format!(
                "INSERT INTO chunks_trigram(chunks_trigram, rowid, title, content, source_id, content_type)
                 SELECT 'delete', id, title, content, source_id, content_type
                 FROM chunks WHERE source_id = '{safe_id}'"
            );
            tx.execute_batch(&trigram_cleanup)
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

            // Retrieve the rowid of the just-inserted chunk and update both FTS tables.
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
        }

        // Populate vocabulary table with unique terms from this artifact.
        // Vocabulary accumulates across all indexed sources for fuzzy correction.
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

        // Update per-repo index counters so callers can track indexing progress
        // without requiring an explicit begin_indexing/finish_indexing lifecycle.
        if let Some(ref rr) = meta.repo_root {
            // Best-effort: counter update failure does not fail the index operation.
            if let Err(e) = self.increment_index_counters(rr, chunks.len() as i64) {
                debug!("index counter update failed (best-effort): {e}");
            }
        }

        Ok(())
    }

    /// Keyword search over indexed chunks using FTS5 with BM25 title weighting.
    ///
    /// Title matches are weighted 10× higher than body content matches via
    /// `bm25(chunks_fts, 10.0, 1.0)`.  Results are ordered by descending score
    /// (lower `bm25()` value = worse; we negate to get descending).
    pub fn search(&self, query: &str, filters: &SearchFilters) -> Result<Vec<ChunkResult>> {
        let fts_query = fts5_escape(query);

        // Build the JOIN with optional filters.
        // bm25(chunks_fts, 10.0, 1.0): weight title column 10× over content.
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
             ORDER BY bm25(chunks_fts, 10.0, 1.0)
             LIMIT 50",
            where_parts.join(" AND ")
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| AtlasError::Db(e.to_string()))?;

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
                chunk_index: row
                    .get::<_, i64>(1)
                    .map_err(|e| AtlasError::Db(e.to_string()))?
                    as usize,
                title: row.get(2).map_err(|e| AtlasError::Db(e.to_string()))?,
                content: row.get(3).map_err(|e| AtlasError::Db(e.to_string()))?,
                content_type: row.get(4).map_err(|e| AtlasError::Db(e.to_string()))?,
            });
        }
        Ok(results)
    }

    /// Trigram search over indexed chunks using `chunks_trigram`.
    ///
    /// Enables substring and typo-tolerant matching.  Results are ordered by
    /// descending BM25 score with 10× title weighting, same as `search()`.
    fn search_trigram(&self, query: &str, filters: &SearchFilters) -> Result<Vec<ChunkResult>> {
        // Trigram MATCH expects raw terms; no FTS5 quoting needed.
        let trigram_query = query.to_string();

        let mut where_parts = vec!["chunks_trigram MATCH ?1".to_string()];
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
             FROM chunks_trigram
             JOIN chunks c ON chunks_trigram.rowid = c.id
             JOIN sources s ON c.source_id = s.id
             WHERE {}
             ORDER BY bm25(chunks_trigram, 10.0, 1.0)
             LIMIT 50",
            where_parts.join(" AND ")
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut rows = stmt
            .query(rusqlite::params_from_iter(
                std::iter::once(trigram_query).chain(extra_params),
            ))
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().map_err(|e| AtlasError::Db(e.to_string()))? {
            results.push(ChunkResult {
                source_id: row.get(0).map_err(|e| AtlasError::Db(e.to_string()))?,
                chunk_index: row
                    .get::<_, i64>(1)
                    .map_err(|e| AtlasError::Db(e.to_string()))?
                    as usize,
                title: row.get(2).map_err(|e| AtlasError::Db(e.to_string()))?,
                content: row.get(3).map_err(|e| AtlasError::Db(e.to_string()))?,
                content_type: row.get(4).map_err(|e| AtlasError::Db(e.to_string()))?,
            });
        }
        Ok(results)
    }

    /// Vocabulary-based fuzzy correction for a single term.
    ///
    /// Looks up vocabulary terms within edit distance ≤ 2 of `term`, filtered
    /// by similar length (±2 chars).  Returns the best suggestion by `doc_freq`
    /// or `None` if `term` is already known or no close candidates exist.
    fn suggest_correction(&self, term: &str) -> Result<Option<String>> {
        let term_low = term.to_lowercase();
        let min_len = term_low.len().saturating_sub(2) as i64;
        let max_len = (term_low.len() + 2) as i64;

        let mut stmt = self
            .conn
            .prepare(
                "SELECT term, doc_freq FROM vocabulary
                 WHERE length(term) BETWEEN ?1 AND ?2
                 ORDER BY doc_freq DESC
                 LIMIT 1000",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut rows = stmt
            .query(params![min_len, max_len])
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut best: Option<(String, u32, usize)> = None; // (term, doc_freq, edit_dist)
        while let Some(row) = rows.next().map_err(|e| AtlasError::Db(e.to_string()))? {
            let candidate: String = row.get(0).map_err(|e| AtlasError::Db(e.to_string()))?;
            let freq: i64 = row.get(1).map_err(|e| AtlasError::Db(e.to_string()))?;

            if candidate == term_low {
                // Term already in vocabulary — no correction needed.
                return Ok(None);
            }

            let dist = levenshtein(&term_low, &candidate);
            if dist <= 2 {
                let better = best
                    .as_ref()
                    .is_none_or(|(_, bf, bd)| dist < *bd || (dist == *bd && freq as u32 > *bf));
                if better {
                    best = Some((candidate, freq as u32, dist));
                }
            }
        }

        Ok(best.map(|(t, _, _)| t))
    }

    /// Search with automatic trigram fallback and reciprocal-rank fusion.
    ///
    /// Strategy:
    /// 1. Run FTS5 search with BM25 title weighting.
    /// 2. If fewer than `config.fallback_min_results` hits, run trigram search.
    /// 3. Merge both ranked lists with Reciprocal-Rank Fusion (RRF, k=60).
    /// 4. Apply proximity reranking to boost chunks where query terms cluster.
    /// 5. If zero results after both passes, suggest a vocabulary correction and
    ///    retry with the corrected query (one correction attempt only).
    pub fn search_with_fallback(
        &self,
        query: &str,
        filters: &SearchFilters,
    ) -> Result<Vec<ChunkResult>> {
        let fts_results = self.search(query, filters)?;
        let needs_fallback = fts_results.len() < self.config.fallback_min_results;

        let trigram_results = if needs_fallback {
            debug!(
                "FTS returned {} results; trying trigram fallback",
                fts_results.len()
            );
            self.search_trigram(query, filters).unwrap_or_default()
        } else {
            Vec::new()
        };

        let mut merged = rrf_merge(&fts_results, &trigram_results);

        // Proximity reranking: boost chunks where query terms appear close together.
        if !merged.is_empty() {
            let terms: Vec<&str> = query.split_whitespace().collect();
            if terms.len() > 1 {
                proximity_rerank(&mut merged, &terms);
            }
        }

        // Vocabulary-based fuzzy correction: retry once if still no results.
        if merged.is_empty() {
            debug!(
                "no results for '{}'; attempting vocabulary correction",
                query
            );
            let terms: Vec<&str> = query.split_whitespace().collect();
            let mut corrected_parts: Vec<String> = Vec::new();
            let mut corrected = false;
            for term in &terms {
                if let Ok(Some(suggestion)) = self.suggest_correction(term) {
                    corrected_parts.push(suggestion);
                    corrected = true;
                } else {
                    corrected_parts.push(term.to_string());
                }
            }
            if corrected {
                let corrected_query = corrected_parts.join(" ");
                debug!("retrying with corrected query: '{}'", corrected_query);
                let fts2 = self.search(&corrected_query, filters)?;
                let tri2 = self
                    .search_trigram(&corrected_query, filters)
                    .unwrap_or_default();
                merged = rrf_merge(&fts2, &tri2);
                if !merged.is_empty() {
                    let cterms: Vec<&str> = corrected_query.split_whitespace().collect();
                    if cterms.len() > 1 {
                        proximity_rerank(&mut merged, &cterms);
                    }
                }
            }
        }

        Ok(merged)
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
                chunk_index: row
                    .get::<_, i64>(1)
                    .map_err(|e| AtlasError::Db(e.to_string()))?
                    as usize,
                title: row.get(2).map_err(|e| AtlasError::Db(e.to_string()))?,
                content: row.get(3).map_err(|e| AtlasError::Db(e.to_string()))?,
                content_type: row.get(4).map_err(|e| AtlasError::Db(e.to_string()))?,
            });
        }
        Ok(out)
    }

    /// Delete a source and its chunks (cascade).
    pub fn delete_source(&mut self, source_id: &str) -> Result<()> {
        // Remove FTS and trigram rows before deleting chunks.
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

    /// Cleanup sources older than `keep_days` days (retention policy).
    pub fn cleanup(&mut self, keep_days: u32) -> Result<usize> {
        let cutoff = format_days_ago(keep_days);
        let count = self
            .conn
            .execute("DELETE FROM sources WHERE created_at < ?1", params![cutoff])
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(count)
    }

    /// Return `(source_count, chunk_count)` for the given session or globally.
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

    /// Delete all sources (and their chunks, via cascade) belonging to `session_id`.
    ///
    /// FTS and trigram indexes are cleaned up before the row delete.
    /// Returns the number of sources deleted.
    pub fn delete_session_sources(&mut self, session_id: &str) -> Result<usize> {
        // Bulk FTS cleanup for chunks belonging to this session's sources.
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

// ---------------------------------------------------------------------------
// Retrieval index lifecycle state (Patch R1)
// ---------------------------------------------------------------------------

/// Lifecycle phase of the retrieval/content index for a given repo.
///
/// - `Indexing`    — a run is in progress (or was interrupted and not cleaned up).
/// - `Indexed`     — last run completed successfully; content is searchable.
/// - `IndexFailed` — last run failed; `last_error` carries the reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexState {
    Indexing,
    Indexed,
    IndexFailed,
}

impl IndexState {
    fn from_str(s: &str) -> Self {
        match s {
            "indexing" => Self::Indexing,
            "index_failed" => Self::IndexFailed,
            _ => Self::Indexed,
        }
    }
}

/// Persisted status row for the retrieval index of one repo root.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetrievalIndexStatus {
    /// Normalized repo root path (primary key).
    pub repo_root: String,
    /// Current lifecycle state.
    pub state: IndexState,
    /// Number of files discovered during the last `begin_indexing` call.
    pub files_discovered: i64,
    /// Number of files successfully indexed in the last run.
    pub files_indexed: i64,
    /// Number of chunks written to the store in the last run.
    pub chunks_written: i64,
    /// Number of chunks reused from a previous run (content-hash dedup).
    pub chunks_reused: i64,
    /// ISO-8601 timestamp of the last successful `finish_indexing` call.
    pub last_indexed_at: Option<String>,
    /// Error message from the last `fail_indexing` call, if any.
    pub last_error: Option<String>,
    /// ISO-8601 timestamp of the last state change.
    pub updated_at: String,
}

/// Progress counters passed to [`ContentStore::finish_indexing`].
#[derive(Debug, Clone, Default)]
pub struct IndexingStats {
    pub files_indexed: i64,
    pub chunks_written: i64,
    pub chunks_reused: i64,
}

impl ContentStore {
    // ── Index lifecycle API ──────────────────────────────────────────────────

    /// Begin an indexing run for `repo_root`.
    ///
    /// Sets state to `indexing` and records `files_discovered`.  If a row
    /// already exists (e.g. from a previous run) it is overwritten in-place so
    /// interrupted runs can be cleanly restarted.
    pub fn begin_indexing(&mut self, repo_root: &str, files_discovered: i64) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT INTO retrieval_index_state
                     (repo_root, state, files_discovered, files_indexed,
                      chunks_written, chunks_reused, last_indexed_at, last_error, updated_at)
                 VALUES (?1, 'indexing', ?2, 0, 0, 0, NULL, NULL, ?3)
                 ON CONFLICT(repo_root) DO UPDATE SET
                     state            = 'indexing',
                     files_discovered = ?2,
                     files_indexed    = 0,
                     chunks_written   = 0,
                     chunks_reused    = 0,
                     last_error       = NULL,
                     updated_at       = ?3",
                params![repo_root, files_discovered, now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Mark an indexing run as successfully completed.
    ///
    /// Sets state to `indexed`, records final counters, and stamps
    /// `last_indexed_at` with the current UTC time.
    pub fn finish_indexing(&mut self, repo_root: &str, stats: &IndexingStats) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT INTO retrieval_index_state
                     (repo_root, state, files_discovered, files_indexed,
                      chunks_written, chunks_reused, last_indexed_at, last_error, updated_at)
                 VALUES (?1, 'indexed', 0, ?2, ?3, ?4, ?5, NULL, ?5)
                 ON CONFLICT(repo_root) DO UPDATE SET
                     state           = 'indexed',
                     files_indexed   = ?2,
                     chunks_written  = ?3,
                     chunks_reused   = ?4,
                     last_indexed_at = ?5,
                     last_error      = NULL,
                     updated_at      = ?5",
                params![
                    repo_root,
                    stats.files_indexed,
                    stats.chunks_written,
                    stats.chunks_reused,
                    now,
                ],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Mark an indexing run as failed, recording the error reason.
    ///
    /// Sets state to `index_failed` so callers can surface "not searchable"
    /// status without inspecting logs.
    pub fn fail_indexing(&mut self, repo_root: &str, error: &str) -> Result<()> {
        let now = format_now();
        self.conn
            .execute(
                "INSERT INTO retrieval_index_state
                     (repo_root, state, files_discovered, files_indexed,
                      chunks_written, chunks_reused, last_indexed_at, last_error, updated_at)
                 VALUES (?1, 'index_failed', 0, 0, 0, 0, NULL, ?2, ?3)
                 ON CONFLICT(repo_root) DO UPDATE SET
                     state      = 'index_failed',
                     last_error = ?2,
                     updated_at = ?3",
                params![repo_root, error, now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }

    /// Return the current index status for `repo_root`, or `None` if no run
    /// has ever been recorded for that repo.
    pub fn get_index_status(&self, repo_root: &str) -> Result<Option<RetrievalIndexStatus>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT repo_root, state, files_discovered, files_indexed,
                        chunks_written, chunks_reused, last_indexed_at, last_error, updated_at
                 FROM retrieval_index_state WHERE repo_root = ?1",
            )
            .map_err(db_err)?;
        let mut rows = stmt.query(params![repo_root]).map_err(db_err)?;
        if let Some(row) = rows.next().map_err(db_err)? {
            Ok(Some(Self::row_to_status(row)?))
        } else {
            Ok(None)
        }
    }

    /// Return status rows for all repos that have ever been indexed.
    ///
    /// Ordered by `updated_at DESC` so the most recently touched repo is first.
    pub fn list_index_statuses(&self) -> Result<Vec<RetrievalIndexStatus>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT repo_root, state, files_discovered, files_indexed,
                        chunks_written, chunks_reused, last_indexed_at, last_error, updated_at
                 FROM retrieval_index_state ORDER BY updated_at DESC",
            )
            .map_err(db_err)?;
        let mut rows = stmt.query([]).map_err(db_err)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(db_err)? {
            out.push(Self::row_to_status(row)?);
        }
        Ok(out)
    }

    fn row_to_status(row: &rusqlite::Row<'_>) -> Result<RetrievalIndexStatus> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        Ok(RetrievalIndexStatus {
            repo_root: row.get::<_, String>(0).map_err(db_err)?,
            state: IndexState::from_str(&row.get::<_, String>(1).map_err(db_err)?),
            files_discovered: row.get::<_, i64>(2).map_err(db_err)?,
            files_indexed: row.get::<_, i64>(3).map_err(db_err)?,
            chunks_written: row.get::<_, i64>(4).map_err(db_err)?,
            chunks_reused: row.get::<_, i64>(5).map_err(db_err)?,
            last_indexed_at: row.get::<_, Option<String>>(6).map_err(db_err)?,
            last_error: row.get::<_, Option<String>>(7).map_err(db_err)?,
            updated_at: row.get::<_, String>(8).map_err(db_err)?,
        })
    }

    /// Increment `chunks_written` and `files_indexed` counters for `repo_root`.
    ///
    /// Called automatically from `index_artifact` when `SourceMeta.repo_root`
    /// is set.  Uses `INSERT OR IGNORE` so no explicit `begin_indexing` call
    /// is required for ad-hoc single-artifact indexing.
    fn increment_index_counters(&mut self, repo_root: &str, chunk_count: i64) -> Result<()> {
        let now = format_now();
        // Ensure a row exists (state defaults to 'indexed' for ad-hoc indexing).
        self.conn
            .execute(
                "INSERT OR IGNORE INTO retrieval_index_state
                     (repo_root, state, files_discovered, files_indexed,
                      chunks_written, chunks_reused, last_indexed_at, last_error, updated_at)
                 VALUES (?1, 'indexed', 0, 0, 0, 0, NULL, NULL, ?2)",
                params![repo_root, now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        self.conn
            .execute(
                "UPDATE retrieval_index_state
                 SET files_indexed  = files_indexed + 1,
                     chunks_written = chunks_written + ?2,
                     updated_at     = ?3
                 WHERE repo_root = ?1",
                params![repo_root, chunk_count, now],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        Ok(())
    }
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

/// Extract vocabulary terms from text for the vocabulary table.
///
/// Splits on whitespace and punctuation, lowercases, and keeps only tokens
/// of length ≥ 3 that consist entirely of ASCII word characters.
fn extract_vocab_terms(text: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for word in text.split(|c: char| !c.is_alphanumeric()) {
        let lower = word.to_lowercase();
        if lower.len() >= 3
            && lower.chars().all(|c| c.is_ascii_alphabetic())
            && seen.insert(lower.clone())
        {
            out.push(lower);
        }
    }
    out
}

/// Reciprocal-Rank Fusion of two ranked result lists.
///
/// Uses RRF(d) = Σ 1/(k + rank(d)) where k = 60 (standard default).
/// Deduplication key is `(source_id, chunk_index)`.
fn rrf_merge(list_a: &[ChunkResult], list_b: &[ChunkResult]) -> Vec<ChunkResult> {
    use std::collections::HashMap;

    // Map (source_id, chunk_index) → score and best ChunkResult reference.
    let mut scores: HashMap<(String, usize), f64> = HashMap::new();
    let mut items: HashMap<(String, usize), &ChunkResult> = HashMap::new();

    for (rank, chunk) in list_a.iter().enumerate() {
        let key = (chunk.source_id.clone(), chunk.chunk_index);
        *scores.entry(key.clone()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        items.entry(key).or_insert(chunk);
    }
    for (rank, chunk) in list_b.iter().enumerate() {
        let key = (chunk.source_id.clone(), chunk.chunk_index);
        *scores.entry(key.clone()).or_default() += 1.0 / (RRF_K + rank as f64 + 1.0);
        items.entry(key).or_insert(chunk);
    }

    let mut ranked: Vec<(&ChunkResult, f64)> = scores
        .iter()
        .filter_map(|(k, &s)| items.get(k).map(|c| (*c, s)))
        .collect();
    // Higher RRF score = better rank.
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    ranked.into_iter().map(|(c, _)| c.clone()).collect()
}

/// Proximity reranking: boost results where query terms appear close together.
///
/// For each chunk, counts how many term pairs appear within a 50-word window.
/// Chunks with more co-occurring term pairs are promoted to the front.
fn proximity_rerank(results: &mut [ChunkResult], terms: &[&str]) {
    let score_chunk = |chunk: &ChunkResult| -> i64 {
        let words: Vec<&str> = chunk.content.split_whitespace().collect();
        let n = words.len();
        if n == 0 {
            return 0;
        }
        let lower_words: Vec<String> = words.iter().map(|w| w.to_lowercase()).collect();
        let lower_terms: Vec<String> = terms.iter().map(|t| t.to_lowercase()).collect();

        // Collect positions for each term.
        let positions: Vec<Vec<usize>> = lower_terms
            .iter()
            .map(|t| {
                lower_words
                    .iter()
                    .enumerate()
                    .filter(|(_, w)| w.contains(t.as_str()))
                    .map(|(i, _)| i)
                    .collect()
            })
            .collect();

        // Count term-pair co-occurrences within a 50-word window.
        let window = 50usize;
        let mut bonus: i64 = 0;
        for i in 0..positions.len() {
            for j in (i + 1)..positions.len() {
                for &pi in &positions[i] {
                    for &pj in &positions[j] {
                        if pi.abs_diff(pj) <= window {
                            bonus += 1;
                        }
                    }
                }
            }
        }
        // Title match bonus.
        if let Some(ref title) = chunk.title {
            let lt = title.to_lowercase();
            for t in &lower_terms {
                if lt.contains(t.as_str()) {
                    bonus += 5;
                }
            }
        }
        bonus
    };

    // Stable sort: higher proximity score first.
    results.sort_by_key(|chunk| std::cmp::Reverse(score_chunk(chunk)));
}

/// Levenshtein edit distance (byte-level; capped at 3 for early exit).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    if m == 0 {
        return n;
    }
    if n == 0 {
        return m;
    }
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
        // Early exit: if minimum possible distance exceeds 2, stop.
        if *prev.iter().min().unwrap_or(&0) > 2 {
            return 3;
        }
    }
    prev[n]
}

/// Return `true` when the error string indicates SQLite database corruption.
fn is_corruption_error(err: &AtlasError) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("malformed")
        || msg.contains("not a database")
        || msg.contains("disk image is malformed")
        || msg.contains("database disk image")
        || msg.contains("file is not a database")
}

/// Rename the corrupt database file to `{path}.quarantine` so a fresh DB can
/// be created on the next open.  Best-effort: logs a warning on failure.
fn quarantine_db(path: &str) {
    let qpath = format!("{path}.quarantine");
    if let Err(e) = std::fs::rename(path, &qpath) {
        debug!("content DB quarantine rename failed: {e}");
    } else {
        info!(
            path = path,
            quarantine = %qpath,
            "corrupt content DB quarantined; a fresh store will be created on next open"
        );
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
        store
            .index_artifact(meta("src-1"), "hello world", "text/plain")
            .unwrap();
        let src = store.get_source("src-1").unwrap();
        assert!(src.is_some());
        let chunks = store.get_chunks("src-1").unwrap();
        assert!(!chunks.is_empty());
    }

    #[test]
    fn delete_source_removes_chunks() {
        let mut store = open_store();
        store
            .index_artifact(meta("src-2"), "some content here", "text/plain")
            .unwrap();
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
        let results = store.search("quick", &SearchFilters::default()).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("quick"));
    }

    #[test]
    fn idempotent_reindex_replaces_chunks() {
        let mut store = open_store();
        store
            .index_artifact(meta("src-6"), "version one content here", "text/plain")
            .unwrap();
        let before = store.get_chunks("src-6").unwrap().len();
        store
            .index_artifact(
                meta("src-6"),
                "version two different content entirely",
                "text/plain",
            )
            .unwrap();
        let after = store.get_chunks("src-6").unwrap();
        // Chunks from v1 should not stack; v2 content must appear.
        assert!(after.iter().any(|c| c.content.contains("version two")));
        // chunk count should not double.
        assert!(after.len() <= before + 5);
    }

    #[test]
    fn trigram_search_finds_substring() {
        let mut store = open_store();
        store
            .index_artifact(
                meta("src-tri"),
                "the mitochondria is the powerhouse of the cell",
                "text/plain",
            )
            .unwrap();
        // Trigram can match on partial substrings.
        let results = store
            .search_trigram("mitochondria", &SearchFilters::default())
            .unwrap();
        assert!(!results.is_empty(), "trigram should find 'mitochondria'");
    }

    #[test]
    fn vocabulary_populated_on_index() {
        let mut store = open_store();
        store
            .index_artifact(
                meta("src-vocab"),
                "photosynthesis occurs in chloroplasts",
                "text/plain",
            )
            .unwrap();
        // Term "photosynthesis" should appear in vocabulary.
        let count: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM vocabulary WHERE term = 'photosynthesis'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "vocabulary should contain indexed terms");
    }

    #[test]
    fn search_with_fallback_returns_fts_results() {
        let mut store = open_store();
        store
            .index_artifact(
                meta("src-fb"),
                "the quick brown fox jumped over the lazy dog",
                "text/plain",
            )
            .unwrap();
        let results = store
            .search_with_fallback("fox", &SearchFilters::default())
            .unwrap();
        assert!(
            !results.is_empty(),
            "search_with_fallback should find 'fox'"
        );
    }

    #[test]
    fn search_with_fallback_uses_trigram_when_fts_sparse() {
        let mut store = open_store();
        // Index something that won't match standard FTS but will match trigram.
        store
            .index_artifact(
                meta("src-tri2"),
                "polychromatic spectroscopy measurements",
                "text/plain",
            )
            .unwrap();
        // "spectrosc" is a substring — FTS won't match it but trigram will.
        let results = store
            .search_with_fallback("spectrosc", &SearchFilters::default())
            .unwrap();
        assert!(
            !results.is_empty(),
            "trigram fallback should find substring 'spectrosc'"
        );
    }

    #[test]
    fn rrf_merge_deduplicates() {
        let make = |source_id: &str, idx: usize, content: &str| ChunkResult {
            source_id: source_id.to_string(),
            chunk_index: idx,
            title: None,
            content: content.to_string(),
            content_type: "text/plain".to_string(),
        };
        let a = vec![make("s1", 0, "alpha"), make("s2", 0, "beta")];
        let b = vec![make("s1", 0, "alpha"), make("s3", 0, "gamma")];
        let merged = rrf_merge(&a, &b);
        // Deduplicated: s1/0 appears once; total 3 unique items.
        assert_eq!(merged.len(), 3, "RRF merge should deduplicate");
        // s1/0 shared by both lists should rank highest.
        assert_eq!(merged[0].source_id, "s1");
    }

    #[test]
    fn levenshtein_distances() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
        assert_eq!(levenshtein("fox", "fog"), 1);
        assert_eq!(levenshtein("identical", "identical"), 0);
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn vocabulary_correction_suggests_close_term() {
        let mut store = open_store();
        store
            .index_artifact(
                meta("src-corr"),
                "the algorithm performs computation efficiently",
                "text/plain",
            )
            .unwrap();
        // "algoritm" (one char off) should suggest "algorithm".
        let correction = store.suggest_correction("algoritm").unwrap();
        assert_eq!(correction, Some("algorithm".to_string()));
    }

    #[test]
    fn configurable_thresholds_respected() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_str().unwrap().to_string();
        std::mem::forget(file);
        let mut store = ContentStore::open_with_config(
            &path,
            ContentStoreConfig {
                small_output_bytes: 10,
                preview_threshold_bytes: 50,
                fallback_min_results: 1,
                max_db_bytes: None,
            },
        )
        .unwrap();
        store.migrate().unwrap();

        // 5 bytes < small_output_bytes=10 → Raw
        let r = store
            .route_output(meta("t1"), "hello", "text/plain")
            .unwrap();
        assert!(matches!(r, OutputRouting::Raw(_)));

        // 30 bytes > 10 but <= 50 → Preview
        let r = store
            .route_output(
                meta("t2"),
                "this is a medium length output text!",
                "text/plain",
            )
            .unwrap();
        assert!(matches!(r, OutputRouting::Preview { .. }));

        // > 50 bytes → Pointer
        let big = "x".repeat(100);
        let r = store.route_output(meta("t3"), &big, "text/plain").unwrap();
        assert!(matches!(r, OutputRouting::Pointer { .. }));
    }

    // ── CM8 tests ─────────────────────────────────────────────────────────

    #[test]
    fn routing_stats_increment_correctly() {
        let mut store = open_store();

        // Raw
        store
            .route_output(meta("rs1"), "tiny", "text/plain")
            .unwrap();
        // Preview (6000-char text > DEFAULT_SMALL_OUTPUT_BYTES=512, ≤ DEFAULT_PREVIEW_THRESHOLD=4096 for default)
        // Use a custom store with lower thresholds to guarantee preview vs pointer split.
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_str().unwrap().to_string();
        std::mem::forget(file);
        let mut custom = ContentStore::open_with_config(
            &path,
            ContentStoreConfig {
                small_output_bytes: 10,
                preview_threshold_bytes: 100,
                fallback_min_results: 3,
                max_db_bytes: None,
            },
        )
        .unwrap();
        custom.migrate().unwrap();

        // Raw (< 10 bytes)
        custom
            .route_output(meta("rs-a"), "hi", "text/plain")
            .unwrap();
        // Preview (11–100 bytes)
        custom
            .route_output(meta("rs-b"), "a".repeat(50).as_str(), "text/plain")
            .unwrap();
        // Pointer (> 100 bytes)
        custom
            .route_output(meta("rs-c"), "b".repeat(200).as_str(), "text/plain")
            .unwrap();

        let stats = custom.routing_stats();
        assert_eq!(stats.raw_count, 1);
        assert_eq!(stats.preview_count, 1);
        assert_eq!(stats.pointer_count, 1);
        assert_eq!(
            stats.avoided_bytes,
            50 + 200,
            "both preview and pointer avoided bytes tracked"
        );
    }

    #[test]
    fn size_limit_enforced_by_pruning_oldest_sources() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = file.path().to_str().unwrap().to_string();
        std::mem::forget(file);
        // Set a very small DB size limit to force pruning.
        let mut store = ContentStore::open_with_config(
            &path,
            ContentStoreConfig {
                small_output_bytes: 0, // index everything
                preview_threshold_bytes: 1,
                fallback_min_results: 3,
                max_db_bytes: Some(1), // 1 byte — always over limit
            },
        )
        .unwrap();
        store.migrate().unwrap();

        // Index several artifacts; size limit should prune old ones.
        for i in 0..5 {
            store
                .route_output(
                    meta(&format!("sl-{i}")),
                    &"content ".repeat(200),
                    "text/plain",
                )
                .unwrap();
        }

        // At least some sources must have been pruned.
        let (src_count, _) = store.stats(None).unwrap();
        assert!(
            src_count < 5,
            "size limit should have pruned old sources; got {src_count}"
        );
    }

    #[test]
    fn routing_stats_default_is_zero() {
        let store = open_store();
        let stats = store.routing_stats();
        assert_eq!(stats, RoutingStats::default());
    }

    // ── Corrupt DB quarantine ───────────────────────────────────────────────

    #[test]
    fn corrupt_content_db_is_quarantined_on_open() {
        use std::path::Path;

        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_str().unwrap().to_string();
        // Keep the file but overwrite with garbage so SQLite rejects it.
        drop(f); // close the NamedTempFile but keep the path
        std::fs::write(&path, b"not a sqlite database").unwrap();

        let result = ContentStore::open(&path);
        assert!(result.is_err(), "corrupt DB must return error");

        let quarantine = format!("{path}.quarantine");
        assert!(
            Path::new(&quarantine).exists(),
            "quarantine file must be created: {quarantine}"
        );
    }

    #[test]
    fn quarantine_allows_fresh_content_db_open() {
        let f = NamedTempFile::new().unwrap();
        let path = f.path().to_str().unwrap().to_string();
        drop(f);
        std::fs::write(&path, b"garbage").unwrap();

        // First open quarantines; original is gone.
        let _ = ContentStore::open(&path);

        // Second open must succeed with a fresh DB.
        let mut store =
            ContentStore::open(&path).expect("fresh open must succeed after quarantine");
        store.migrate().unwrap();
    }

    #[test]
    fn is_corruption_error_detects_known_messages() {
        let cases = [
            "database disk image is malformed",
            "file is not a database",
            "not a database",
        ];
        for msg in cases {
            let err = AtlasError::Db(msg.to_string());
            assert!(is_corruption_error(&err), "must match: {msg}");
        }
    }

    // ── Retrieval index lifecycle (Patch R1) ────────────────────────────────

    #[test]
    fn begin_indexing_sets_state_to_indexing() {
        let mut store = open_store();
        store.begin_indexing("/repo/a", 42).unwrap();
        let status = store.get_index_status("/repo/a").unwrap().unwrap();
        assert_eq!(status.state, IndexState::Indexing);
        assert_eq!(status.files_discovered, 42);
        assert_eq!(status.files_indexed, 0);
        assert_eq!(status.chunks_written, 0);
        assert!(status.last_indexed_at.is_none());
        assert!(status.last_error.is_none());
    }

    #[test]
    fn finish_indexing_marks_indexed_and_stamps_time() {
        let mut store = open_store();
        store.begin_indexing("/repo/b", 10).unwrap();
        store
            .finish_indexing(
                "/repo/b",
                &IndexingStats {
                    files_indexed: 9,
                    chunks_written: 30,
                    chunks_reused: 1,
                },
            )
            .unwrap();
        let status = store.get_index_status("/repo/b").unwrap().unwrap();
        assert_eq!(status.state, IndexState::Indexed);
        assert_eq!(status.files_indexed, 9);
        assert_eq!(status.chunks_written, 30);
        assert_eq!(status.chunks_reused, 1);
        assert!(status.last_indexed_at.is_some());
        assert!(status.last_error.is_none());
    }

    #[test]
    fn fail_indexing_sets_error_state() {
        let mut store = open_store();
        store.begin_indexing("/repo/c", 5).unwrap();
        store
            .fail_indexing("/repo/c", "parse error on main.rs")
            .unwrap();
        let status = store.get_index_status("/repo/c").unwrap().unwrap();
        assert_eq!(status.state, IndexState::IndexFailed);
        assert_eq!(status.last_error.unwrap(), "parse error on main.rs");
    }

    #[test]
    fn missing_repo_returns_none() {
        let store = open_store();
        let status = store.get_index_status("/nonexistent/repo").unwrap();
        assert!(status.is_none());
    }

    #[test]
    fn list_index_statuses_returns_all_repos() {
        let mut store = open_store();
        store.begin_indexing("/repo/x", 1).unwrap();
        store
            .finish_indexing("/repo/x", &IndexingStats::default())
            .unwrap();
        store.begin_indexing("/repo/y", 2).unwrap();
        let statuses = store.list_index_statuses().unwrap();
        assert_eq!(statuses.len(), 2);
        let roots: Vec<&str> = statuses.iter().map(|s| s.repo_root.as_str()).collect();
        assert!(roots.contains(&"/repo/x"));
        assert!(roots.contains(&"/repo/y"));
    }

    #[test]
    fn begin_indexing_resets_counters_on_restart() {
        let mut store = open_store();
        // First run
        store.begin_indexing("/repo/d", 20).unwrap();
        store
            .finish_indexing(
                "/repo/d",
                &IndexingStats {
                    files_indexed: 18,
                    chunks_written: 60,
                    chunks_reused: 2,
                },
            )
            .unwrap();
        // Second run — counters must reset.
        store.begin_indexing("/repo/d", 25).unwrap();
        let status = store.get_index_status("/repo/d").unwrap().unwrap();
        assert_eq!(status.state, IndexState::Indexing);
        assert_eq!(status.files_discovered, 25);
        assert_eq!(status.files_indexed, 0);
        assert_eq!(status.chunks_written, 0);
    }

    #[test]
    fn index_artifact_auto_increments_index_counters() {
        let mut store = open_store();
        // No begin_indexing call — ad-hoc single artifact with repo_root.
        store
            .index_artifact(meta("src-ai"), "auto increment test content", "text/plain")
            .unwrap();
        // meta() sets repo_root = "/repo", so a row should have been created.
        let status = store.get_index_status("/repo").unwrap().unwrap();
        assert_eq!(status.state, IndexState::Indexed);
        assert!(status.files_indexed >= 1);
        assert!(status.chunks_written >= 1);
    }

    #[test]
    fn interrupted_indexing_visible_as_indexing_state() {
        let mut store = open_store();
        // Simulate a run that starts but never finishes (e.g. process crash).
        store.begin_indexing("/repo/interrupted", 100).unwrap();
        // Reopen the store (new struct instance, same DB).
        let status = store
            .get_index_status("/repo/interrupted")
            .unwrap()
            .unwrap();
        assert_eq!(
            status.state,
            IndexState::Indexing,
            "interrupted run must show as 'indexing' to signal recovery needed"
        );
    }
}
