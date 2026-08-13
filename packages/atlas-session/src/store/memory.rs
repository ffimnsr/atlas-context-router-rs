//! ICM-A — Shared memory model storage layer.
//!
//! The `memories` table lives in the continuity-owned session database, next
//! to decision memory and global memory. This module owns validation, schema
//! checks, and CRUD used by CLI and MCP memory surfaces so the two cannot
//! drift on record shape, defaults, or validation.

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use time::OffsetDateTime;

use atlas_core::{AtlasError, Clock, Result, SystemClock, format_rfc3339};

use super::types::{
    MemoryConsolidationGroup, MemoryConsolidationPlan, MemoryDecayPolicy, MemoryDecayReport,
    MemoryDeleteResult, MemoryHealthCategory, MemoryHealthFinding, MemoryHealthReport,
    MemoryImportance, MemoryListFilter, MemoryPruneResult, MemoryRecord, MemorySearchHit,
    MemoryViewer, NOISY_TOPIC_ENTRIES, NewMemory, OVERSIZED_BODY_CHARS,
};
use super::util::{hex_encode, to_from_sql_error};

pub(super) const MEMORIES_TABLE: &str = "memories";

/// Exact column set of the `memories` table (migrations 007 + 009).
pub(super) const MEMORY_COLUMNS: &[&str] = &[
    "id",
    "repo_root",
    "session_id",
    "frontend",
    "scope",
    "topic",
    "title",
    "body",
    "importance",
    "created_at",
    "updated_at",
    "last_accessed_at",
    "decay_score",
    "source_id",
    "metadata_json",
    "superseded_by",
];

/// Exact index set of the `memories` table (migrations 007 + 009).
pub(super) const MEMORY_INDEXES: &[&str] = &[
    "idx_memories_repo_topic",
    "idx_memories_repo_importance",
    "idx_memories_repo_scope",
    "idx_memories_repo_session",
    "idx_memories_repo_accessed",
    "idx_memories_superseded",
];

/// Exact column set of the `memory_supersessions` table (migration 009).
pub(super) const SUPERSESSION_COLUMNS: &[&str] =
    &["old_memory_id", "new_memory_id", "reason", "created_at"];

/// Exact index set of the `memory_supersessions` table (migration 009).
pub(super) const SUPERSESSION_INDEXES: &[&str] = &["idx_memory_supersessions_new"];

// ── IDs and timestamps ────────────────────────────────────────────────────────

/// Stable per-record id: hex SHA-256 of `repo_root`, `body`, and a creation
/// nanosecond nonce. The nonce keeps identical texts stored in the same second
/// distinct without adding a dependency for random ids.
pub(super) fn derive_memory_id(repo_root: &str, body: &str, created_at_nanos: i128) -> String {
    let mut hasher = Sha256::new();
    hasher.update(repo_root.as_bytes());
    hasher.update(b"\x00");
    hasher.update(body.as_bytes());
    hasher.update(b"\x00");
    hasher.update(created_at_nanos.to_string().as_bytes());
    hex_encode(&hasher.finalize())
}

/// RFC 3339 timestamp normalized to second precision so lexicographic order
/// equals chronological order (`format_rfc3339` emits subseconds when nonzero,
/// which breaks string ordering across rows).
pub(super) fn format_memory_now() -> String {
    format_memory_timestamp(SystemClock.now_utc())
}

fn format_memory_timestamp(ts: OffsetDateTime) -> String {
    format_rfc3339(
        ts.replace_nanosecond(0)
            .expect("0 nanoseconds is always valid"),
    )
}

// ── Schema validation ─────────────────────────────────────────────────────────

/// Returns schema issues for the `memories` table; empty when healthy.
///
/// Used by `atlas db check` (CLI and MCP) to validate the memory schema.
pub(super) fn memory_schema_issues(conn: &Connection) -> Vec<String> {
    let mut issues = Vec::new();

    let table_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memories'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if table_exists == 0 {
        issues.push(format!("missing table: {MEMORIES_TABLE}"));
        return issues;
    }

    let present_columns = conn
        .prepare("PRAGMA table_info('memories')")
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
    for column in MEMORY_COLUMNS {
        if !present_columns.iter().any(|present| present == column) {
            issues.push(format!("missing column: memories.{column}"));
        }
    }

    for index in MEMORY_INDEXES {
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

    let supersessions_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'memory_supersessions'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if supersessions_exists == 0 {
        issues.push("missing table: memory_supersessions".to_owned());
    } else {
        let present_columns = conn
            .prepare("PRAGMA table_info('memory_supersessions')")
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
        for column in SUPERSESSION_COLUMNS {
            if !present_columns.iter().any(|present| present == column) {
                issues.push(format!("missing column: memory_supersessions.{column}"));
            }
        }
        for index in SUPERSESSION_INDEXES {
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
    }

    issues
}

// ── Write ─────────────────────────────────────────────────────────────────────

/// Validate and persist a new memory, deriving its id and timestamps.
pub(super) fn store_memory(conn: &Connection, input: &NewMemory) -> Result<MemoryRecord> {
    input.validate()?;
    let id = derive_memory_id(
        &input.repo_root,
        &input.body,
        SystemClock.now_utc().unix_timestamp_nanos(),
    );
    store_memory_at(conn, input, &format_memory_now(), &id)
}

/// Raw insert used by [`store_memory`] and deterministic tests.
pub(super) fn store_memory_at(
    conn: &Connection,
    input: &NewMemory,
    now: &str,
    id: &str,
) -> Result<MemoryRecord> {
    input.validate()?;
    let metadata_json = serde_json::to_string(&input.metadata)?;
    conn.execute(
        "INSERT INTO memories
            (id, repo_root, session_id, frontend, scope, topic, title, body, importance,
             created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json,
             superseded_by)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, ?10, 0, ?11, ?12, NULL)",
        params![
            id,
            input.repo_root,
            input.session_id,
            input.frontend,
            input.scope.as_str(),
            input.topic,
            input.title,
            input.body,
            input.importance.as_str(),
            now,
            input.source_id,
            metadata_json,
        ],
    )
    .map_err(|e| AtlasError::Db(e.to_string()))?;
    Ok(MemoryRecord {
        id: id.to_owned(),
        repo_root: input.repo_root.clone(),
        session_id: input.session_id.clone(),
        frontend: input.frontend.clone(),
        scope: input.scope,
        topic: input.topic.clone(),
        title: input.title.clone(),
        body: input.body.clone(),
        importance: input.importance,
        created_at: now.to_owned(),
        updated_at: now.to_owned(),
        last_accessed_at: now.to_owned(),
        decay_score: 0.0,
        source_id: input.source_id.clone(),
        metadata: input.metadata.clone(),
        superseded_by: None,
    })
}

// ── Recall and list ───────────────────────────────────────────────────────────

fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn append_filter_clauses(sql: &mut String, params: &mut Vec<String>, filter: &MemoryListFilter) {
    if let Some(topic) = &filter.topic {
        sql.push_str(" AND topic = ? COLLATE NOCASE");
        params.push(topic.clone());
    }
    if let Some(importance) = filter.importance {
        sql.push_str(" AND importance = ?");
        params.push(importance.as_str().to_owned());
    }
    if let Some(scope) = filter.scope {
        sql.push_str(" AND scope = ?");
        params.push(scope.as_str().to_owned());
    }
    if let Some(older_than) = &filter.older_than {
        sql.push_str(" AND updated_at < ?");
        params.push(older_than.clone());
    }
    if let Some(newer_than) = &filter.newer_than {
        sql.push_str(" AND updated_at > ?");
        params.push(newer_than.clone());
    }
}

fn query_memories(
    conn: &Connection,
    select_sql: &str,
    params: &[String],
) -> Result<Vec<MemoryRecord>> {
    let mut stmt = conn
        .prepare(select_sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs = params.iter().map(String::as_str).collect::<Vec<_>>();
    stmt.query_map(rusqlite::params_from_iter(param_refs), row_to_memory)
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|e| AtlasError::Db(e.to_string()))
}

/// Lexical recall: exact topic matches rank above topic/title contains
/// matches, which rank above body-only matches; ties break on importance,
/// then recency. Visibility (ICM-A3) is enforced for the given viewer:
/// `session`-scoped memories require the same session id, `frontend`-scoped
/// memories require the same frontend. `shared_only` narrows results to
/// `project` + `global` and bypasses viewer-based conditions.
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
    sql.push_str(
        " ORDER BY match_tier ASC, importance_rank ASC, superseded_rank ASC,
                  updated_at DESC, created_at DESC
         LIMIT ?",
    );
    params.push(limit.to_string());

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let param_refs = params.iter().map(String::as_str).collect::<Vec<_>>();
    stmt.query_map(rusqlite::params_from_iter(param_refs), |row| {
        let relevance_score: i32 = row.get(16)?;
        Ok(MemorySearchHit {
            memory: row_to_memory(row)?,
            relevance_score,
        })
    })
    .map_err(|e| AtlasError::Db(e.to_string()))?
    .collect::<rusqlite::Result<Vec<_>>>()
    .map_err(|e| AtlasError::Db(e.to_string()))
}

/// List memories for a repo, filtered and sorted by `updated_at DESC`.
pub(super) fn list_memories(
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
) -> Result<Vec<MemoryRecord>> {
    let mut sql = String::from(
        "SELECT id, repo_root, session_id, frontend, scope, topic, title, body, importance,
                created_at, updated_at, last_accessed_at, decay_score, source_id, metadata_json,
                superseded_by
         FROM memories
         WHERE repo_root = ?1",
    );
    let mut params = vec![repo_root.to_owned()];
    append_filter_clauses(&mut sql, &mut params, filter);
    if !filter.include_superseded {
        sql.push_str(" AND superseded_by IS NULL");
    }
    sql.push_str(" ORDER BY updated_at DESC, created_at DESC, id");
    query_memories(conn, &sql, &params)
}

// ── Delete ────────────────────────────────────────────────────────────────────

/// Delete a memory by exact id within a repo. Dry-run only reports whether
/// the row exists. Linked saved-context artifacts are never touched here.
pub(super) fn delete_memory(
    conn: &Connection,
    repo_root: &str,
    memory_id: &str,
    dry_run: bool,
) -> Result<MemoryDeleteResult> {
    let found = conn
        .query_row(
            "SELECT 1 FROM memories WHERE id = ?1 AND repo_root = ?2",
            params![memory_id, repo_root],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|e| AtlasError::Db(e.to_string()))?
        .is_some();

    let deleted = if found && !dry_run {
        let removed = conn
            .execute(
                "DELETE FROM memories WHERE id = ?1 AND repo_root = ?2",
                params![memory_id, repo_root],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        removed > 0
    } else {
        false
    };

    Ok(MemoryDeleteResult {
        memory_id: memory_id.to_owned(),
        found,
        deleted,
        dry_run,
    })
}

// ── ICM-B — decay, stale, prune, health, consolidation ────────────────────────

/// Parse a second-precision RFC 3339 timestamp stored on memory rows.
fn parse_memory_timestamp(value: &str) -> Option<OffsetDateTime> {
    OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
}

/// Whole-day age of a memory row relative to `now`, anchored on `updated_at`.
fn age_days(updated_at: &str, now: OffsetDateTime) -> f64 {
    let Some(ts) = parse_memory_timestamp(updated_at) else {
        return 0.0;
    };
    let seconds = now.unix_timestamp() - ts.unix_timestamp();
    if seconds <= 0 {
        0.0
    } else {
        seconds as f64 / 86_400.0
    }
}

/// Compute (and, when not dry-running, persist) the updated `decay_score` for
/// every memory matching `filter`. Never deletes rows; protected critical
/// memories report `score = 0.0` and are never written as stale.
pub(super) fn decay_reports(
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    policy: &MemoryDecayPolicy,
    now: OffsetDateTime,
    dry_run: bool,
) -> Result<Vec<MemoryDecayReport>> {
    if !policy.enabled {
        return Ok(Vec::new());
    }
    let mut full_filter = filter.clone();
    full_filter.include_superseded = true;
    let rows = list_memories(conn, repo_root, &full_filter)?;
    let mut reports = Vec::with_capacity(rows.len());
    for row in rows {
        let age = age_days(&row.updated_at, now);
        let score = policy.score(row.importance, age);
        let report_importance = row.importance;
        let protected = policy.is_protected(row.importance);
        let stale = !protected && score >= 1.0;
        if !dry_run {
            conn.execute(
                "UPDATE memories SET decay_score = ?1 WHERE id = ?2 AND repo_root = ?3",
                params![score, row.id, repo_root],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        }
        reports.push(MemoryDecayReport {
            memory: row,
            age_days: age,
            retention_days: policy.retention_days(report_importance),
            updated_decay_score: score,
            protected,
            stale,
        });
    }
    Ok(reports)
}

/// Rows past their retention window per policy; protected critical memories
/// are never reported as stale candidates.
pub(super) fn stale_memories(
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    policy: &MemoryDecayPolicy,
    now: OffsetDateTime,
) -> Result<Vec<MemoryDecayReport>> {
    let reports = decay_reports(conn, repo_root, filter, policy, now, true)?;
    Ok(reports.into_iter().filter(|report| report.stale).collect())
}

/// Delete (or, in dry-run, only report) memories past their retention window.
///
/// Critical rows are excluded unless `allow_critical` is set; passing
/// `--importance critical` without the override fails validation so a
/// critical-memory prune path only exists behind an explicit override.
pub(super) fn prune_memories(
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    policy: &MemoryDecayPolicy,
    now: OffsetDateTime,
    dry_run: bool,
    allow_critical: bool,
) -> Result<MemoryPruneResult> {
    if !policy.enabled {
        return Ok(MemoryPruneResult {
            dry_run,
            candidate_count: 0,
            deleted_count: 0,
            protected_count: 0,
            candidates: Vec::new(),
        });
    }
    if !allow_critical && filter.importance == Some(MemoryImportance::Critical) {
        return Err(AtlasError::Other(
            "critical memories are protected by memory.decay.critical_never_prune; \
             pass --allow-critical to include them in prune candidates"
                .to_owned(),
        ));
    }
    let reports = decay_reports(conn, repo_root, filter, policy, now, true)?;
    let mut protected_count = 0usize;
    let mut candidates = Vec::new();
    for report in reports {
        if report.protected && !allow_critical {
            protected_count += 1;
            continue;
        }
        if report.stale {
            candidates.push(report.memory);
        }
    }

    let deleted_count = if dry_run {
        0
    } else {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let mut deleted = 0usize;
        for memory in &candidates {
            deleted += tx
                .execute(
                    "DELETE FROM memories WHERE id = ?1 AND repo_root = ?2",
                    params![memory.id, repo_root],
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?;
        }
        tx.commit().map_err(|e| AtlasError::Db(e.to_string()))?;
        deleted
    };

    Ok(MemoryPruneResult {
        dry_run,
        candidate_count: candidates.len(),
        deleted_count,
        protected_count,
        candidates,
    })
}

/// Deterministic health report: stale, duplicated, orphaned, and oversized
/// findings per memory, plus noisy topics and topics without a critical
/// decision. Never depends on opaque LLM behavior.
pub(super) fn health_report(
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    policy: &MemoryDecayPolicy,
    now: OffsetDateTime,
    source_exists: &dyn Fn(&str) -> bool,
) -> Result<MemoryHealthReport> {
    let rows = list_memories(conn, repo_root, filter)?;
    let mut findings = Vec::new();
    let mut by_category: BTreeMap<String, usize> = BTreeMap::new();
    let mut count = |category: MemoryHealthCategory| {
        *by_category.entry(category.as_str().to_owned()).or_insert(0) += 1;
    };

    for (index, row) in rows.iter().enumerate() {
        let age = age_days(&row.updated_at, now);
        let protected = policy.is_protected(row.importance);
        let stale = !protected && policy.score(row.importance, age) >= 1.0;
        if stale {
            count(MemoryHealthCategory::Stale);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Stale,
                kind: "stale_memory".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} in topic '{}' is past its retention window",
                    row.id, row.topic
                ),
                suggestion: "refresh the fact or prune the row".to_owned(),
                command: prune_command(&row.topic),
            });
            continue;
        }

        let duplicated = rows[..index].iter().any(|other| {
            other.topic == row.topic
                && ((!other.title.trim().is_empty()
                    && normalize_text(&other.title) == normalize_text(&row.title))
                    || normalize_text(&other.body) == normalize_text(&row.body))
        });
        if duplicated {
            count(MemoryHealthCategory::Duplicated);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Duplicated,
                kind: "duplicated_memory".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} repeats an earlier memory in topic '{}'",
                    row.id, row.topic
                ),
                suggestion: "merge the duplicates into one fact".to_owned(),
                command: consolidate_command(&row.topic),
            });
            continue;
        }

        let orphaned = row
            .source_id
            .as_deref()
            .is_some_and(|source_id| !source_exists(source_id));
        if orphaned {
            count(MemoryHealthCategory::Orphaned);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Orphaned,
                kind: "orphaned_source".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} references a missing saved-context artifact",
                    row.id
                ),
                suggestion: "re-link the source artifact or delete the memory".to_owned(),
                command: format!("atlas memory delete {} --dry-run", row.id),
            });
            continue;
        }

        if row.body.chars().count() > OVERSIZED_BODY_CHARS {
            count(MemoryHealthCategory::Oversized);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Oversized,
                kind: "oversized_memory".to_owned(),
                memory_id: Some(row.id.clone()),
                topic: (!row.topic.is_empty()).then(|| row.topic.clone()),
                detail: format!(
                    "memory {} exceeds {} characters",
                    row.id, OVERSIZED_BODY_CHARS
                ),
                suggestion: "split the body into one-fact memories".to_owned(),
                command: format!("atlas memory store \"<fact>\" --topic {}", row.topic),
            });
        }
    }

    // Topic-level findings, deterministic alphabetical topic order.
    let mut topic_stats: BTreeMap<&str, (usize, bool)> = BTreeMap::new();
    for row in &rows {
        if row.topic.is_empty() {
            continue;
        }
        let entry = topic_stats.entry(row.topic.as_str()).or_insert((0, false));
        entry.0 += 1;
        entry.1 |= row.importance == MemoryImportance::Critical;
    }
    for (topic, (entries, has_critical)) in topic_stats {
        if entries > NOISY_TOPIC_ENTRIES {
            count(MemoryHealthCategory::Noisy);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Noisy,
                kind: "noisy_topic".to_owned(),
                memory_id: None,
                topic: Some(topic.to_owned()),
                detail: format!(
                    "topic '{topic}' has {entries} memories (noisy above {NOISY_TOPIC_ENTRIES})"
                ),
                suggestion: "consolidate the topic into fewer facts".to_owned(),
                command: consolidate_command(topic),
            });
        }
        if !has_critical {
            count(MemoryHealthCategory::Noisy);
            findings.push(MemoryHealthFinding {
                category: MemoryHealthCategory::Noisy,
                kind: "topic_without_critical".to_owned(),
                memory_id: None,
                topic: Some(topic.to_owned()),
                detail: format!(
                    "topic '{topic}' has {entries} memories but no critical decision memory"
                ),
                suggestion: "record the key decision with critical importance".to_owned(),
                command: format!(
                    "atlas memory store \"<decision>\" --topic {topic} --importance critical"
                ),
            });
        }
    }

    Ok(MemoryHealthReport {
        total_memories: rows.len(),
        findings,
        by_category,
    })
}

/// Deterministic consolidation planner (and, when not dry-running, applier).
///
/// Groups memories by topic plus one of: normalized title, normalized body,
/// same `source_id`, or same metadata category. Dry-run reports kept ids,
/// merged ids, and preserved source ids without mutating storage. Apply mode
/// creates a consolidated memory, marks merged rows as superseded, and stores
/// supersession links.
pub(super) fn consolidation_plan(
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    dry_run: bool,
) -> Result<MemoryConsolidationPlan> {
    let rows = list_memories(conn, repo_root, filter)?;

    #[derive(Clone)]
    struct MemberKeys {
        title: Option<String>,
        body: String,
        source: Option<String>,
        category: Option<String>,
    }

    struct Group {
        members: Vec<MemoryRecord>,
        keys: Vec<MemberKeys>,
    }

    let mut groups: Vec<Group> = Vec::new();
    for row in rows {
        let keys = MemberKeys {
            title: (!row.title.trim().is_empty()).then(|| normalize_text(&row.title)),
            body: normalize_text(&row.body),
            source: row.source_id.clone(),
            category: memory_category(&row.metadata),
        };
        let matches = |other: &MemberKeys| -> bool {
            (keys.title.is_some() && other.title.is_some() && keys.title == other.title)
                || keys.body == other.body
                || (keys.source.is_some() && keys.source == other.source)
                || (keys.category.is_some() && keys.category == other.category)
        };
        let mut matched: Vec<usize> = groups
            .iter()
            .enumerate()
            .filter(|(_, group)| {
                group.members[0].topic == row.topic && group.keys.iter().any(&matches)
            })
            .map(|(index, _)| index)
            .collect();
        if matched.is_empty() {
            groups.push(Group {
                members: vec![row],
                keys: vec![keys],
            });
        } else {
            matched.sort_unstable();
            let first = matched.remove(0);
            // A row may match several groups (e.g. same body + same source):
            // merge them into the earliest group so grouping stays deterministic.
            for extra in matched.into_iter().rev() {
                let merged = groups.remove(extra);
                groups[first].members.extend(merged.members);
                groups[first].keys.extend(merged.keys);
            }
            groups[first].members.push(row);
            groups[first].keys.push(keys);
        }
    }

    let now = format_memory_now();
    let mut plan_groups = Vec::new();
    let mut kept_ids = Vec::new();
    let mut merged_ids = Vec::new();

    for group in groups.into_iter().filter(|group| group.members.len() >= 2) {
        let members = group.members;
        let kept = &members[0];
        let group_key = consolidation_group_key(&members);
        let source_ids = members
            .iter()
            .filter_map(|member| member.source_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let merged = members[1..]
            .iter()
            .map(|member| member.id.clone())
            .collect::<Vec<_>>();

        let consolidated_id = if dry_run {
            None
        } else {
            let importance = members
                .iter()
                .map(|member| importance_rank(member.importance))
                .min()
                .map(importance_from_rank)
                .unwrap_or(kept.importance);
            let mut metadata = kept.metadata.clone();
            metadata["consolidated"] = serde_json::json!(true);
            metadata["merged_ids"] = serde_json::to_value(&merged).unwrap_or_default();
            metadata["merged_source_ids"] = serde_json::to_value(&source_ids).unwrap_or_default();
            let input = NewMemory {
                repo_root: repo_root.to_owned(),
                session_id: kept.session_id.clone(),
                frontend: kept.frontend.clone(),
                scope: kept.scope,
                topic: kept.topic.clone(),
                title: kept.title.clone(),
                body: kept.body.clone(),
                importance,
                source_id: kept.source_id.clone(),
                metadata,
            };
            let record = store_memory(conn, &input)?;
            for member in &members[1..] {
                conn.execute(
                    "UPDATE memories SET superseded_by = ?1, updated_at = ?2
                     WHERE id = ?3 AND repo_root = ?4",
                    params![record.id, now, member.id, repo_root],
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?;
                conn.execute(
                    "INSERT INTO memory_supersessions
                        (old_memory_id, new_memory_id, reason, created_at)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![member.id, record.id, group_key, now],
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?;
            }
            Some(record.id)
        };

        kept_ids.push(kept.id.clone());
        merged_ids.extend(merged.iter().cloned());
        plan_groups.push(MemoryConsolidationGroup {
            topic: kept.topic.clone(),
            group_key,
            kept_memory_id: kept.id.clone(),
            merged_memory_ids: merged,
            source_ids,
            consolidated_id,
        });
    }

    Ok(MemoryConsolidationPlan {
        dry_run,
        groups: plan_groups,
        kept_ids,
        merged_ids,
    })
}

/// Deterministic grouping reason for a consolidated group, in fixed priority
/// order: title, body, source id, category.
fn consolidation_group_key(members: &[MemoryRecord]) -> String {
    for (a, b) in pairs(members) {
        if !a.title.trim().is_empty()
            && !b.title.trim().is_empty()
            && normalize_text(&a.title) == normalize_text(&b.title)
        {
            return "same_title".to_owned();
        }
    }
    for (a, b) in pairs(members) {
        if normalize_text(&a.body) == normalize_text(&b.body) {
            return "same_body".to_owned();
        }
    }
    for (a, b) in pairs(members) {
        if a.source_id.is_some() && a.source_id == b.source_id {
            return "same_source".to_owned();
        }
    }
    for (a, b) in pairs(members) {
        if memory_category(&a.metadata).is_some()
            && memory_category(&a.metadata) == memory_category(&b.metadata)
        {
            return "same_category".to_owned();
        }
    }
    "same_body".to_owned()
}

fn pairs<T>(items: &[T]) -> impl Iterator<Item = (&T, &T)> {
    (0..items.len()).flat_map(move |i| (i + 1..items.len()).map(move |j| (&items[i], &items[j])))
}

/// Collapse whitespace and lowercase for deterministic similarity comparison.
fn normalize_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Extract the optional `metadata.category` string (feedback/decision kind).
fn memory_category(metadata: &serde_json::Value) -> Option<String> {
    metadata
        .get("category")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn importance_rank(importance: MemoryImportance) -> u8 {
    match importance {
        MemoryImportance::Critical => 0,
        MemoryImportance::High => 1,
        MemoryImportance::Normal => 2,
        MemoryImportance::Low => 3,
    }
}

fn importance_from_rank(rank: u8) -> MemoryImportance {
    match rank {
        0 => MemoryImportance::Critical,
        1 => MemoryImportance::High,
        2 => MemoryImportance::Normal,
        _ => MemoryImportance::Low,
    }
}

fn prune_command(topic: &str) -> String {
    if topic.is_empty() {
        "atlas memory prune --dry-run".to_owned()
    } else {
        format!("atlas memory prune --topic {topic} --dry-run")
    }
}

fn consolidate_command(topic: &str) -> String {
    if topic.is_empty() {
        "atlas memory consolidate --dry-run".to_owned()
    } else {
        format!("atlas memory consolidate --topic {topic} --dry-run")
    }
}

// ── Row mapping ───────────────────────────────────────────────────────────────

/// Maps a `memories` row (column order from migrations 007 + 009) to
/// [`MemoryRecord`].
pub(super) fn row_to_memory(row: &rusqlite::Row<'_>) -> rusqlite::Result<MemoryRecord> {
    let scope: String = row.get(4)?;
    let importance: String = row.get(8)?;
    Ok(MemoryRecord {
        id: row.get(0)?,
        repo_root: row.get(1)?,
        session_id: row.get(2)?,
        frontend: row.get(3)?,
        scope: scope.parse().map_err(to_from_sql_error)?,
        topic: row.get(5)?,
        title: row.get(6)?,
        body: row.get(7)?,
        importance: importance.parse().map_err(to_from_sql_error)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        last_accessed_at: row.get(11)?,
        decay_score: row.get(12)?,
        source_id: row.get(13)?,
        metadata: serde_json::from_str(&row.get::<_, String>(14)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                14,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?,
        superseded_by: row.get(15)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_ids_are_deterministic_and_distinct() {
        let a = derive_memory_id("/repo", "same body", 1_700_000_000_000_000_000);
        let b = derive_memory_id("/repo", "same body", 1_700_000_000_000_000_000);
        let c = derive_memory_id("/repo", "same body", 1_700_000_000_000_000_001);
        let d = derive_memory_id("/repo", "other body", 1_700_000_000_000_000_000);
        assert_eq!(a, b);
        assert_ne!(a, c, "nonce must separate same-second writes");
        assert_ne!(a, d, "body must be part of the id");
        assert_eq!(a.len(), 64);
    }

    #[test]
    fn memory_timestamps_are_second_precision() {
        let now = OffsetDateTime::from_unix_timestamp_nanos(1_700_000_000_123_456_789)
            .expect("valid nanos");
        assert_eq!(
            format_memory_timestamp(now),
            "2023-11-14T22:13:20Z",
            "subseconds must be dropped so string order equals time order"
        );
    }
}
