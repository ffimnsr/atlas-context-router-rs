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

use rusqlite::{Connection, OpenFlags, params};
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
#[derive(Debug, Clone)]
pub struct ContentStoreConfig {
    /// Outputs at or below this size are returned raw without indexing.
    pub small_output_bytes: usize,
    /// Outputs above this size return only a pointer (source_id) rather than a preview.
    pub preview_threshold_bytes: usize,
    /// Minimum number of FTS hits before `search_with_fallback` skips trigram search.
    pub fallback_min_results: usize,
}

impl Default for ContentStoreConfig {
    fn default() -> Self {
        Self {
            small_output_bytes: DEFAULT_SMALL_OUTPUT_BYTES,
            preview_threshold_bytes: DEFAULT_PREVIEW_THRESHOLD_BYTES,
            fallback_min_results: DEFAULT_FALLBACK_MIN_RESULTS,
        }
    }
}

/// SQLite-backed content store.
pub struct ContentStore {
    conn: Connection,
    config: ContentStoreConfig,
}

impl ContentStore {
    /// Open (or create) the content store database at `path` with default config.
    pub fn open(path: &str) -> Result<Self> {
        Self::open_with_config(path, ContentStoreConfig::default())
    }

    /// Open (or create) the content store database at `path` with custom config.
    pub fn open_with_config(path: &str, config: ContentStoreConfig) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE,
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        let store = Self { conn, config };
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
            return Ok(OutputRouting::Raw(raw_text.to_string()));
        }

        let source_id = meta.id.clone();
        self.index_artifact(meta, raw_text, content_type)?;

        if raw_text.len() <= self.config.preview_threshold_bytes {
            let preview: String = raw_text.chars().take(512).collect();
            debug!("content routing: medium output, returning preview");
            Ok(OutputRouting::Preview { source_id, preview })
        } else {
            debug!("content routing: large output, returning pointer");
            Ok(OutputRouting::Pointer { source_id })
        }
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
}
