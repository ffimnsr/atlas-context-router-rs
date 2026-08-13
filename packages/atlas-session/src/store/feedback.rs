//! ICM-C — Feedback record storage layer.
//!
//! The `feedback_records` table lives in the continuity-owned session
//! database, next to memories and decision memory. This module owns
//! validation, schema checks, search, stats, and the exact-match query used
//! by confidence adjustment. Feedback is never coupled to graph tables.

use std::collections::BTreeMap;

use rusqlite::{Connection, params};
use sha2::{Digest, Sha256};

use atlas_core::{AtlasError, Clock, Result, SystemClock, format_rfc3339};

use super::types::{
    FeedbackRecord, FeedbackSearchFilter, FeedbackSearchHit, FeedbackStats, NewFeedback,
};
use super::util::hex_encode;

pub(super) const FEEDBACK_TABLE: &str = "feedback_records";
pub(super) const FEEDBACK_FTS_TABLE: &str = "feedback_records_fts";

/// Exact column set of the `feedback_records` table (migration 010).
pub(super) const FEEDBACK_COLUMNS: &[&str] = &[
    "id",
    "repo_root",
    "session_id",
    "tool_name",
    "analysis_kind",
    "predicted",
    "actual",
    "correction",
    "related_symbol",
    "related_file",
    "source_id",
    "created_at",
    "metadata_json",
];

/// Exact index set of the `feedback_records` table (migration 010).
pub(super) const FEEDBACK_INDEXES: &[&str] = &[
    "idx_feedback_repo_kind",
    "idx_feedback_repo_symbol",
    "idx_feedback_repo_file",
];

/// Candidate multiplier used before FTS ranking caps the result set.
const FTS_CANDIDATE_MULTIPLIER: usize = 8;

// ── IDs and timestamps ────────────────────────────────────────────────────────

/// Stable per-record id: hex SHA-256 of `repo_root`, `predicted`, `actual`,
/// and a creation nanosecond nonce.
fn derive_feedback_id(
    repo_root: &str,
    predicted: &str,
    actual: &str,
    created_at_nanos: i128,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_root.as_bytes());
    hasher.update(b"\x00");
    hasher.update(predicted.as_bytes());
    hasher.update(b"\x00");
    hasher.update(actual.as_bytes());
    hasher.update(b"\x00");
    hasher.update(created_at_nanos.to_string().as_bytes());
    hex_encode(&hasher.finalize())
}

/// RFC 3339 timestamp normalized to second precision (matches memory rows).
fn format_feedback_now() -> String {
    format_rfc3339(
        SystemClock
            .now_utc()
            .replace_nanosecond(0)
            .expect("0 nanoseconds is always valid"),
    )
}

// ── Schema validation ─────────────────────────────────────────────────────────

/// Returns schema issues for the feedback tables; empty when healthy.
///
/// Used by `atlas db check` (CLI and MCP) to validate the feedback schema.
pub(super) fn feedback_schema_issues(conn: &Connection) -> Vec<String> {
    let mut issues = Vec::new();

    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'feedback_records'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        issues.push(format!("missing table: {FEEDBACK_TABLE}"));
        return issues;
    }

    let present_columns = conn
        .prepare("PRAGMA table_info('feedback_records')")
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
    for column in FEEDBACK_COLUMNS {
        if !present_columns.iter().any(|present| present == column) {
            issues.push(format!("missing column: feedback_records.{column}"));
        }
    }

    for index in FEEDBACK_INDEXES {
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

    let fts_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'feedback_records_fts'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if fts_exists == 0 {
        issues.push(format!("missing table: {FEEDBACK_FTS_TABLE}"));
    }

    issues
}

// ── Write ─────────────────────────────────────────────────────────────────────

/// Validate and persist a new feedback record, deriving its id and timestamp.
pub(super) fn store_feedback(conn: &Connection, input: &NewFeedback) -> Result<FeedbackRecord> {
    input.validate()?;
    let id = derive_feedback_id(
        &input.repo_root,
        &input.predicted,
        &input.actual,
        SystemClock.now_utc().unix_timestamp_nanos(),
    );
    let created_at = format_feedback_now();
    let metadata_json = serde_json::to_string(&input.metadata)?;
    conn.execute(
        "INSERT INTO feedback_records
            (id, repo_root, session_id, tool_name, analysis_kind, predicted, actual,
             correction, related_symbol, related_file, source_id, created_at, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            id,
            input.repo_root,
            input.session_id,
            input.tool_name,
            input.analysis_kind,
            input.predicted,
            input.actual,
            input.correction,
            input.related_symbol,
            input.related_file,
            input.source_id,
            created_at,
            metadata_json,
        ],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(FeedbackRecord {
        id,
        repo_root: input.repo_root.clone(),
        session_id: input.session_id.clone(),
        tool_name: input.tool_name.clone(),
        analysis_kind: input.analysis_kind.clone(),
        predicted: input.predicted.clone(),
        actual: input.actual.clone(),
        correction: input.correction.clone(),
        related_symbol: input.related_symbol.clone(),
        related_file: input.related_file.clone(),
        source_id: input.source_id.clone(),
        created_at,
        metadata: input.metadata.clone(),
    })
}

// ── Search ────────────────────────────────────────────────────────────────────

/// Build a safe FTS5 MATCH expression: safe tokens become prefix queries,
/// everything else is quoted verbatim.
fn build_fts_query(query: &str) -> String {
    query
        .split_whitespace()
        .map(|token| {
            if token
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
            {
                if token.len() >= 3 {
                    format!("{token}*")
                } else {
                    token.to_owned()
                }
            } else {
                format!("\"{}\"", token.replace('"', "\"\""))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_fts_fallback_error(error: &AtlasError) -> bool {
    match error {
        AtlasError::Db(message) => {
            message.contains("no such table: feedback_records_fts")
                || message.contains("no such module: fts5")
                || message.contains("unable to use function MATCH")
                || message.contains("malformed MATCH expression")
        }
        _ => false,
    }
}

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Exact-match filter clauses appended to feedback queries.
///
/// `prefix` qualifies column names (e.g. `"f."` when joined against the FTS
/// table, `""` for the plain LIKE scan) so joined queries stay unambiguous.
fn append_feedback_filters(
    sql: &mut String,
    params: &mut Vec<String>,
    filter: &FeedbackSearchFilter,
    prefix: &str,
) {
    if let Some(tool_name) = &filter.tool_name {
        sql.push_str(&format!(" AND {prefix}tool_name = ?"));
        params.push(tool_name.clone());
    }
    if let Some(kind) = &filter.analysis_kind {
        sql.push_str(&format!(" AND {prefix}analysis_kind = ?"));
        params.push(kind.clone());
    }
    if let Some(symbol) = &filter.related_symbol {
        sql.push_str(&format!(" AND {prefix}related_symbol = ?"));
        params.push(symbol.clone());
    }
    if let Some(file) = &filter.related_file {
        sql.push_str(&format!(" AND {prefix}related_file = ?"));
        params.push(file.clone());
    }
}

/// Lexical feedback search: FTS5 (BM25) over predicted/actual/correction/
/// related_symbol/related_file with exact-match filters, falling back to a
/// deterministic LIKE scan when FTS is unavailable.
pub(super) fn search_feedback(
    conn: &Connection,
    repo_root: &str,
    query: &str,
    filter: &FeedbackSearchFilter,
    limit: usize,
) -> Result<Vec<FeedbackSearchHit>> {
    let normalized = query.trim().to_lowercase();
    if normalized.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    let fts_query = build_fts_query(&normalized);
    let fts_limit = (limit * FTS_CANDIDATE_MULTIPLIER).max(limit);
    match search_feedback_fts(conn, repo_root, &fts_query, filter, fts_limit) {
        Ok(mut hits) => {
            hits.truncate(limit);
            Ok(hits)
        }
        Err(error) if is_fts_fallback_error(&error) => {
            search_feedback_like(conn, repo_root, &normalized, filter, limit)
        }
        Err(error) => Err(error),
    }
}

fn search_feedback_fts(
    conn: &Connection,
    repo_root: &str,
    fts_query: &str,
    filter: &FeedbackSearchFilter,
    limit: usize,
) -> Result<Vec<FeedbackSearchHit>> {
    let mut sql = String::from(
        "SELECT f.id, f.repo_root, f.session_id, f.tool_name, f.analysis_kind, f.predicted,
                f.actual, f.correction, f.related_symbol, f.related_file, f.source_id,
                f.created_at, f.metadata_json,
                bm25(feedback_records_fts, 8.0, 8.0, 5.0, 4.0, 4.0) AS fts_rank
         FROM feedback_records_fts
         JOIN feedback_records f ON f.rowid = feedback_records_fts.rowid
         WHERE feedback_records_fts MATCH ?1
           AND f.repo_root = ?2",
    );
    let mut params = vec![fts_query.to_owned(), repo_root.to_owned()];
    append_feedback_filters(&mut sql, &mut params, filter, "f.");
    sql.push_str(" ORDER BY fts_rank ASC, f.created_at DESC, f.id LIMIT ?");
    params.push(limit.to_string());

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs = params.iter().map(String::as_str).collect::<Vec<_>>();
    stmt.query_map(rusqlite::params_from_iter(param_refs), |row| {
        // FTS5 bm25() returns negative ranks (lower = better); negate so
        // higher scores mean more relevant.
        let fts_rank: f64 = row.get(13)?;
        Ok(FeedbackSearchHit {
            feedback: row_to_feedback(row)?,
            relevance_score: (-fts_rank).max(0.0) as f32,
        })
    })
    .map_err(|e| AtlasError::Db(e.to_string()))?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(|e| AtlasError::Db(e.to_string()))
}

/// Deterministic LIKE fallback used when FTS is unavailable.
fn search_feedback_like(
    conn: &Connection,
    repo_root: &str,
    normalized: &str,
    filter: &FeedbackSearchFilter,
    limit: usize,
) -> Result<Vec<FeedbackSearchHit>> {
    let like = format!("%{}%", escape_like(normalized));
    let mut sql = String::from(
        "SELECT id, repo_root, session_id, tool_name, analysis_kind, predicted,
                actual, correction, related_symbol, related_file, source_id,
                created_at, metadata_json
         FROM feedback_records
         WHERE repo_root = ?1
           AND (LOWER(predicted) LIKE ?2 ESCAPE '\\'
                OR LOWER(actual) LIKE ?2 ESCAPE '\\'
                OR LOWER(correction) LIKE ?2 ESCAPE '\\'
                OR LOWER(COALESCE(related_symbol, '')) LIKE ?2 ESCAPE '\\'
                OR LOWER(COALESCE(related_file, '')) LIKE ?2 ESCAPE '\\')",
    );
    let mut params = vec![repo_root.to_owned(), like];
    append_feedback_filters(&mut sql, &mut params, filter, "");
    sql.push_str(" ORDER BY created_at DESC, id LIMIT ?");
    params.push(limit.to_string());

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs = params.iter().map(String::as_str).collect::<Vec<_>>();
    stmt.query_map(rusqlite::params_from_iter(param_refs), |row| {
        Ok(FeedbackSearchHit {
            feedback: row_to_feedback(row)?,
            relevance_score: 0.0,
        })
    })
    .map_err(|e| AtlasError::Db(e.to_string()))?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(|e| AtlasError::Db(e.to_string()))
}

// ── Recent records ─────────────────────────────────────────────────────────────

/// Most recent feedback records for a repo, newest first.
///
/// Used by wake-up packs (ICM-D) so session-start recall surfaces recent
/// corrections without requiring a search query.
pub(super) fn recent_feedback(
    conn: &Connection,
    repo_root: &str,
    limit: usize,
) -> Result<Vec<FeedbackRecord>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(
            "SELECT id, repo_root, session_id, tool_name, analysis_kind, predicted,
                    actual, correction, related_symbol, related_file, source_id,
                    created_at, metadata_json
             FROM feedback_records
             WHERE repo_root = ?1
             ORDER BY created_at DESC, id DESC
             LIMIT ?2",
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    stmt.query_map(params![repo_root, limit as i64], row_to_feedback)
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))
}

// ── Stats ─────────────────────────────────────────────────────────────────────

/// Deterministic feedback statistics; stable zero-counts on an empty table.
pub(super) fn feedback_stats(conn: &Connection, repo_root: &str) -> Result<FeedbackStats> {
    let total_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM feedback_records WHERE repo_root = ?1",
            params![repo_root],
            |row| row.get(0),
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let correction_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM feedback_records WHERE repo_root = ?1 AND correction <> ''",
            params![repo_root],
            |row| row.get(0),
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let rows = conn
        .prepare(
            "SELECT id, repo_root, session_id, tool_name, analysis_kind, predicted,
                    actual, correction, related_symbol, related_file, source_id,
                    created_at, metadata_json
             FROM feedback_records WHERE repo_root = ?1",
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .query_map(params![repo_root], row_to_feedback)
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let mut false_positive_count = 0usize;
    let mut by_analysis_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_tool: BTreeMap<String, usize> = BTreeMap::new();
    for row in &rows {
        if row.is_false_positive_evidence() {
            false_positive_count += 1;
        }
        if !row.analysis_kind.is_empty() {
            *by_analysis_kind
                .entry(row.analysis_kind.clone())
                .or_insert(0) += 1;
        }
        if !row.tool_name.is_empty() {
            *by_tool.entry(row.tool_name.clone()).or_insert(0) += 1;
        }
    }

    Ok(FeedbackStats {
        total_count: total_count as usize,
        correction_count: correction_count as usize,
        false_positive_count,
        by_analysis_kind,
        by_tool,
    })
}

// ── Confidence adjustment support ─────────────────────────────────────────────

/// Records that can serve as false-positive evidence for a symbol/file/kind.
///
/// Used by the CLI confidence adjuster (ICM-C3); the adjuster itself applies
/// the matching rule (symbol, file, or analysis kind) and never lowers
/// confidence without a matching record.
pub(super) fn feedback_matching(
    conn: &Connection,
    repo_root: &str,
    analysis_kind: &str,
    symbol: Option<&str>,
    file: Option<&str>,
) -> Result<Vec<FeedbackRecord>> {
    let mut clauses = Vec::new();
    let mut params: Vec<String> = Vec::new();
    if !analysis_kind.is_empty() {
        clauses.push("analysis_kind = ?".to_owned());
        params.push(analysis_kind.to_owned());
    }
    if let Some(symbol) = symbol.filter(|value| !value.trim().is_empty()) {
        clauses.push("related_symbol = ?".to_owned());
        params.push(symbol.to_owned());
    }
    if let Some(file) = file.filter(|value| !value.trim().is_empty()) {
        clauses.push("related_file = ?".to_owned());
        params.push(file.to_owned());
    }
    if clauses.is_empty() {
        return Ok(Vec::new());
    }
    let mut sql = String::from(
        "SELECT id, repo_root, session_id, tool_name, analysis_kind, predicted,
                actual, correction, related_symbol, related_file, source_id,
                created_at, metadata_json
         FROM feedback_records
         WHERE repo_root = ?1",
    );
    sql.push_str(&format!(" AND ({})", clauses.join(" OR ")));
    sql.push_str(" ORDER BY created_at DESC, id");

    let mut all_params = vec![repo_root.to_owned()];
    all_params.extend(params);

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs = all_params.iter().map(String::as_str).collect::<Vec<_>>();
    stmt.query_map(rusqlite::params_from_iter(param_refs), row_to_feedback)
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))
}

// ── Row mapping ───────────────────────────────────────────────────────────────

/// Maps a `feedback_records` row (column order from migration 010) to
/// [`FeedbackRecord`].
fn row_to_feedback(row: &rusqlite::Row<'_>) -> rusqlite::Result<FeedbackRecord> {
    Ok(FeedbackRecord {
        id: row.get(0)?,
        repo_root: row.get(1)?,
        session_id: row.get(2)?,
        tool_name: row.get(3)?,
        analysis_kind: row.get(4)?,
        predicted: row.get(5)?,
        actual: row.get(6)?,
        correction: row.get(7)?,
        related_symbol: row.get(8)?,
        related_file: row.get(9)?,
        source_id: row.get(10)?,
        created_at: row.get(11)?,
        metadata: serde_json::from_str(&row.get::<_, String>(12)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                12,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feedback_fts_queries_escape_unsafe_tokens() {
        assert_eq!(build_fts_query("dead code"), "dead* code*");
        assert_eq!(build_fts_query("a"), "a");
        assert_eq!(build_fts_query("json\"quoted"), "\"json\"\"quoted\"");
    }

    #[test]
    fn feedback_id_is_deterministic_and_distinct() {
        let a = derive_feedback_id("/repo", "dead", "alive", 1_700_000_000_000_000_000);
        let b = derive_feedback_id("/repo", "dead", "alive", 1_700_000_000_000_000_000);
        let c = derive_feedback_id("/repo", "dead", "alive", 1_700_000_000_000_000_001);
        let d = derive_feedback_id("/repo", "dead", "used", 1_700_000_000_000_000_000);
        assert_eq!(a, b);
        assert_ne!(a, c, "nonce must separate same-second writes");
        assert_ne!(a, d, "actual must be part of the id");
    }
}
