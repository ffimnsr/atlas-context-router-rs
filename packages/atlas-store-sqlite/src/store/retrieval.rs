use atlas_core::{
    AtlasError, Node, RankingEvidence, Result, RetrievalMode, ScoredNode, SearchMatchedField,
};
use rusqlite::params;

use super::{
    Store,
    helpers::{repeat_placeholders, row_to_node},
};

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| *x as f64 * *y as f64)
        .sum();
    let norm_a: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }
    dot / (norm_a * norm_b)
}

impl Store {
    /// Insert or update the text for a single retrieval chunk.
    ///
    /// Embeddings are not touched; call [`set_chunk_embedding`] separately.
    pub fn upsert_chunk(&self, node_qn: &str, chunk_idx: i32, text: &str) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute(
                "INSERT INTO retrieval_chunks (node_qn, chunk_idx, text)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(node_qn, chunk_idx) DO UPDATE SET text = excluded.text",
                params![node_qn, chunk_idx, text],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Delete all retrieval chunks whose symbol belongs to `file_path`.
    ///
    /// Call before re-indexing a file so stale / renamed symbols are removed.
    pub fn delete_chunks_for_file(&self, file_path: &str) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute(
                "DELETE FROM retrieval_chunks
                 WHERE node_qn IN (
                     SELECT qualified_name FROM nodes WHERE file_path = ?1
                 )",
                params![file_path],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Return up to `limit` chunks that have no embedding yet.
    ///
    /// Returns `(id, node_qn, text)` triples ready for embedding generation.
    pub fn chunks_missing_embeddings(&self, limit: usize) -> Result<Vec<(i64, String, String)>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, node_qn, text FROM retrieval_chunks
                 WHERE embedding IS NULL
                 LIMIT ?1",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Persist a computed embedding for the given chunk `id`.
    ///
    /// `embedding` is stored as little-endian IEEE 754 `f32` bytes.
    pub fn set_chunk_embedding(&self, id: i64, embedding: &[f32]) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let blob: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
        self.conn
            .execute(
                "UPDATE retrieval_chunks SET embedding = ?1 WHERE id = ?2",
                params![blob, id],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Return the top-`limit` nodes ranked by cosine similarity to `query_embedding`.
    ///
    /// Fetches all chunks that have an embedding, scores them in-process, and
    /// returns the matching nodes. Chunks whose symbol no longer exists in the
    /// `nodes` table are silently skipped.
    pub fn nodes_by_vector_similarity(
        &self,
        query_embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<ScoredNode>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());

        let mut stmt = self
            .conn
            .prepare(
                "SELECT node_qn, embedding FROM retrieval_chunks
                 WHERE embedding IS NOT NULL",
            )
            .map_err(db_err)?;

        let mut candidates: Vec<(String, f64)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .map(|(qn, blob)| {
                let vec: Vec<f32> = blob
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect();
                let sim = cosine_similarity(query_embedding, &vec);
                (qn, sim)
            })
            .collect();

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(limit);

        if candidates.is_empty() {
            return Ok(vec![]);
        }

        let qns: Vec<&str> = candidates.iter().map(|(q, _)| q.as_str()).collect();
        let nodes = self.nodes_by_qualified_names(&qns)?;

        let score_map: std::collections::HashMap<&str, f64> =
            candidates.iter().map(|(q, s)| (q.as_str(), *s)).collect();

        let mut results: Vec<ScoredNode> = nodes
            .into_iter()
            .map(|n| {
                let score = score_map
                    .get(n.qualified_name.as_str())
                    .copied()
                    .unwrap_or(0.0);
                ScoredNode::with_ranking_evidence(
                    n,
                    score,
                    RankingEvidence::new(RetrievalMode::Vector, score)
                        .with_raw_score(score)
                        .with_matched_field(SearchMatchedField::Embedding),
                )
            })
            .collect();

        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Ok(results)
    }

    /// Fetch nodes by their `qualified_name` values.
    pub fn nodes_by_qualified_names(&self, qualified_names: &[&str]) -> Result<Vec<Node>> {
        if qualified_names.is_empty() {
            return Ok(vec![]);
        }
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let ph = repeat_placeholders(qualified_names.len());
        let sql = format!(
            "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                    language, parent_name, params, return_type, modifiers,
                    is_test, file_hash, extra_json
             FROM nodes WHERE qualified_name IN ({ph})"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(
                rusqlite::params_from_iter(qualified_names.iter()),
                row_to_node,
            )
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
}
