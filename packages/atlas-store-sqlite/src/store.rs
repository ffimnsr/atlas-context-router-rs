use atlas_core::{
    AtlasError, EdgeKind, GraphStats, ImpactResult, Node, NodeId, NodeKind, ParsedFile, Result,
    ScoredNode, SearchQuery,
};
use rusqlite::{Connection, OpenFlags, Row, params};
use tracing::{debug, info};

use crate::migrations::MIGRATIONS;

// ---------------------------------------------------------------------------
// Row-mapping helpers
// ---------------------------------------------------------------------------

fn row_to_node(row: &Row<'_>) -> rusqlite::Result<Node> {
    let kind_str: String = row.get(1)?;
    let kind = kind_str.parse::<NodeKind>().unwrap_or(NodeKind::Function);

    let extra_str: Option<String> = row.get(14)?;
    let extra_json = extra_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);

    Ok(Node {
        id: NodeId(row.get(0)?),
        kind,
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
        extra_json,
    })
}

fn row_to_edge(row: &Row<'_>) -> rusqlite::Result<atlas_core::Edge> {
    let kind_str: String = row.get(1)?;
    let kind = kind_str.parse::<EdgeKind>().unwrap_or(EdgeKind::References);

    let extra_str: Option<String> = row.get(8)?;
    let extra_json = extra_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);

    Ok(atlas_core::Edge {
        id: row.get(0)?,
        kind,
        source_qn: row.get(2)?,
        target_qn: row.get(3)?,
        file_path: row.get(4)?,
        line: row.get(5)?,
        confidence: row.get(6)?,
        confidence_tier: row.get(7)?,
        extra_json,
    })
}

/// Build a comma-separated `?,?,?` placeholder string for `n` params.
fn repeat_placeholders(n: usize) -> String {
    (0..n).map(|_| "?").collect::<Vec<_>>().join(",")
}

/// Wrap a user-provided FTS5 query so special characters don't break syntax.
/// Simple approach: if the string has FTS5 operators, quote it as a phrase.
fn fts5_escape(input: &str) -> String {
    // If it looks like a plain word/words without FTS5 syntax, leave it as-is
    // so users can still use operators intentionally.  Otherwise wrap in "".
    let has_special = input
        .chars()
        .any(|c| matches!(c, '"' | '(' | ')' | '^' | '-' | '*'));
    if has_special {
        // Escape internal double-quotes and wrap as phrase.
        format!("\"{}\"", input.replace('"', "\"\""))
    } else {
        input.to_string()
    }
}

/// SQLite-backed graph store.
///
/// Holds a single write connection; all mutation goes through this struct.
/// Parallel read access is left for a future read-pool layer.
pub struct Store {
    conn: Connection,
}

impl Store {
    /// Open (or create) the atlas database at `path` and apply any pending
    /// migrations.  The directory containing `path` must already exist.
    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        Self::apply_pragmas(&conn)?;

        let mut store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn apply_pragmas(conn: &Connection) -> Result<()> {
        // Pragmas in SQLite may or may not return result rows depending on the
        // pragma and the SQLite version. Prepare + drain rows so we never hit
        // the "Execute returned results" error from rusqlite's execute_batch.
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        for sql in &[
            "PRAGMA journal_mode=WAL",
            "PRAGMA synchronous=NORMAL",
            "PRAGMA foreign_keys=ON",
            "PRAGMA busy_timeout=5000",
        ] {
            let mut stmt = conn.prepare(sql).map_err(db_err)?;
            let mut rows = stmt.query([]).map_err(db_err)?;
            while rows.next().map_err(db_err)?.is_some() {}
        }
        Ok(())
    }

    /// Apply any migrations that have not yet been applied to this database.
    pub fn migrate(&mut self) -> Result<()> {
        // Bootstrap the metadata table so we can store schema_version.
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS metadata (
                     key   TEXT PRIMARY KEY,
                     value TEXT NOT NULL
                 );",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let current_version: i32 = self
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);

        debug!(current_version, "checking migrations");

        for migration in MIGRATIONS {
            if migration.version <= current_version {
                continue;
            }
            info!(version = migration.version, "applying migration");
            self.conn
                .execute_batch(migration.sql)
                .map_err(|e| AtlasError::Db(format!("migration {}: {e}", migration.version)))?;
            self.conn
                .execute(
                    "INSERT OR REPLACE INTO metadata (key, value) VALUES ('schema_version', ?1)",
                    params![migration.version.to_string()],
                )
                .map_err(|e| AtlasError::Db(e.to_string()))?;
        }
        Ok(())
    }

    /// Return high-level statistics about the stored graph.
    pub fn stats(&self) -> Result<GraphStats> {
        let file_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let node_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM nodes", [], |r| r.get(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let edge_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM edges", [], |r| r.get(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let mut stmt = self
            .conn
            .prepare("SELECT kind, COUNT(*) FROM nodes GROUP BY kind ORDER BY COUNT(*) DESC")
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let nodes_by_kind: Vec<(String, i64)> = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| AtlasError::Db(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let mut stmt = self
            .conn
            .prepare(
                "SELECT DISTINCT language FROM nodes WHERE language IS NOT NULL ORDER BY language",
            )
            .map_err(|e| AtlasError::Db(e.to_string()))?;
        let languages: Vec<String> = stmt
            .query_map([], |r| r.get(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?
            .filter_map(|r| r.ok())
            .collect();

        let last_indexed_at: Option<String> = self
            .conn
            .query_row("SELECT MAX(indexed_at) FROM files", [], |r| r.get(0))
            .unwrap_or(None);

        Ok(GraphStats {
            file_count,
            node_count,
            edge_count,
            nodes_by_kind,
            languages,
            last_indexed_at,
        })
    }

    /// Run SQLite integrity checks and return any issues found.
    ///
    /// Runs both `PRAGMA integrity_check` and `PRAGMA foreign_key_check`.
    /// Returns `Ok(vec![])` when the database is healthy. Any returned strings
    /// describe individual integrity violations.
    pub fn integrity_check(&self) -> Result<Vec<String>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut issues = Vec::new();

        // PRAGMA integrity_check returns "ok" on a clean DB.
        {
            let mut stmt = self
                .conn
                .prepare("PRAGMA integrity_check")
                .map_err(db_err)?;
            let mut rows = stmt.query([]).map_err(db_err)?;
            while let Some(row) = rows.next().map_err(db_err)? {
                let msg: String = row.get(0).map_err(db_err)?;
                if msg != "ok" {
                    issues.push(format!("integrity_check: {msg}"));
                }
            }
        }

        // PRAGMA foreign_key_check returns rows for each violation.
        {
            let mut stmt = self
                .conn
                .prepare("PRAGMA foreign_key_check")
                .map_err(db_err)?;
            let mut rows = stmt.query([]).map_err(db_err)?;
            while let Some(row) = rows.next().map_err(db_err)? {
                let table: String = row.get(0).map_err(db_err)?;
                let rowid: Option<i64> = row.get(1).map_err(db_err)?;
                let parent: String = row.get(2).map_err(db_err)?;
                let fkid: i64 = row.get(3).map_err(db_err)?;
                issues.push(format!(
                    "foreign_key_check: table={table} rowid={rowid:?} parent={parent} fkid={fkid}"
                ));
            }
        }

        Ok(issues)
    }

    // -------------------------------------------------------------------------
    // File-graph mutation
    // -------------------------------------------------------------------------

    /// Atomically replace every node, edge and FTS entry belonging to `path`.
    ///
    /// Transaction semantics (per spec §3.11):
    /// 1. BEGIN IMMEDIATE
    /// 2. FTS-delete old nodes for file
    /// 3. DELETE edges for file
    /// 4. DELETE nodes for file
    /// 5. UPSERT file row
    /// 6. INSERT nodes → INSERT FTS row per node (using the new rowid)
    /// 7. INSERT edges
    /// 8. COMMIT
    pub fn replace_file_graph(
        &mut self,
        path: &str,
        hash: &str,
        language: Option<&str>,
        size: Option<i64>,
        nodes: &[Node],
        edges: &[atlas_core::Edge],
    ) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());

        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;

        // Step 2: FTS-unindex old nodes.
        let old_nodes = {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                            language, parent_name, params, return_type, modifiers,
                            is_test, file_hash, extra_json
                     FROM nodes WHERE file_path = ?1",
                )
                .map_err(db_err)?;
            let rows: Vec<Node> = stmt
                .query_map([path], row_to_node)
                .map_err(db_err)?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        for n in &old_nodes {
            self.conn
                .execute(
                    "INSERT INTO nodes_fts(nodes_fts, rowid,
                             qualified_name, name, kind, file_path, language,
                             params, return_type, modifiers)
                     VALUES('delete', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        n.id.0,
                        n.qualified_name,
                        n.name,
                        n.kind.as_str(),
                        n.file_path,
                        n.language,
                        n.params,
                        n.return_type,
                        n.modifiers,
                    ],
                )
                .map_err(db_err)?;
        }

        // Steps 3–4: clear edges and nodes for this file.
        self.conn
            .execute("DELETE FROM edges WHERE file_path = ?1", [path])
            .map_err(db_err)?;
        // Remove dangling cross-file edges referencing old nodes from this file.
        self.conn
            .execute(
                "DELETE FROM edges
                 WHERE source_qualified IN (SELECT qualified_name FROM nodes WHERE file_path = ?1)
                    OR target_qualified IN (SELECT qualified_name FROM nodes WHERE file_path = ?1)",
                [path],
            )
            .map_err(db_err)?;
        self.conn
            .execute("DELETE FROM nodes WHERE file_path = ?1", [path])
            .map_err(db_err)?;

        // Step 5: upsert the file row.
        self.conn
            .execute(
                "INSERT OR REPLACE INTO files (path, language, hash, size, indexed_at)
                 VALUES (?1, ?2, ?3, ?4, datetime('now'))",
                params![path, language, hash, size],
            )
            .map_err(db_err)?;

        // Steps 6a + 6b: insert each node then its FTS row.
        for n in nodes {
            let extra = serde_json::to_string(&n.extra_json).map_err(AtlasError::Serde)?;
            self.conn
                .execute(
                    "INSERT OR REPLACE INTO nodes
                         (kind, name, qualified_name, file_path, line_start, line_end,
                          language, parent_name, params, return_type, modifiers,
                          is_test, file_hash, extra_json)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                    params![
                        n.kind.as_str(),
                        n.name,
                        n.qualified_name,
                        n.file_path,
                        n.line_start,
                        n.line_end,
                        n.language,
                        n.parent_name,
                        n.params,
                        n.return_type,
                        n.modifiers,
                        n.is_test as i32,
                        n.file_hash,
                        extra,
                    ],
                )
                .map_err(db_err)?;

            let rowid = self.conn.last_insert_rowid();
            self.conn
                .execute(
                    "INSERT INTO nodes_fts (rowid,
                             qualified_name, name, kind, file_path, language,
                             params, return_type, modifiers)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        rowid,
                        n.qualified_name,
                        n.name,
                        n.kind.as_str(),
                        n.file_path,
                        n.language,
                        n.params,
                        n.return_type,
                        n.modifiers,
                    ],
                )
                .map_err(db_err)?;
        }

        // Step 7: insert edges.
        for e in edges {
            let extra = serde_json::to_string(&e.extra_json).map_err(AtlasError::Serde)?;
            self.conn
                .execute(
                    "INSERT INTO edges
                         (kind, source_qualified, target_qualified, file_path,
                          line, confidence, confidence_tier, extra_json)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![
                        e.kind.as_str(),
                        e.source_qn,
                        e.target_qn,
                        e.file_path,
                        e.line,
                        e.confidence,
                        e.confidence_tier,
                        extra,
                    ],
                )
                .map_err(db_err)?;
        }

        self.conn.execute_batch("COMMIT").map_err(db_err)?;

        info!(
            path,
            nodes = nodes.len(),
            edges = edges.len(),
            "replaced file graph"
        );
        Ok(())
    }

    /// Replace graph slices for a batch of parsed files (calls
    /// `replace_file_graph` for each entry).
    pub fn replace_batch(&mut self, files: &[ParsedFile]) -> Result<()> {
        for f in files {
            self.replace_file_graph(
                &f.path,
                &f.hash,
                f.language.as_deref(),
                f.size,
                &f.nodes,
                &f.edges,
            )?;
        }
        Ok(())
    }

    /// Atomically remove every node, edge and FTS row for `path`.
    pub fn delete_file_graph(&mut self, path: &str) -> Result<()> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());

        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;

        // FTS-unindex first.
        let old_nodes = {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                            language, parent_name, params, return_type, modifiers,
                            is_test, file_hash, extra_json
                     FROM nodes WHERE file_path = ?1",
                )
                .map_err(db_err)?;
            let rows: Vec<Node> = stmt
                .query_map([path], row_to_node)
                .map_err(db_err)?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        for n in &old_nodes {
            self.conn
                .execute(
                    "INSERT INTO nodes_fts(nodes_fts, rowid,
                             qualified_name, name, kind, file_path, language,
                             params, return_type, modifiers)
                     VALUES('delete', ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        n.id.0,
                        n.qualified_name,
                        n.name,
                        n.kind.as_str(),
                        n.file_path,
                        n.language,
                        n.params,
                        n.return_type,
                        n.modifiers,
                    ],
                )
                .map_err(db_err)?;
        }

        self.conn
            .execute("DELETE FROM edges WHERE file_path = ?1", [path])
            .map_err(db_err)?;
        // Also remove dangling cross-file edges whose source or target
        // qualified name belongs to a node in the deleted file.  These edges
        // originate from other files and would otherwise linger as stale
        // references after the target nodes are gone.
        self.conn
            .execute(
                "DELETE FROM edges
                 WHERE source_qualified IN (SELECT qualified_name FROM nodes WHERE file_path = ?1)
                    OR target_qualified IN (SELECT qualified_name FROM nodes WHERE file_path = ?1)",
                [path],
            )
            .map_err(db_err)?;
        self.conn
            .execute("DELETE FROM nodes WHERE file_path = ?1", [path])
            .map_err(db_err)?;
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", [path])
            .map_err(db_err)?;

        self.conn.execute_batch("COMMIT").map_err(db_err)?;

        info!(path, "deleted file graph");
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Query helpers
    // -------------------------------------------------------------------------

    /// All nodes belonging to a file.
    pub fn nodes_by_file(&self, path: &str) -> Result<Vec<Node>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                        language, parent_name, params, return_type, modifiers,
                        is_test, file_hash, extra_json
                 FROM nodes WHERE file_path = ?1
                 ORDER BY line_start",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([path], row_to_node)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// All edges whose `file_path` column matches `path`.
    pub fn edges_by_file(&self, path: &str) -> Result<Vec<atlas_core::Edge>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, source_qualified, target_qualified, file_path,
                        line, confidence, confidence_tier, extra_json
                 FROM edges WHERE file_path = ?1",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([path], row_to_edge)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Returns a map of `file_path → stored_hash` for all indexed files.
    ///
    /// Used by the build command to skip re-parsing files whose content has not
    /// changed since the last indexed pass.
    pub fn file_hashes(&self) -> Result<std::collections::HashMap<String, String>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare("SELECT path, hash FROM files")
            .map_err(db_err)?;
        let map = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(map)
    }

    /// Files that have at least one edge pointing **into** a node defined in
    /// any of `changed_paths` (i.e. direct importers / callers).
    pub fn find_dependents(&self, changed_paths: &[&str]) -> Result<Vec<String>> {
        if changed_paths.is_empty() {
            return Ok(vec![]);
        }
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());

        let placeholders = repeat_placeholders(changed_paths.len());
        let sql = format!(
            "SELECT DISTINCT ns.file_path
             FROM edges  e
             JOIN nodes  nt ON e.target_qualified = nt.qualified_name
             JOIN nodes  ns ON e.source_qualified = ns.qualified_name
             WHERE nt.file_path IN ({placeholders})
               AND ns.file_path NOT IN ({placeholders})
             ORDER BY ns.file_path"
        );

        // bind the same list twice (target IN, source NOT IN).
        let params: Vec<&dyn rusqlite::types::ToSql> = changed_paths
            .iter()
            .chain(changed_paths.iter())
            .map(|p| p as &dyn rusqlite::types::ToSql)
            .collect();

        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(params.as_slice(), |r| r.get(0))
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Bi-directional impact radius via a recursive SQLite CTE seeded from
    /// nodes in `changed_paths`.
    ///
    /// Traverses both forward edges (source→target) and backward edges
    /// (target→source) up to `max_depth` hops, capped at `max_nodes` total.
    pub fn impact_radius(
        &self,
        changed_paths: &[&str],
        max_depth: u32,
        max_nodes: usize,
    ) -> Result<ImpactResult> {
        if changed_paths.is_empty() {
            return Ok(ImpactResult {
                changed_nodes: vec![],
                impacted_nodes: vec![],
                impacted_files: vec![],
                relevant_edges: vec![],
            });
        }
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let placeholders = repeat_placeholders(changed_paths.len());

        // Collect seed (changed) nodes.
        let seed_sql = format!(
            "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                    language, parent_name, params, return_type, modifiers,
                    is_test, file_hash, extra_json
             FROM nodes WHERE file_path IN ({placeholders})"
        );
        let mut stmt = self.conn.prepare(&seed_sql).map_err(db_err)?;
        let params_seed: Vec<&dyn rusqlite::types::ToSql> = changed_paths
            .iter()
            .map(|p| p as &dyn rusqlite::types::ToSql)
            .collect();
        let changed_nodes: Vec<Node> = stmt
            .query_map(params_seed.as_slice(), row_to_node)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();

        // Recursive CTE: bidirectional traversal, UNION deduplicates.
        let cte_sql = format!(
            "WITH RECURSIVE impact(qn, depth) AS (
               SELECT qualified_name, 0 FROM nodes WHERE file_path IN ({placeholders})
               UNION
               SELECT e.source_qualified, i.depth + 1
               FROM   impact i
               JOIN   edges  e ON e.target_qualified = i.qn
               WHERE  i.depth < ?
               UNION
               SELECT e.target_qualified, i.depth + 1
               FROM   impact i
               JOIN   edges  e ON e.source_qualified = i.qn
               WHERE  i.depth < ?
             )
             SELECT DISTINCT qn FROM impact LIMIT ?"
        );

        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = changed_paths
            .iter()
            .map(|p| Box::new(p.to_string()) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        all_params.push(Box::new(max_depth as i64));
        all_params.push(Box::new(max_depth as i64));
        all_params.push(Box::new(max_nodes as i64));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            all_params.iter().map(|b| b.as_ref()).collect();

        let mut stmt = self.conn.prepare(&cte_sql).map_err(db_err)?;
        let all_qns: Vec<String> = stmt
            .query_map(params_ref.as_slice(), |r| r.get(0))
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();

        // Separate impacted (non-seed) nodes.
        let seed_qns: std::collections::HashSet<&str> = changed_nodes
            .iter()
            .map(|n| n.qualified_name.as_str())
            .collect();

        let impacted_qns: Vec<&str> = all_qns
            .iter()
            .filter(|qn| !seed_qns.contains(qn.as_str()))
            .map(|s| s.as_str())
            .collect();

        let impacted_nodes = if impacted_qns.is_empty() {
            vec![]
        } else {
            let ph = repeat_placeholders(impacted_qns.len());
            let sql = format!(
                "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                        language, parent_name, params, return_type, modifiers,
                        is_test, file_hash, extra_json
                 FROM nodes WHERE qualified_name IN ({ph})"
            );
            let p: Vec<&dyn rusqlite::types::ToSql> = impacted_qns
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
            stmt.query_map(p.as_slice(), row_to_node)
                .map_err(db_err)?
                .filter_map(|r| r.ok())
                .collect()
        };

        let impacted_files: Vec<String> = {
            let mut files: Vec<String> = impacted_nodes
                .iter()
                .map(|n: &Node| n.file_path.clone())
                .collect();
            files.sort();
            files.dedup();
            files
        };

        // Edges within the full impacted set.
        let relevant_edges = if all_qns.is_empty() {
            vec![]
        } else {
            let ph = repeat_placeholders(all_qns.len());
            let sql = format!(
                "SELECT id, kind, source_qualified, target_qualified, file_path,
                        line, confidence, confidence_tier, extra_json
                 FROM edges
                 WHERE source_qualified IN ({ph}) AND target_qualified IN ({ph})"
            );
            let p: Vec<&dyn rusqlite::types::ToSql> = all_qns
                .iter()
                .chain(all_qns.iter())
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
            stmt.query_map(p.as_slice(), row_to_edge)
                .map_err(db_err)?
                .filter_map(|r| r.ok())
                .collect()
        };

        Ok(ImpactResult {
            changed_nodes,
            impacted_nodes,
            impacted_files,
            relevant_edges,
        })
    }

    // -------------------------------------------------------------------------
    // FTS search
    // -------------------------------------------------------------------------

    /// Full-text search over `nodes_fts` with optional field filters.
    ///
    /// Returns nodes ordered by BM25 relevance (best first), capped at
    /// `query.limit`.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<ScoredNode>> {
        if query.text.trim().is_empty() {
            return Ok(vec![]);
        }
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());

        // Build dynamic WHERE clause for the optional filters.
        let mut filters = vec!["nodes_fts MATCH ?1".to_string()];
        if query.kind.is_some() {
            filters.push("n.kind = ?2".to_string());
        }
        if query.language.is_some() {
            filters.push("n.language = ?3".to_string());
        }
        if query.file_path.is_some() {
            filters.push("n.file_path = ?4".to_string());
        }
        if let Some(is_test) = query.is_test {
            filters.push(format!("n.is_test = {}", is_test as i32));
        }

        let where_clause = filters.join(" AND ");
        let sql = format!(
            "SELECT n.id, n.kind, n.name, n.qualified_name, n.file_path,
                    n.line_start, n.line_end, n.language, n.parent_name,
                    n.params, n.return_type, n.modifiers, n.is_test,
                    n.file_hash, n.extra_json,
                    bm25(nodes_fts) AS score
             FROM   nodes_fts
             JOIN   nodes n ON n.id = nodes_fts.rowid
             WHERE  {where_clause}
             ORDER  BY score
             LIMIT  ?5"
        );

        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;

        // FTS5 expects the MATCH operand to be an unquoted query string.
        // Escape any special chars the user may have typed so we don't break
        // FTS5 query syntax.
        let fts_query = fts5_escape(&query.text);

        let results = stmt
            .query_map(
                rusqlite::params![
                    fts_query,
                    query.kind.as_deref().unwrap_or(""),
                    query.language.as_deref().unwrap_or(""),
                    query.file_path.as_deref().unwrap_or(""),
                    query.limit as i64,
                ],
                |row| {
                    let node = row_to_node(row)?;
                    let score: f64 = row.get(15)?;
                    Ok(ScoredNode {
                        node,
                        // BM25 returns negative values; negate for ascending score.
                        score: -score,
                    })
                },
            )
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile, SearchQuery};

    use super::*;

    fn open_in_memory() -> Store {
        let conn = Connection::open_in_memory().unwrap();
        Store::apply_pragmas(&conn).unwrap();
        let mut store = Store { conn };
        store.migrate().unwrap();
        store
    }

    fn make_node(kind: NodeKind, name: &str, qn: &str, file_path: &str, language: &str) -> Node {
        Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_string(),
            qualified_name: qn.to_string(),
            file_path: file_path.to_string(),
            line_start: 1,
            line_end: 10,
            language: language.to_string(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "abc123".to_string(),
            extra_json: serde_json::Value::Null,
        }
    }

    fn make_edge(kind: EdgeKind, src: &str, tgt: &str, file_path: &str) -> Edge {
        Edge {
            id: 0,
            kind,
            source_qn: src.to_string(),
            target_qn: tgt.to_string(),
            file_path: file_path.to_string(),
            line: None,
            confidence: 1.0,
            confidence_tier: None,
            extra_json: serde_json::Value::Null,
        }
    }

    // --- existing foundation tests -------------------------------------------

    #[test]
    fn migration_creates_schema() {
        let store = open_in_memory();
        let stats = store.stats().unwrap();
        assert_eq!(stats.file_count, 0);
        assert_eq!(stats.node_count, 0);
        assert_eq!(stats.edge_count, 0);
    }

    #[test]
    fn schema_version_stored() {
        let store = open_in_memory();
        let version: i32 = store
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn wal_mode_enabled() {
        let store = open_in_memory();
        let mode: String = store
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        // In-memory databases report "memory" regardless; file DBs report "wal".
        assert!(!mode.is_empty());
    }

    // --- replace_file_graph --------------------------------------------------

    #[test]
    fn replace_file_graph_inserts_nodes_and_edges() {
        let mut store = open_in_memory();
        let nodes = vec![
            make_node(
                NodeKind::Function,
                "foo",
                "src/a.rs::fn::foo",
                "src/a.rs",
                "rust",
            ),
            make_node(
                NodeKind::Function,
                "bar",
                "src/a.rs::fn::bar",
                "src/a.rs",
                "rust",
            ),
        ];
        let edges = vec![make_edge(
            EdgeKind::Calls,
            "src/a.rs::fn::foo",
            "src/a.rs::fn::bar",
            "src/a.rs",
        )];

        store
            .replace_file_graph("src/a.rs", "hash1", Some("rust"), Some(200), &nodes, &edges)
            .unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.file_count, 1);
        assert_eq!(stats.node_count, 2);
        assert_eq!(stats.edge_count, 1);
    }

    #[test]
    fn replace_file_graph_is_idempotent() {
        let mut store = open_in_memory();
        let nodes = vec![make_node(
            NodeKind::Function,
            "foo",
            "a.rs::fn::foo",
            "a.rs",
            "rust",
        )];
        let edges: Vec<Edge> = vec![];

        store
            .replace_file_graph("a.rs", "h1", Some("rust"), None, &nodes, &edges)
            .unwrap();
        store
            .replace_file_graph("a.rs", "h2", Some("rust"), None, &nodes, &edges)
            .unwrap();

        // Second replace must not double the counts.
        let stats = store.stats().unwrap();
        assert_eq!(stats.node_count, 1);
        assert_eq!(stats.file_count, 1);
    }

    #[test]
    fn replace_file_graph_updates_nodes() {
        let mut store = open_in_memory();
        let first = vec![make_node(
            NodeKind::Function,
            "old",
            "a.rs::fn::old",
            "a.rs",
            "rust",
        )];
        store
            .replace_file_graph("a.rs", "h1", None, None, &first, &[])
            .unwrap();

        let second = vec![make_node(
            NodeKind::Function,
            "new_fn",
            "a.rs::fn::new_fn",
            "a.rs",
            "rust",
        )];
        store
            .replace_file_graph("a.rs", "h2", None, None, &second, &[])
            .unwrap();

        let got = store.nodes_by_file("a.rs").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "new_fn");
    }

    // --- delete_file_graph ---------------------------------------------------

    #[test]
    fn delete_file_graph_removes_all_rows() {
        let mut store = open_in_memory();
        let nodes = vec![make_node(
            NodeKind::Function,
            "f",
            "a.rs::fn::f",
            "a.rs",
            "rust",
        )];
        let edges = vec![make_edge(
            EdgeKind::Calls,
            "a.rs::fn::f",
            "b.rs::fn::g",
            "a.rs",
        )];
        store
            .replace_file_graph("a.rs", "h", None, None, &nodes, &edges)
            .unwrap();

        store.delete_file_graph("a.rs").unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.file_count, 0);
        assert_eq!(stats.node_count, 0);
        assert_eq!(stats.edge_count, 0);
    }

    #[test]
    fn delete_file_graph_removes_dangling_cross_file_edges() {
        let mut store = open_in_memory();
        // b.rs has an edge pointing INTO a node from a.rs.
        let na = make_node(NodeKind::Function, "fa", "a.rs::fn::fa", "a.rs", "rust");
        let nb = make_node(NodeKind::Function, "fb", "b.rs::fn::fb", "b.rs", "rust");
        // Edge lives in b.rs but targets a.rs::fn::fa.
        let cross_edge = make_edge(EdgeKind::Calls, "b.rs::fn::fb", "a.rs::fn::fa", "b.rs");
        store
            .replace_file_graph("a.rs", "h", None, None, &[na], &[])
            .unwrap();
        store
            .replace_file_graph("b.rs", "h", None, None, &[nb], &[cross_edge])
            .unwrap();

        // Verify the cross-file edge is present before deletion.
        assert_eq!(store.stats().unwrap().edge_count, 1);

        // Deleting a.rs must also remove the dangling edge from b.rs.
        store.delete_file_graph("a.rs").unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.file_count, 1, "b.rs file should remain");
        assert_eq!(
            stats.edge_count, 0,
            "dangling cross-file edge must be removed"
        );
    }

    #[test]
    fn replace_file_graph_removes_stale_cross_file_edges_on_update() {
        let mut store = open_in_memory();
        // b.rs references old_fn from a.rs; after a.rs is re-indexed with only
        // new_fn the edge must be cleaned up.
        let na = make_node(
            NodeKind::Function,
            "old_fn",
            "a.rs::fn::old_fn",
            "a.rs",
            "rust",
        );
        let nb = make_node(
            NodeKind::Function,
            "caller",
            "b.rs::fn::caller",
            "b.rs",
            "rust",
        );
        let stale = make_edge(
            EdgeKind::Calls,
            "b.rs::fn::caller",
            "a.rs::fn::old_fn",
            "b.rs",
        );
        store
            .replace_file_graph("a.rs", "h1", None, None, &[na], &[])
            .unwrap();
        store
            .replace_file_graph("b.rs", "h1", None, None, &[nb], &[stale])
            .unwrap();
        assert_eq!(store.stats().unwrap().edge_count, 1);

        // Re-index a.rs with a *different* function; old_fn is now gone.
        let new_na = make_node(
            NodeKind::Function,
            "new_fn",
            "a.rs::fn::new_fn",
            "a.rs",
            "rust",
        );
        store
            .replace_file_graph("a.rs", "h2", None, None, &[new_na], &[])
            .unwrap();

        // The stale edge from b.rs towards the now-gone old_fn must be removed.
        assert_eq!(
            store.stats().unwrap().edge_count,
            0,
            "stale cross-file edge must be cleaned up"
        );
    }

    // --- replace_batch -------------------------------------------------------

    #[test]
    fn replace_batch_processes_multiple_files() {
        let mut store = open_in_memory();
        let batch = vec![
            ParsedFile {
                path: "a.rs".to_string(),
                language: Some("rust".to_string()),
                hash: "ha".to_string(),
                size: None,
                nodes: vec![make_node(
                    NodeKind::Function,
                    "a",
                    "a.rs::fn::a",
                    "a.rs",
                    "rust",
                )],
                edges: vec![],
            },
            ParsedFile {
                path: "b.rs".to_string(),
                language: Some("rust".to_string()),
                hash: "hb".to_string(),
                size: None,
                nodes: vec![make_node(
                    NodeKind::Function,
                    "b",
                    "b.rs::fn::b",
                    "b.rs",
                    "rust",
                )],
                edges: vec![make_edge(
                    EdgeKind::Calls,
                    "b.rs::fn::b",
                    "a.rs::fn::a",
                    "b.rs",
                )],
            },
        ];
        store.replace_batch(&batch).unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.file_count, 2);
        assert_eq!(stats.node_count, 2);
        assert_eq!(stats.edge_count, 1);
    }

    // --- nodes_by_file / edges_by_file ---------------------------------------

    #[test]
    fn nodes_by_file_returns_only_that_file() {
        let mut store = open_in_memory();
        let na = make_node(NodeKind::Function, "fa", "a.rs::fn::fa", "a.rs", "rust");
        let nb = make_node(NodeKind::Function, "fb", "b.rs::fn::fb", "b.rs", "rust");
        store
            .replace_file_graph("a.rs", "h", None, None, &[na], &[])
            .unwrap();
        store
            .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
            .unwrap();

        let got = store.nodes_by_file("a.rs").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].name, "fa");
    }

    #[test]
    fn edges_by_file_returns_only_that_file() {
        let mut store = open_in_memory();
        let nodes_a = vec![make_node(
            NodeKind::Function,
            "fa",
            "a.rs::fn::fa",
            "a.rs",
            "rust",
        )];
        let nodes_b = vec![make_node(
            NodeKind::Function,
            "fb",
            "b.rs::fn::fb",
            "b.rs",
            "rust",
        )];
        let edges_a = vec![make_edge(
            EdgeKind::Calls,
            "a.rs::fn::fa",
            "b.rs::fn::fb",
            "a.rs",
        )];
        let edges_b = vec![make_edge(
            EdgeKind::Calls,
            "b.rs::fn::fb",
            "a.rs::fn::fa",
            "b.rs",
        )];
        store
            .replace_file_graph("a.rs", "h", None, None, &nodes_a, &edges_a)
            .unwrap();
        store
            .replace_file_graph("b.rs", "h", None, None, &nodes_b, &edges_b)
            .unwrap();

        let got = store.edges_by_file("a.rs").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].source_qn, "a.rs::fn::fa");
    }

    // --- find_dependents -----------------------------------------------------

    #[test]
    fn find_dependents_returns_importing_files() {
        let mut store = open_in_memory();
        // a.rs defines "Foo"; b.rs calls Foo.
        let na = make_node(NodeKind::Struct, "Foo", "a.rs::struct::Foo", "a.rs", "rust");
        let nb = make_node(
            NodeKind::Function,
            "use_foo",
            "b.rs::fn::use_foo",
            "b.rs",
            "rust",
        );
        let edge = make_edge(
            EdgeKind::References,
            "b.rs::fn::use_foo",
            "a.rs::struct::Foo",
            "b.rs",
        );
        store
            .replace_file_graph("a.rs", "h", None, None, &[na], &[])
            .unwrap();
        store
            .replace_file_graph("b.rs", "h", None, None, &[nb], &[edge])
            .unwrap();

        let deps = store.find_dependents(&["a.rs"]).unwrap();
        assert!(deps.contains(&"b.rs".to_string()));
        assert!(!deps.contains(&"a.rs".to_string()));
    }

    #[test]
    fn find_dependents_empty_input_returns_empty() {
        let store = open_in_memory();
        let deps = store.find_dependents(&[]).unwrap();
        assert!(deps.is_empty());
    }

    // --- impact_radius -------------------------------------------------------

    #[test]
    fn impact_radius_one_hop() {
        let mut store = open_in_memory();
        // a.rs::fn::a  →Calls→  b.rs::fn::b
        let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
        let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
        let edge = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
        store
            .replace_file_graph("a.rs", "h", None, None, &[na], &[edge])
            .unwrap();
        store
            .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
            .unwrap();

        let result = store.impact_radius(&["a.rs"], 3, 200).unwrap();
        assert_eq!(result.changed_nodes.len(), 1);
        assert!(
            result
                .impacted_nodes
                .iter()
                .any(|n| n.qualified_name == "b.rs::fn::b")
        );
        assert!(result.impacted_files.contains(&"b.rs".to_string()));
    }

    #[test]
    fn impact_radius_cyclic_graph_terminates() {
        let mut store = open_in_memory();
        // a → b → a (cycle)
        let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
        let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
        let e1 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
        let e2 = make_edge(EdgeKind::Calls, "b.rs::fn::b", "a.rs::fn::a", "b.rs");
        store
            .replace_file_graph("a.rs", "h", None, None, &[na], &[e1])
            .unwrap();
        store
            .replace_file_graph("b.rs", "h", None, None, &[nb], &[e2])
            .unwrap();

        // Must not loop forever and must return both nodes.
        let result = store.impact_radius(&["a.rs"], 5, 200).unwrap();
        let all_qns: Vec<&str> = result
            .changed_nodes
            .iter()
            .chain(result.impacted_nodes.iter())
            .map(|n| n.qualified_name.as_str())
            .collect();
        assert!(all_qns.contains(&"a.rs::fn::a"));
        assert!(all_qns.contains(&"b.rs::fn::b"));
    }

    #[test]
    fn impact_radius_empty_input_returns_empty() {
        let store = open_in_memory();
        let result = store.impact_radius(&[], 3, 200).unwrap();
        assert!(result.changed_nodes.is_empty());
        assert!(result.impacted_nodes.is_empty());
    }

    // --- FTS search ----------------------------------------------------------

    #[test]
    fn fts_search_finds_indexed_node() {
        let mut store = open_in_memory();
        let node = make_node(
            NodeKind::Function,
            "replace_file_graph",
            "store.rs::fn::replace_file_graph",
            "store.rs",
            "rust",
        );
        store
            .replace_file_graph("store.rs", "h", Some("rust"), None, &[node], &[])
            .unwrap();

        let q = SearchQuery {
            text: "replace_file_graph".to_string(),
            limit: 5,
            ..Default::default()
        };
        let results = store.search(&q).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].node.name, "replace_file_graph");
    }

    #[test]
    fn fts_search_empty_query_returns_empty() {
        let store = open_in_memory();
        let q = SearchQuery {
            text: "".to_string(),
            ..Default::default()
        };
        let results = store.search(&q).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn fts_search_respects_kind_filter() {
        let mut store = open_in_memory();
        let func = make_node(
            NodeKind::Function,
            "process",
            "a.rs::fn::process",
            "a.rs",
            "rust",
        );
        let strct = make_node(
            NodeKind::Struct,
            "ProcessConfig",
            "a.rs::struct::ProcessConfig",
            "a.rs",
            "rust",
        );
        store
            .replace_file_graph("a.rs", "h", None, None, &[func, strct], &[])
            .unwrap();

        let q = SearchQuery {
            text: "process".to_string(),
            kind: Some("struct".to_string()),
            limit: 10,
            ..Default::default()
        };
        let results = store.search(&q).unwrap();
        assert!(
            results
                .iter()
                .all(|r| matches!(r.node.kind, NodeKind::Struct))
        );
    }

    #[test]
    fn fts_search_not_found_after_delete() {
        let mut store = open_in_memory();
        let node = make_node(
            NodeKind::Function,
            "vanishing_fn",
            "a.rs::fn::vanishing_fn",
            "a.rs",
            "rust",
        );
        store
            .replace_file_graph("a.rs", "h", None, None, &[node], &[])
            .unwrap();
        store.delete_file_graph("a.rs").unwrap();

        let q = SearchQuery {
            text: "vanishing_fn".to_string(),
            limit: 5,
            ..Default::default()
        };
        let results = store.search(&q).unwrap();
        assert!(results.is_empty());
    }

    // --- stats correctness ---------------------------------------------------

    #[test]
    fn stats_returns_nodes_by_kind() {
        let mut store = open_in_memory();
        let func = make_node(NodeKind::Function, "fn1", "a.rs::fn::fn1", "a.rs", "rust");
        let strct = make_node(
            NodeKind::Struct,
            "MyStruct",
            "a.rs::struct::MyStruct",
            "a.rs",
            "rust",
        );
        store
            .replace_file_graph("a.rs", "h", Some("rust"), None, &[func, strct], &[])
            .unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.node_count, 2);
        assert!(stats.nodes_by_kind.iter().any(|(k, _)| k == "function"));
        assert!(stats.nodes_by_kind.iter().any(|(k, _)| k == "struct"));
    }

    #[test]
    fn stats_returns_languages() {
        let mut store = open_in_memory();
        let node = make_node(NodeKind::Function, "fn1", "a.rs::fn::fn1", "a.rs", "rust");
        store
            .replace_file_graph("a.rs", "h", Some("rust"), None, &[node], &[])
            .unwrap();

        let stats = store.stats().unwrap();
        assert!(stats.languages.contains(&"rust".to_string()));
    }

    #[test]
    fn stats_last_indexed_at_set_after_replace() {
        let mut store = open_in_memory();
        let node = make_node(NodeKind::Function, "fn1", "a.rs::fn::fn1", "a.rs", "rust");
        store
            .replace_file_graph("a.rs", "h", Some("rust"), None, &[node], &[])
            .unwrap();

        let stats = store.stats().unwrap();
        assert!(
            stats.last_indexed_at.is_some(),
            "last_indexed_at should be set"
        );
    }

    // --- file_hashes ---------------------------------------------------------

    #[test]
    fn file_hashes_returns_stored_hashes() {
        let mut store = open_in_memory();
        let nodes_a = vec![make_node(
            NodeKind::Function,
            "f",
            "a.rs::fn::f",
            "a.rs",
            "rust",
        )];
        let nodes_b = vec![make_node(
            NodeKind::Function,
            "g",
            "b.rs::fn::g",
            "b.rs",
            "go",
        )];
        store
            .replace_file_graph("a.rs", "hash_aaa", Some("rust"), None, &nodes_a, &[])
            .unwrap();
        store
            .replace_file_graph("b.rs", "hash_bbb", Some("go"), None, &nodes_b, &[])
            .unwrap();

        let hashes = store.file_hashes().unwrap();
        assert_eq!(hashes.get("a.rs").map(String::as_str), Some("hash_aaa"));
        assert_eq!(hashes.get("b.rs").map(String::as_str), Some("hash_bbb"));
    }

    #[test]
    fn file_hashes_empty_when_no_files() {
        let store = open_in_memory();
        let hashes = store.file_hashes().unwrap();
        assert!(hashes.is_empty());
    }

    #[test]
    fn file_hashes_updated_after_replace() {
        let mut store = open_in_memory();
        let nodes = vec![make_node(
            NodeKind::Function,
            "f",
            "a.rs::fn::f",
            "a.rs",
            "rust",
        )];
        store
            .replace_file_graph("a.rs", "old_hash", None, None, &nodes, &[])
            .unwrap();
        store
            .replace_file_graph("a.rs", "new_hash", None, None, &nodes, &[])
            .unwrap();

        let hashes = store.file_hashes().unwrap();
        assert_eq!(hashes.get("a.rs").map(String::as_str), Some("new_hash"));
        assert_eq!(hashes.len(), 1);
    }

    // --- impact_radius limits ------------------------------------------------

    #[test]
    fn impact_radius_respects_depth_limit() {
        let mut store = open_in_memory();
        // a → b → c → d: with depth=1, only b should be reachable from a.
        let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
        let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
        let nc = make_node(NodeKind::Function, "c", "c.rs::fn::c", "c.rs", "rust");
        let nd = make_node(NodeKind::Function, "d", "d.rs::fn::d", "d.rs", "rust");
        let e1 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
        let e2 = make_edge(EdgeKind::Calls, "b.rs::fn::b", "c.rs::fn::c", "b.rs");
        let e3 = make_edge(EdgeKind::Calls, "c.rs::fn::c", "d.rs::fn::d", "c.rs");
        store
            .replace_file_graph("a.rs", "h", None, None, &[na], &[e1])
            .unwrap();
        store
            .replace_file_graph("b.rs", "h", None, None, &[nb], &[e2])
            .unwrap();
        store
            .replace_file_graph("c.rs", "h", None, None, &[nc], &[e3])
            .unwrap();
        store
            .replace_file_graph("d.rs", "h", None, None, &[nd], &[])
            .unwrap();

        let result = store.impact_radius(&["a.rs"], 1, 200).unwrap();
        let all_files: Vec<&str> = result
            .changed_nodes
            .iter()
            .chain(result.impacted_nodes.iter())
            .map(|n| n.file_path.as_str())
            .collect();
        // At depth 1 from a.rs, b.rs should be reachable but c.rs and d.rs should not.
        assert!(
            all_files.contains(&"b.rs"),
            "b.rs should be reached at depth 1"
        );
        assert!(
            !all_files.contains(&"d.rs"),
            "d.rs beyond depth limit should not appear"
        );
    }

    #[test]
    fn impact_radius_respects_node_count_limit() {
        let mut store = open_in_memory();
        // Create a fan-out: a → b, a → c, a → d, a → e (4 targets)
        let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
        let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
        let nc = make_node(NodeKind::Function, "c", "c.rs::fn::c", "c.rs", "rust");
        let nd = make_node(NodeKind::Function, "d", "d.rs::fn::d", "d.rs", "rust");
        let e1 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
        let e2 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "c.rs::fn::c", "a.rs");
        let e3 = make_edge(EdgeKind::Calls, "a.rs::fn::a", "d.rs::fn::d", "a.rs");
        store
            .replace_file_graph("a.rs", "h", None, None, &[na], &[e1, e2, e3])
            .unwrap();
        store
            .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
            .unwrap();
        store
            .replace_file_graph("c.rs", "h", None, None, &[nc], &[])
            .unwrap();
        store
            .replace_file_graph("d.rs", "h", None, None, &[nd], &[])
            .unwrap();

        // Limit to 2 total nodes — should stop before visiting all of b/c/d.
        let result = store.impact_radius(&["a.rs"], 5, 2).unwrap();
        let total = result.changed_nodes.len() + result.impacted_nodes.len();
        assert!(
            total <= 2,
            "node count limit must be respected; got {total}"
        );
    }

    // --- FTS language / file_path / is_test filters --------------------------

    #[test]
    fn fts_search_respects_language_filter() {
        let mut store = open_in_memory();
        let rust_fn = make_node(
            NodeKind::Function,
            "shared_name",
            "a.rs::fn::shared_name",
            "a.rs",
            "rust",
        );
        let go_fn = make_node(
            NodeKind::Function,
            "shared_name",
            "b.go::fn::shared_name",
            "b.go",
            "go",
        );
        store
            .replace_file_graph("a.rs", "h", Some("rust"), None, &[rust_fn], &[])
            .unwrap();
        store
            .replace_file_graph("b.go", "h", Some("go"), None, &[go_fn], &[])
            .unwrap();

        let q = SearchQuery {
            text: "shared_name".to_string(),
            language: Some("go".to_string()),
            limit: 10,
            ..Default::default()
        };
        let results = store.search(&q).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.node.language == "go"));
    }

    #[test]
    fn fts_search_respects_file_path_filter() {
        let mut store = open_in_memory();
        let na = make_node(
            NodeKind::Function,
            "common",
            "a.rs::fn::common",
            "a.rs",
            "rust",
        );
        let nb = make_node(
            NodeKind::Function,
            "common",
            "b.rs::fn::common",
            "b.rs",
            "rust",
        );
        store
            .replace_file_graph("a.rs", "h", None, None, &[na], &[])
            .unwrap();
        store
            .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
            .unwrap();

        let q = SearchQuery {
            text: "common".to_string(),
            file_path: Some("a.rs".to_string()),
            limit: 10,
            ..Default::default()
        };
        let results = store.search(&q).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.node.file_path == "a.rs"));
    }

    #[test]
    fn fts_search_respects_is_test_filter() {
        let mut store = open_in_memory();
        let mut test_node = make_node(
            NodeKind::Function,
            "test_foo",
            "a.rs::fn::test_foo",
            "a.rs",
            "rust",
        );
        test_node.is_test = true;
        let prod_node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
        store
            .replace_file_graph("a.rs", "h", None, None, &[test_node, prod_node], &[])
            .unwrap();

        // Search for is_test = true should only return test nodes.
        let q = SearchQuery {
            text: "foo".to_string(),
            is_test: Some(true),
            limit: 10,
            ..Default::default()
        };
        let results = store.search(&q).unwrap();
        assert!(!results.is_empty());
        assert!(results.iter().all(|r| r.node.is_test));
    }

    // --- transaction rollback on schema mismatch is covered by migration_creates_schema ---
    // --- NodeId type ---------------------------------------------------------

    #[test]
    fn node_id_assigned_after_insert() {
        let mut store = open_in_memory();
        let node = make_node(
            NodeKind::Function,
            "fn_alpha",
            "a.rs::fn::fn_alpha",
            "a.rs",
            "rust",
        );
        assert_eq!(node.id, NodeId::UNSET, "before insert id must be UNSET");
        store
            .replace_file_graph("a.rs", "h", None, None, &[node], &[])
            .unwrap();
        let fetched = store.nodes_by_file("a.rs").unwrap();
        assert_eq!(fetched.len(), 1);
        assert_ne!(
            fetched[0].id,
            NodeId::UNSET,
            "after insert id must be a real DB id"
        );
        assert!(fetched[0].id.0 > 0);
    }

    #[test]
    fn integrity_check_clean_db() {
        let store = open_in_memory();
        let issues = store
            .integrity_check()
            .expect("integrity_check should not error");
        assert!(
            issues.is_empty(),
            "fresh in-memory DB should have no issues: {issues:?}"
        );
    }

    #[test]
    fn integrity_check_after_writes() {
        let mut store = open_in_memory();
        let node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
        store
            .replace_file_graph("a.rs", "h1", Some("rust"), None, &[node], &[])
            .unwrap();
        let issues = store
            .integrity_check()
            .expect("integrity_check should not error");
        assert!(
            issues.is_empty(),
            "DB with data should still pass integrity check: {issues:?}"
        );
    }
}
