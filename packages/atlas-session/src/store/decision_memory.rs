use std::collections::BTreeSet;

use rusqlite::{Connection, Transaction, params};
use serde_json::Value;
use sha2::{Digest, Sha256};

use atlas_core::{AtlasError, Result};

use crate::SessionId;

use super::types::{DecisionRecord, DecisionSearchHit};
use super::util::{hex_encode, normalize_repo_path_string};

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

    let mut stmt = if session_id.is_some() {
        conn.prepare(
            "SELECT decision_id, session_id, repo_root, summary, rationale, conclusion,
                    query_text, source_ids_json, evidence_json, related_files_json,
                    related_symbols_json, created_at, updated_at
             FROM decision_memory
             WHERE repo_root = ?1 AND session_id = ?2
             ORDER BY updated_at DESC, created_at DESC",
        )
    } else {
        conn.prepare(
            "SELECT decision_id, session_id, repo_root, summary, rationale, conclusion,
                    query_text, source_ids_json, evidence_json, related_files_json,
                    related_symbols_json, created_at, updated_at
             FROM decision_memory
             WHERE repo_root = ?1
             ORDER BY updated_at DESC, created_at DESC",
        )
    }
    .map_err(|e| AtlasError::Db(e.to_string()))?;

    let rows = if let Some(session_id) = session_id {
        stmt.query_map(params![repo_root, session_id], row_to_decision)
    } else {
        stmt.query_map(params![repo_root], row_to_decision)
    }
    .map_err(|e| AtlasError::Db(e.to_string()))?;

    let tokens = query_tokens(&normalized_query);
    let mut hits = Vec::new();
    for row in rows {
        let decision = row.map_err(|e| AtlasError::Db(e.to_string()))?;
        let (score, matched_terms) = score_decision(&decision, &normalized_query, &tokens);
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
