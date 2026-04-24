use std::collections::BTreeSet;

use rusqlite::{Connection, ToSql, Transaction, params};
use serde_json::Value;
use sha2::{Digest, Sha256};

use atlas_core::{AtlasError, Result};

use crate::SessionId;

use super::types::{DecisionRecord, DecisionSearchHit};
use super::util::{hex_encode, normalize_repo_path_string};

const DECISION_FTS_CANDIDATE_MULTIPLIER: usize = 8;
const DECISION_PREFILTER_MULTIPLIER: usize = 4;
const DECISION_PREFILTER_MIN_ROWS: usize = 25;
const DECISION_RECENT_MIN_ROWS: usize = 10;

struct DecisionDraft {
    summary: String,
    rationale: Option<String>,
    conclusion: Option<String>,
    query_text: Option<String>,
    source_ids: Vec<String>,
    evidence: Vec<Value>,
    related_files: Vec<String>,
    related_symbols: Vec<String>,
}

pub(super) fn upsert_decision_from_event(
    tx: &Transaction<'_>,
    session_id: &SessionId,
    event_id: i64,
    payload: &Value,
    created_at: &str,
) -> Result<()> {
    let repo_root: String = tx
        .query_row(
            "SELECT repo_root FROM session_meta WHERE session_id = ?1",
            params![session_id.as_str()],
            |row| row.get(0),
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let Some(draft) = DecisionDraft::from_payload(&repo_root, payload) else {
        return Ok(());
    };

    let decision_id = derive_decision_id(&repo_root, session_id.as_str(), &draft);
    let source_ids_json = serde_json::to_string(&draft.source_ids)
        .map_err(|e| AtlasError::Other(format!("cannot serialize decision source ids: {e}")))?;
    let evidence_json = serde_json::to_string(&draft.evidence)
        .map_err(|e| AtlasError::Other(format!("cannot serialize decision evidence: {e}")))?;
    let related_files_json = serde_json::to_string(&draft.related_files)
        .map_err(|e| AtlasError::Other(format!("cannot serialize decision files: {e}")))?;
    let related_symbols_json = serde_json::to_string(&draft.related_symbols)
        .map_err(|e| AtlasError::Other(format!("cannot serialize decision symbols: {e}")))?;

    tx.execute(
        "INSERT INTO decision_memory (
            decision_id, session_id, event_id, repo_root, summary, rationale,
            conclusion, query_text, source_ids_json, evidence_json,
            related_files_json, related_symbols_json, created_at, updated_at
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6,
            ?7, ?8, ?9, ?10,
            ?11, ?12, ?13, ?13
         )
         ON CONFLICT(decision_id) DO UPDATE SET
            event_id = excluded.event_id,
            rationale = excluded.rationale,
            conclusion = excluded.conclusion,
            query_text = excluded.query_text,
            source_ids_json = excluded.source_ids_json,
            evidence_json = excluded.evidence_json,
            related_files_json = excluded.related_files_json,
            related_symbols_json = excluded.related_symbols_json,
            updated_at = excluded.updated_at",
        params![
            decision_id,
            session_id.as_str(),
            event_id,
            repo_root,
            draft.summary,
            draft.rationale,
            draft.conclusion,
            draft.query_text,
            source_ids_json,
            evidence_json,
            related_files_json,
            related_symbols_json,
            created_at,
        ],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;

    Ok(())
}

pub(super) fn search_decisions(
    conn: &Connection,
    repo_root: &str,
    session_id: Option<&str>,
    query: &str,
    limit: usize,
) -> Result<Vec<DecisionSearchHit>> {
    let normalized_query = query.trim().to_lowercase();
    if normalized_query.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    let tokens = query_tokens(&normalized_query);

    if let Some(fts_query) = build_fts_query(&normalized_query, &tokens) {
        let fts_limit = candidate_limit(limit, DECISION_FTS_CANDIDATE_MULTIPLIER, limit);
        match search_decisions_fts(
            conn,
            repo_root,
            session_id,
            &fts_query,
            &normalized_query,
            &tokens,
            fts_limit,
        ) {
            Ok(hits) if !hits.is_empty() => return Ok(finalize_fts_hits(hits, limit)),
            Ok(_) => {}
            Err(error) if is_fts_fallback_error(&error) => {}
            Err(error) => return Err(error),
        }
    }

    search_decisions_prefilter(
        conn,
        repo_root,
        session_id,
        &normalized_query,
        &tokens,
        limit,
    )
}

fn search_decisions_fts(
    conn: &Connection,
    repo_root: &str,
    session_id: Option<&str>,
    fts_query: &str,
    normalized_query: &str,
    tokens: &[String],
    limit: usize,
) -> Result<Vec<FtsDecisionHit>> {
    let phrase_pattern = like_pattern(normalized_query);
    let (sql, params): (String, Vec<Box<dyn ToSql>>) = if let Some(session_id) = session_id {
        (
            "SELECT d.decision_id, d.session_id, d.repo_root, d.summary, d.rationale, d.conclusion,
                    d.query_text, d.source_ids_json, d.evidence_json, d.related_files_json,
                    d.related_symbols_json, d.created_at, d.updated_at,
                    CASE
                        WHEN LOWER(d.summary) = ?4 THEN 0
                        WHEN LOWER(d.summary) LIKE ?5 ESCAPE '\\' THEN 1
                        WHEN LOWER(COALESCE(d.conclusion, '')) LIKE ?5 ESCAPE '\\' THEN 2
                        WHEN LOWER(COALESCE(d.query_text, '')) LIKE ?5 ESCAPE '\\' THEN 3
                        WHEN LOWER(COALESCE(d.rationale, '')) LIKE ?5 ESCAPE '\\' THEN 4
                        ELSE 5
                    END AS field_bucket,
                    bm25(decision_memory_fts, 8.0, 2.0, 5.0, 4.0, 1.0, 1.0, 1.0) AS fts_rank
             FROM decision_memory_fts
             JOIN decision_memory d ON d.decision_id = decision_memory_fts.decision_id
             WHERE decision_memory_fts MATCH ?1
               AND d.repo_root = ?2
               AND d.session_id = ?3
             ORDER BY field_bucket ASC, fts_rank ASC, d.updated_at DESC, d.created_at DESC
             LIMIT ?6"
                .to_string(),
            vec![
                Box::new(fts_query.to_owned()),
                Box::new(repo_root.to_owned()),
                Box::new(session_id.to_owned()),
                Box::new(normalized_query.to_owned()),
                Box::new(phrase_pattern.clone()),
                Box::new(limit as i64),
            ],
        )
    } else {
        (
            "SELECT d.decision_id, d.session_id, d.repo_root, d.summary, d.rationale, d.conclusion,
                    d.query_text, d.source_ids_json, d.evidence_json, d.related_files_json,
                    d.related_symbols_json, d.created_at, d.updated_at,
                    CASE
                        WHEN LOWER(d.summary) = ?3 THEN 0
                        WHEN LOWER(d.summary) LIKE ?4 ESCAPE '\\' THEN 1
                        WHEN LOWER(COALESCE(d.conclusion, '')) LIKE ?4 ESCAPE '\\' THEN 2
                        WHEN LOWER(COALESCE(d.query_text, '')) LIKE ?4 ESCAPE '\\' THEN 3
                        WHEN LOWER(COALESCE(d.rationale, '')) LIKE ?4 ESCAPE '\\' THEN 4
                        ELSE 5
                    END AS field_bucket,
                    bm25(decision_memory_fts, 8.0, 2.0, 5.0, 4.0, 1.0, 1.0, 1.0) AS fts_rank
             FROM decision_memory_fts
             JOIN decision_memory d ON d.decision_id = decision_memory_fts.decision_id
             WHERE decision_memory_fts MATCH ?1
               AND d.repo_root = ?2
             ORDER BY field_bucket ASC, fts_rank ASC, d.updated_at DESC, d.created_at DESC
             LIMIT ?5"
                .to_string(),
            vec![
                Box::new(fts_query.to_owned()),
                Box::new(repo_root.to_owned()),
                Box::new(normalized_query.to_owned()),
                Box::new(phrase_pattern.clone()),
                Box::new(limit as i64),
            ],
        )
    };

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs: Vec<&dyn ToSql> = params.iter().map(|value| value.as_ref()).collect();
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_refs), |row| {
            let decision = row_to_decision(row)?;
            let field_bucket: i64 = row.get(13)?;
            let fts_rank: f64 = row.get(14)?;
            Ok(FtsDecisionHit {
                decision,
                field_bucket,
                fts_rank,
                rust_score: 0.0,
                matched_terms: Vec::new(),
            })
        })
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let mut hits = Vec::new();
    for row in rows {
        let hit = row.map_err(|e| AtlasError::Db(e.to_string()))?;
        let (rust_score, matched_terms) = score_decision(&hit.decision, normalized_query, tokens);
        hits.push(FtsDecisionHit {
            rust_score,
            matched_terms,
            ..hit
        });
    }
    Ok(hits)
}

fn search_decisions_prefilter(
    conn: &Connection,
    repo_root: &str,
    session_id: Option<&str>,
    normalized_query: &str,
    tokens: &[String],
    limit: usize,
) -> Result<Vec<DecisionSearchHit>> {
    let terms = search_terms(normalized_query, tokens);
    let prefilter_limit = candidate_limit(
        limit,
        DECISION_PREFILTER_MULTIPLIER,
        DECISION_PREFILTER_MIN_ROWS,
    );
    let recent_limit = candidate_limit(limit, 2, DECISION_RECENT_MIN_ROWS);

    let mut candidates =
        prefilter_like_candidates(conn, repo_root, session_id, &terms, prefilter_limit)?;
    if candidates.len() < prefilter_limit {
        let recent = recent_candidates(conn, repo_root, session_id, recent_limit)?;
        merge_candidates(&mut candidates, recent);
    }

    let mut hits = Vec::new();
    for decision in candidates {
        let (score, matched_terms) = score_decision(&decision, normalized_query, tokens);
        if score <= 0.0 {
            continue;
        }
        hits.push(DecisionSearchHit {
            decision,
            relevance_score: score,
            matched_terms,
        });
    }

    hits.sort_by(|left, right| {
        right
            .relevance_score
            .total_cmp(&left.relevance_score)
            .then_with(|| right.decision.updated_at.cmp(&left.decision.updated_at))
            .then_with(|| left.decision.decision_id.cmp(&right.decision.decision_id))
    });
    hits.truncate(limit);
    Ok(hits)
}

fn prefilter_like_candidates(
    conn: &Connection,
    repo_root: &str,
    session_id: Option<&str>,
    terms: &[String],
    limit: usize,
) -> Result<Vec<DecisionRecord>> {
    if terms.is_empty() {
        return Ok(Vec::new());
    }

    let columns = [
        "summary",
        "COALESCE(rationale, '')",
        "COALESCE(conclusion, '')",
        "COALESCE(query_text, '')",
        "related_files_json",
        "related_symbols_json",
        "source_ids_json",
    ];

    let mut predicates = Vec::new();
    let mut params: Vec<Box<dyn ToSql>> = vec![Box::new(repo_root.to_owned())];
    let mut next_index = 2;
    if let Some(session_id) = session_id {
        params.push(Box::new(session_id.to_owned()));
        next_index += 1;
    }

    for term in terms {
        let pattern = like_pattern(term);
        for column in columns {
            predicates.push(format!("{column} LIKE ?{next_index} ESCAPE '\\'"));
            params.push(Box::new(pattern.clone()));
            next_index += 1;
        }
    }

    params.push(Box::new(limit as i64));
    let limit_index = next_index;
    let base_where = if session_id.is_some() {
        "repo_root = ?1 AND session_id = ?2"
    } else {
        "repo_root = ?1"
    };
    let sql = format!(
        "SELECT decision_id, session_id, repo_root, summary, rationale, conclusion,
                query_text, source_ids_json, evidence_json, related_files_json,
                related_symbols_json, created_at, updated_at
         FROM decision_memory
         WHERE {base_where} AND ({})
         ORDER BY updated_at DESC, created_at DESC
         LIMIT ?{limit_index}",
        predicates.join(" OR ")
    );

    query_candidates(conn, &sql, params)
}

fn recent_candidates(
    conn: &Connection,
    repo_root: &str,
    session_id: Option<&str>,
    limit: usize,
) -> Result<Vec<DecisionRecord>> {
    let (sql, params): (String, Vec<Box<dyn ToSql>>) = if let Some(session_id) = session_id {
        (
            "SELECT decision_id, session_id, repo_root, summary, rationale, conclusion,
                    query_text, source_ids_json, evidence_json, related_files_json,
                    related_symbols_json, created_at, updated_at
             FROM decision_memory
             WHERE repo_root = ?1 AND session_id = ?2
             ORDER BY updated_at DESC, created_at DESC
             LIMIT ?3"
                .to_string(),
            vec![
                Box::new(repo_root.to_owned()),
                Box::new(session_id.to_owned()),
                Box::new(limit as i64),
            ],
        )
    } else {
        (
            "SELECT decision_id, session_id, repo_root, summary, rationale, conclusion,
                    query_text, source_ids_json, evidence_json, related_files_json,
                    related_symbols_json, created_at, updated_at
             FROM decision_memory
             WHERE repo_root = ?1
             ORDER BY updated_at DESC, created_at DESC
             LIMIT ?2"
                .to_string(),
            vec![Box::new(repo_root.to_owned()), Box::new(limit as i64)],
        )
    };

    query_candidates(conn, &sql, params)
}

fn query_candidates(
    conn: &Connection,
    sql: &str,
    params: Vec<Box<dyn ToSql>>,
) -> Result<Vec<DecisionRecord>> {
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs: Vec<&dyn ToSql> = params.iter().map(|value| value.as_ref()).collect();
    let rows = stmt
        .query_map(rusqlite::params_from_iter(param_refs), row_to_decision)
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let mut decisions = Vec::new();
    for row in rows {
        decisions.push(row.map_err(|e| AtlasError::Db(e.to_string()))?);
    }
    Ok(decisions)
}

fn merge_candidates(into: &mut Vec<DecisionRecord>, more: Vec<DecisionRecord>) {
    let mut seen = into
        .iter()
        .map(|decision| decision.decision_id.clone())
        .collect::<BTreeSet<_>>();
    for decision in more {
        if seen.insert(decision.decision_id.clone()) {
            into.push(decision);
        }
    }
}

fn finalize_fts_hits(mut hits: Vec<FtsDecisionHit>, limit: usize) -> Vec<DecisionSearchHit> {
    hits.sort_by(|left, right| {
        left.field_bucket
            .cmp(&right.field_bucket)
            .then_with(|| left.fts_rank.total_cmp(&right.fts_rank))
            .then_with(|| right.rust_score.total_cmp(&left.rust_score))
            .then_with(|| right.decision.updated_at.cmp(&left.decision.updated_at))
            .then_with(|| left.decision.decision_id.cmp(&right.decision.decision_id))
    });
    hits.truncate(limit);
    hits.into_iter()
        .map(|hit| DecisionSearchHit {
            decision: hit.decision,
            relevance_score: hit.rust_score.max(1.0),
            matched_terms: hit.matched_terms,
        })
        .collect()
}

fn search_terms(normalized_query: &str, tokens: &[String]) -> Vec<String> {
    let mut terms = Vec::new();
    push_term(&mut terms, normalized_query);
    for token in tokens {
        push_term(&mut terms, token);
    }
    terms
}

fn candidate_limit(limit: usize, multiplier: usize, floor: usize) -> usize {
    limit.saturating_mul(multiplier).max(floor)
}

fn build_fts_query(normalized_query: &str, tokens: &[String]) -> Option<String> {
    let mut terms = search_terms(normalized_query, tokens)
        .into_iter()
        .flat_map(|term| tokenize_search_term(&term))
        .collect::<Vec<_>>();
    terms.sort();
    terms.dedup();
    if terms.is_empty() {
        return None;
    }

    Some(
        terms
            .into_iter()
            .map(|term| fts_term_query(&term))
            .collect::<Vec<_>>()
            .join(" AND "),
    )
}

fn tokenize_search_term(term: &str) -> Vec<String> {
    term.split(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-')
        .map(str::trim)
        .filter(|part| part.len() >= 2)
        .map(str::to_lowercase)
        .collect()
}

fn fts_term_query(term: &str) -> String {
    let safe = term
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_');
    if safe {
        if term.len() >= 3 {
            format!("{term}*")
        } else {
            term.to_owned()
        }
    } else {
        fts5_escape(term)
    }
}

fn fts5_escape(input: &str) -> String {
    let has_special = input
        .chars()
        .any(|ch| matches!(ch, '"' | '(' | ')' | '^' | '-' | '*'));
    if has_special {
        format!("\"{}\"", input.replace('"', "\"\""))
    } else {
        input.to_owned()
    }
}

fn like_pattern(term: &str) -> String {
    let escaped = term
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    format!("%{escaped}%")
}

fn is_fts_fallback_error(error: &AtlasError) -> bool {
    match error {
        AtlasError::Db(message) => {
            message.contains("no such table: decision_memory_fts")
                || message.contains("no such module: fts5")
                || message.contains("unable to use function MATCH")
                || message.contains("malformed MATCH expression")
        }
        _ => false,
    }
}

struct FtsDecisionHit {
    decision: DecisionRecord,
    field_bucket: i64,
    fts_rank: f64,
    rust_score: f32,
    matched_terms: Vec<String>,
}

impl DecisionDraft {
    fn from_payload(repo_root: &str, payload: &Value) -> Option<Self> {
        let summary = first_non_empty_string(&[
            payload.get("summary"),
            payload.get("decision"),
            payload.get("hook_event"),
        ])?;

        let rationale = first_non_empty_string(&[payload.get("rationale")]);
        let conclusion = first_non_empty_string(&[
            payload.get("conclusion"),
            payload.get("verdict"),
            payload.get("result"),
        ]);
        let query_text = first_non_empty_string(&[
            payload.get("query"),
            payload.get("query_hint"),
            payload.get("lookup_query"),
        ]);

        let mut source_ids = Vec::new();
        collect_string_values(payload.get("source_id"), &mut source_ids);
        collect_string_values(payload.get("source_ids"), &mut source_ids);
        collect_string_values(payload.get("saved_artifact_refs"), &mut source_ids);
        collect_string_values(payload.get("artifact_refs"), &mut source_ids);
        if let Some(meta) = payload.get("hook_metadata") {
            collect_string_values(meta.get("saved_artifact_refs"), &mut source_ids);
        }
        dedup_strings(&mut source_ids);

        let mut evidence = Vec::new();
        collect_evidence_values(payload.get("evidence"), &mut evidence);
        if let Some(meta) = payload.get("hook_metadata") {
            collect_evidence_values(meta.get("source_summaries"), &mut evidence);
            collect_evidence_values(meta.get("retrieval_hints"), &mut evidence);
        }

        let mut related_files = Vec::new();
        collect_string_values(payload.get("files"), &mut related_files);
        collect_string_values(payload.get("related_files"), &mut related_files);
        if let Some(meta) = payload.get("hook_metadata") {
            collect_string_values(meta.get("changed_files"), &mut related_files);
        }
        related_files = related_files
            .into_iter()
            .filter_map(|path| normalize_repo_path_string(repo_root, &path).or(Some(path)))
            .collect();
        dedup_strings(&mut related_files);

        let mut related_symbols = Vec::new();
        collect_string_values(payload.get("symbols"), &mut related_symbols);
        collect_string_values(payload.get("related_symbols"), &mut related_symbols);
        collect_string_values(payload.get("impacted_symbols"), &mut related_symbols);
        dedup_strings(&mut related_symbols);

        Some(Self {
            summary,
            rationale,
            conclusion,
            query_text,
            source_ids,
            evidence,
            related_files,
            related_symbols,
        })
    }
}

fn row_to_decision(row: &rusqlite::Row<'_>) -> rusqlite::Result<DecisionRecord> {
    let parse_json = |raw: String| -> rusqlite::Result<Value> {
        serde_json::from_str(&raw).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })
    };

    let source_ids = parse_json(row.get(7)?)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    let evidence = parse_json(row.get(8)?)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let related_files = parse_json(row.get(9)?)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();
    let related_symbols = parse_json(row.get(10)?)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|value| value.as_str().map(str::to_owned))
        .collect();

    Ok(DecisionRecord {
        decision_id: row.get(0)?,
        session_id: row.get(1)?,
        repo_root: row.get(2)?,
        summary: row.get(3)?,
        rationale: row.get(4)?,
        conclusion: row.get(5)?,
        query_text: row.get(6)?,
        source_ids,
        evidence,
        related_files,
        related_symbols,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}

fn derive_decision_id(repo_root: &str, session_id: &str, draft: &DecisionDraft) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_root.as_bytes());
    hasher.update(b"\x00");
    hasher.update(session_id.as_bytes());
    hasher.update(b"\x00");
    hasher.update(draft.summary.as_bytes());
    hasher.update(b"\x00");
    hasher.update(draft.conclusion.as_deref().unwrap_or_default().as_bytes());
    hasher.update(b"\x00");
    hasher.update(draft.query_text.as_deref().unwrap_or_default().as_bytes());
    hasher.update(b"\x00");
    hasher.update(draft.related_files.join("\n").as_bytes());
    hasher.update(b"\x00");
    hasher.update(draft.related_symbols.join("\n").as_bytes());
    hex_encode(&hasher.finalize())
}

fn query_tokens(query: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    query
        .split_whitespace()
        .map(|token| token.trim_matches(|ch: char| !ch.is_alphanumeric() && ch != '_' && ch != '-'))
        .filter(|token| token.len() >= 2)
        .filter_map(|token| {
            let token = token.to_lowercase();
            seen.insert(token.clone()).then_some(token)
        })
        .collect()
}

fn score_decision(
    decision: &DecisionRecord,
    normalized_query: &str,
    tokens: &[String],
) -> (f32, Vec<String>) {
    let summary = decision.summary.to_lowercase();
    let rationale = decision
        .rationale
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    let conclusion = decision
        .conclusion
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    let query_text = decision
        .query_text
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    let files = decision.related_files.join(" ").to_lowercase();
    let symbols = decision.related_symbols.join(" ").to_lowercase();
    let source_ids = decision.source_ids.join(" ").to_lowercase();

    let mut score = 0.0;
    let mut matched_terms = Vec::new();

    if summary == normalized_query {
        score += 8.0;
        matched_terms.push(normalized_query.to_owned());
    } else if summary.contains(normalized_query) {
        score += 5.0;
        matched_terms.push(normalized_query.to_owned());
    }
    if conclusion.contains(normalized_query) {
        score += 4.0;
        push_term(&mut matched_terms, normalized_query);
    }
    if query_text.contains(normalized_query) {
        score += 3.0;
        push_term(&mut matched_terms, normalized_query);
    }
    if rationale.contains(normalized_query) {
        score += 2.0;
        push_term(&mut matched_terms, normalized_query);
    }

    for token in tokens {
        if summary.contains(token) {
            score += 2.0;
            push_term(&mut matched_terms, token);
        }
        if conclusion.contains(token) {
            score += 1.5;
            push_term(&mut matched_terms, token);
        }
        if rationale.contains(token) {
            score += 1.0;
            push_term(&mut matched_terms, token);
        }
        if query_text.contains(token) {
            score += 1.0;
            push_term(&mut matched_terms, token);
        }
        if files.contains(token) || symbols.contains(token) || source_ids.contains(token) {
            score += 0.75;
            push_term(&mut matched_terms, token);
        }
    }

    (score, matched_terms)
}

fn push_term(terms: &mut Vec<String>, candidate: &str) {
    if !terms.iter().any(|term| term == candidate) {
        terms.push(candidate.to_owned());
    }
}

fn first_non_empty_string(values: &[Option<&Value>]) -> Option<String> {
    values.iter().find_map(|value| {
        value
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn collect_string_values(value: Option<&Value>, output: &mut Vec<String>) {
    match value {
        Some(Value::String(text)) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                output.push(trimmed.to_owned());
            }
        }
        Some(Value::Array(values)) => {
            for value in values {
                collect_string_values(Some(value), output);
            }
        }
        _ => {}
    }
}

fn collect_evidence_values(value: Option<&Value>, output: &mut Vec<Value>) {
    match value {
        Some(Value::Array(values)) => {
            for value in values {
                output.push(value.clone());
            }
        }
        Some(Value::Object(_)) => output.push(value.cloned().unwrap_or(Value::Null)),
        _ => {}
    }
}

fn dedup_strings(values: &mut Vec<String>) {
    let mut seen = BTreeSet::new();
    values.retain(|value| seen.insert(value.clone()));
}
