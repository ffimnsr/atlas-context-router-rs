//! ICM-E1 — deterministic cross-session recall ranking.
//!
//! Recall stays lexical-first: the match tier from [`super::memory`] dominates
//! every other signal so exact keyword evidence is never buried by softer
//! heuristics. Within a tier, deterministic weights add importance, broad
//! scope visibility, source-backed evidence, and recency. Raw session history
//! is never part of the candidate set; only curated `memories` rows are ranked.

use rusqlite::Connection;
use time::OffsetDateTime;

use atlas_core::{AtlasError, Clock, Result, SystemClock};

use super::memory::{age_days, append_filter_clauses, row_to_memory};
use super::types::{
    MemoryImportance, MemoryListFilter, MemoryRecallRanking, MemoryScope, MemorySearchHit,
    MemoryViewer,
};

/// Score gap between adjacent lexical tiers. Must stay larger than the maximum
/// sum of every non-lexical weight so exact matches always outrank softer hits.
const LEXICAL_TIER_WEIGHT: f64 = 2000.0;

/// Importance points indexed by rank (0 = critical ... 3 = low).
const IMPORTANCE_WEIGHTS: [f64; 4] = [1200.0, 800.0, 400.0, 0.0];

/// Scope-visibility points indexed by rank (0 = global ... 3 = session).
const SCOPE_WEIGHTS: [f64; 4] = [250.0, 150.0, 50.0, 0.0];

/// Bonus for carrying a `source_id` that points at saved evidence.
const SOURCE_BACKED_WEIGHT: f64 = 50.0;

/// Maximum recency bonus; decays with a 30-day half-life.
const RECENCY_WEIGHT: f64 = 25.0;

/// Small drag for superseded rows so identical active rows win ties.
const SUPERSEDED_PENALTY: f64 = 10.0;

/// Lexical cross-session recall with deterministic composite ranking.
///
/// Exact topic matches rank above topic/title contains matches, which rank above
/// body-only matches; ties break on importance, scope visibility, source-backed
/// evidence, then recency. Visibility (ICM-A3) is enforced for the given viewer:
/// `session`-scoped memories require the same session id, `frontend`-scoped
/// memories require the same frontend. `shared_only` narrows results to
/// `project` + `global` and bypasses viewer-based conditions.
///
/// Cross-session recall (ICM-E1) considers every memory stored for the repo, so
/// memories written by prior sessions remain eligible without ever returning
/// raw session events. Only the curated `memories` row is ranked.
pub(super) fn recall_memories(
    conn: &Connection,
    repo_root: &str,
    query: &str,
    filter: &MemoryListFilter,
    shared_only: bool,
    viewer: &MemoryViewer,
    limit: usize,
) -> Result<Vec<MemorySearchHit>> {
    let like = format!("%{}%", escape_like(query));
    let mut sql = String::from(
        "SELECT id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json,
                superseded_by,
                CASE WHEN topic = ?2 THEN 0
                     WHEN topic LIKE ?3 ESCAPE '\\' OR title LIKE ?3 ESCAPE '\\' THEN 1
                     ELSE 2 END AS match_tier,
                CASE importance WHEN 'critical' THEN 0 WHEN 'high' THEN 1
                                WHEN 'normal' THEN 2 ELSE 3 END AS importance_rank,
                CASE WHEN superseded_by IS NULL THEN 0 ELSE 1 END AS superseded_rank
         FROM memories
         WHERE repo_root = ?1
           AND (topic LIKE ?3 ESCAPE '\\' OR title LIKE ?3 ESCAPE '\\'
                OR body LIKE ?3 ESCAPE '\\')",
    );
    let mut params = vec![repo_root.to_owned(), query.to_owned(), like];
    append_filter_clauses(&mut sql, &mut params, filter);
    if shared_only {
        sql.push_str(" AND scope IN ('project', 'global')");
    } else {
        sql.push_str(
            " AND (scope IN ('project', 'global')
                  OR (scope = 'session' AND session_id = ?)
                  OR (scope = 'frontend' AND frontend = ?))",
        );
        params.push(viewer.session_id.clone());
        params.push(viewer.frontend.clone());
    }
    sql.push_str(concat!(
        " ORDER BY match_tier ASC, importance_rank ASC,",
        " CASE scope WHEN 'global' THEN 0 WHEN 'project' THEN 1",
        " WHEN 'frontend' THEN 2 ELSE 3 END ASC,",
        " CASE WHEN source_id IS NULL OR trim(source_id) = '' THEN 1 ELSE 0 END ASC,",
        " superseded_rank ASC, updated_at DESC, created_at DESC",
        " LIMIT ?",
    ));
    params.push(candidate_limit(limit).to_string());

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs = params.iter().map(String::as_str).collect::<Vec<_>>();
    let mut hits = stmt
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
            let relevance_score: i32 = row.get(16)?;
            Ok(MemorySearchHit {
                memory: row_to_memory(row)?,
                relevance_score,
                recall_score: 0.0,
                ranking: MemoryRecallRanking {
                    topic_match: String::new(),
                    topic_match_tier: 0,
                    importance_rank: 0,
                    scope_rank: 0,
                    source_backed: false,
                    superseded: false,
                    age_days: 0,
                },
            })
        })
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    rank_hits(&mut hits, limit, SystemClock.now_utc());
    Ok(hits)
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Number of recall candidates to fetch before re-ranking. Over-fetching keeps
/// the SQL candidate window from hiding a high-signal row that the composite
/// score would promote.
pub(super) fn candidate_limit(limit: usize) -> usize {
    if limit == 0 {
        0
    } else {
        limit.saturating_mul(8).max(256)
    }
}

/// Score and sort recall hits, then keep the requested limit.
pub(super) fn rank_hits(hits: &mut Vec<MemorySearchHit>, limit: usize, now: OffsetDateTime) {
    for hit in hits.iter_mut() {
        let ranking = ranking_for(hit, now);
        hit.recall_score = recall_score(&ranking);
        hit.ranking = ranking;
    }

    hits.sort_by(|left, right| {
        right
            .recall_score
            .total_cmp(&left.recall_score)
            .then_with(|| right.memory.updated_at.cmp(&left.memory.updated_at))
            .then_with(|| right.memory.created_at.cmp(&left.memory.created_at))
            .then_with(|| left.memory.id.cmp(&right.memory.id))
    });
    hits.truncate(limit);
}

fn ranking_for(hit: &MemorySearchHit, now: OffsetDateTime) -> MemoryRecallRanking {
    let topic_match_tier = hit.relevance_score.clamp(0, 2) as u8;
    let importance_rank = importance_rank(hit.memory.importance);
    let scope_rank = scope_rank(hit.memory.scope);
    let source_backed = hit
        .memory
        .source_id
        .as_deref()
        .is_some_and(|source| !source.trim().is_empty());
    let superseded = hit.memory.superseded_by.is_some();
    let age_days = age_days(&hit.memory.updated_at, now).max(0.0).round() as u32;

    MemoryRecallRanking {
        topic_match: topic_match_label(topic_match_tier).to_owned(),
        topic_match_tier,
        importance_rank,
        scope_rank,
        source_backed,
        superseded,
        age_days,
    }
}

fn recall_score(ranking: &MemoryRecallRanking) -> f64 {
    let lexical = (2 - ranking.topic_match_tier as usize) as f64 * LEXICAL_TIER_WEIGHT;
    let importance = IMPORTANCE_WEIGHTS[ranking.importance_rank as usize];
    let scope = SCOPE_WEIGHTS[ranking.scope_rank as usize];
    let source = if ranking.source_backed {
        SOURCE_BACKED_WEIGHT
    } else {
        0.0
    };
    let recency = RECENCY_WEIGHT / (1.0 + ranking.age_days as f64 / 30.0);
    let superseded = if ranking.superseded {
        SUPERSEDED_PENALTY
    } else {
        0.0
    };

    lexical + importance + scope + source + recency - superseded
}

fn topic_match_label(tier: u8) -> &'static str {
    match tier {
        0 => "exact_topic",
        1 => "topic_or_title",
        _ => "body",
    }
}

fn importance_rank(importance: MemoryImportance) -> u8 {
    match importance {
        MemoryImportance::Critical => 0,
        MemoryImportance::High => 1,
        MemoryImportance::Normal => 2,
        MemoryImportance::Low => 3,
    }
}

fn scope_rank(scope: MemoryScope) -> u8 {
    match scope {
        MemoryScope::Global => 0,
        MemoryScope::Project => 1,
        MemoryScope::Frontend => 2,
        MemoryScope::Session => 3,
    }
}
