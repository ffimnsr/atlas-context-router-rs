//! VM3.2 — auxiliary memoir recall path.
//!
//! The primary lexical path only sees `memories`. This module widens recall
//! using the shipped memoir concept graph: query tokens seed concepts, a
//! bounded BFS walks 1–2 relation hops, and concept `source_id`s map back to
//! memory rows. The auxiliary path ranks independently with depth and
//! relation-type penalties and never pools raw scores with the primary path.
//!
//! Recall is strictly read-only: missing concepts are never auto-created.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use rusqlite::{Connection, OptionalExtension};

use atlas_core::{AtlasError, Clock, Result, SystemClock};

use super::memory::{age_days, append_filter_clauses, row_to_memory};
use super::memory_recall::{query_tokens, time_score};
use super::types::{
    MemoryImportance, MemoryListFilter, MemoryRecallPolicy, MemoryRecallRanking, MemoryScope,
    MemorySearchHit, MemoryViewer,
};

/// Hard ceiling for concepts visited by one auxiliary recall.
const MAX_AUX_CONCEPTS: usize = 200;

/// Auxiliary hit scored with independently ranked memoir evidence.
#[allow(clippy::too_many_arguments)]
pub(super) fn auxiliary_recall(
    conn: &Connection,
    repo_root: &str,
    query: &str,
    filter: &MemoryListFilter,
    shared_only: bool,
    viewer: &MemoryViewer,
    limit: usize,
    policy: &MemoryRecallPolicy,
    exclude: &BTreeSet<String>,
) -> Result<Vec<MemorySearchHit>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let tokens = query_tokens(query);
    if tokens.is_empty() {
        return Ok(Vec::new());
    }

    let seeds = seed_concepts(conn, repo_root, &tokens)?;
    if seeds.is_empty() {
        return Ok(Vec::new());
    }

    let max_hops = policy.aux_max_hops.clamp(1, 2);
    let mut scores = BTreeMap::<String, ConceptScore>::new();
    for seed in &seeds {
        scores.insert(
            seed.id.clone(),
            ConceptScore {
                source_id: seed.source_id.clone(),
                strength: seed.strength,
                depth: 0,
                relation_penalty: 0.0,
            },
        );
    }

    let mut frontier = seeds
        .iter()
        .map(|seed| (seed.id.clone(), 0u32))
        .collect::<VecDeque<_>>();
    let mut visited = seeds
        .iter()
        .map(|seed| seed.id.clone())
        .collect::<BTreeSet<_>>();

    while let Some((concept_id, depth)) = frontier.pop_front() {
        if depth >= max_hops || visited.len() >= MAX_AUX_CONCEPTS {
            continue;
        }
        for neighbor in neighbors(conn, repo_root, &concept_id)? {
            let next_depth = depth + 1;
            let penalty = relation_penalty(&neighbor.relation_type);
            let entry = scores
                .entry(neighbor.concept_id.clone())
                .or_insert(ConceptScore {
                    source_id: None,
                    strength: 0.0,
                    depth: next_depth,
                    relation_penalty: penalty,
                });
            if entry.depth > next_depth {
                entry.depth = next_depth;
                entry.relation_penalty = penalty;
            }
            if visited.insert(neighbor.concept_id.clone()) {
                frontier.push_back((neighbor.concept_id, next_depth));
            }
        }
    }

    let source_scores = collect_source_scores(conn, repo_root, &scores)?;
    if source_scores.is_empty() {
        return Ok(Vec::new());
    }

    let mut hits = load_auxiliary_memories(
        conn,
        repo_root,
        &source_scores,
        filter,
        shared_only,
        viewer,
        limit,
    )?;
    let now = SystemClock.now_utc();
    hits.retain(|hit| !exclude.contains(&hit.memory.id));
    for hit in hits.iter_mut() {
        let score = source_scores
            .get(hit.memory.source_id.as_deref().unwrap_or_default())
            .copied()
            .unwrap_or(0.0);
        let age_days = age_days(&hit.memory.updated_at, now).max(0.0).round() as u32;
        hit.relevance_score = 3;
        hit.recall_score = score.clamp(0.0, 1.0);
        hit.ranking = MemoryRecallRanking {
            topic_match: "auxiliary".to_owned(),
            topic_match_tier: 3,
            importance_rank: importance_rank(hit.memory.importance),
            scope_rank: scope_rank(hit.memory.scope),
            source_backed: true,
            superseded: false,
            age_days,
            origin_score: score.clamp(0.0, 1.0),
            time_score: time_score(age_days, policy.freshness_window_days),
            importance_score: importance_score(hit.memory.importance),
            recall_path: "auxiliary".to_owned(),
        };
    }
    hits.sort_by(|left, right| {
        right
            .recall_score
            .total_cmp(&left.recall_score)
            .then_with(|| right.memory.updated_at.cmp(&left.memory.updated_at))
            .then_with(|| left.memory.id.cmp(&right.memory.id))
    });
    hits.truncate(limit);
    Ok(hits)
}

#[derive(Debug, Clone)]
struct SeedConcept {
    id: String,
    source_id: Option<String>,
    strength: f64,
}

#[derive(Debug, Clone)]
struct ConceptScore {
    source_id: Option<String>,
    strength: f64,
    depth: u32,
    relation_penalty: f64,
}

#[derive(Debug, Clone)]
struct NeighborConcept {
    concept_id: String,
    relation_type: String,
}

fn seed_concepts(
    conn: &Connection,
    repo_root: &str,
    tokens: &[String],
) -> Result<Vec<SeedConcept>> {
    let mut stmt = conn
        .prepare(
            "SELECT id, name, description, source_id FROM memoir_concepts
             WHERE repo_root = ?1 ORDER BY id ASC LIMIT ?2",
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let rows = stmt
        .query_map(
            rusqlite::params![repo_root, MAX_AUX_CONCEPTS as i64],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let mut seeds = Vec::new();
    for (id, name, description, source_id) in rows {
        let name = name.to_lowercase();
        let description = description.to_lowercase();
        let mut score = 0usize;
        for token in tokens {
            if name.contains(token) {
                score += 2;
            }
            if description.contains(token) {
                score += 1;
            }
        }
        if score == 0 {
            continue;
        }
        let max_score = tokens.len() * 2;
        seeds.push(SeedConcept {
            id,
            source_id,
            strength: (score as f64 / max_score as f64).clamp(0.0, 1.0),
        });
    }
    Ok(seeds)
}

fn neighbors(conn: &Connection, repo_root: &str, current_id: &str) -> Result<Vec<NeighborConcept>> {
    let mut stmt = conn
        .prepare(
            "SELECT source_concept_id, target_concept_id, relation_type
             FROM memoir_relations
             WHERE repo_root = ?1 AND (source_concept_id = ?2 OR target_concept_id = ?2)
             ORDER BY relation_type ASC, id ASC",
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![repo_root, current_id], |row| {
            let source: String = row.get(0)?;
            let target: String = row.get(1)?;
            let relation_type: String = row.get(2)?;
            Ok((source, target, relation_type))
        })
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    Ok(rows
        .into_iter()
        .filter_map(|(source, target, relation_type)| {
            let concept_id = if source == current_id { target } else { source };
            (concept_id != current_id).then_some(NeighborConcept {
                concept_id,
                relation_type,
            })
        })
        .collect())
}

/// Resolve concept `source_id`s and attach depth/relation penalties.
fn collect_source_scores(
    conn: &Connection,
    repo_root: &str,
    scores: &BTreeMap<String, ConceptScore>,
) -> Result<BTreeMap<String, f64>> {
    let mut result = BTreeMap::new();
    for (concept_id, score) in scores {
        let source_id = match &score.source_id {
            Some(source_id) => Some(source_id.clone()),
            None => concept_source_id(conn, repo_root, concept_id)?,
        };
        let Some(source_id) = source_id else {
            continue;
        };
        if source_id.trim().is_empty() {
            continue;
        }
        let strength = if score.strength > 0.0 {
            0.5 + 0.5 * score.strength
        } else {
            0.5
        };
        let depth_penalty = 0.1 * score.depth as f64;
        let value = (strength - depth_penalty - score.relation_penalty).clamp(0.0, 1.0);
        result
            .entry(source_id)
            .and_modify(|current: &mut f64| {
                if value > *current {
                    *current = value;
                }
            })
            .or_insert(value);
    }
    Ok(result)
}

fn concept_source_id(
    conn: &Connection,
    repo_root: &str,
    concept_id: &str,
) -> Result<Option<String>> {
    conn.query_row(
        "SELECT source_id FROM memoir_concepts WHERE repo_root = ?1 AND id = ?2",
        rusqlite::params![repo_root, concept_id],
        |row| row.get(0),
    )
    .optional()
    .map_err(|e| AtlasError::Db(e.to_string()))
    .map(Option::flatten)
}

#[allow(clippy::too_many_arguments)]
fn load_auxiliary_memories(
    conn: &Connection,
    repo_root: &str,
    source_scores: &BTreeMap<String, f64>,
    filter: &MemoryListFilter,
    shared_only: bool,
    viewer: &MemoryViewer,
    limit: usize,
) -> Result<Vec<MemorySearchHit>> {
    let source_ids = source_scores.keys().cloned().collect::<Vec<_>>();
    let placeholders = (0..source_ids.len())
        .map(|_| "?")
        .collect::<Vec<_>>()
        .join(", ");
    let mut sql = format!(
        "SELECT m.id, m.repo_root, m.session_id, m.frontend, m.scope, m.topic, m.title, m.body,
                m.importance, m.created_at, m.updated_at, m.last_accessed_at, m.decay_score,
                m.source_id, m.metadata_json, m.superseded_by
         FROM memories m
         WHERE m.repo_root = ? AND m.superseded_by IS NULL
           AND m.source_id IN ({placeholders})"
    );
    let mut params = vec![repo_root.to_owned()];
    params.extend(source_ids);
    append_filter_clauses(&mut sql, &mut params, filter, "m.");
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
    sql.push_str(" ORDER BY m.updated_at DESC, m.id ASC LIMIT ?");
    params.push(limit.saturating_mul(8).max(64).to_string());

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs = params.iter().map(String::as_str).collect::<Vec<_>>();
    let hits = stmt
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
            let bm25_score = 0.0;
            Ok(MemorySearchHit {
                memory: row_to_memory(row)?,
                relevance_score: 3,
                bm25_score,
                recall_score: 0.0,
                ranking: MemoryRecallRanking {
                    topic_match: "auxiliary".to_owned(),
                    topic_match_tier: 3,
                    importance_rank: 0,
                    scope_rank: 0,
                    source_backed: true,
                    superseded: false,
                    age_days: 0,
                    origin_score: 0.0,
                    time_score: 0.0,
                    importance_score: 0.0,
                    recall_path: "auxiliary".to_owned(),
                },
            })
        })
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(hits)
}

fn relation_penalty(relation_type: &str) -> f64 {
    match relation_type {
        "replaces" | "refines" => 0.05,
        "depends_on" | "part_of" | "fixed_by" | "decided_by" => 0.1,
        "related_to" => 0.15,
        "contradicts" | "blocked_by" | "caused_by" => 0.2,
        _ => 0.15,
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

fn importance_score(importance: MemoryImportance) -> f64 {
    match importance {
        MemoryImportance::Critical => 1.0,
        MemoryImportance::High => 0.75,
        MemoryImportance::Normal => 0.5,
        MemoryImportance::Low => 0.25,
    }
}
