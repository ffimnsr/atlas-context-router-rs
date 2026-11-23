use atlas_core::AtlasError;
use atlas_core::Clock;
use atlas_core::Result;
use atlas_core::SystemClock;
use atlas_store_markdown::MarkdownStore;
use rusqlite::Connection;

use super::super::memory::derive_memory_id;
use super::super::memory::format_memory_now;
use super::MEMORY_LINK_RELATIONS;
use super::MemoryLink;
use super::MemoryRecord;
use super::NewMemory;
use super::encoding::dedupe_tags;
use super::encoding::encode_record;
use super::encoding::markdown_error;
use super::encoding::relative_path;
use super::encoding::strip_internal_metadata;
use super::index::index_upsert;
use super::index::refresh_supersession_index;
use super::scan::load_by_id;

/// Write an existing record's truth file and refresh its derived index row.
pub(crate) fn write_existing_record(
    store: &MarkdownStore,
    conn: &Connection,
    record: &MemoryRecord,
) -> Result<()> {
    store
        .with_writer_lock("memories", || write_record_with_index(store, conn, record))
        .map_err(markdown_error)
}

// ── CRUD ──────────────────────────────────────────────────────────────────────

/// Validate and persist a new memory, deriving its id and timestamps.
pub(crate) fn store_memory(
    store: &MarkdownStore,
    conn: &Connection,
    input: &NewMemory,
) -> Result<MemoryRecord> {
    input.validate()?;
    let id = derive_memory_id(
        &input.repo_root,
        &input.body,
        SystemClock.now_utc().unix_timestamp_nanos(),
    );
    store_memory_at(store, conn, input, &format_memory_now(), &id)
}

/// Raw write used by [`store_memory`], consolidation, and deterministic tests.
pub(crate) fn store_memory_at(
    store: &MarkdownStore,
    conn: &Connection,
    input: &NewMemory,
    now: &str,
    id: &str,
) -> Result<MemoryRecord> {
    input.validate()?;
    validate_links_for_write(store, &input.repo_root, id, &input.links)?;
    let mut metadata = input.metadata.clone();
    if let Some(object) = metadata.as_object_mut()
        && !object.contains_key("__op_history")
    {
        object.insert(
            "__op_history".to_owned(),
            serde_json::json!([{
                "op": "store",
                "at": now,
                "fields": ["body"],
            }]),
        );
    }
    let record = MemoryRecord {
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
        metadata,
        tags: dedupe_tags(input.tags.clone()),
        links: normalize_links(&input.links),
        superseded_by: None,
    };
    store
        .with_writer_lock("memories", || write_record_with_index(store, conn, &record))
        .map_err(markdown_error)?;
    let mut returned = record;
    strip_internal_metadata(&mut returned.metadata);
    Ok(returned)
}

fn write_record_with_index(
    store: &MarkdownStore,
    conn: &Connection,
    record: &MemoryRecord,
) -> std::result::Result<(), atlas_store_markdown::MarkdownError> {
    let relative = relative_path(&record.topic, &record.id);
    let encoded = encode_record(record).map_err(|error| {
        atlas_store_markdown::MarkdownError::InvalidValue {
            field: "record".to_owned(),
            message: error.to_string(),
        }
    })?;
    store.write_record(&relative, &encoded)?;
    index_upsert(store, conn, record, &relative).map_err(|error| {
        atlas_store_markdown::MarkdownError::InvalidValue {
            field: "index".to_owned(),
            message: error.to_string(),
        }
    })?;
    refresh_supersession_index(conn, &record.repo_root).map_err(|error| {
        atlas_store_markdown::MarkdownError::InvalidValue {
            field: "index".to_owned(),
            message: error.to_string(),
        }
    })?;
    Ok(())
}

fn normalize_links(links: &[MemoryLink]) -> Vec<MemoryLink> {
    let mut out = links.to_vec();
    out.sort_by(|left, right| {
        left.id
            .cmp(&right.id)
            .then_with(|| left.relation.cmp(&right.relation))
    });
    out.dedup();
    out
}

fn validate_links_for_write(
    store: &MarkdownStore,
    repo_root: &str,
    source_id: &str,
    links: &[MemoryLink],
) -> Result<()> {
    for link in links {
        if link.id == source_id {
            return Err(AtlasError::Other(
                "memory link must not target itself".to_owned(),
            ));
        }
        if !MEMORY_LINK_RELATIONS.contains(&link.relation.as_str()) {
            return Err(AtlasError::Other(format!(
                "invalid memory link relation '{}'; expected one of {}",
                link.relation,
                MEMORY_LINK_RELATIONS.join(", ")
            )));
        }
        if load_by_id(store, repo_root, &link.id)?.is_none() {
            return Err(AtlasError::Other(format!(
                "memory link target {} does not exist",
                link.id
            )));
        }
    }
    Ok(())
}

/// Create one directed link between two existing memories. Duplicate links
/// collapse, self-links are rejected, and the target must exist.
pub(crate) fn add_memory_link(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
    memory_id: &str,
    target_id: &str,
    relation: &str,
    now: &str,
) -> Result<MemoryRecord> {
    if memory_id == target_id {
        return Err(AtlasError::Other(
            "memory link must not target itself".to_owned(),
        ));
    }
    if !MEMORY_LINK_RELATIONS.contains(&relation) {
        return Err(AtlasError::Other(format!(
            "invalid memory link relation '{relation}'; expected one of {}",
            MEMORY_LINK_RELATIONS.join(", ")
        )));
    }
    let mut record = load_by_id(store, repo_root, memory_id)?.ok_or_else(|| {
        AtlasError::Other(format!("no memory with id {memory_id} in {repo_root}"))
    })?;
    if load_by_id(store, repo_root, target_id)?.is_none() {
        return Err(AtlasError::Other(format!(
            "memory link target {target_id} does not exist"
        )));
    }
    let link = MemoryLink {
        id: target_id.to_owned(),
        relation: relation.to_owned(),
    };
    if record.links.contains(&link) {
        return Ok(record);
    }
    record.links.push(link);
    record.links = normalize_links(&record.links);
    record.updated_at = now.to_owned();
    store
        .with_writer_lock("memories", || write_record_with_index(store, conn, &record))
        .map_err(markdown_error)?;
    load_by_id(store, repo_root, memory_id)?.ok_or_else(|| {
        AtlasError::Other(format!("memory {memory_id} disappeared after link write"))
    })
}
