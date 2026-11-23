use std::path::{Path, PathBuf};

use atlas_core::AtlasError;
use atlas_core::Result;
use atlas_store_markdown::MarkdownStore;
use rusqlite::Connection;

use super::MEMORIES_DIR;
use super::MemoryDeleteResult;
use super::MemoryListFilter;
use super::MemoryRecord;
use super::encoding::markdown_error;
use super::encoding::strip_internal_metadata;
use super::index::apply_access_index;
use super::index::ensure_index;
use super::index::index_delete;
use super::index::indexed_ids;
use super::index::record_belongs_to_repo;
use super::index::refresh_supersession_index;
use super::scan::load_by_id;
use super::scan::scan_lenient;

/// List memories for a repo, filtered and sorted by `updated_at DESC`.
pub(crate) fn list_memories(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    filter: &MemoryListFilter,
) -> Result<Vec<MemoryRecord>> {
    ensure_index(store, conn, repo_root)?;
    let allowed = indexed_ids(conn, repo_root)?;
    let (mut records, issues) = scan_lenient(store, repo_root);
    if records.is_empty() && !issues.is_empty() {
        return Err(AtlasError::Other(issues.join("; ")));
    }
    apply_access_index(conn, repo_root, &mut records)?;
    records.retain(|record| allowed.contains(&record.id));
    for record in &mut records {
        strip_internal_metadata(&mut record.metadata);
    }
    records.retain(|record| matches_filter(record, filter));
    Ok(records)
}

pub(crate) fn matches_filter(record: &MemoryRecord, filter: &MemoryListFilter) -> bool {
    if !filter.include_superseded && record.superseded_by.is_some() {
        return false;
    }
    if let Some(topic) = &filter.topic
        && !record.topic.eq_ignore_ascii_case(topic)
    {
        return false;
    }
    if let Some(importance) = filter.importance
        && record.importance != importance
    {
        return false;
    }
    if let Some(scope) = filter.scope
        && record.scope != scope
    {
        return false;
    }
    if let Some(older_than) = &filter.older_than
        && record.updated_at >= *older_than
    {
        return false;
    }
    if let Some(newer_than) = &filter.newer_than
        && record.updated_at <= *newer_than
    {
        return false;
    }
    for tag in &filter.tags {
        if !record
            .tags
            .iter()
            .any(|candidate| candidate.eq_ignore_ascii_case(tag))
        {
            return false;
        }
    }
    true
}

/// Fetch one memory by exact id within a repo, with access bookkeeping merged
/// from the derived index.
pub(crate) fn get_memory(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    memory_id: &str,
) -> Result<Option<MemoryRecord>> {
    ensure_index(store, conn, repo_root)?;
    let Some(mut record) = load_by_id(store, repo_root, memory_id)? else {
        return Ok(None);
    };
    if !record_belongs_to_repo(conn, memory_id, repo_root) {
        return Ok(None);
    }
    let mut records = vec![record.clone()];
    apply_access_index(conn, repo_root, &mut records)?;
    record = records.remove(0);
    strip_internal_metadata(&mut record.metadata);
    Ok(Some(record))
}

/// Delete a memory by exact id within a repo.
pub(crate) fn delete_memory(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    memory_id: &str,
    dry_run: bool,
) -> Result<MemoryDeleteResult> {
    let found = {
        ensure_index(store, conn, repo_root)?;
        load_by_id(store, repo_root, memory_id)?.is_some()
            && record_belongs_to_repo(conn, memory_id, repo_root)
    };
    let deleted = if found && !dry_run {
        store
            .with_writer_lock("memories", || {
                let relative = find_relative_path(store, memory_id)?;
                let removed = match relative {
                    Some(relative) => store.delete_record(&relative)?,
                    None => false,
                };
                index_delete(conn, memory_id).map_err(|error| {
                    atlas_store_markdown::MarkdownError::InvalidValue {
                        field: "index".to_owned(),
                        message: error.to_string(),
                    }
                })?;
                refresh_supersession_index(conn, repo_root).map_err(|error| {
                    atlas_store_markdown::MarkdownError::InvalidValue {
                        field: "index".to_owned(),
                        message: error.to_string(),
                    }
                })?;
                Ok::<bool, atlas_store_markdown::MarkdownError>(removed)
            })
            .map_err(markdown_error)?
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

fn find_relative_path(
    store: &MarkdownStore,
    memory_id: &str,
) -> std::result::Result<Option<PathBuf>, atlas_store_markdown::MarkdownError> {
    for relative in store.list_markdown(Path::new(MEMORIES_DIR))? {
        let frontmatter = store.read_frontmatter(&relative)?;
        if frontmatter.get_str("id") == Some(memory_id) {
            return Ok(Some(relative));
        }
    }
    Ok(None)
}
