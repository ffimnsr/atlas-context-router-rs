//! ICM-E2 — optional dense-vector recall for the shared memory surface.
//!
//! Embeddings are strictly opt-in: lexical `memories_fts` recall remains the
//! default path and never needs an embedding provider. This module owns vector
//! storage, coverage inspection, and the hybrid fusion step that reranks
//! lexical hits with reciprocal-rank fusion before appending vector-only
//! matches. Raw lexical and vector scores are never pooled into one number, and
//! exact lexical tiers always outrank vector-only evidence.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};

use atlas_core::{AtlasError, Clock, Result};

use super::memory::{append_filter_clauses, row_to_memory};
use super::memory_recall::{
    append_visibility_clause, candidate_limit, ranking_for, recall_memories_with_policy,
};
use super::types::{
    MEMORY_VECTOR_SCAN_LIMIT, MemoryEmbeddingCoverage, MemoryEmbeddingPolicy, MemoryListFilter,
    MemoryRecallPolicy, MemoryRecord, MemorySearchHit, MemoryViewer,
};

pub(super) const MEMORY_EMBEDDINGS_TABLE: &str = "memory_embeddings";

/// Exact column set of the `memory_embeddings` table (migration 015).
pub(super) const MEMORY_EMBEDDINGS_COLUMNS: &[&str] = &[
    "memory_id",
    "embedding_model",
    "dimension",
    "vector_blob",
    "created_at",
];

/// Exact index set of the `memory_embeddings` table (migration 015).
pub(super) const MEMORY_EMBEDDINGS_INDEXES: &[&str] = &["idx_memory_embeddings_model"];

const MEMORY_SELECT: &str = "m.id, m.repo_root, m.session_id, m.frontend, m.scope, m.topic,
        m.title, m.body, m.importance, m.created_at, m.updated_at, m.last_accessed_at,
        m.decay_score, m.source_id, m.metadata_json, m.superseded_by";

// ── Storage ───────────────────────────────────────────────────────────────────

/// Persist one memory embedding, rejecting dimension and model drift.
pub(super) fn upsert_memory_embedding(
    conn: &Connection,
    memory_id: &str,
    embedding_model: &str,
    dimension: usize,
    vector: &[f32],
    now: &str,
) -> Result<()> {
    if embedding_model.trim().is_empty() {
        return Err(AtlasError::Other(
            "memory_embedding_invalid: embedding_model must not be empty".to_owned(),
        ));
    }
    if dimension == 0 {
        return Err(AtlasError::Other(
            "memory_embedding_invalid: dimension must be greater than 0".to_owned(),
        ));
    }
    if vector.len() != dimension {
        return Err(AtlasError::Other(format!(
            "memory_embedding_dimension_mismatch: configured dimension {dimension} does not match vector length {}",
            vector.len()
        )));
    }
    if vector.iter().any(|value| !value.is_finite()) {
        return Err(AtlasError::Other(
            "memory_embedding_invalid: vector contains non-finite values".to_owned(),
        ));
    }

    let memory_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE id = ?1",
            [memory_id],
            |row| row.get(0),
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    if memory_exists == 0 {
        return Err(AtlasError::Other(format!(
            "memory_not_found: no memory with id {memory_id}"
        )));
    }

    let stored: Option<(String, i64)> = conn
        .query_row(
            "SELECT embedding_model, dimension FROM memory_embeddings WHERE memory_id = ?1",
            [memory_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    if let Some((stored_model, stored_dimension)) = stored
        && stored_dimension != dimension as i64
    {
        return Err(AtlasError::Other(format!(
            "memory_embedding_dimension_mismatch: stored dimension {stored_dimension} for model {stored_model} does not match configured dimension {dimension}"
        )));
    }

    let blob = encode_vector(vector);
    conn.execute(
        "INSERT INTO memory_embeddings (memory_id, embedding_model, dimension, vector_blob, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(memory_id) DO UPDATE SET
             embedding_model = excluded.embedding_model,
             dimension = excluded.dimension,
             vector_blob = excluded.vector_blob,
             created_at = excluded.created_at",
        params![memory_id, embedding_model, dimension as i64, blob, now],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(())
}

/// Coverage of one embedding model across recallable memories in a repo.
pub(super) fn memory_embedding_coverage(
    conn: &Connection,
    repo_root: &str,
    embedding_model: &str,
    dimension: usize,
) -> Result<MemoryEmbeddingCoverage> {
    let total: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories WHERE repo_root = ?1 AND superseded_by IS NULL",
            [repo_root],
            |row| row.get(0),
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let embedded: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memories m
             JOIN memory_embeddings e ON e.memory_id = m.id
             WHERE m.repo_root = ?1
               AND m.superseded_by IS NULL
               AND e.embedding_model = ?2
               AND e.dimension = ?3",
            params![repo_root, embedding_model, dimension as i64],
            |row| row.get(0),
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let embedded = embedded.max(0) as usize;
    let total = total.max(0) as usize;
    Ok(MemoryEmbeddingCoverage {
        embedding_model: embedding_model.to_owned(),
        dimension,
        embedded,
        missing: total.saturating_sub(embedded),
    })
}

/// Recallable memories in a repo that still need an embedding for `model`.
pub(super) fn memories_missing_embeddings(
    conn: &Connection,
    repo_root: &str,
    embedding_model: &str,
    dimension: usize,
    filter: &MemoryListFilter,
    limit: usize,
) -> Result<Vec<MemoryRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut sql = format!(
        "SELECT {MEMORY_SELECT}
         FROM memories m
         LEFT JOIN memory_embeddings e
                ON e.memory_id = m.id
               AND e.embedding_model = ?2
               AND e.dimension = ?3
         WHERE m.repo_root = ?1
           AND m.superseded_by IS NULL
           AND e.memory_id IS NULL"
    );
    let mut params = vec![
        repo_root.to_owned(),
        embedding_model.to_owned(),
        (dimension as i64).to_string(),
    ];
    append_filter_clauses(&mut sql, &mut params, filter, "m.");
    sql.push_str(" ORDER BY m.updated_at DESC, m.created_at DESC, m.id ASC LIMIT ?");
    params.push(limit.to_string());

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    stmt.query_map(
        rusqlite::params_from_iter(params.iter().map(String::as_str)),
        row_to_memory,
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(|e| AtlasError::Db(e.to_string()))
}

/// Stored vector for one memory, decoded into `f32` components.
pub(super) fn get_memory_embedding(
    conn: &Connection,
    memory_id: &str,
) -> Result<Option<(String, usize, Vec<f32>)>> {
    let row: Option<(String, i64, Vec<u8>)> = conn
        .query_row(
            "SELECT embedding_model, dimension, vector_blob
             FROM memory_embeddings WHERE memory_id = ?1",
            [memory_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    row.map(|(model, dimension, blob)| {
        let vector = decode_vector(&blob).ok_or_else(|| {
            AtlasError::Other(format!(
                "memory_embedding_invalid: vector blob for {memory_id} is not a valid dimension-multiple byte array"
            ))
        })?;
        Ok((model, dimension.max(0) as usize, vector))
    })
    .transpose()
}

// ── Scoring ───────────────────────────────────────────────────────────────────

/// Hybrid recall: lexical hits reranked by RRF, then vector-only matches.
///
/// `query_vector` must already be produced by the configured embedding model;
/// this layer never calls an embedding provider so recall stays deterministic
/// and testable without network access.
#[allow(clippy::too_many_arguments)]
pub(super) fn hybrid_recall_with_policy(
    conn: &Connection,
    repo_root: &str,
    query: &str,
    query_vector: &[f32],
    filter: &MemoryListFilter,
    shared_only: bool,
    viewer: &MemoryViewer,
    limit: usize,
    recall_policy: &MemoryRecallPolicy,
    embedding_policy: &MemoryEmbeddingPolicy,
) -> Result<Vec<MemorySearchHit>> {
    embedding_policy.validate()?;
    if limit == 0 {
        return Ok(Vec::new());
    }
    if query_vector.is_empty() {
        return Err(AtlasError::Other(
            "memory_embedding_invalid: query vector must not be empty".to_owned(),
        ));
    }

    let vector_ranking = rank_vector_candidates(
        conn,
        repo_root,
        query_vector,
        filter,
        shared_only,
        viewer,
        embedding_policy.min_similarity,
    )?;

    // Lexical path runs first and keeps its lexical-first tier order.
    let mut lexical = recall_memories_with_policy(
        conn,
        repo_root,
        query,
        filter,
        shared_only,
        viewer,
        candidate_limit(limit),
        recall_policy,
    )?;

    let k = embedding_policy.rrf_k as f64;
    let mut seen = lexical
        .iter()
        .map(|hit| hit.memory.id.clone())
        .collect::<std::collections::BTreeSet<_>>();

    for (lexical_rank, hit) in lexical.iter_mut().enumerate() {
        let vector = vector_ranking.positions.get(&hit.memory.id).copied().map(
            |(similarity, vector_rank)| {
                (
                    similarity,
                    embedding_policy.weight_vector / (k + vector_rank as f64),
                )
            },
        );
        let lexical_component = embedding_policy.weight_fts / (k + (lexical_rank + 1) as f64);
        hit.ranking.vector_score = vector.map(|(similarity, _)| similarity);
        hit.ranking.hybrid_score = Some(
            (lexical_component + vector.map_or(0.0, |(_, component)| component)).clamp(0.0, 1.0),
        );
    }

    // Tier order first, then fusion score: a body-tier exact keyword hit can
    // never be displaced by a vector-only or higher-cosine match.
    lexical.sort_by(|left, right| {
        left.relevance_score
            .cmp(&right.relevance_score)
            .then_with(|| {
                right
                    .ranking
                    .hybrid_score
                    .unwrap_or(0.0)
                    .total_cmp(&left.ranking.hybrid_score.unwrap_or(0.0))
            })
            .then_with(|| right.recall_score.total_cmp(&left.recall_score))
            .then_with(|| right.memory.updated_at.cmp(&left.memory.updated_at))
            .then_with(|| left.memory.id.cmp(&right.memory.id))
    });
    lexical.truncate(limit);

    if lexical.len() >= limit {
        return Ok(lexical);
    }

    let now = atlas_core::SystemClock.now_utc();
    for (memory_id, (similarity, vector_rank)) in &vector_ranking.ordered {
        if lexical.len() >= limit {
            break;
        }
        if !seen.insert(memory_id.clone()) {
            continue;
        }
        let Some(memory) = vector_ranking.memories.get(memory_id).cloned() else {
            continue;
        };
        let mut ranking = ranking_for_stub(&memory, now, recall_policy);
        ranking.recall_path = "vector".to_owned();
        ranking.vector_score = Some(*similarity);
        ranking.hybrid_score =
            Some((embedding_policy.weight_vector / (k + *vector_rank as f64)).clamp(0.0, 1.0));
        lexical.push(MemorySearchHit {
            memory,
            // Vector-only hits sort after every lexical tier by construction.
            relevance_score: 3,
            bm25_score: 0.0,
            recall_score: ranking.hybrid_score.unwrap_or(0.0),
            ranking,
        });
    }

    Ok(lexical)
}

struct VectorRanking {
    /// `memory_id -> (cosine similarity, 1-based rank)`.
    positions: HashMap<String, (f64, usize)>,
    /// Ranked `(memory_id, (similarity, rank))` pairs in similarity order.
    ordered: Vec<(String, (f64, usize))>,
    /// Memory rows needed to materialize vector-only hits.
    memories: HashMap<String, MemoryRecord>,
}

/// Score every visible embedded memory by cosine similarity against the query.
fn rank_vector_candidates(
    conn: &Connection,
    repo_root: &str,
    query_vector: &[f32],
    filter: &MemoryListFilter,
    shared_only: bool,
    viewer: &MemoryViewer,
    min_similarity: f64,
) -> Result<VectorRanking> {
    let mut sql = format!(
        "SELECT {MEMORY_SELECT}, e.vector_blob
         FROM memories m
         JOIN memory_embeddings e ON e.memory_id = m.id
         WHERE m.repo_root = ?1
           AND m.superseded_by IS NULL"
    );
    let mut params = vec![repo_root.to_owned()];
    append_filter_clauses(&mut sql, &mut params, filter, "m.");
    append_visibility_clause(&mut sql, &mut params, shared_only, viewer);
    sql.push_str(" ORDER BY m.updated_at DESC, m.id ASC LIMIT ?");
    params.push(MEMORY_VECTOR_SCAN_LIMIT.to_string());

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let rows = stmt
        .query_map(
            rusqlite::params_from_iter(params.iter().map(String::as_str)),
            |row| {
                let memory = row_to_memory(row)?;
                let blob: Vec<u8> = row.get(16)?;
                Ok((memory, blob))
            },
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let query_norm = vector_norm(query_vector);
    let mut scored = Vec::with_capacity(rows.len());
    for (memory, blob) in rows {
        let Some(vector) = decode_vector(&blob) else {
            continue;
        };
        if vector.len() != query_vector.len() {
            continue;
        }
        let similarity = cosine_similarity(query_vector, query_norm, &vector);
        if similarity < min_similarity {
            continue;
        }
        scored.push((memory, similarity));
    }
    scored.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.id.cmp(&right.0.id))
    });

    let mut ranking = VectorRanking {
        positions: HashMap::new(),
        ordered: Vec::with_capacity(scored.len()),
        memories: HashMap::new(),
    };
    for (index, (memory, similarity)) in scored.into_iter().enumerate() {
        let rank = index + 1;
        ranking
            .positions
            .insert(memory.id.clone(), (similarity, rank));
        ranking
            .ordered
            .push((memory.id.clone(), (similarity, rank)));
        ranking.memories.insert(memory.id.clone(), memory);
    }
    Ok(ranking)
}

/// Ranking stub for a vector-only hit: body tier, no BM25, no lexical origin.
fn ranking_for_stub(
    memory: &MemoryRecord,
    now: time::OffsetDateTime,
    recall_policy: &MemoryRecallPolicy,
) -> super::types::MemoryRecallRanking {
    let stub = MemorySearchHit {
        memory: memory.clone(),
        relevance_score: 2,
        bm25_score: 0.0,
        recall_score: 0.0,
        ranking: super::types::MemoryRecallRanking {
            topic_match: String::new(),
            topic_match_tier: 2,
            importance_rank: 0,
            scope_rank: 0,
            source_backed: false,
            superseded: false,
            age_days: 0,
            origin_score: 0.0,
            time_score: 0.0,
            importance_score: 0.0,
            recall_path: String::new(),
            vector_score: None,
            hybrid_score: None,
        },
    };
    ranking_for(&stub, now, recall_policy)
}

// ── Vector codec ──────────────────────────────────────────────────────────────

fn encode_vector(vector: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(vector.len() * 4);
    for value in vector {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob
}

fn decode_vector(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.is_empty() || !blob.len().is_multiple_of(4) {
        return None;
    }
    Some(
        blob.chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

fn vector_norm(vector: &[f32]) -> f64 {
    vector
        .iter()
        .map(|value| (*value as f64) * (*value as f64))
        .sum::<f64>()
        .sqrt()
}

fn cosine_similarity(query: &[f32], query_norm: f64, candidate: &[f32]) -> f64 {
    let candidate_norm = vector_norm(candidate);
    if query_norm <= f64::EPSILON || candidate_norm <= f64::EPSILON {
        return 0.0;
    }
    let dot: f64 = query
        .iter()
        .zip(candidate.iter())
        .map(|(left, right)| (*left as f64) * (*right as f64))
        .sum();
    (dot / (query_norm * candidate_norm)).clamp(-1.0, 1.0)
}

// ── Schema checks ─────────────────────────────────────────────────────────────

/// Report missing `memory_embeddings` structure for `atlas db check`.
pub(super) fn schema_issues(conn: &Connection) -> Vec<String> {
    let mut issues = Vec::new();
    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [MEMORY_EMBEDDINGS_TABLE],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        issues.push(format!("missing table: {MEMORY_EMBEDDINGS_TABLE}"));
        return issues;
    }

    let present_columns = conn
        .prepare("PRAGMA table_info('memory_embeddings')")
        .ok()
        .and_then(|mut stmt| {
            stmt.query_map([], |row| row.get::<_, String>(1))
                .ok()
                .map(|rows| {
                    rows.filter_map(std::result::Result::ok)
                        .collect::<Vec<String>>()
                })
        })
        .unwrap_or_default();
    for column in MEMORY_EMBEDDINGS_COLUMNS {
        if !present_columns.iter().any(|present| present == column) {
            issues.push(format!("missing column: memory_embeddings.{column}"));
        }
    }

    for index in MEMORY_EMBEDDINGS_INDEXES {
        let exists: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = ?1",
                [index],
                |row| row.get(0),
            )
            .unwrap_or(0);
        if exists == 0 {
            issues.push(format!("missing index: {index}"));
        }
    }

    let orphaned: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memory_embeddings e
             LEFT JOIN memories m ON m.id = e.memory_id
             WHERE m.id IS NULL",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if orphaned > 0 {
        issues.push(format!("orphaned memory_embeddings rows: {orphaned}"));
    }

    issues
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vector_codec_round_trips() {
        let vector = vec![0.5_f32, -1.25, 3.0];
        let blob = encode_vector(&vector);
        assert_eq!(decode_vector(&blob), Some(vector));
    }

    #[test]
    fn vector_codec_rejects_bad_lengths() {
        assert_eq!(decode_vector(&[]), None);
        assert_eq!(decode_vector(&[0, 1, 2]), None);
    }

    #[test]
    fn cosine_similarity_is_bounded_and_orthogonal_is_zero() {
        let query = vec![1.0_f32, 0.0];
        let norm = vector_norm(&query);
        assert!((cosine_similarity(&query, norm, &[1.0, 0.0]) - 1.0).abs() < 1e-9);
        assert!((cosine_similarity(&query, norm, &[0.0, 1.0])).abs() < 1e-9);
        assert!((cosine_similarity(&query, norm, &[-2.0, 0.0]) + 1.0).abs() < 1e-9);
        assert_eq!(cosine_similarity(&query, norm, &[0.0, 0.0]), 0.0);
    }
}
