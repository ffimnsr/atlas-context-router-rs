//! VR4 — query-conditioned experience edges.
//!
//! Experience edges live only in `session.db`. They record "this query led from
//! this source URI to this target URI" and are never treated as code-graph
//! truth. Every edge is bound to a commit and endpoint file hashes, and stale
//! or invalid edges are never activated.

use std::collections::BTreeSet;

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use atlas_core::{AtlasError, Result};

/// Maximum stored trace summary length.
pub const EXPERIENCE_TRACE_SUMMARY_MAX_CHARS: usize = 512;
/// Maximum normalized query tokens stored per edge.
pub const EXPERIENCE_QUERY_TOKEN_CAP: usize = 64;
/// Default cosine activation threshold.
pub const DEFAULT_EXPERIENCE_ACTIVATION_COSINE: f64 = 0.80;
/// Default token-Jaccard activation threshold.
pub const DEFAULT_EXPERIENCE_ACTIVATION_JACCARD: f64 = 0.65;
/// Default maximum activated shortcuts per context build.
pub const DEFAULT_EXPERIENCE_MAX_EDGES: usize = 3;
/// Default prune retention window in days.
pub const DEFAULT_EXPERIENCE_PRUNE_DAYS: u64 = 30;

/// Freshness of one experience edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceFreshness {
    Fresh,
    PossiblyStale,
    Invalid,
}

impl ExperienceFreshness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::PossiblyStale => "possibly_stale",
            Self::Invalid => "invalid",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "possibly_stale" => Self::PossiblyStale,
            "invalid" => Self::Invalid,
            _ => Self::Fresh,
        }
    }
}

/// Direction filter for experience inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceDirection {
    #[default]
    Both,
    Outgoing,
    Incoming,
}

impl std::str::FromStr for ExperienceDirection {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "both" | "" => Ok(Self::Both),
            "outgoing" | "out" => Ok(Self::Outgoing),
            "incoming" | "in" => Ok(Self::Incoming),
            other => Err(AtlasError::Other(format!(
                "unknown experience direction: {other}; expected both, outgoing, or incoming"
            ))),
        }
    }
}

/// One experience edge row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExperienceEdgeRecord {
    pub id: String,
    pub repo_root: String,
    pub source_uri: String,
    pub target_uri: String,
    pub query_text: String,
    pub query_tokens: Vec<String>,
    pub trace_summary: String,
    pub commit_sha: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_file_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_file_hash: Option<String>,
    pub use_count: i64,
    pub confidence: f64,
    pub freshness: ExperienceFreshness,
    pub created_at: String,
    pub last_validated_at: String,
}

/// New experience edge submitted by retrieval/update workflows.
#[derive(Debug, Clone)]
pub struct NewExperienceEdge {
    pub repo_root: String,
    pub source_uri: String,
    pub target_uri: String,
    pub query_text: String,
    pub trace_summary: String,
    pub commit_sha: String,
    pub source_file_hash: Option<String>,
    pub target_file_hash: Option<String>,
    pub confidence: f64,
    pub query_embedding: Option<Vec<f32>>,
}

impl NewExperienceEdge {
    fn validated(&self) -> Result<Self> {
        let mut edge = self.clone();
        edge.repo_root = edge.repo_root.trim().to_owned();
        if edge.repo_root.is_empty() {
            return Err(AtlasError::Other(
                "experience edge repo_root must not be empty".to_owned(),
            ));
        }
        validate_atlas_uri("source_uri", &edge.source_uri)?;
        validate_atlas_uri("target_uri", &edge.target_uri)?;
        if edge.source_uri == edge.target_uri {
            return Err(AtlasError::Other(
                "experience edge must connect two distinct URIs".to_owned(),
            ));
        }
        if edge.query_text.trim().is_empty() {
            return Err(AtlasError::Other(
                "experience edge query_text must not be empty".to_owned(),
            ));
        }
        if edge.commit_sha.trim().is_empty() {
            return Err(AtlasError::Other(
                "experience edge commit_sha must not be empty".to_owned(),
            ));
        }
        if !(0.0..=1.0).contains(&edge.confidence) {
            return Err(AtlasError::Other(
                "experience edge confidence must be within 0..=1".to_owned(),
            ));
        }
        edge.trace_summary = edge
            .trace_summary
            .chars()
            .take(EXPERIENCE_TRACE_SUMMARY_MAX_CHARS)
            .collect();
        if edge.trace_summary.trim().is_empty() {
            return Err(AtlasError::Other(
                "experience edge trace_summary must not be empty".to_owned(),
            ));
        }
        Ok(edge)
    }
}

fn validate_atlas_uri(field: &str, uri: &str) -> Result<()> {
    if !uri.starts_with("atlas://") {
        return Err(AtlasError::Other(format!(
            "experience edge {field} must be an atlas:// URI"
        )));
    }
    if uri.contains("..") || uri.contains('\\') {
        return Err(AtlasError::Other(format!(
            "experience edge {field} must not contain traversal"
        )));
    }
    Ok(())
}

/// Normalize a query into lowercase alphanumeric tokens.
pub fn experience_query_tokens(query: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    let mut tokens = Vec::new();
    for token in query
        .split(|ch: char| !ch.is_alphanumeric())
        .map(|token| token.to_ascii_lowercase())
        .filter(|token| !token.is_empty())
    {
        if seen.insert(token.clone()) {
            tokens.push(token);
            if tokens.len() >= EXPERIENCE_QUERY_TOKEN_CAP {
                break;
            }
        }
    }
    tokens
}

/// Token Jaccard similarity in `0..=1`.
fn jaccard(left: &[String], right: &[String]) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let left: BTreeSet<&String> = left.iter().collect();
    let right: BTreeSet<&String> = right.iter().collect();
    let intersection = left.intersection(&right).count() as f64;
    let union = left.union(&right).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f64 = a.iter().zip(b).map(|(x, y)| *x as f64 * *y as f64).sum();
    let norm_a: f64 = a.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    let norm_b: f64 = b.iter().map(|x| (*x as f64).powi(2)).sum::<f64>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot / (norm_a * norm_b)
    }
}

fn edge_id(repo_root: &str, source_uri: &str, target_uri: &str, query_text: &str) -> String {
    let mut hasher = Sha256::new();
    for part in [repo_root, source_uri, target_uri, query_text] {
        hasher.update(part.as_bytes());
        hasher.update([0u8]);
    }
    format!("xedge_{:x}", hasher.finalize())
}

fn encode_embedding(values: &[f32]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

fn decode_embedding(blob: &[u8]) -> Vec<f32> {
    blob.as_chunks::<4>()
        .0
        .iter()
        .map(|chunk| f32::from_le_bytes(*chunk))
        .collect()
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<ExperienceEdgeRecord> {
    let tokens_json: String = row.get(5)?;
    let freshness: String = row.get(12)?;
    Ok(ExperienceEdgeRecord {
        id: row.get(0)?,
        repo_root: row.get(1)?,
        source_uri: row.get(2)?,
        target_uri: row.get(3)?,
        query_text: row.get(4)?,
        query_tokens: serde_json::from_str(&tokens_json).unwrap_or_default(),
        trace_summary: row.get(6)?,
        commit_sha: row.get(7)?,
        source_file_hash: row.get(8)?,
        target_file_hash: row.get(9)?,
        use_count: row.get(10)?,
        confidence: row.get(11)?,
        freshness: ExperienceFreshness::parse(&freshness),
        created_at: row.get(13)?,
        last_validated_at: row.get(14)?,
    })
}

const SELECT_EDGE: &str = "SELECT id, repo_root, source_uri, target_uri, query_text,
        query_tokens_json, trace_summary, commit_sha, source_file_hash, target_file_hash,
        use_count, confidence, freshness, created_at, last_validated_at
 FROM experience_edges";

fn file_path_from_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("atlas://repo/")?;
    let (_, remainder) = rest.split_once('/')?;
    let (kind, tail) = remainder.split_once('/')?;
    if kind != "file" {
        return None;
    }
    // Percent-decode the path segment pair.
    let mut out = Vec::new();
    let bytes = tail.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Insert or upsert one experience edge with redundancy suppression.
pub(super) fn upsert_experience_edge(
    conn: &Connection,
    edge: &NewExperienceEdge,
    now: &str,
) -> Result<ExperienceEdgeRecord> {
    let edge = edge.validated()?;
    let tokens = experience_query_tokens(&edge.query_text);
    let tokens_json =
        serde_json::to_string(&tokens).map_err(|error| AtlasError::Other(error.to_string()))?;
    let embedding = edge.query_embedding.as_deref().map(encode_embedding);
    let id = edge_id(
        &edge.repo_root,
        &edge.source_uri,
        &edge.target_uri,
        &edge.query_text,
    );

    // Redundancy suppression: an existing edge with the same
    // (repo_root, source_uri, target_uri, query_text) is an upsert, never a
    // second row.
    conn.execute(
        "INSERT INTO experience_edges (
             id, repo_root, source_uri, target_uri, query_text, query_tokens_json,
             query_embedding, trace_summary, commit_sha, source_file_hash, target_file_hash,
             use_count, confidence, freshness, created_at, last_validated_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, ?12, 'fresh', ?13, ?13)
         ON CONFLICT(repo_root, source_uri, target_uri, query_text) DO UPDATE SET
             use_count = experience_edges.use_count + 1,
             last_validated_at = excluded.last_validated_at,
             confidence = MAX(experience_edges.confidence, excluded.confidence),
             commit_sha = excluded.commit_sha,
             source_file_hash = COALESCE(excluded.source_file_hash, experience_edges.source_file_hash),
             target_file_hash = COALESCE(excluded.target_file_hash, experience_edges.target_file_hash),
             query_tokens_json = excluded.query_tokens_json,
             query_embedding = COALESCE(excluded.query_embedding, experience_edges.query_embedding),
             freshness = 'fresh'",
        params![
            id,
            edge.repo_root,
            edge.source_uri,
            edge.target_uri,
            edge.query_text,
            tokens_json,
            embedding,
            edge.trace_summary,
            edge.commit_sha,
            edge.source_file_hash,
            edge.target_file_hash,
            edge.confidence,
            now,
        ],
    )
    .map_err(|error| AtlasError::Db(error.to_string()))?;

    let record = conn
        .query_row(
            &format!(
                "{SELECT_EDGE} WHERE repo_root = ?1 AND source_uri = ?2 AND target_uri = ?3 AND query_text = ?4"
            ),
            params![
                edge.repo_root,
                edge.source_uri,
                edge.target_uri,
                edge.query_text
            ],
            row_to_edge,
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;

    for (uri, hash) in [
        (edge.source_uri.as_str(), edge.source_file_hash.as_deref()),
        (edge.target_uri.as_str(), edge.target_file_hash.as_deref()),
    ] {
        if let Some(path) = file_path_from_uri(uri) {
            conn.execute(
                "INSERT OR REPLACE INTO experience_edge_deps (edge_id, repo_root, file_path, file_hash)
                 VALUES (?1, ?2, ?3, ?4)",
                params![record.id, edge.repo_root, path, hash],
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        }
    }

    Ok(record)
}

/// Invalidate edges touched by changed or deleted file paths.
///
/// `changed` carries `(path, new_hash)`; a `None` hash means the path was
/// deleted, which invalidates every edge endpointing it. Otherwise the edge is
/// marked `possibly_stale` until re-validated.
pub(super) fn invalidate_experience_paths(
    conn: &Connection,
    repo_root: &str,
    changed: &[(String, Option<String>)],
) -> Result<usize> {
    let mut touched = 0usize;
    for (path, hash) in changed {
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT edge_id FROM experience_edge_deps
                 WHERE repo_root = ?1 AND file_path = ?2",
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        let edge_ids: Vec<String> = stmt
            .query_map(params![repo_root, path], |row| row.get::<_, String>(0))
            .map_err(|error| AtlasError::Db(error.to_string()))?
            .filter_map(|row| row.ok())
            .collect();
        drop(stmt);

        let freshness = if hash.is_some() {
            "possibly_stale"
        } else {
            "invalid"
        };
        for edge_id in edge_ids {
            conn.execute(
                "UPDATE experience_edges
                 SET freshness = CASE
                         WHEN freshness = 'invalid' THEN 'invalid'
                         WHEN ?2 = 'invalid' THEN 'invalid'
                         ELSE 'possibly_stale'
                     END,
                     last_validated_at = last_validated_at
                 WHERE id = ?1 AND repo_root = ?3",
                params![edge_id, freshness, repo_root],
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
            let _ = conn.execute(
                "UPDATE experience_edge_deps SET file_hash = ?1
                 WHERE edge_id = ?2 AND repo_root = ?3 AND file_path = ?4",
                params![hash, edge_id, repo_root, path],
            );
            touched += 1;
        }
    }
    Ok(touched)
}

/// Activate only `fresh` edges for one query, ranked deterministically.
pub(super) fn activate_experience_edges(
    conn: &Connection,
    repo_root: &str,
    query_text: &str,
    intent: &str,
    limit: usize,
    query_embedding: Option<&[f32]>,
) -> Result<Vec<ExperienceEdgeRecord>> {
    let _ = intent;
    let query_tokens = experience_query_tokens(query_text);
    if query_tokens.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }
    let mut stmt = conn
        .prepare(&format!(
            "{SELECT_EDGE} WHERE repo_root = ?1 AND freshness = 'fresh'"
        ))
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let mut rows = stmt
        .query(params![repo_root])
        .map_err(|error| AtlasError::Db(error.to_string()))?;

    let mut scored: Vec<(f64, ExperienceEdgeRecord)> = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| AtlasError::Db(error.to_string()))?
    {
        let record = row_to_edge(row).map_err(|error| AtlasError::Db(error.to_string()))?;
        let similarity = match (query_embedding, record_embedding(conn, &record.id)?) {
            (Some(left), Some(right)) => cosine(left, &right),
            _ => jaccard(&query_tokens, &record.query_tokens),
        };
        let threshold =
            if query_embedding.is_some() && record_embedding(conn, &record.id)?.is_some() {
                DEFAULT_EXPERIENCE_ACTIVATION_COSINE
            } else {
                DEFAULT_EXPERIENCE_ACTIVATION_JACCARD
            };
        if similarity >= threshold {
            let utility = record.confidence * (1.0 + record.use_count as f64).ln();
            scored.push((utility, record));
        }
    }

    scored.sort_by(|left, right| {
        right
            .0
            .partial_cmp(&left.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.1.target_uri.cmp(&right.1.target_uri))
            .then_with(|| left.1.id.cmp(&right.1.id))
    });
    scored.truncate(limit);
    Ok(scored.into_iter().map(|(_, edge)| edge).collect())
}

fn record_embedding(conn: &Connection, edge_id: &str) -> Result<Option<Vec<f32>>> {
    let blob: Option<Option<Vec<u8>>> = conn
        .query_row(
            "SELECT query_embedding FROM experience_edges WHERE id = ?1",
            params![edge_id],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .optional()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    Ok(blob.flatten().map(|blob| decode_embedding(&blob)))
}

/// Inspect edges touching one URI.
pub(super) fn inspect_experience_edges(
    conn: &Connection,
    repo_root: &str,
    uri: &str,
    direction: ExperienceDirection,
    limit: usize,
) -> Result<Vec<ExperienceEdgeRecord>> {
    let predicate = match direction {
        ExperienceDirection::Outgoing => "source_uri = ?2",
        ExperienceDirection::Incoming => "target_uri = ?2",
        ExperienceDirection::Both => "(source_uri = ?2 OR target_uri = ?2)",
    };
    let mut stmt = conn
        .prepare(&format!(
            "{SELECT_EDGE} WHERE repo_root = ?1 AND {predicate}
             ORDER BY last_validated_at DESC, source_uri, target_uri LIMIT ?3"
        ))
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map(params![repo_root, uri, limit as i64], row_to_edge)
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .filter_map(|row| row.ok())
        .collect();
    Ok(rows)
}

/// Prune edges older than `older_than_days`; dry-run reports without deleting.
pub(super) fn prune_experience_edges(
    conn: &Connection,
    repo_root: &str,
    older_than_days: u64,
    dry_run: bool,
) -> Result<usize> {
    let modifier = format!("-{} days", older_than_days.min(i64::MAX as u64));
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM experience_edges
             WHERE repo_root = ?1 AND last_validated_at < datetime('now', ?2)",
            params![repo_root, modifier],
            |row| row.get(0),
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    if !dry_run {
        conn.execute(
            "DELETE FROM experience_edges
             WHERE repo_root = ?1 AND last_validated_at < datetime('now', ?2)",
            params![repo_root, modifier],
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    }
    Ok(count.max(0) as usize)
}

/// Doctor checks for experience-edge invariants.
pub(super) fn experience_schema_issues(conn: &Connection) -> Vec<String> {
    let mut issues = Vec::new();
    let mut stmt = match conn.prepare(
        "SELECT id, source_uri, target_uri, freshness, confidence, use_count FROM experience_edges",
    ) {
        Ok(stmt) => stmt,
        Err(_) => return issues,
    };
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, f64>(4)?,
            row.get::<_, i64>(5)?,
        ))
    });
    let Ok(rows) = rows else {
        return issues;
    };
    for row in rows.filter_map(|row| row.ok()) {
        let (id, source_uri, target_uri, freshness, confidence, use_count) = row;
        if !source_uri.starts_with("atlas://") || !target_uri.starts_with("atlas://") {
            issues.push(format!("orphan_uri: edge_id={id}"));
        }
        if !matches!(freshness.as_str(), "fresh" | "possibly_stale" | "invalid") {
            issues.push(format!("invalid_freshness: edge_id={id}"));
        }
        if !(0.0..=1.0).contains(&confidence) {
            issues.push(format!("confidence_out_of_range: edge_id={id}"));
        }
        if use_count <= 0 {
            issues.push(format!("nonpositive_use_count: edge_id={id}"));
        }
    }
    issues
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SessionStore;
    use tempfile::TempDir;

    fn open_store() -> (TempDir, SessionStore) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".atlas").join("session.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let store = SessionStore::open(path.to_str().unwrap()).unwrap();
        (dir, store)
    }

    fn repo(dir: &TempDir) -> String {
        dir.path().to_str().unwrap().to_owned()
    }

    fn edge(repo_root: &str, target: &str, query: &str) -> NewExperienceEdge {
        NewExperienceEdge {
            repo_root: repo_root.to_owned(),
            source_uri: "atlas://repo/r/file/src%2Fauth.rs".to_owned(),
            target_uri: target.to_owned(),
            query_text: query.to_owned(),
            trace_summary: "resolve_symbol -> read_file_excerpt".to_owned(),
            commit_sha: "abc123".to_owned(),
            source_file_hash: Some("hash-a".to_owned()),
            target_file_hash: Some("hash-b".to_owned()),
            confidence: 0.9,
            query_embedding: None,
        }
    }

    #[test]
    fn upsert_increments_use_count_and_suppresses_duplicates() {
        let (dir, mut store) = open_store();
        let repo = repo(&dir);
        let first = store
            .record_experience_edge(&edge(&repo, "atlas://repo/r/file/src%2Fb.rs", "auth token"))
            .unwrap();
        let second = store
            .record_experience_edge(&edge(&repo, "atlas://repo/r/file/src%2Fb.rs", "auth token"))
            .unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(second.use_count, 2);
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM experience_edges", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn changed_hashes_mark_edges_possibly_stale_and_deletes_invalidate() {
        let (dir, mut store) = open_store();
        let repo = repo(&dir);
        let record = store
            .record_experience_edge(&edge(&repo, "atlas://repo/r/file/src%2Fb.rs", "auth token"))
            .unwrap();

        let touched = store
            .invalidate_experience_paths(
                &repo,
                &[("src/auth.rs".to_owned(), Some("new".to_owned()))],
            )
            .unwrap();
        assert_eq!(touched, 1);
        let stale = store
            .inspect_experience_edges(&repo, &record.source_uri, ExperienceDirection::Outgoing, 10)
            .unwrap();
        assert_eq!(stale[0].freshness, ExperienceFreshness::PossiblyStale);

        store
            .invalidate_experience_paths(&repo, &[("src/b.rs".to_owned(), None)])
            .unwrap();
        let invalid = store
            .inspect_experience_edges(&repo, &record.target_uri, ExperienceDirection::Incoming, 10)
            .unwrap();
        assert_eq!(invalid[0].freshness, ExperienceFreshness::Invalid);
    }

    #[test]
    fn activation_requires_threshold_and_fresh_edges() {
        let (dir, mut store) = open_store();
        let repo = repo(&dir);
        let record = store
            .record_experience_edge(&edge(&repo, "atlas://repo/r/file/src%2Fb.rs", "auth token"))
            .unwrap();

        let hits = store
            .activate_experience_edges(
                &repo,
                "auth token refresh",
                "review",
                DEFAULT_EXPERIENCE_MAX_EDGES,
            )
            .unwrap();
        assert_eq!(hits.len(), 1);

        let misses = store
            .activate_experience_edges(
                &repo,
                "database migration",
                "review",
                DEFAULT_EXPERIENCE_MAX_EDGES,
            )
            .unwrap();
        assert!(misses.is_empty());

        store
            .invalidate_experience_paths(&repo, &[("src/auth.rs".to_owned(), None)])
            .unwrap();
        let stale = store
            .activate_experience_edges(&repo, "auth token", "review", DEFAULT_EXPERIENCE_MAX_EDGES)
            .unwrap();
        assert!(stale.is_empty(), "{record:?}");
    }

    #[test]
    fn prune_dry_run_reports_without_deleting() {
        let (dir, mut store) = open_store();
        let repo = repo(&dir);
        store
            .record_experience_edge(&edge(&repo, "atlas://repo/r/file/src%2Fb.rs", "auth token"))
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE experience_edges SET last_validated_at = '2000-01-01T00:00:00Z'",
                [],
            )
            .unwrap();
        let reported = store.prune_experience_edges(&repo, 30, true).unwrap();
        assert_eq!(reported, 1);
        let count: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM experience_edges", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 1);
        let deleted = store.prune_experience_edges(&repo, 30, false).unwrap();
        assert_eq!(deleted, 1);
    }

    #[test]
    fn migration_014_applies_to_a_013_database() {
        let (dir, mut store) = open_store();
        let repo = repo(&dir);
        store
            .conn
            .execute_batch(
                "DROP TABLE IF EXISTS experience_edge_deps;
                 DROP TABLE IF EXISTS experience_edges;
                 UPDATE metadata SET value = '13' WHERE key = 'schema_version';",
            )
            .unwrap();
        store.migrate_to(14).unwrap();
        let record = store
            .record_experience_edge(&edge(&repo, "atlas://repo/r/file/src%2Fb.rs", "auth token"))
            .unwrap();
        assert!(record.id.starts_with("xedge_"));
    }

    #[test]
    fn schema_issues_flag_non_atlas_uris() {
        let (dir, mut store) = open_store();
        let repo = repo(&dir);
        store
            .record_experience_edge(&edge(&repo, "atlas://repo/r/file/src%2Fb.rs", "auth"))
            .unwrap();
        store
            .conn
            .execute("UPDATE experience_edges SET target_uri = 'file://nope'", [])
            .unwrap();
        let issues = store.experience_schema_issues();
        assert!(issues.iter().any(|issue| issue.starts_with("orphan_uri")));
    }
}
