//! Markdown-to-index reconciliation for memories.
//!
//! Recall ranks from the derived `memory_index`. This module keeps that index
//! aligned with `.atlas/memories/**` without reading bodies for unchanged files:
//! one stat per file plus the persisted `(mtime, size)` identity decides whether
//! a row needs decoding. New files are indexed, deleted files drop their derived
//! rows, and malformed files are skipped so they never block the rest.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use atlas_core::{AtlasError, Result};
use atlas_store_markdown::MarkdownStore;
use rusqlite::Connection;

use super::markdown_memory;

/// Reconcile one repo's memory index with the markdown tree.
///
/// Returns the number of upserted rows. A no-op when every file identity
/// matches the persisted row.
pub(super) fn sync_index(
    store: &MarkdownStore,
    conn: &Connection,
    repo_root: &str,
) -> Result<usize> {
    let files = store
        .list_markdown(Path::new(markdown_memory::MEMORIES_DIR))
        .map_err(|error| AtlasError::Other(format!("markdown store error: {error}")))?;

    // Workspace markdown files carry no repo path, so an id already owned by a
    // different repo must not be claimed by the repo currently asking.
    let mut ownership: BTreeMap<String, String> = BTreeMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT id, repo_root FROM memory_index")
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| AtlasError::Db(error.to_string()))?;
        for row in rows {
            let (id, owner) = row.map_err(|error| AtlasError::Db(error.to_string()))?;
            ownership.insert(id, owner);
        }
    }

    let mut on_disk: BTreeMap<String, FileIdentity> = BTreeMap::new();
    for relative in files {
        let Ok(frontmatter) = store.read_frontmatter(&relative) else {
            // Malformed file: leave any previous derived row untouched and let
            // health report the path.
            continue;
        };
        let Some(id) = frontmatter.get_str("id").map(str::to_owned) else {
            continue;
        };
        if ownership.get(&id).is_some_and(|owner| owner != repo_root) {
            continue;
        }
        let Ok(metadata) = std::fs::metadata(store.absolute(&relative)) else {
            continue;
        };
        let mtime_ns = metadata
            .modified()
            .ok()
            .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|value| i64::try_from(value.as_nanos()).ok());
        let size = i64::try_from(metadata.len()).ok();
        on_disk.insert(
            id,
            FileIdentity {
                relative,
                mtime_ns,
                size,
            },
        );
    }

    let mut stmt = conn
        .prepare(
            "SELECT id, path, file_mtime_ns, file_size
             FROM memory_index WHERE repo_root = ?1",
        )
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    let rows = stmt
        .query_map([repo_root], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<i64>>(2)?,
                row.get::<_, Option<i64>>(3)?,
            ))
        })
        .map_err(|error| AtlasError::Db(error.to_string()))?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| AtlasError::Db(error.to_string()))?;
    drop(stmt);

    let mut changed = 0usize;
    let mut removed = 0usize;
    let mut indexed_ids = std::collections::BTreeSet::new();
    for (id, path, mtime_ns, size) in rows {
        indexed_ids.insert(id.clone());
        // Rows without a markdown path are legacy imports; leave them for
        // `export_pending` instead of deleting data.
        let Some(path) = path else {
            continue;
        };
        let Some(identity) = on_disk.get(&id) else {
            markdown_memory::index_delete(conn, &id)?;
            indexed_ids.remove(&id);
            removed += 1;
            continue;
        };
        let relative = identity.relative.to_string_lossy();
        if relative == path && identity.mtime_ns == mtime_ns && identity.size == size {
            continue;
        }
        let record = store.read_record(&identity.relative).map_err(|error| {
            AtlasError::Other(format!("read {}: {error}", identity.relative.display()))
        })?;
        let Ok(decoded) = markdown_memory::decode_record(&identity.relative, &record, repo_root)
        else {
            // Malformed record: do not block the remaining files.
            continue;
        };
        markdown_memory::index_upsert(store, conn, &decoded, &identity.relative)?;
        changed += 1;
    }

    for (id, identity) in on_disk {
        if indexed_ids.contains(&id) {
            continue;
        }
        let record = store.read_record(&identity.relative).map_err(|error| {
            AtlasError::Other(format!("read {}: {error}", identity.relative.display()))
        })?;
        let Ok(decoded) = markdown_memory::decode_record(&identity.relative, &record, repo_root)
        else {
            continue;
        };
        markdown_memory::index_upsert(store, conn, &decoded, &identity.relative)?;
        changed += 1;
    }

    if changed > 0 || removed > 0 {
        markdown_memory::refresh_supersession_index(conn, repo_root)?;
    }
    Ok(changed)
}

struct FileIdentity {
    relative: PathBuf,
    mtime_ns: Option<i64>,
    size: Option<i64>,
}
