use atlas_core::AtlasError;
use atlas_core::Result;
use atlas_store_markdown::MarkdownStore;
use rusqlite::Connection;
use rusqlite::params;
use time::OffsetDateTime;

use super::super::memory::age_days;
use super::super::memory::format_memory_now;
use super::MemoryDecayPolicy;
use super::MemoryDecayReport;
use super::MemoryImportance;
use super::MemoryListFilter;
use super::MemoryPruneResult;
use super::read::delete_memory;
use super::read::list_memories;

// ── ICM-B — decay, stale, prune, health ───────────────────────────────────────

/// Compute (and, when not dry-running, persist) decay scores in the derived
/// access index. Truth files are never rewritten.
pub(crate) fn decay_reports(
    store: &MarkdownStore,
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
    let rows = list_memories(store, conn, repo_root, &full_filter)?;
    let mut reports = Vec::with_capacity(rows.len());
    let mut updates = Vec::new();
    for row in rows {
        let age = age_days(&row.updated_at, now);
        let score = policy.score(row.importance, age);
        let report_importance = row.importance;
        let protected = policy.is_protected(row.importance);
        let stale = !protected && score >= 1.0;
        if !dry_run {
            updates.push((row.id.clone(), score));
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
    if !dry_run && !updates.is_empty() {
        let tx = conn
            .unchecked_transaction()
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        for (id, score) in updates {
            tx.execute(
                "INSERT INTO memory_access (memory_id, repo_root, last_accessed_at, access_count, decay_score)
                 VALUES (?1, ?2, ?3, 0, ?4)
                 ON CONFLICT(memory_id) DO UPDATE SET decay_score = excluded.decay_score",
                params![id, repo_root, format_memory_now(), score],
            )
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        }
        tx.commit()
            .map_err(|error| AtlasError::Db(error.to_string()))?;
    }
    Ok(reports)
}

/// Rows past their retention window per policy.
pub(crate) fn stale_memories(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
    policy: &MemoryDecayPolicy,
    now: OffsetDateTime,
) -> Result<Vec<MemoryDecayReport>> {
    let reports = decay_reports(store, conn, repo_root, filter, policy, now, true)?;
    Ok(reports.into_iter().filter(|report| report.stale).collect())
}

/// Delete (or, in dry-run, only report) memories past retention.
#[allow(clippy::too_many_arguments)]
pub(crate) fn prune_memories(
    store: &MarkdownStore,
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
    let reports = decay_reports(store, conn, repo_root, filter, policy, now, true)?;
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
        let mut deleted = 0usize;
        for memory in &candidates {
            let result = delete_memory(store, conn, repo_root, &memory.id, false)?;
            deleted += usize::from(result.deleted);
        }
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
