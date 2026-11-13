//! Memoir graph deletion.
//!
//! Extracted from `memoir.rs` so the storage module stays below the 1,000-line
//! limit. Deletion is scoped to `(repo_root, scope, name)` and removes the
//! graph, its concepts, and its relations in one transaction.

use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};

use atlas_core::{AtlasError, Result};

use super::memoir::{memoir_graph_select, row_to_memoir_graph};
use super::types::MemoryScope;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoirDeleteResult {
    pub graph: String,
    pub scope: MemoryScope,
    /// Whether the graph exists in this repo and scope.
    pub found: bool,
    /// Whether rows were actually removed (false for dry-run).
    pub deleted: bool,
    pub dry_run: bool,
    pub concepts_deleted: usize,
    pub relations_deleted: usize,
}

/// Delete a memoir graph and all of its concepts and relations.
///
/// Deletion is scoped to `(repo_root, scope, name)` and is idempotent: a
/// missing graph reports `found: false` instead of an error. `dry_run` reports
/// what would be removed without writing.
pub(super) fn delete_memoir_graph(
    conn: &Connection,
    repo_root: &str,
    scope: MemoryScope,
    name: &str,
    dry_run: bool,
) -> Result<MemoirDeleteResult> {
    let graph = conn
        .query_row(
            &format!(
                "{} WHERE repo_root = ?1 AND scope = ?2 AND name = ?3",
                memoir_graph_select()
            ),
            params![repo_root, scope.as_str(), name],
            row_to_memoir_graph,
        )
        .optional()
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    let Some(graph) = graph else {
        return Ok(MemoirDeleteResult {
            graph: name.to_owned(),
            scope,
            found: false,
            deleted: false,
            dry_run,
            concepts_deleted: 0,
            relations_deleted: 0,
        });
    };

    let relations_deleted: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memoir_relations WHERE graph_id = ?1",
            params![graph.id],
            |row| row.get(0),
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;
    let concepts_deleted: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM memoir_concepts WHERE graph_id = ?1",
            params![graph.id],
            |row| row.get(0),
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

    if !dry_run {
        let transaction = conn
            .unchecked_transaction()
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        transaction
            .execute(
                "DELETE FROM memoir_relations WHERE graph_id = ?1",
                params![graph.id],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        transaction
            .execute(
                "DELETE FROM memoir_concepts WHERE graph_id = ?1",
                params![graph.id],
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        transaction
            .execute("DELETE FROM memoir_graphs WHERE id = ?1", params![graph.id])
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        transaction
            .commit()
            .map_err(|e| AtlasError::Db(e.to_string()))?;
    }

    Ok(MemoirDeleteResult {
        graph: graph.name,
        scope,
        found: true,
        deleted: !dry_run,
        dry_run,
        concepts_deleted: concepts_deleted.max(0) as usize,
        relations_deleted: relations_deleted.max(0) as usize,
    })
}
