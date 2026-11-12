use std::collections::HashMap;

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
#[derive(Debug, Clone)]
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
    /// Chunk count above which the in-memory TrieHI index is used
    /// (`retrieval.triehi_min_chunks`).
    pub triehi_min_chunks: usize,
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
            triehi_min_chunks: self.triehi_min_chunks,
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
pub(super) fn scope_upper_bound(prefix: &str) -> String {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        if let Some(next) = char::from_u32(last as u32 + 1) {
            chars.push(next);
            break;
        }
    }
    chars.into_iter().collect()
}

/// Default `retrieval.triehi_min_chunks` threshold.
pub const DEFAULT_TRIEHI_MIN_CHUNKS: usize = 10_000;

impl Default for VectorScope {
    fn default() -> Self {
        Self {
            subpath_prefix: None,
            exact_dir: None,
            exclude_prefixes: Vec::new(),
            repo_ids: Vec::new(),
            kinds: Vec::new(),
            languages: Vec::new(),
            triehi_min_chunks: DEFAULT_TRIEHI_MIN_CHUNKS,
        }
    }
}

/// True when no scope predicate needs SQL pre-filtering.
///
/// Unfiltered (and single-repo) queries map directly onto vec0: no id set is
/// needed, and a single repo id becomes the vec0 partition key.
fn scope_has_no_filters(scope: &VectorScope) -> bool {
    scope.subpath_prefix.is_none()
        && scope.exact_dir.is_none()
        && scope.exclude_prefixes.is_empty()
        && scope.repo_ids.len() <= 1
        && scope.kinds.is_empty()
        && scope.languages.is_empty()
}

/// Build the SQL predicates and bind values that realise a scope exactly.
///
/// `skip_path_clauses` is set when the TrieHI bitmap already resolves path
/// scopes in memory, matching the BLOB-scan behaviour.
fn scope_filter(scope: &VectorScope, skip_path_clauses: bool) -> (Vec<String>, Vec<String>) {
    let mut clauses: Vec<String> = vec!["rc.embedding IS NOT NULL".to_owned()];
    let mut bind: Vec<String> = Vec::new();

    if !skip_path_clauses {
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

    (clauses, bind)
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
        self.invalidate_trie_cache();
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
        let ids = self.chunk_ids_for_file(source_repo_id, file_path)?;
        let _ = crate::vec_index::delete_ids(&self.conn, &ids);
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
        self.invalidate_trie_cache();
        Ok(())
    }

    fn chunk_ids_for_file(&self, source_repo_id: &str, file_path: &str) -> Result<Vec<i64>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id FROM retrieval_chunks
                 WHERE source_repo_id = ?1
                   AND node_qn IN (
                       SELECT qualified_name FROM nodes
                       WHERE source_repo_id = ?1 AND file_path = ?2
                   )",
            )
            .map_err(db_err)?;
        let ids = stmt
            .query_map(params![source_repo_id, file_path], |row| row.get(0))
            .map_err(db_err)?
            .filter_map(|row| row.ok())
            .collect();
        Ok(ids)
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
            // Preserve vectors for chunks whose text did not change. Re-parsing a
            // file must not force a full re-embed of every symbol it contains.
            let mut select_stmt = self
                .conn
                .prepare(
                    "SELECT rc.id, rc.node_qn, rc.chunk_idx, rc.text, rc.embedding
                     FROM retrieval_chunks rc
                     WHERE rc.source_repo_id = ?1
                       AND rc.node_qn IN (
                           SELECT qualified_name FROM nodes
                           WHERE source_repo_id = ?1 AND file_path = ?2
                       )",
                )
                .map_err(db_err)?;
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
                    "INSERT INTO retrieval_chunks (source_repo_id, node_qn, chunk_idx, text, embedding)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(source_repo_id, node_qn, chunk_idx)
                     DO UPDATE SET text = excluded.text, embedding = excluded.embedding
                     RETURNING id",
                )
                .map_err(db_err)?;

            let mut written = 0usize;
            for parsed_file in files {
                type ChunkKey = (String, i32);
                type ChunkSnapshot = (String, Option<Vec<u8>>);
                let mut previous: HashMap<ChunkKey, ChunkSnapshot> = HashMap::new();
                let mut old_ids: Vec<i64> = Vec::new();
                {
                    let rows = select_stmt
                        .query_map(params![source_repo_id, parsed_file.path], |row| {
                            Ok((
                                row.get::<_, i64>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, i32>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, Option<Vec<u8>>>(4)?,
                            ))
                        })
                        .map_err(db_err)?;
                    for row in rows {
                        let (id, node_qn, chunk_idx, text, embedding) = row.map_err(db_err)?;
                        old_ids.push(id);
                        previous.insert((node_qn, chunk_idx), (text, embedding));
                    }
                }

                let _ = crate::vec_index::delete_ids(&self.conn, &old_ids);
                delete_stmt
                    .execute(params![source_repo_id, parsed_file.path])
                    .map_err(db_err)?;
                for node in &parsed_file.nodes {
                    let text = node.chunk_text();
                    let embedding = previous
                        .get(&(node.qualified_name.clone(), 0))
                        .filter(|(old_text, _)| old_text == &text)
                        .and_then(|(_, embedding)| embedding.clone());
                    let index_blob = embedding.clone();
                    let new_id: i64 = upsert_stmt
                        .query_row(
                            params![source_repo_id, node.qualified_name, 0, text, embedding],
                            |row| row.get(0),
                        )
                        .map_err(db_err)?;
                    if let Some(blob) = index_blob.as_deref()
                        && let Err(error) = crate::vec_index::upsert(&self.conn, new_id, blob)
                    {
                        tracing::debug!(chunk_id = new_id, "vec0 index update failed: {error}");
                    }
                    written += 1;
                }
            }
            Ok(written)
        })();

        match result {
            Ok(written) => {
                self.conn.execute_batch("COMMIT").map_err(db_err)?;
                self.invalidate_trie_cache();
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
        if let Err(error) = crate::vec_index::upsert(&self.conn, id, &blob) {
            tracing::debug!(chunk_id = id, "vec0 index update failed: {error}");
        }
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
        // Unfiltered queries use the sqlite-vec KNN index directly. Any index
        // error keeps the exact BLOB-scan path below.
        if scope_has_no_filters(&scope)
            && let Some((candidates, scope_candidates)) = crate::vec_index::try_knn(
                &self.conn,
                query_embedding,
                limit.max(1),
                scope.repo_ids.first().map(String::as_str),
                None,
            )
        {
            tracing::debug!(scope_candidates, "vec0 KNN path");
            return self.finish_vector_candidates(candidates, scope_candidates, limit);
        }

        let trie_bitmap = self.trie_scope_bitmap(&scope)?;
        let (clauses, bind) = scope_filter(&scope, trie_bitmap.is_some());
        let from_sql = "FROM retrieval_chunks rc
             JOIN nodes n
               ON n.qualified_name = rc.node_qn
              AND n.source_repo_id = rc.source_repo_id";

        // Scoped queries keep the accelerated path by resolving the exact
        // in-scope chunk ids in SQL, then restricting the KNN to that rowid set.
        if let Some((candidates, scope_candidates)) = self.scoped_vec_candidates(
            query_embedding,
            limit,
            &scope,
            trie_bitmap.as_ref(),
            from_sql,
            &clauses,
            &bind,
        ) {
            tracing::debug!(scope_candidates, "vec0 scoped KNN path");
            return self.finish_vector_candidates(candidates, scope_candidates, limit);
        }

        let select_columns = if trie_bitmap.is_some() {
            "n.qualified_name, rc.embedding, rc.id"
        } else {
            "n.qualified_name, rc.embedding"
        };
        let sql = format!(
            "SELECT {select_columns} {from_sql} WHERE {}",
            clauses.join(" AND ")
        );

        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
        let candidates: Vec<(String, f64)> = stmt
            .query_map(rusqlite::params_from_iter(bind.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Vec<u8>>(1)?,
                    row.get::<_, Option<i64>>(2).unwrap_or(None),
                ))
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .filter(|(_, _, chunk_id)| match (trie_bitmap.as_ref(), chunk_id) {
                (Some(bitmap), Some(id)) => u32::try_from(*id).is_ok_and(|id| bitmap.contains(id)),
                (Some(_), None) => false,
                (None, _) => true,
            })
            .map(|(qn, blob, _)| {
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
        tracing::debug!(
            scope_candidates,
            triehi = trie_bitmap.is_some(),
            "vector scope pre-filter applied"
        );

        self.finish_vector_candidates(candidates, scope_candidates, limit)
    }

    /// Resolve the exact in-scope chunk ids, then rank them with vec0.
    ///
    /// This keeps subpath, directory, exclusion, kind, language, and multi-repo
    /// scopes on the accelerated path: SQL resolves scope to ids, and the KNN
    /// ranks only those rows. Returns `None` so the caller runs the BLOB scan
    /// when the extension/index is unavailable.
    #[allow(clippy::too_many_arguments)]
    fn scoped_vec_candidates(
        &self,
        query_embedding: &[f32],
        limit: usize,
        scope: &VectorScope,
        trie_bitmap: Option<&roaring::RoaringBitmap>,
        from_sql: &str,
        clauses: &[String],
        bind: &[String],
    ) -> Option<(Vec<(String, f64)>, usize)> {
        let sql = format!("SELECT rc.id {from_sql} WHERE {}", clauses.join(" AND "));
        let mut stmt = self.conn.prepare(&sql).ok()?;
        let ids: Vec<i64> = stmt
            .query_map(rusqlite::params_from_iter(bind.iter()), |row| {
                row.get::<_, i64>(0)
            })
            .ok()?
            .collect::<rusqlite::Result<Vec<_>>>()
            .ok()?
            .into_iter()
            .filter(|id| match trie_bitmap {
                Some(bitmap) => u32::try_from(*id).is_ok_and(|id| bitmap.contains(id)),
                None => true,
            })
            .collect();
        crate::vec_index::try_knn(
            &self.conn,
            query_embedding,
            limit.max(1),
            // The id set already encodes the repo restriction exactly; a
            // partition-key filter would wrongly drop every repo but the first.
            scope
                .repo_ids
                .first()
                .filter(|_| scope.repo_ids.len() == 1)
                .map(String::as_str),
            Some(&ids),
        )
    }

    fn finish_vector_candidates(
        &self,
        mut candidates: Vec<(String, f64)>,
        scope_candidates: usize,
        limit: usize,
    ) -> Result<(Vec<ScoredNode>, usize)> {
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

    /// Drop the cached TrieHI index after a chunk mutation.
    pub(crate) fn invalidate_trie_cache(&self) {
        *self.trie_cache.borrow_mut() = None;
    }

    /// Relink cached trie entries after a stable file rename (DSM MOVE).
    ///
    /// When any chunk cannot be relinked deterministically the cache is
    /// dropped and rebuilt lazily from `retrieval_chunks` on the next scoped
    /// vector query.
    pub(crate) fn relink_trie_chunks(
        &self,
        repo_id: &str,
        old_path: &str,
        new_path: &str,
        chunk_ids: &[i64],
    ) {
        if self.trie_cache.borrow().is_none() {
            return;
        }
        let mut cache = self.trie_cache.borrow_mut();
        let Some(trie) = cache.as_mut() else {
            return;
        };
        for chunk_id in chunk_ids {
            let Ok(chunk_id) = u32::try_from(*chunk_id) else {
                *cache = None;
                return;
            };
            if !trie.relink(repo_id, old_path, new_path, chunk_id) {
                *cache = None;
                return;
            }
        }
    }

    /// Resolve scope to a bitmap when the TrieHI threshold is met.
    fn trie_scope_bitmap(&self, scope: &VectorScope) -> Result<Option<roaring::RoaringBitmap>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM retrieval_chunks", [], |row| {
                row.get(0)
            })
            .map_err(db_err)?;
        if (count as usize) < scope.triehi_min_chunks {
            return Ok(None);
        }
        {
            let mut cache = self.trie_cache.borrow_mut();
            if cache.is_none() {
                *cache = Some(self.build_directory_trie()?);
            }
        }
        let cache = self.trie_cache.borrow();
        let trie = cache
            .as_ref()
            .expect("trie cache populated immediately above");
        Ok(Some(trie.resolve(scope)?))
    }

    fn build_directory_trie(&self) -> Result<super::directory_trie::DirectoryTrie> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT rc.id, rc.source_repo_id, n.file_path
                 FROM retrieval_chunks rc
                 JOIN nodes n
                   ON n.qualified_name = rc.node_qn
                  AND n.source_repo_id = rc.source_repo_id",
            )
            .map_err(db_err)?;
        let mut trie = super::directory_trie::DirectoryTrie::new();
        let mut rows = stmt.query([]).map_err(db_err)?;
        while let Some(row) = rows.next().map_err(db_err)? {
            let Ok(chunk_id) = row.get::<_, i64>(0) else {
                continue;
            };
            let Ok(repo_id) = row.get::<_, String>(1) else {
                continue;
            };
            let Ok(file_path) = row.get::<_, String>(2) else {
                continue;
            };
            if let Ok(chunk_id) = u32::try_from(chunk_id) {
                trie.insert(&repo_id, &file_path, chunk_id);
            }
        }
        Ok(trie)
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
