//! ICM-G2 — overview storage, search, and freshness persistence.
//!
//! All overview rows live in the continuity-owned session database. The code
//! graph never observes these tables unless an explicit overview surface is
//! invoked.

use std::collections::HashMap;

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use atlas_core::{AtlasError, Result};
use atlas_repo::CanonicalRepoPath;
use camino::Utf8Path;

use super::budget::bound_new_overview_record;
use super::types::{
    NewOverviewRecord, OverviewDeleteResult, OverviewFreshness, OverviewListFilter,
    OverviewRanking, OverviewRecord, OverviewSearchFilter, OverviewSearchHit, OverviewSourceKind,
    OverviewStaleFilter, OverviewSubjectKind, subject_id,
};
use crate::store::SessionStore;
use crate::store::memory::format_memory_now;

const DEFAULT_SEARCH_LIMIT: usize = 20;
pub(super) const MAX_SEARCH_LIMIT: usize = 200;

const OVERVIEW_COLUMNS: &str = "id, repo_root, subject_id, subject_kind, package_name, \
     module_path, canonical_file_path, qualified_name, node_kind, line_start, line_end, \
     content_hash, title, summary, description_preview, responsibilities_json, flow_json, \
     inputs_json, outputs_json, invariants_json, gotchas_json, examples_json, tags_json, \
     source_kind, analyzer_name, analyzer_version, model_name, commit_sha, \
     graph_last_indexed_at, freshness, confidence, source_id, supersedes_id, created_at, \
     updated_at, metadata_json";

const ACTIVE_PREDICATE: &str = "NOT EXISTS (SELECT 1 FROM overview_records newer WHERE newer.supersedes_id = overview_records.id)";

fn parse_subject_kind(raw: String) -> rusqlite::Result<OverviewSubjectKind> {
    raw.parse::<OverviewSubjectKind>().map_err(to_sql_error)
}

fn parse_freshness(raw: String) -> rusqlite::Result<OverviewFreshness> {
    raw.parse::<OverviewFreshness>().map_err(to_sql_error)
}

fn parse_source_kind(raw: String) -> rusqlite::Result<OverviewSourceKind> {
    raw.parse::<OverviewSourceKind>().map_err(to_sql_error)
}

fn to_sql_error(error: AtlasError) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error))
}

fn parse_array(raw: String) -> rusqlite::Result<Vec<String>> {
    serde_json::from_str::<Vec<String>>(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Text,
            Box::new(AtlasError::Other(format!(
                "malformed overview JSON array: {error}"
            ))),
        )
    })
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<OverviewRecord> {
    Ok(OverviewRecord {
        id: row.get(0)?,
        repo_root: row.get(1)?,
        subject_id: row.get(2)?,
        subject_kind: parse_subject_kind(row.get(3)?)?,
        package_name: row.get(4)?,
        module_path: row.get(5)?,
        canonical_file_path: row.get(6)?,
        qualified_name: row.get(7)?,
        node_kind: row.get(8)?,
        line_start: row.get(9)?,
        line_end: row.get(10)?,
        content_hash: row.get(11)?,
        title: row.get(12)?,
        summary: row.get(13)?,
        description_preview: row.get(14)?,
        responsibilities: parse_array(row.get(15)?)?,
        flow: parse_array(row.get(16)?)?,
        inputs: parse_array(row.get(17)?)?,
        outputs: parse_array(row.get(18)?)?,
        invariants: parse_array(row.get(19)?)?,
        gotchas: parse_array(row.get(20)?)?,
        examples: parse_array(row.get(21)?)?,
        tags: parse_array(row.get(22)?)?,
        source_kind: parse_source_kind(row.get(23)?)?,
        analyzer_name: row.get(24)?,
        analyzer_version: row.get(25)?,
        model_name: row.get(26)?,
        commit_sha: row.get(27)?,
        graph_last_indexed_at: row.get(28)?,
        freshness: parse_freshness(row.get(29)?)?,
        confidence: row.get(30)?,
        source_id: row.get(31)?,
        supersedes_id: row.get(32)?,
        created_at: row.get(33)?,
        updated_at: row.get(34)?,
        metadata: serde_json::from_str(&row.get::<_, String>(35)?).unwrap_or_default(),
    })
}

fn encode_array(values: &[String]) -> String {
    serde_json::to_string(values).unwrap_or_else(|_| "[]".to_owned())
}

fn derive_overview_id(
    repo_root: &str,
    subject_id: &str,
    commit_sha: &str,
    source_kind: OverviewSourceKind,
    supersedes_id: Option<&str>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"overview_record");
    for part in [
        repo_root,
        subject_id,
        commit_sha,
        source_kind.as_str(),
        supersedes_id.unwrap_or(""),
    ] {
        hasher.update(b"\x00");
        hasher.update(part.as_bytes());
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn insert_fts(conn: &Connection, record: &OverviewRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO overview_record_fts
            (overview_id, title, summary, description_preview, responsibilities_json, flow_json,
             invariants_json, gotchas_json, tags_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            record.id,
            record.title,
            record.summary,
            record.description_preview,
            encode_array(&record.responsibilities),
            encode_array(&record.flow),
            encode_array(&record.invariants),
            encode_array(&record.gotchas),
            encode_array(&record.tags),
        ],
    )
    .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(())
}

fn delete_fts(conn: &Connection, overview_id: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM overview_record_fts WHERE overview_id = ?1",
        params![overview_id],
    )
    .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(())
}

fn find_previous_active(
    conn: &Connection,
    repo_root: &str,
    subject_id: &str,
    source_kind: OverviewSourceKind,
) -> Result<Option<String>> {
    let sql = format!(
        "SELECT id FROM overview_records
         WHERE repo_root = ?1 AND subject_id = ?2 AND source_kind = ?3 AND {ACTIVE_PREDICATE}
         ORDER BY created_at DESC, id DESC LIMIT 1"
    );
    conn.query_row(
        &sql,
        params![repo_root, subject_id, source_kind.as_str()],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| AtlasError::Db(error.to_string()))
}

pub(super) fn load_records(
    conn: &Connection,
    repo_root: &str,
    active_only: bool,
) -> Result<Vec<OverviewRecord>> {
    let sql = if active_only {
        format!(
            "SELECT {OVERVIEW_COLUMNS} FROM overview_records
             WHERE repo_root = ?1 AND {ACTIVE_PREDICATE}
             ORDER BY updated_at DESC, id DESC"
        )
    } else {
        format!(
            "SELECT {OVERVIEW_COLUMNS} FROM overview_records
             WHERE repo_root = ?1 ORDER BY updated_at DESC, id DESC"
        )
    };
    let mut stmt = conn
        .prepare(&sql)
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map(params![repo_root], row_to_record)
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    rows.collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))
}

fn fts_scores(conn: &Connection, query: &str) -> Result<HashMap<String, f64>> {
    let Some(match_query) = fts_match_query(query) else {
        return Ok(HashMap::new());
    };
    let mut stmt = conn
        .prepare(
            "SELECT overview_id, bm25(overview_record_fts) FROM overview_record_fts
             WHERE overview_record_fts MATCH ?1",
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map(params![match_query], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let mut scores = HashMap::new();
    for row in rows {
        let (id, bm25) = row.map_err(|error| AtlasError::Db(error.to_string()))?;
        // bm25 returns a negative score where lower is better.
        scores.insert(id, -bm25);
    }
    Ok(scores)
}

fn fts_match_query(query: &str) -> Option<String> {
    let tokens = query
        .split_whitespace()
        .map(|token| token.trim_matches(|c: char| !c.is_alphanumeric() && c != '_'))
        .filter(|token| !token.is_empty())
        .map(|token| format!("\"{}\"", token.replace('"', "")))
        .collect::<Vec<_>>();
    (!tokens.is_empty()).then(|| tokens.join(" OR "))
}

pub(super) fn recency_rank(updated_at: &str) -> f64 {
    let Ok(timestamp) = OffsetDateTime::parse(updated_at, &Rfc3339) else {
        return 0.0;
    };
    let now = OffsetDateTime::now_utc();
    let age_days = (now - timestamp).whole_seconds().max(0) as f64 / 86_400.0;
    1.0 / (1.0 + age_days)
}

fn matches_subject_kind(record: &OverviewRecord, kind: Option<OverviewSubjectKind>) -> bool {
    kind.is_none_or(|kind| record.subject_kind == kind)
}

fn matches_freshness(record: &OverviewRecord, freshness: Option<OverviewFreshness>) -> bool {
    freshness.is_none_or(|freshness| record.freshness == freshness)
}

fn matches_source_kind(record: &OverviewRecord, kind: Option<OverviewSourceKind>) -> bool {
    kind.is_none_or(|kind| record.source_kind == kind)
}

impl SessionStore {
    /// Insert a new overview record. Re-ingest supersedes the previous active
    /// record for the same repo/subject/source kind.
    pub fn insert_overview_record(&mut self, input: &NewOverviewRecord) -> Result<OverviewRecord> {
        let subject = input.subject.clone().validated()?;
        let subject_id = subject_id(&subject);
        let mut bounded = input.clone();
        bounded.subject = subject.clone();
        let mut budget = bound_new_overview_record(&mut bounded)?;
        // Ingest records whether an oversized body was routed to the content
        // store; keep that signal in the same budget metadata block the store
        // owns so readers see one consistent object.
        budget.source_body_routed = bounded
            .metadata
            .pointer("/budget/source_body_routed")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let source_kind = bounded.source_kind;
        let previous = match bounded.supersedes_id.clone() {
            Some(id) => Some(id),
            None => find_previous_active(&self.conn, &subject.repo_root, &subject_id, source_kind)?,
        };
        let id = derive_overview_id(
            &subject.repo_root,
            &subject_id,
            &subject.commit_sha,
            source_kind,
            previous.as_deref(),
        );
        let now = format_memory_now();
        let freshness = bounded.freshness.unwrap_or(OverviewFreshness::Unverified);

        let mut metadata = bounded.metadata.clone();
        if let Value::Object(map) = &mut metadata {
            map.insert(
                "budget".to_owned(),
                serde_json::to_value(&budget).unwrap_or(Value::Null),
            );
        }

        let record = OverviewRecord {
            id: id.clone(),
            repo_root: subject.repo_root.clone(),
            subject_id,
            subject_kind: subject.kind,
            package_name: subject.package_name.clone(),
            module_path: subject.module_path.clone(),
            canonical_file_path: subject.canonical_file_path.clone(),
            qualified_name: subject.qualified_name.clone(),
            node_kind: subject.node_kind.clone(),
            line_start: subject.line_start,
            line_end: subject.line_end,
            content_hash: subject.content_hash.clone(),
            title: bounded.title,
            summary: bounded.summary,
            description_preview: bounded.description_preview,
            responsibilities: bounded.responsibilities,
            flow: bounded.flow,
            inputs: bounded.inputs,
            outputs: bounded.outputs,
            invariants: bounded.invariants,
            gotchas: bounded.gotchas,
            examples: bounded.examples,
            tags: bounded.tags,
            source_kind,
            analyzer_name: bounded.analyzer_name,
            analyzer_version: bounded.analyzer_version,
            model_name: bounded.model_name,
            commit_sha: subject.commit_sha,
            graph_last_indexed_at: bounded.graph_last_indexed_at,
            freshness,
            confidence: bounded.confidence,
            source_id: bounded.source_id,
            supersedes_id: previous,
            created_at: now.clone(),
            updated_at: now,
            metadata,
        };

        self.conn
            .execute(
                "INSERT INTO overview_records
                    (id, repo_root, subject_id, subject_kind, package_name, module_path,
                     canonical_file_path, qualified_name, node_kind, line_start, line_end,
                     content_hash, title, summary, description_preview, responsibilities_json,
                     flow_json, inputs_json, outputs_json, invariants_json, gotchas_json,
                     examples_json, tags_json, source_kind, analyzer_name, analyzer_version,
                     model_name, commit_sha, graph_last_indexed_at, freshness, confidence,
                     source_id, supersedes_id, created_at, updated_at, metadata_json)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15,
                         ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28,
                         ?29, ?30, ?31, ?32, ?33, ?34, ?34, ?35)",
                params![
                    record.id,
                    record.repo_root,
                    record.subject_id,
                    record.subject_kind.as_str(),
                    record.package_name,
                    record.module_path,
                    record.canonical_file_path,
                    record.qualified_name,
                    record.node_kind,
                    record.line_start,
                    record.line_end,
                    record.content_hash,
                    record.title,
                    record.summary,
                    record.description_preview,
                    encode_array(&record.responsibilities),
                    encode_array(&record.flow),
                    encode_array(&record.inputs),
                    encode_array(&record.outputs),
                    encode_array(&record.invariants),
                    encode_array(&record.gotchas),
                    encode_array(&record.examples),
                    encode_array(&record.tags),
                    record.source_kind.as_str(),
                    record.analyzer_name,
                    record.analyzer_version,
                    record.model_name,
                    record.commit_sha,
                    record.graph_last_indexed_at,
                    record.freshness.as_str(),
                    record.confidence,
                    record.source_id,
                    record.supersedes_id,
                    record.created_at,
                    serde_json::to_string(&record.metadata).unwrap_or_else(|_| "{}".to_owned()),
                ],
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        insert_fts(&self.conn, &record)?;
        Ok(record)
    }

    pub fn get_overview_record(&self, overview_id: &str) -> Result<Option<OverviewRecord>> {
        let sql = format!("SELECT {OVERVIEW_COLUMNS} FROM overview_records WHERE id = ?1");
        self.conn
            .query_row(&sql, params![overview_id], row_to_record)
            .optional()
            .map_err(|error| AtlasError::Db(error.to_string()))
    }

    /// Resolve a subject by qualified name, canonical file path, package name,
    /// or module path. Ambiguous matches return an error listing candidates.
    pub fn resolve_overview_subject(
        &self,
        repo_root: &str,
        target: &str,
    ) -> Result<OverviewRecord> {
        let canonical_target =
            CanonicalRepoPath::from_cli_argument(Utf8Path::new(repo_root), Utf8Path::new(target))
                .ok()
                .map(|path| path.as_str().to_owned());
        let records = load_records(&self.conn, repo_root, true)?;

        let mut matches = records
            .into_iter()
            .filter(|record| {
                record.id == target
                    || record.subject_id == target
                    || record.qualified_name.as_deref() == Some(target)
                    || canonical_target
                        .as_deref()
                        .is_some_and(|target| record.canonical_file_path.as_deref() == Some(target))
                    || record.package_name.as_deref() == Some(target)
                    || record.module_path.as_deref() == Some(target)
            })
            .collect::<Vec<_>>();

        match matches.len() {
            0 => Err(AtlasError::Other(format!(
                "overview subject '{target}' not found"
            ))),
            1 => Ok(matches.remove(0)),
            _ => {
                let candidates = matches
                    .iter()
                    .map(|record| {
                        format!(
                            "{} ({})",
                            record
                                .qualified_name
                                .clone()
                                .or_else(|| record.canonical_file_path.clone())
                                .or_else(|| record.package_name.clone())
                                .or_else(|| record.module_path.clone())
                                .unwrap_or_else(|| record.subject_id.clone()),
                            record.subject_kind
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(AtlasError::Other(format!(
                    "overview subject '{target}' is ambiguous; candidates: {candidates}. Disambiguate with a subject id or qualified name"
                )))
            }
        }
    }

    pub fn search_overview_records(
        &self,
        repo_root: &str,
        query: &str,
        filter: &OverviewSearchFilter,
    ) -> Result<Vec<OverviewSearchHit>> {
        let limit = filter
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);
        let scores = fts_scores(&self.conn, query)?;
        let normalized_query = query.trim().to_ascii_lowercase();
        let canonical_file = filter.canonical_file_path.as_deref().and_then(|path| {
            CanonicalRepoPath::from_cli_argument(Utf8Path::new(repo_root), Utf8Path::new(path))
                .ok()
                .map(|path| path.as_str().to_owned())
        });

        let mut hits = Vec::new();
        for record in load_records(&self.conn, repo_root, true)? {
            if !matches_subject_kind(&record, filter.subject_kind)
                || !matches_freshness(&record, filter.freshness)
                || !matches_source_kind(&record, filter.source_kind)
            {
                continue;
            }
            if let Some(package) = filter.package_name.as_deref()
                && record.package_name.as_deref() != Some(package)
            {
                continue;
            }
            if let Some(module) = filter.module_path.as_deref()
                && record.module_path.as_deref() != Some(module)
            {
                continue;
            }
            if let Some(file) = canonical_file.as_deref()
                && record.canonical_file_path.as_deref() != Some(file)
            {
                continue;
            }

            let exact_subject_match = !normalized_query.is_empty()
                && (record.qualified_name.as_deref() == Some(query)
                    || record.canonical_file_path.as_deref() == Some(query)
                    || record.package_name.as_deref() == Some(query)
                    || record.module_path.as_deref() == Some(query)
                    || record.subject_id == query);
            let exact_title_match = !normalized_query.is_empty()
                && record.title.to_ascii_lowercase() == normalized_query;
            let exact_tag_match = !normalized_query.is_empty()
                && record
                    .tags
                    .iter()
                    .any(|tag| tag.to_ascii_lowercase() == normalized_query);
            let fts_score = scores.get(&record.id).copied().unwrap_or(0.0);
            let recency = recency_rank(&record.updated_at);
            let ranking = OverviewRanking {
                exact_subject_match,
                exact_title_match,
                exact_tag_match,
                fts_score,
                freshness_rank: record.freshness.rank(),
                confidence: record.confidence,
                recency_rank: recency,
                subject_specificity: record.subject_kind.specificity(),
            };
            let score = f64::from(ranking.exact_subject_match) * 5.0
                + f64::from(ranking.exact_title_match) * 3.0
                + f64::from(ranking.exact_tag_match) * 2.0
                + fts_score.max(0.0) * 2.0
                + f64::from(ranking.freshness_rank) * 0.5
                + ranking.confidence
                + recency * 0.5
                + f64::from(ranking.subject_specificity) * 0.1;

            // An empty query behaves like a filtered listing.
            if normalized_query.is_empty() || score > 0.0 || fts_score.abs() > 0.0 {
                hits.push(OverviewSearchHit {
                    record,
                    score,
                    ranking,
                });
            }
        }

        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.record.updated_at.cmp(&right.record.updated_at))
                .then_with(|| left.record.id.cmp(&right.record.id))
        });
        hits.truncate(limit);
        Ok(hits)
    }

    pub fn list_overview_records(
        &self,
        repo_root: &str,
        filter: &OverviewListFilter,
    ) -> Result<Vec<OverviewRecord>> {
        let limit = filter
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);
        let mut records = load_records(&self.conn, repo_root, true)?
            .into_iter()
            .filter(|record| {
                matches_subject_kind(record, filter.subject_kind)
                    && matches_freshness(record, filter.freshness)
                    && matches_source_kind(record, filter.source_kind)
                    && filter
                        .older_than
                        .as_deref()
                        .is_none_or(|cutoff| record.updated_at.as_str() < cutoff)
                    && filter
                        .newer_than
                        .as_deref()
                        .is_none_or(|cutoff| record.updated_at.as_str() >= cutoff)
            })
            .collect::<Vec<_>>();
        records.truncate(limit);
        Ok(records)
    }

    pub fn list_stale_overview_records(
        &self,
        repo_root: &str,
        filter: &OverviewStaleFilter,
    ) -> Result<Vec<OverviewRecord>> {
        let limit = filter
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);
        let canonical_file = filter.canonical_file_path.as_deref().and_then(|path| {
            CanonicalRepoPath::from_cli_argument(Utf8Path::new(repo_root), Utf8Path::new(path))
                .ok()
                .map(|path| path.as_str().to_owned())
        });
        let mut records = load_records(&self.conn, repo_root, true)?
            .into_iter()
            .filter(|record| {
                record.freshness.is_stale() || record.freshness == OverviewFreshness::Orphaned
            })
            .filter(|record| matches_subject_kind(record, filter.subject_kind))
            .filter(|record| {
                filter
                    .package_name
                    .as_deref()
                    .is_none_or(|package| record.package_name.as_deref() == Some(package))
            })
            .filter(|record| {
                filter
                    .module_path
                    .as_deref()
                    .is_none_or(|module| record.module_path.as_deref() == Some(module))
            })
            .filter(|record| {
                canonical_file
                    .as_deref()
                    .is_none_or(|file| record.canonical_file_path.as_deref() == Some(file))
            })
            .filter(|record| {
                filter
                    .since
                    .as_deref()
                    .is_none_or(|since| record.updated_at.as_str() >= since)
            })
            .collect::<Vec<_>>();
        records.truncate(limit);
        Ok(records)
    }

    pub fn mark_overview_freshness(
        &mut self,
        overview_id: &str,
        freshness: OverviewFreshness,
    ) -> Result<bool> {
        let now = format_memory_now();
        let changed = self
            .conn
            .execute(
                "UPDATE overview_records SET freshness = ?2, updated_at = ?3 WHERE id = ?1",
                params![overview_id, freshness.as_str(), now],
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        Ok(changed > 0)
    }

    pub fn supersede_overview_record(
        &mut self,
        overview_id: &str,
        supersedes_id: &str,
    ) -> Result<bool> {
        let now = format_memory_now();
        let changed = self
            .conn
            .execute(
                "UPDATE overview_records SET supersedes_id = ?2, updated_at = ?3 WHERE id = ?1",
                params![overview_id, supersedes_id, now],
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        Ok(changed > 0)
    }

    pub fn delete_overview_record(
        &mut self,
        overview_id: &str,
        dry_run: bool,
    ) -> Result<OverviewDeleteResult> {
        let found = self.get_overview_record(overview_id)?.is_some();
        if !dry_run && found {
            self.conn
                .execute(
                    "UPDATE overview_records SET supersedes_id = NULL WHERE supersedes_id = ?1",
                    params![overview_id],
                )
                .map_err(|error| AtlasError::Db(error.to_string()))?;
            delete_fts(&self.conn, overview_id)?;
            self.conn
                .execute(
                    "DELETE FROM overview_records WHERE id = ?1",
                    params![overview_id],
                )
                .map_err(|error| AtlasError::Db(error.to_string()))?;
        }
        Ok(OverviewDeleteResult {
            overview_id: overview_id.to_owned(),
            found,
            deleted: !dry_run && found,
            dry_run,
        })
    }

    /// Record a bounded packet reference for later ingest validation.
    pub fn record_overview_packet(
        &mut self,
        packet_id: &str,
        repo_root: &str,
        subject_id: &str,
        commit_sha: &str,
        evidence_ids: &[String],
    ) -> Result<()> {
        let now = format_memory_now();
        self.conn
            .execute(
                "INSERT INTO overview_packets
                    (packet_id, repo_root, subject_id, commit_sha, schema_version, evidence_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(packet_id) DO NOTHING",
                params![
                    packet_id,
                    repo_root,
                    subject_id,
                    commit_sha,
                    super::types::OVERVIEW_SCHEMA_VERSION,
                    encode_array(evidence_ids),
                    now,
                ],
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        Ok(())
    }

    pub fn get_overview_packet(&self, packet_id: &str) -> Result<Option<(String, String, String)>> {
        self.conn
            .query_row(
                "SELECT subject_id, commit_sha, repo_root FROM overview_packets WHERE packet_id = ?1",
                params![packet_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()
            .map_err(|error| AtlasError::Db(error.to_string()))
    }

    /// Validate overview schema, enum values, JSON arrays, and canonical paths.
    pub fn overview_schema_issues(&self) -> Vec<String> {
        let mut issues = Vec::new();
        match self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='overview_records'",
            [],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(0) => issues.push("missing overview_records table".to_owned()),
            Ok(_) => {}
            Err(error) => issues.push(format!("cannot inspect overview schema: {error}")),
        }

        for column in [
            "id",
            "repo_root",
            "subject_id",
            "subject_kind",
            "title",
            "summary",
            "description_preview",
            "source_kind",
            "commit_sha",
            "freshness",
            "confidence",
            "updated_at",
        ] {
            let exists: rusqlite::Result<i64> = self.conn.query_row(
                "SELECT COUNT(*) FROM pragma_table_info('overview_records') WHERE name = ?1",
                params![column],
                |row| row.get(0),
            );
            if exists.unwrap_or(0) == 0 {
                issues.push(format!("overview_records missing column: {column}"));
            }
        }

        let fts_exists: rusqlite::Result<i64> = self.conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='overview_record_fts'",
            [],
            |row| row.get(0),
        );
        if fts_exists.unwrap_or(0) == 0 {
            issues.push("missing overview_record_fts table".to_owned());
        }

        // `load_records` filters by repo_root, so scan directly for validation.
        let mut stmt = match self
            .conn
            .prepare(&format!("SELECT {OVERVIEW_COLUMNS} FROM overview_records"))
        {
            Ok(stmt) => stmt,
            Err(error) => {
                issues.push(format!("cannot read overview_records: {error}"));
                return issues;
            }
        };
        let rows = match stmt.query_map([], row_to_record) {
            Ok(rows) => rows,
            Err(error) => {
                issues.push(format!("cannot read overview_records: {error}"));
                return issues;
            }
        };
        let mut ids = std::collections::HashSet::new();
        for row in rows {
            let record = match row {
                Ok(record) => record,
                Err(error) => {
                    issues.push(format!("invalid overview row: {error}"));
                    continue;
                }
            };
            ids.insert(record.id.clone());
            if !(0.0..=1.0).contains(&record.confidence) {
                issues.push(format!(
                    "invalid overview confidence: id={} value={}",
                    record.id, record.confidence
                ));
            }
            if let Some(path) = record.canonical_file_path.as_deref() {
                let canonical = CanonicalRepoPath::from_cli_argument(
                    Utf8Path::new(&record.repo_root),
                    Utf8Path::new(path),
                );
                if canonical.as_ref().map(|path| path.as_str()) != Ok(path) {
                    issues.push(format!(
                        "noncanonical_path_rows: overview id={} path={path}",
                        record.id
                    ));
                }
            }
            if let Some(previous) = record.supersedes_id.as_deref()
                && !self.overview_id_exists(previous)
            {
                issues.push(format!(
                    "orphan overview supersedes_id: id={} supersedes_id={previous}",
                    record.id
                ));
            }
        }

        let fts_count: rusqlite::Result<i64> =
            self.conn
                .query_row("SELECT COUNT(*) FROM overview_record_fts", [], |row| {
                    row.get(0)
                });
        let record_count: rusqlite::Result<i64> =
            self.conn
                .query_row("SELECT COUNT(*) FROM overview_records", [], |row| {
                    row.get(0)
                });
        if let (Ok(fts), Ok(records)) = (fts_count, record_count)
            && fts != records
        {
            issues.push(format!(
                "overview FTS row count mismatch: fts={fts} records={records}"
            ));
        }
        issues
    }

    /// All non-empty routed body source ids, for cross-store orphan checks.
    pub fn overview_source_ids(&self) -> Vec<(String, String)> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT id, source_id FROM overview_records WHERE source_id IS NOT NULL AND source_id <> ''",
        ) else {
            return Vec::new();
        };
        stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map(|rows| rows.filter_map(|row| row.ok()).collect())
        .unwrap_or_default()
    }

    fn overview_id_exists(&self, overview_id: &str) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM overview_records WHERE id = ?1",
                params![overview_id],
                |_| Ok(()),
            )
            .optional()
            .ok()
            .flatten()
            .is_some()
    }

    /// Summary JSON helper used by CLI/MCP output.
    pub fn overview_record_summary(record: &OverviewRecord) -> Value {
        json!({
            "overview_id": record.id,
            "subject_id": record.subject_id,
            "subject_kind": record.subject_kind.as_str(),
            "freshness": record.freshness.as_str(),
            "confidence": record.confidence,
            "title": record.title,
            "canonical_file_path": record.canonical_file_path,
            "qualified_name": record.qualified_name,
            "package_name": record.package_name,
            "module_path": record.module_path,
            "source_id": record.source_id,
            "commit_sha": record.commit_sha,
        })
    }
}
