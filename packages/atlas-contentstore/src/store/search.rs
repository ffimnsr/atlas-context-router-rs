use rusqlite::params;
use tracing::debug;

use atlas_core::{AtlasError, Result};

use super::util::{fts5_escape, levenshtein, proximity_rerank, rrf_merge};
use super::{ChunkResult, ContentStore, SearchFilters};

impl ContentStore {
    /// Keyword search over indexed chunks using FTS5 with BM25 title weighting.
    pub fn search(&self, query: &str, filters: &SearchFilters) -> Result<Vec<ChunkResult>> {
        let fts_query = fts5_escape(query);

        let mut where_parts = vec!["chunks_fts MATCH ?1".to_string()];
        let mut extra_params: Vec<String> = Vec::new();

        if let Some(ref sid) = filters.session_id {
            extra_params.push(sid.clone());
            where_parts.push(format!("s.session_id = ?{}", extra_params.len() + 1));
        }
        if let Some(ref source_type) = filters.source_type {
            extra_params.push(source_type.clone());
            where_parts.push(format!("s.source_type = ?{}", extra_params.len() + 1));
        }
        if let Some(ref repo_root) = filters.repo_root {
            extra_params.push(repo_root.clone());
            where_parts.push(format!("s.repo_root = ?{}", extra_params.len() + 1));
        }

        let sql = format!(
            "SELECT c.source_id, c.chunk_id, c.chunk_index, c.title, c.content, c.content_type
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

        let mut rows = stmt
            .query(rusqlite::params_from_iter(
                std::iter::once(fts_query).chain(extra_params),
            ))
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut results = Vec::new();
        while let Some(row) = rows.next().map_err(|e| AtlasError::Db(e.to_string()))? {
            results.push(ChunkResult {
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
        Ok(results)
    }

    /// Trigram search over indexed chunks using `chunks_trigram`.
    pub(super) fn search_trigram(
        &self,
        query: &str,
        filters: &SearchFilters,
    ) -> Result<Vec<ChunkResult>> {
        let trigram_query = query.to_string();

        let mut where_parts = vec!["chunks_trigram MATCH ?1".to_string()];
        let mut extra_params: Vec<String> = Vec::new();

        if let Some(ref sid) = filters.session_id {
            extra_params.push(sid.clone());
            where_parts.push(format!("s.session_id = ?{}", extra_params.len() + 1));
        }
        if let Some(ref source_type) = filters.source_type {
            extra_params.push(source_type.clone());
            where_parts.push(format!("s.source_type = ?{}", extra_params.len() + 1));
        }
        if let Some(ref repo_root) = filters.repo_root {
            extra_params.push(repo_root.clone());
            where_parts.push(format!("s.repo_root = ?{}", extra_params.len() + 1));
        }

        let sql = format!(
            "SELECT c.source_id, c.chunk_id, c.chunk_index, c.title, c.content, c.content_type
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
        Ok(results)
    }

    /// Vocabulary-based fuzzy correction for a single term.
    pub(super) fn suggest_correction(&self, term: &str) -> Result<Option<String>> {
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

        let mut best: Option<(String, u32, usize)> = None;
        while let Some(row) = rows.next().map_err(|e| AtlasError::Db(e.to_string()))? {
            let candidate: String = row.get(0).map_err(|e| AtlasError::Db(e.to_string()))?;
            let freq: i64 = row.get(1).map_err(|e| AtlasError::Db(e.to_string()))?;

            if candidate == term_low {
                return Ok(None);
            }

            let dist = levenshtein(&term_low, &candidate);
            if dist <= 2 {
                let better = best.as_ref().is_none_or(|(_, best_freq, best_dist)| {
                    dist < *best_dist || (dist == *best_dist && freq as u32 > *best_freq)
                });
                if better {
                    best = Some((candidate, freq as u32, dist));
                }
            }
        }

        Ok(best.map(|(term, _, _)| term))
    }

    /// Search with automatic trigram fallback and reciprocal-rank fusion.
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

        if !merged.is_empty() {
            let terms: Vec<&str> = query.split_whitespace().collect();
            if terms.len() > 1 {
                proximity_rerank(&mut merged, &terms);
            }
        }

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
                let fts_retry = self.search(&corrected_query, filters)?;
                let trigram_retry = self
                    .search_trigram(&corrected_query, filters)
                    .unwrap_or_default();
                merged = rrf_merge(&fts_retry, &trigram_retry);
                if !merged.is_empty() {
                    let corrected_terms: Vec<&str> = corrected_query.split_whitespace().collect();
                    if corrected_terms.len() > 1 {
                        proximity_rerank(&mut merged, &corrected_terms);
                    }
                }
            }
        }

        Ok(merged)
    }
}
