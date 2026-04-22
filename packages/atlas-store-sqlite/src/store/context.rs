use atlas_core::{AtlasError, Node, Result};
use rusqlite::params;

use super::{Store, helpers::row_to_node};

fn row_to_node_and_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<(Node, atlas_core::Edge)> {
    // -- node --
    let node_kind_str: String = row.get(1)?;
    let node_kind = node_kind_str
        .parse::<atlas_core::NodeKind>()
        .unwrap_or(atlas_core::NodeKind::Function);
    let node_extra_str: Option<String> = row.get(14)?;
    let node_extra = node_extra_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);
    let node = Node {
        id: atlas_core::NodeId(row.get(0)?),
        kind: node_kind,
        name: row.get(2)?,
        qualified_name: row.get(3)?,
        file_path: row.get(4)?,
        line_start: row.get(5)?,
        line_end: row.get(6)?,
        language: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        parent_name: row.get(8)?,
        params: row.get(9)?,
        return_type: row.get(10)?,
        modifiers: row.get(11)?,
        is_test: row.get::<_, i32>(12)? != 0,
        file_hash: row.get::<_, Option<String>>(13)?.unwrap_or_default(),
        extra_json: node_extra,
    };

    // -- edge --
    let edge_kind_str: String = row.get(16)?;
    let edge_kind = edge_kind_str
        .parse::<atlas_core::EdgeKind>()
        .unwrap_or(atlas_core::EdgeKind::References);
    let edge_extra_str: Option<String> = row.get(23)?;
    let edge_extra = edge_extra_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);
    let edge = atlas_core::Edge {
        id: row.get(15)?,
        kind: edge_kind,
        source_qn: row.get(17)?,
        target_qn: row.get(18)?,
        file_path: row.get(19)?,
        line: row.get(20)?,
        confidence: row.get(21)?,
        confidence_tier: row.get(22)?,
        extra_json: edge_extra,
    };

    Ok((node, edge))
}

impl Store {
    pub fn node_by_qname(&self, qname: &str) -> Result<Option<Node>> {
        use rusqlite::OptionalExtension;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn
            .query_row(
                "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                        language, parent_name, params, return_type, modifiers,
                        is_test, file_hash, extra_json
                 FROM nodes WHERE qualified_name = ?1",
                [qname],
                row_to_node,
            )
            .optional()
            .map_err(db_err)
    }

    /// Return all nodes whose `name` column exactly matches `name`, bounded by
    /// `limit`.  Results are ordered by `file_path, line_start` for stability.
    pub fn nodes_by_name(&self, name: &str, limit: usize) -> Result<Vec<Node>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                        language, parent_name, params, return_type, modifiers,
                        is_test, file_hash, extra_json
                 FROM nodes
                 WHERE name = ?1
                 ORDER BY file_path, line_start
                 LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![name, limit as i64], row_to_node)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return nodes that call `qname` (i.e. edges of kind `calls` with
    /// `target_qualified = qname`), paired with their edges, bounded by
    /// `limit`.  Results ordered by edge confidence descending then
    /// `source_qualified` for stability.
    pub fn direct_callers(
        &self,
        qname: &str,
        limit: usize,
    ) -> Result<Vec<(Node, atlas_core::Edge)>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT n.id, n.kind, n.name, n.qualified_name, n.file_path,
                        n.line_start, n.line_end, n.language, n.parent_name,
                        n.params, n.return_type, n.modifiers, n.is_test,
                        n.file_hash, n.extra_json,
                        e.id, e.kind, e.source_qualified, e.target_qualified,
                        e.file_path, e.line, e.confidence, e.confidence_tier, e.extra_json
                 FROM edges e
                 JOIN nodes n ON n.qualified_name = e.source_qualified
                 WHERE e.target_qualified = ?1
                   AND e.kind = 'calls'
                 ORDER BY e.confidence DESC, e.source_qualified
                 LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![qname, limit as i64], row_to_node_and_edge)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return nodes called by `qname` (i.e. edges of kind `calls` with
    /// `source_qualified = qname`), paired with their edges, bounded by
    /// `limit`.  Results ordered by edge confidence descending then
    /// `target_qualified` for stability.
    pub fn direct_callees(
        &self,
        qname: &str,
        limit: usize,
    ) -> Result<Vec<(Node, atlas_core::Edge)>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT n.id, n.kind, n.name, n.qualified_name, n.file_path,
                        n.line_start, n.line_end, n.language, n.parent_name,
                        n.params, n.return_type, n.modifiers, n.is_test,
                        n.file_hash, n.extra_json,
                        e.id, e.kind, e.source_qualified, e.target_qualified,
                        e.file_path, e.line, e.confidence, e.confidence_tier, e.extra_json
                 FROM edges e
                 JOIN nodes n ON n.qualified_name = e.target_qualified
                 WHERE e.source_qualified = ?1
                   AND e.kind = 'calls'
                 ORDER BY e.confidence DESC, e.target_qualified
                 LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![qname, limit as i64], row_to_node_and_edge)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return nodes connected to `qname` via `imports` edges (either
    /// direction), paired with their edges, bounded by `limit`.
    ///
    /// Covers both "this node imports X" (source = qname) and "X is imported
    /// by this node" (target = qname).  Results are deduplicated by
    /// `qualified_name` and ordered by file_path for stability.
    pub fn import_neighbors(
        &self,
        qname: &str,
        limit: usize,
    ) -> Result<Vec<(Node, atlas_core::Edge)>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        // Forward: qname imports something → join on target_qualified.
        // Backward: something imports qname → join on source_qualified.
        let mut stmt = self
            .conn
            .prepare(
                "SELECT n.id, n.kind, n.name, n.qualified_name, n.file_path,
                        n.line_start, n.line_end, n.language, n.parent_name,
                        n.params, n.return_type, n.modifiers, n.is_test,
                        n.file_hash, n.extra_json,
                        e.id, e.kind, e.source_qualified, e.target_qualified,
                        e.file_path, e.line, e.confidence, e.confidence_tier, e.extra_json
                 FROM edges e
                 JOIN nodes n ON (
                     (e.source_qualified = ?1 AND n.qualified_name = e.target_qualified)
                     OR
                     (e.target_qualified = ?1 AND n.qualified_name = e.source_qualified)
                 )
                 WHERE e.kind = 'imports'
                   AND n.qualified_name != ?1
                 ORDER BY n.file_path, n.qualified_name
                 LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![qname, limit as i64], row_to_node_and_edge)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return nodes that share the same `parent_name` and `file_path` as the
    /// node identified by `qname`, excluding `qname` itself.  Bounded by
    /// `limit`.  Returns an empty vec when the node has no parent or does not
    /// exist.
    pub fn containment_siblings(&self, qname: &str, limit: usize) -> Result<Vec<Node>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT s.id, s.kind, s.name, s.qualified_name, s.file_path,
                        s.line_start, s.line_end, s.language, s.parent_name,
                        s.params, s.return_type, s.modifiers, s.is_test,
                        s.file_hash, s.extra_json
                 FROM nodes seed
                 JOIN nodes s ON s.file_path = seed.file_path
                              AND s.parent_name = seed.parent_name
                              AND s.qualified_name != seed.qualified_name
                 WHERE seed.qualified_name = ?1
                   AND seed.parent_name IS NOT NULL
                 ORDER BY s.line_start
                 LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![qname, limit as i64], row_to_node)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return nodes connected to `qname` via `tests` or `tested_by` edges
    /// (either direction), paired with their edges, bounded by `limit`.
    ///
    /// Covers both `qname` tests something and something tests `qname`.
    pub fn test_neighbors(
        &self,
        qname: &str,
        limit: usize,
    ) -> Result<Vec<(Node, atlas_core::Edge)>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT n.id, n.kind, n.name, n.qualified_name, n.file_path,
                        n.line_start, n.line_end, n.language, n.parent_name,
                        n.params, n.return_type, n.modifiers, n.is_test,
                        n.file_hash, n.extra_json,
                        e.id, e.kind, e.source_qualified, e.target_qualified,
                        e.file_path, e.line, e.confidence, e.confidence_tier, e.extra_json
                 FROM edges e
                 JOIN nodes n ON (
                     (e.source_qualified = ?1 AND n.qualified_name = e.target_qualified)
                     OR
                     (e.target_qualified = ?1 AND n.qualified_name = e.source_qualified)
                 )
                 WHERE e.kind IN ('tests', 'tested_by')
                   AND n.qualified_name != ?1
                 ORDER BY n.file_path, n.qualified_name
                 LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![qname, limit as i64], row_to_node_and_edge)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// All edges targeting `qname` (inbound), any kind, paired with the source
    /// node. Bounded by `limit`. Supports dead-code and fan-in analysis.
    pub fn inbound_edges(
        &self,
        qname: &str,
        limit: usize,
    ) -> Result<Vec<(Node, atlas_core::Edge)>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT n.id, n.kind, n.name, n.qualified_name, n.file_path,
                        n.line_start, n.line_end, n.language, n.parent_name,
                        n.params, n.return_type, n.modifiers, n.is_test,
                        n.file_hash, n.extra_json,
                        e.id, e.kind, e.source_qualified, e.target_qualified,
                        e.file_path, e.line, e.confidence, e.confidence_tier, e.extra_json
                 FROM edges e
                 JOIN nodes n ON n.qualified_name = e.source_qualified
                 WHERE e.target_qualified = ?1
                 ORDER BY e.confidence DESC, e.source_qualified
                 LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![qname, limit as i64], row_to_node_and_edge)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// All edges sourcing from `qname` (outbound), any kind, paired with the
    /// target node. Bounded by `limit`. Supports fan-out analysis.
    pub fn outbound_edges(
        &self,
        qname: &str,
        limit: usize,
    ) -> Result<Vec<(Node, atlas_core::Edge)>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT n.id, n.kind, n.name, n.qualified_name, n.file_path,
                        n.line_start, n.line_end, n.language, n.parent_name,
                        n.params, n.return_type, n.modifiers, n.is_test,
                        n.file_hash, n.extra_json,
                        e.id, e.kind, e.source_qualified, e.target_qualified,
                        e.file_path, e.line, e.confidence, e.confidence_tier, e.extra_json
                 FROM edges e
                 JOIN nodes n ON n.qualified_name = e.target_qualified
                 WHERE e.source_qualified = ?1
                 ORDER BY e.confidence DESC, e.target_qualified
                 LIMIT ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![qname, limit as i64], row_to_node_and_edge)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    // -------------------------------------------------------------------------
    // Semantic retrieval helpers (Phase CM9)
    // -------------------------------------------------------------------------

    /// Edges from other files pointing at symbols defined in `file_path`.
    ///
    /// Returns `(referencing_file_path, target_qualified_name)` pairs.
    /// Bounded by `max_edges` to avoid overwhelming the caller; group in Rust.
    pub fn files_referencing_symbols_in(
        &self,
        file_path: &str,
        max_edges: usize,
    ) -> Result<Vec<(String, String)>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ns.file_path, e.target_qualified
                 FROM   edges e
                 JOIN   nodes nt ON e.target_qualified = nt.qualified_name
                              AND  nt.file_path = ?1
                 JOIN   nodes ns ON e.source_qualified = ns.qualified_name
                 WHERE  ns.file_path != ?1
                 ORDER  BY ns.file_path, e.target_qualified
                 LIMIT  ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![file_path, max_edges as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Files that share imported/called/referenced targets with `file_path`.
    ///
    /// Returns `(co_file_path, shared_target_qualified_name)` pairs.
    /// Only edges of kind `imports`, `calls`, or `references` are considered.
    /// Bounded by `max_edges`; the caller groups by co-file in Rust.
    pub fn files_sharing_references_with(
        &self,
        file_path: &str,
        max_edges: usize,
    ) -> Result<Vec<(String, String)>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT ns2.file_path, e1.target_qualified
                 FROM   edges e1
                 JOIN   nodes ns1 ON e1.source_qualified = ns1.qualified_name
                               AND  ns1.file_path = ?1
                 JOIN   edges e2  ON e1.target_qualified = e2.target_qualified
                 JOIN   nodes ns2 ON e2.source_qualified = ns2.qualified_name
                 WHERE  ns2.file_path != ?1
                   AND  e1.kind IN ('imports', 'calls', 'references')
                   AND  e2.kind IN ('imports', 'calls', 'references')
                 ORDER  BY ns2.file_path, e1.target_qualified
                 LIMIT  ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![file_path, max_edges as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return nodes that are dead-code candidates: no inbound semantic edges
    /// (calls, references, imports, extends, implements), not a test, not
    /// public/exported, and of a semantic kind (function, method, class, etc.).
    ///
    /// The caller is responsible for allowlist suppression and framework checks.
    /// Bounded by `limit`.
    pub fn dead_code_candidates(&self, limit: usize) -> Result<Vec<Node>> {
        self.dead_code_candidates_filtered(None, limit)
    }

    /// Same as [`Store::dead_code_candidates`], optionally restricted to a
    /// repo-relative file-path prefix.
    pub fn dead_code_candidates_filtered(
        &self,
        subpath: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Node>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut sql = String::from(
            "SELECT n.id, n.kind, n.name, n.qualified_name, n.file_path,
                    n.line_start, n.line_end, n.language, n.parent_name,
                    n.params, n.return_type, n.modifiers, n.is_test,
                    n.file_hash, n.extra_json
             FROM nodes n
             WHERE n.is_test = 0
               AND n.kind IN ('function','method','class','struct','enum',
                              'trait','interface','constant','variable')
               AND NOT (
                   COALESCE(n.modifiers,'') LIKE '%pub%'
                   OR COALESCE(n.modifiers,'') LIKE '%export%'
                   OR COALESCE(n.modifiers,'') LIKE '%public%'
               )
               AND NOT EXISTS (
                   SELECT 1 FROM edges e
                   WHERE e.target_qualified = n.qualified_name
                     AND e.kind IN ('calls','references','imports','extends','implements')
               )",
        );
        let rows = if let Some(subpath) = subpath {
            sql.push_str("\n               AND n.file_path LIKE ?1 ESCAPE '\\'");
            sql.push_str(
                "\n             ORDER BY n.file_path, n.line_start\n             LIMIT ?2",
            );

            let like_pattern = {
                let escaped = subpath
                    .replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_");
                format!("{escaped}%")
            };
            let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
            stmt.query_map(params![like_pattern, limit as i64], row_to_node)
                .map_err(db_err)?
                .filter_map(|r| r.ok())
                .collect()
        } else {
            sql.push_str(
                "\n             ORDER BY n.file_path, n.line_start\n             LIMIT ?1",
            );
            let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
            stmt.query_map(params![limit as i64], row_to_node)
                .map_err(db_err)?
                .filter_map(|r| r.ok())
                .collect()
        };
        Ok(rows)
    }
}
