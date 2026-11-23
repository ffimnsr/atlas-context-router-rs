use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;

use atlas_core::{AtlasError, Result};
use atlas_store_markdown::MarkdownStore;
use rusqlite::Connection;

use super::MEMORIES_DIR;
use super::MemoryRecord;
use super::RELATION_SUPERSEDES;
use super::encoding::*;
use super::index::apply_access_index;

// ── Reads ─────────────────────────────────────────────────────────────────────

/// All memory records for a repo, derived active set applied, sorted newest
/// first. A malformed file fails the scan with its path in the error.
#[cfg(test)]
pub(crate) fn scan(store: &MarkdownStore, repo_root: &str) -> Result<Vec<MemoryRecord>> {
    let mut records = Vec::new();
    for relative in store
        .list_markdown(Path::new(MEMORIES_DIR))
        .map_err(markdown_error)?
    {
        let record = store.read_record(&relative).map_err(markdown_error)?;
        records.push(decode_record(&relative, &record, repo_root)?);
    }
    sort_records(&mut records);
    apply_supersessions(&mut records);
    Ok(records)
}

/// Lenient scan used by list/health: malformed files become issues instead of
/// failing every other record in the directory.
pub(crate) fn scan_lenient(
    store: &MarkdownStore,
    repo_root: &str,
) -> (Vec<MemoryRecord>, Vec<String>) {
    let mut records = Vec::new();
    let mut issues = Vec::new();
    let listed = match store.list_markdown(Path::new(MEMORIES_DIR)) {
        Ok(listed) => listed,
        Err(error) => return (records, vec![error.to_string()]),
    };
    for relative in listed {
        let record = match store.read_record(&relative) {
            Ok(record) => record,
            Err(error) => {
                issues.push(error.to_string());
                continue;
            }
        };
        match decode_record(&relative, &record, repo_root) {
            Ok(record) => records.push(record),
            Err(error) => issues.push(error.to_string()),
        }
    }
    sort_records(&mut records);
    apply_supersessions(&mut records);
    (records, issues)
}

fn sort_records(records: &mut [MemoryRecord]) {
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.created_at.cmp(&left.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

/// Derive `superseded_by` from `supersedes` links. Deterministic when a target
/// is superseded more than once (lexicographically smallest new id wins);
/// cycles simply leave every member of the cycle superseded.
pub(crate) fn apply_supersessions(records: &mut [MemoryRecord]) {
    let mut by_target: BTreeMap<String, String> = BTreeMap::new();
    for record in records.iter() {
        for link in record
            .links
            .iter()
            .filter(|link| link.relation == RELATION_SUPERSEDES)
        {
            by_target
                .entry(link.id.clone())
                .and_modify(|existing| {
                    if record.id < *existing {
                        *existing = record.id.clone();
                    }
                })
                .or_insert_with(|| record.id.clone());
        }
    }
    for record in records.iter_mut() {
        record.superseded_by = by_target.get(&record.id).cloned();
    }
}

/// Load one record by exact id within a repo. Other malformed files are
/// reported by health but never block this lookup.
pub(crate) fn load_by_id(
    store: &MarkdownStore,
    repo_root: &str,
    memory_id: &str,
) -> Result<Option<MemoryRecord>> {
    let (records, _issues) = scan_lenient(store, repo_root);
    Ok(records.into_iter().find(|record| record.id == memory_id))
}

/// Load a bounded set of records in one directory scan, with access
/// bookkeeping merged from the derived index.
pub(crate) fn load_records(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    ids: &BTreeSet<String>,
) -> Result<BTreeMap<String, MemoryRecord>> {
    let (records, issues) = scan_lenient(store, repo_root);
    if records.is_empty() && !issues.is_empty() {
        return Err(AtlasError::Other(issues.join("; ")));
    }
    let mut selected = records
        .into_iter()
        .filter(|record| ids.contains(&record.id))
        .collect::<Vec<_>>();
    apply_access_index(conn, repo_root, &mut selected)?;
    for record in &mut selected {
        strip_internal_metadata(&mut record.metadata);
    }
    Ok(selected
        .into_iter()
        .map(|record| (record.id.clone(), record))
        .collect())
}
