use atlas_core::{
    AtlasError, Node, ParsedFile, RankingEvidence, Result, RetrievalMode, ScoredNode,
    SearchMatchedField,
};
use atlas_repo::CanonicalRepoPath;
use rusqlite::params;

use super::{
    Store,
    helpers::{repeat_placeholders, row_to_node},
};

/// Directory/repo scope applied to vector search before cosine scoring.
///
/// This is the TrieHI DSQ stage-1 contract: scope is resolved into SQL
/// predicates so out-of-scope chunks are never ranked.
#[derive(Debug, Clone, Default)]
pub struct VectorScope {
    /// Recursive canonical path prefix.
    pub subpath_prefix: Option<String>,
    /// Non-recursive exact parent directory.
    pub exact_dir: Option<String>,
    /// Canonical prefixes excluded from the candidate set.
    pub exclude_prefixes: Vec<String>,
    /// Indexed repo ids to restrict to. Empty means every repo.
    pub repo_ids: Vec<String>,
    /// Node kinds to restrict to.
    pub kinds: Vec<String>,
    /// Languages to restrict to.
    pub languages: Vec<String>,
}

impl VectorScope {
    /// Canonicalize and validate every path scope.
    ///
    /// Fails closed when a path scope is present without an explicit repo
    /// scope, because a path-only scope cannot be attributed safely across
    /// multiple indexed repos.
    fn validated(&self) -> Result<Self> {
        let canonicalize = |value: &str| {
            CanonicalRepoPath::from_repo_relative(value.trim())
                .map(|path| path.as_str().to_owned())
                .map_err(|error| {
                    AtlasError::Other(format!("invalid directory scope '{value}': {error}"))
                })
        };
        let scope = Self {
            subpath_prefix: self
                .subpath_prefix
                .as_deref()
                .map(canonicalize)
                .transpose()?,
            exact_dir: self.exact_dir.as_deref().map(canonicalize).transpose()?,
            exclude_prefixes: self
                .exclude_prefixes
                .iter()
                .map(|value| canonicalize(value))
                .collect::<Result<Vec<_>>>()?,
            repo_ids: self.repo_ids.clone(),
            kinds: self.kinds.clone(),
            languages: self.languages.clone(),
        };
        let has_path_scope = scope.subpath_prefix.is_some()
            || scope.exact_dir.is_some()
            || !scope.exclude_prefixes.is_empty();
        if has_path_scope && scope.repo_ids.is_empty() {
            return Err(AtlasError::Other(
                "directory scope requires an explicit repo scope; refusing to scan every repo"
                    .to_owned(),
            ));
        }
        Ok(scope)
    }
}

/// Strict lexicographic upper bound equivalent to `path.starts_with(prefix)`.
fn scope_upper_bound(prefix: &str) -> String {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = char::from_u32(last as u32 + 1) {
            chars.push(next);
            break;
        }
    }
    chars.into_iter().collect()
}

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
    /// Chunk identity is `(source_repo_id, node_qn, chunk_idx)` so two repos
    /// that share an un-namespaced qname keep independent chunks. Embeddings
    /// are not touched; call `Store::set_chunk_embedding()` separately.
    pub fn upsert_chunk(
        &self,
        source_repo_id: &str,
        node_qn: &str,
        chunk_idx: i32,
        text: &str,
    ) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute(
                "INSERT INTO retrieval_chunks (source_repo_id, node_qn, chunk_idx, text)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(source_repo_id, node_qn, chunk_idx)
                 DO UPDATE SET text = excluded.text",
                params![source_repo_id, node_qn, chunk_idx, text],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Delete all retrieval chunks whose symbol belongs to `file_path` within
    /// `source_repo_id`.
    ///
    /// Call before re-indexing or deleting a file so stale / renamed symbols
    /// are removed. Chunks are repo-scoped, so identical relative paths (and
    /// identical qnames) in other repos never lose their chunks.
    pub fn delete_chunks_for_file(&self, source_repo_id: &str, file_path: &str) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .execute(
                "DELETE FROM retrieval_chunks
                 WHERE source_repo_id = ?1
                   AND node_qn IN (
                       SELECT qualified_name FROM nodes
                       WHERE source_repo_id = ?1 AND file_path = ?2
                   )",
                params![source_repo_id, file_path],
            )
            .map_err(db_err)?;
        Ok(())
    }

    pub fn replace_chunks_for_parsed_files(
        &self,
        source_repo_id: &str,
        files: &[ParsedFile],
    ) -> Result<usize> {
        if files.is_empty() {
            return Ok(0);
        }

        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;

        let result = (|| {
            let mut delete_stmt = self
                .conn
                .prepare(
                    "DELETE FROM retrieval_chunks
                     WHERE source_repo_id = ?1
                       AND node_qn IN (
                           SELECT qualified_name FROM nodes
                           WHERE source_repo_id = ?1 AND file_path = ?2
                       )",
                )
                .map_err(db_err)?;
            let mut upsert_stmt = self
                .conn
                .prepare(
                    "INSERT INTO retrieval_chunks (source_repo_id, node_qn, chunk_idx, text)
                     VALUES (?1, ?2, ?3, ?4)
                     ON CONFLICT(source_repo_id, node_qn, chunk_idx)
                     DO UPDATE SET text = excluded.text",
                )
                .map_err(db_err)?;

            let mut written = 0usize;
            for parsed_file in files {
                delete_stmt
                    .execute(params![source_repo_id, parsed_file.path])
                    .map_err(db_err)?;
                for node in &parsed_file.nodes {
                    upsert_stmt
                        .execute(params![
                            source_repo_id,
                            node.qualified_name,
                            0,
                            node.chunk_text()
                        ])
                        .map_err(db_err)?;
                    written += 1;
                }
            }
            Ok(written)
        })();

        match result {
            Ok(written) => {
                self.conn.execute_batch("COMMIT").map_err(db_err)?;
                Ok(written)
            }
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(error)
            }
        }
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
    /// Scope, repo, kind, and language filters are applied in SQL before any
    /// cosine scoring, so out-of-scope chunks are never ranked. Returns
    /// `(results, scope_candidates)` so callers can report candidate counts.
    pub fn nodes_by_vector_similarity(
        &self,
        query_embedding: &[f32],
        scope: &VectorScope,
        limit: usize,
    ) -> Result<Vec<ScoredNode>> {
        Ok(self
            .nodes_by_vector_similarity_with_stats(query_embedding, scope, limit)?
            .0)
    }

    /// Same as [`Store::nodes_by_vector_similarity`] but also reports the
    /// number of in-scope candidates evaluated.
    pub fn nodes_by_vector_similarity_with_stats(
        &self,
        query_embedding: &[f32],
        scope: &VectorScope,
        limit: usize,
    ) -> Result<(Vec<ScoredNode>, usize)> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let scope = scope.validated()?;

        let mut clauses: Vec<String> = vec!["rc.embedding IS NOT NULL".to_owned()];
        let mut bind: Vec<String> = Vec::new();

        if let Some(prefix) = scope.subpath_prefix.as_deref() {
            bind.push(prefix.to_owned());
            let low = bind.len();
            bind.push(scope_upper_bound(prefix));
            let high = bind.len();
            clauses.push(format!("n.file_path >= ?{low} AND n.file_path < ?{high}"));
        }
        if let Some(dir) = scope.exact_dir.as_deref() {
            bind.push(dir.to_owned());
            let index = bind.len();
            clauses.push(format!(
                "(substr(n.file_path, 1, length(?{index}) + 1) = ?{index} || '/' AND instr(substr(n.file_path, length(?{index}) + 2), '/') = 0)"
            ));
        }
        for prefix in &scope.exclude_prefixes {
            bind.push(prefix.clone());
            let low = bind.len();
            bind.push(scope_upper_bound(prefix));
            let high = bind.len();
            clauses.push(format!(
                "NOT (n.file_path >= ?{low} AND n.file_path < ?{high})"
            ));
        }
        if !scope.repo_ids.is_empty() {
            let mut placeholders = Vec::new();
            for repo_id in &scope.repo_ids {
                bind.push(repo_id.clone());
                placeholders.push(format!("?{}", bind.len()));
            }
            clauses.push(format!(
                "rc.source_repo_id IN ({})",
                placeholders.join(", ")
            ));
        }
        if !scope.kinds.is_empty() {
            let mut placeholders = Vec::new();
            for kind in &scope.kinds {
                bind.push(kind.clone());
                placeholders.push(format!("?{}", bind.len()));
            }
            clauses.push(format!("n.kind IN ({})", placeholders.join(", ")));
        }
        if !scope.languages.is_empty() {
            let mut placeholders = Vec::new();
            for language in &scope.languages {
                bind.push(language.clone());
                placeholders.push(format!("?{}", bind.len()));
            }
            clauses.push(format!("n.language IN ({})", placeholders.join(", ")));
        }

        let sql = format!(
            "SELECT n.qualified_name, rc.embedding
             FROM retrieval_chunks rc
             JOIN nodes n
               ON n.qualified_name = rc.node_qn
              AND n.source_repo_id = rc.source_repo_id
             WHERE {}",
            clauses.join(" AND ")
        );

        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
        let mut candidates: Vec<(String, f64)> = stmt
            .query_map(rusqlite::params_from_iter(bind.iter()), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .map(|(qn, blob)| {
                let vec: Vec<f32> = blob
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|chunk| f32::from_le_bytes(*chunk))
                    .collect();
                let sim = cosine_similarity(query_embedding, &vec);
                (qn, sim)
            })
            .collect();
        let scope_candidates = candidates.len();

        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(limit);

        if candidates.is_empty() {
            return Ok((vec![], scope_candidates));
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
        Ok((results, scope_candidates))
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
