//! ICM-E1 / VM3 — deterministic cross-session recall ranking.
//!
//! Recall is lexical-first and token aware: `memories_fts` finds candidates
//! whose query tokens appear anywhere in topic, title, or body (multi-word
//! queries no longer require adjacency). Exact topic matches keep a dedicated
//! top tier, then origin evidence, freshness, and business importance combine
//! into a clamped `0..=1` composite score.
//!
//! VM3.2 adds a second path: a bounded auxiliary recall over memoir concepts
//! that fills the 25% quota. Each path ranks independently; raw scores are
//! never pooled across paths.

use rusqlite::Connection;
use time::OffsetDateTime;

use atlas_core::{AtlasError, Clock, Result, SystemClock};

use super::memory::{age_days, append_filter_clauses, row_to_memory};
use super::memory_recall_aux::auxiliary_recall;
use super::types::{
    MemoryImportance, MemoryListFilter, MemoryRecallPolicy, MemoryRecallRanking, MemoryScope,
    MemorySearchHit, MemoryViewer,
};

/// Maximum origin contribution (0..=ORIGIN_MAX) so lexical evidence survives
/// every softer signal even after composite weighting.
const LEXICAL_TIER_WEIGHT: f64 = 0.45;
/// Importance contribution inside the origin score.
const IMPORTANCE_ORIGIN_WEIGHT: f64 = 0.03;
/// Scope-visibility contribution inside the origin score.
const SCOPE_ORIGIN_WEIGHT: f64 = 0.015;
/// Source-backed contribution inside the origin score.
const SOURCE_ORIGIN_WEIGHT: f64 = 0.005;
/// BM25 contribution inside the origin score.
const BM25_ORIGIN_WEIGHT: f64 = 0.05;
/// Origin score for a body-only match.
const BODY_ORIGIN_FLOOR: f64 = 0.0;

/// Lexical cross-session recall with deterministic composite ranking.
///
/// Defaults to [`MemoryRecallPolicy::default`]; call
/// [`recall_memories_with_policy`] to apply `[memory.recall]` config.
pub(super) fn recall_memories(
    conn: &Connection,
    repo_root: &str,
    query: &str,
    filter: &MemoryListFilter,
    shared_only: bool,
    viewer: &MemoryViewer,
    limit: usize,
) -> Result<Vec<MemorySearchHit>> {
    recall_memories_with_policy(
        conn,
        repo_root,
        query,
        filter,
        shared_only,
        viewer,
        limit,
        &MemoryRecallPolicy::default(),
    )
}

/// Two-path quota recall: primary lexical path plus the auxiliary memoir path.
#[allow(clippy::too_many_arguments)]
pub(super) fn recall_memories_with_policy(
    conn: &Connection,
    repo_root: &str,
    query: &str,
    filter: &MemoryListFilter,
    shared_only: bool,
    viewer: &MemoryViewer,
    limit: usize,
    policy: &MemoryRecallPolicy,
) -> Result<Vec<MemorySearchHit>> {
    policy.validate()?;
    if limit == 0 {
        return Ok(Vec::new());
    }

    let mut primary = primary_recall(
        conn,
        repo_root,
        query,
        filter,
        shared_only,
        viewer,
        limit,
        policy,
    )?;

    let primary_quota = primary_quota_limit(limit, policy.primary_quota_ratio);
    primary.truncate(primary_quota);

    let mut seen = primary
        .iter()
        .map(|hit| hit.memory.id.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let remaining = limit.saturating_sub(primary.len());
    if remaining > 0 {
        let auxiliary = auxiliary_recall(
            conn,
            repo_root,
            query,
            filter,
            shared_only,
            viewer,
            remaining,
            policy,
            &seen,
        )?;
        for hit in auxiliary {
            if seen.insert(hit.memory.id.clone()) {
                primary.push(hit);
            }
        }
    }

    Ok(primary)
}

/// Primary lexical path: FTS5 candidates ranked by tier + composite score.
#[allow(clippy::too_many_arguments)]
fn primary_recall(
    conn: &Connection,
    repo_root: &str,
    query: &str,
    filter: &MemoryListFilter,
    shared_only: bool,
    viewer: &MemoryViewer,
    limit: usize,
    policy: &MemoryRecallPolicy,
) -> Result<Vec<MemorySearchHit>> {
    let tokens = query_tokens(query);
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    // AND first; fall back to OR only when the strict form finds nothing.
    let mut hits = run_primary_query(
        conn,
        repo_root,
        query,
        filter,
        shared_only,
        viewer,
        limit,
        &tokens.join(" AND "),
    )?;
    if hits.is_empty() && tokens.len() > 1 {
        hits = run_primary_query(
            conn,
            repo_root,
            query,
            filter,
            shared_only,
            viewer,
            limit,
            &tokens.join(" OR "),
        )?;
    }

    rank_hits(&mut hits, limit, SystemClock.now_utc(), policy);
    Ok(hits)
}

#[allow(clippy::too_many_arguments)]
fn run_primary_query(
    conn: &Connection,
    repo_root: &str,
    query: &str,
    filter: &MemoryListFilter,
    shared_only: bool,
    viewer: &MemoryViewer,
    limit: usize,
    fts_operator_expr: &str,
) -> Result<Vec<MemorySearchHit>> {
    let fts_query = quote_fts_expression(fts_operator_expr);
    let mut sql = String::from(
        "SELECT m.id, m.repo_root, m.session_id, m.frontend, m.scope, m.topic, m.title, m.body,
                m.importance, m.created_at, m.updated_at, m.last_accessed_at, m.decay_score,
                m.source_id, m.metadata_json, m.superseded_by,
                CASE WHEN m.topic = ?2 COLLATE NOCASE THEN 0
                     WHEN instr(lower(m.topic), lower(?2)) > 0
                       OR instr(lower(m.title), lower(?2)) > 0 THEN 1
                     ELSE 2 END AS match_tier,
                bm25(memories_fts) AS bm25
         FROM memories_fts
         JOIN memories m ON m.rowid = memories_fts.rowid
         WHERE memories_fts MATCH ?3
           AND m.repo_root = ?1
           AND m.superseded_by IS NULL",
    );
    let mut params = vec![repo_root.to_owned(), query.to_owned(), fts_query];
    append_filter_clauses(&mut sql, &mut params, filter, "m.");
    append_visibility_clause(&mut sql, &mut params, shared_only, viewer);
    sql.push_str(concat!(
        " ORDER BY match_tier ASC,",
        " CASE m.importance WHEN 'critical' THEN 0 WHEN 'high' THEN 1",
        " WHEN 'normal' THEN 2 ELSE 3 END ASC,",
        " CASE m.scope WHEN 'global' THEN 0 WHEN 'project' THEN 1",
        " WHEN 'frontend' THEN 2 ELSE 3 END ASC,",
        " CASE WHEN m.source_id IS NULL OR trim(m.source_id) = '' THEN 1 ELSE 0 END ASC,",
        " CASE WHEN m.superseded_by IS NULL THEN 0 ELSE 1 END ASC,",
        " m.updated_at DESC, m.created_at DESC",
        " LIMIT ?",
    ));
    params.push(candidate_limit(limit).to_string());

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs = params.iter().map(String::as_str).collect::<Vec<_>>();
    let hits = stmt
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
            let relevance_score: i32 = row.get(16)?;
            let bm25_score: f64 = row.get(17)?;
            Ok(MemorySearchHit {
                memory: row_to_memory(row)?,
                relevance_score,
                bm25_score,
                recall_score: 0.0,
                ranking: MemoryRecallRanking {
                    topic_match: String::new(),
                    topic_match_tier: 0,
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
            })
        })
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(hits)
}

/// Split a query into lowercase alphanumeric tokens for FTS and memoir
/// matching. Punctuation-only queries produce no tokens.
pub(super) fn query_tokens(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in query.chars() {
        if ch.is_alphanumeric() {
            current.extend(ch.to_lowercase());
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens.sort();
    tokens.dedup();
    tokens
}

/// Quote each FTS5 token and join with the selected operator.
fn quote_fts_expression(expression: &str) -> String {
    expression
        .split(" AND ")
        .flat_map(|part| part.split(" OR "))
        .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(if expression.contains(" OR ") {
            " OR "
        } else {
            " AND "
        })
}

/// Number of recall candidates to fetch before re-ranking.
pub(super) fn candidate_limit(limit: usize) -> usize {
    if limit == 0 {
        0
    } else {
        limit.saturating_mul(8).max(256)
    }
}

/// Append the scope-visibility predicate for `viewer` (ICM-A3).
///
/// Callers must alias `memories` as `m` and push the two visibility params in
/// the same order as the placeholders when `shared_only` is false.
pub(super) fn append_visibility_clause(
    sql: &mut String,
    params: &mut Vec<String>,
    shared_only: bool,
    viewer: &MemoryViewer,
) {
    if shared_only {
        sql.push_str(" AND m.scope IN ('project', 'global')");
    } else {
        sql.push_str(
            " AND (m.scope IN ('project', 'global')
                  OR (m.scope = 'session' AND m.session_id = ?)
                  OR (m.scope = 'frontend' AND m.frontend = ?))",
        );
        params.push(viewer.session_id.clone());
        params.push(viewer.frontend.clone());
    }
}

/// Primary quota: `ceil(ratio * limit)`, leaving at least one slot for the
/// auxiliary path when the ratio allows it.
fn primary_quota_limit(limit: usize, ratio: f64) -> usize {
    let quota = (ratio * limit as f64).ceil() as usize;
    quota.clamp(1, limit)
}

/// Score and sort recall hits, then keep the requested limit.
///
/// Tier order stays primary so an exact topic match can never be buried by the
/// freshness or business weights.
pub(super) fn rank_hits(
    hits: &mut Vec<MemorySearchHit>,
    limit: usize,
    now: OffsetDateTime,
    policy: &MemoryRecallPolicy,
) {
    for hit in hits.iter_mut() {
        let ranking = ranking_for(hit, now, policy);
        hit.recall_score = composite_score_with(&ranking, policy);
        hit.ranking = ranking;
    }

    hits.sort_by(|left, right| {
        left.relevance_score
            .cmp(&right.relevance_score)
            .then_with(|| right.recall_score.total_cmp(&left.recall_score))
            .then_with(|| right.memory.updated_at.cmp(&left.memory.updated_at))
            .then_with(|| right.memory.created_at.cmp(&left.memory.created_at))
            .then_with(|| left.memory.id.cmp(&right.memory.id))
    });
    hits.truncate(limit);
}

pub(super) fn ranking_for(
    hit: &MemorySearchHit,
    now: OffsetDateTime,
    policy: &MemoryRecallPolicy,
) -> MemoryRecallRanking {
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

    let origin_score = origin_score(
        topic_match_tier,
        hit.bm25_score,
        importance_rank,
        scope_rank,
        source_backed,
    );
    let time_score = time_score(age_days, policy.freshness_window_days);
    let importance_score = importance_score(hit.memory.importance);

    MemoryRecallRanking {
        topic_match: topic_match_label(topic_match_tier).to_owned(),
        topic_match_tier,
        importance_rank,
        scope_rank,
        source_backed,
        superseded,
        age_days,
        origin_score,
        time_score,
        importance_score,
        recall_path: "primary".to_owned(),
        vector_score: None,
        hybrid_score: None,
    }
}

/// `S_origin`: lexical tier dominates, then BM25, importance, scope, source.
fn origin_score(
    tier: u8,
    bm25: f64,
    importance_rank: u8,
    scope_rank: u8,
    source_backed: bool,
) -> f64 {
    let tier_component = (2 - tier.min(2)) as f64 * LEXICAL_TIER_WEIGHT;
    let bm25_component = normalize_bm25(bm25) * BM25_ORIGIN_WEIGHT;
    let importance_component = (3 - importance_rank.min(3)) as f64 / 3.0 * IMPORTANCE_ORIGIN_WEIGHT;
    let scope_component = (3 - scope_rank.min(3)) as f64 / 3.0 * SCOPE_ORIGIN_WEIGHT;
    let source_component = if source_backed {
        SOURCE_ORIGIN_WEIGHT
    } else {
        0.0
    };
    (tier_component + bm25_component + importance_component + scope_component + source_component)
        .clamp(BODY_ORIGIN_FLOOR, 1.0)
}

/// FTS5 `bm25()` is negative and lower is better; map it into `0..=1`.
fn normalize_bm25(raw: f64) -> f64 {
    if !raw.is_finite() || raw >= 0.0 {
        return 0.0;
    }
    1.0 / (1.0 + raw.abs())
}

/// `S_time`: inside the freshness window the score is 1.0.
pub(super) fn time_score(age_days: u32, freshness_window_days: u32) -> f64 {
    if age_days <= freshness_window_days {
        1.0
    } else {
        1.0 / (1.0 + age_days as f64 / 30.0)
    }
}

/// `S_busi` from importance.
fn importance_score(importance: MemoryImportance) -> f64 {
    match importance {
        MemoryImportance::Critical => 1.0,
        MemoryImportance::High => 0.75,
        MemoryImportance::Normal => 0.5,
        MemoryImportance::Low => 0.25,
    }
}

/// `(1 - w_t - w_b) * S_origin + w_t * S_time + w_b * S_busi`, clamped.
fn composite_score_with(ranking: &MemoryRecallRanking, policy: &MemoryRecallPolicy) -> f64 {
    let origin_weight = (1.0 - policy.weight_time - policy.weight_business).clamp(0.0, 1.0);
    (origin_weight * ranking.origin_score
        + policy.weight_time * ranking.time_score
        + policy.weight_business * ranking.importance_score)
        .clamp(0.0, 1.0)
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
