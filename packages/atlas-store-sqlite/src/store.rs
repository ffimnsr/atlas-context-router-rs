use std::collections::HashMap;

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
        match do_replace_file_graph(&self.conn, path, hash, language, size, nodes, edges) {
            Ok(()) => {
                self.conn.execute_batch("COMMIT").map_err(db_err)?;
                info!(path, nodes = nodes.len(), edges = edges.len(), "replaced file graph");
                Ok(())
            }
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }

    /// Replace graph slices for multiple parsed files in one transaction.
    ///
    /// Significantly faster than calling `replace_file_graph` per file: the
    /// SQLite write-ahead log is flushed once per batch rather than once per
    /// file.  If any file fails the entire batch is rolled back.
    ///
    /// Returns `(total_nodes, total_edges)` inserted.
    pub fn replace_files_transactional(
        &mut self,
        files: &[ParsedFile],
    ) -> Result<(usize, usize)> {
        if files.is_empty() {
            return Ok((0, 0));
        }
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;

        let mut total_nodes = 0usize;
        let mut total_edges = 0usize;
        for f in files {
            match do_replace_file_graph(
                &self.conn,
                &f.path,
                &f.hash,
                f.language.as_deref(),
                f.size,
                &f.nodes,
                &f.edges,
            ) {
                Ok(()) => {
                    total_nodes += f.nodes.len();
                    total_edges += f.edges.len();
                    info!(
                        path = f.path.as_str(),
                        nodes = f.nodes.len(),
                        edges = f.edges.len(),
                        "replaced file graph"
                    );
                }
                Err(e) => {
                    let _ = self.conn.execute_batch("ROLLBACK");
                    return Err(e);
                }
            }
        }
        self.conn.execute_batch("COMMIT").map_err(db_err)?;
        Ok((total_nodes, total_edges))
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

    /// Returns a map of `qualified_name → content-signature` for every node
    /// stored for `path`.
    ///
    /// The signature encodes the structural attributes that determine whether
    /// dependents of a symbol need re-evaluation: `kind`, `params`,
    /// `return_type`, `modifiers`, and `is_test`.  Line positions are excluded
    /// intentionally — moving a function within a file does not change its
    /// interface and must not trigger unnecessary dependent reparsing.
    pub fn node_signatures_by_file(&self, path: &str) -> Result<HashMap<String, String>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT qualified_name, kind, params, return_type, modifiers, is_test
                 FROM nodes WHERE file_path = ?1",
            )
            .map_err(db_err)?;
        let map = stmt
            .query_map([path], |row| {
                let qn: String = row.get(0)?;
                let kind: String = row.get(1)?;
                let params: Option<String> = row.get(2)?;
                let ret: Option<String> = row.get(3)?;
                let mods: Option<String> = row.get(4)?;
                let is_test: i32 = row.get(5)?;
                let sig = format!(
                    "{kind}|{}|{}|{}|{is_test}",
                    params.as_deref().unwrap_or(""),
                    ret.as_deref().unwrap_or(""),
                    mods.as_deref().unwrap_or(""),
                );
                Ok((qn, sig))
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(map)
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

    /// Files that have at least one edge pointing into any of `changed_qnames`.
    ///
    /// More targeted than [`find_dependents`] which operates on file paths:
    /// this accepts specific qualified names so the caller can restrict
    /// invalidation to symbols whose signatures actually changed, avoiding
    /// unnecessary reparsing of files that only depend on stable symbols.
    pub fn find_dependents_for_qnames(&self, changed_qnames: &[&str]) -> Result<Vec<String>> {
        if changed_qnames.is_empty() {
            return Ok(vec![]);
        }
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());

        let placeholders = repeat_placeholders(changed_qnames.len());
        // Find source files of edges whose target is one of the changed QNs.
        // Source files that define those QNs are excluded (they are the changed
        // files themselves and will be processed by the caller already).
        let sql = format!(
            "SELECT DISTINCT ns.file_path
             FROM edges  e
             JOIN nodes  ns ON e.source_qualified = ns.qualified_name
             WHERE e.target_qualified IN ({placeholders})
               AND e.source_qualified NOT IN (
                   SELECT qualified_name FROM nodes
                   WHERE qualified_name IN ({placeholders})
               )
             ORDER BY ns.file_path"
        );

        let params: Vec<&dyn rusqlite::types::ToSql> = changed_qnames
            .iter()
            .chain(changed_qnames.iter())
            .map(|q| q as &dyn rusqlite::types::ToSql)
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

        // FTS5 expects the MATCH operand to be an unquoted query string.
        let fts_query = fts5_escape(&query.text);

        // Build a LIKE pattern from the subpath (escape SQLite LIKE wildcards).
        let subpath_like = query.subpath.as_deref().map(|sp| {
            let escaped = sp.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
            format!("{escaped}%")
        });

        // Build dynamic WHERE clause and a matching params vector so the
        // number of `?` placeholders always equals the number of bound values.
        let mut filters: Vec<String> = vec!["nodes_fts MATCH ?".to_string()];
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(fts_query)];

        if let Some(kind) = &query.kind {
            filters.push("n.kind = ?".to_string());
            params.push(Box::new(kind.clone()));
        }
        if let Some(lang) = &query.language {
            filters.push("n.language = ?".to_string());
            params.push(Box::new(lang.clone()));
        }
        if let Some(fp) = &query.file_path {
            filters.push("n.file_path = ?".to_string());
            params.push(Box::new(fp.clone()));
        }
        if let Some(is_test) = query.is_test {
            filters.push(format!("n.is_test = {}", is_test as i32));
        }
        if let Some(ref like_pat) = subpath_like {
            filters.push("n.file_path LIKE ? ESCAPE '\\'".to_string());
            params.push(Box::new(like_pat.clone()));
        }

        // LIMIT is always the last positional parameter.
        params.push(Box::new(query.limit as i64));

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
             LIMIT  ?"
        );

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|b| b.as_ref()).collect();

        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
        let results = stmt
            .query_map(params_ref.as_slice(), |row| {
                    let node = row_to_node(row)?;
                    let score: f64 = row.get(15)?;
                    Ok(ScoredNode {
                        node,
                        // BM25 returns negative values; negate for ascending score.
                        score: -score,
                    })
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();

        Ok(results)
    }

    /// Return all nodes reachable by exactly one edge hop from any of the
    /// given `qualified_names`, excluding those names themselves.
    ///
    /// Used by the search layer for graph-aware result expansion.
    pub fn nodes_connected_to(&self, qualified_names: &[&str]) -> Result<Vec<Node>> {
        if qualified_names.is_empty() {
            return Ok(vec![]);
        }
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let ph = repeat_placeholders(qualified_names.len());

        // Collect target_qualified names reachable forward OR backward,
        // then look them up as nodes, excluding the seed set.
        let sql = format!(
            "SELECT DISTINCT n.id, n.kind, n.name, n.qualified_name, n.file_path,
                    n.line_start, n.line_end, n.language, n.parent_name,
                    n.params, n.return_type, n.modifiers, n.is_test,
                    n.file_hash, n.extra_json
             FROM nodes n
             WHERE n.qualified_name IN (
                 SELECT e.target_qualified FROM edges e WHERE e.source_qualified IN ({ph})
                 UNION
                 SELECT e.source_qualified FROM edges e WHERE e.target_qualified IN ({ph})
             )
             AND n.qualified_name NOT IN ({ph})"
        );

        // Bind the list three times: forward targets, backward targets, exclusion.
        let params_vec: Vec<&dyn rusqlite::types::ToSql> = qualified_names
            .iter()
            .chain(qualified_names.iter())
            .chain(qualified_names.iter())
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();

        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(params_vec.as_slice(), row_to_node)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Core per-file graph replacement logic without transaction management.
///
/// Performs all the FTS-delete / DELETE / UPSERT / INSERT steps for a single
/// file.  The caller is responsible for wrapping calls in a transaction
/// (either per-file with `BEGIN IMMEDIATE`/`COMMIT` or a multi-file batch).
fn do_replace_file_graph(
    conn: &Connection,
    path: &str,
    hash: &str,
    language: Option<&str>,
    size: Option<i64>,
    nodes: &[Node],
    edges: &[atlas_core::Edge],
) -> Result<()> {
    let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());

    // Step 2: FTS-unindex old nodes.
    let old_nodes = {
        let mut stmt = conn
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
        conn.execute(
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
    conn.execute("DELETE FROM edges WHERE file_path = ?1", [path])
        .map_err(db_err)?;
    // Remove dangling cross-file edges referencing old nodes from this file.
    conn.execute(
        "DELETE FROM edges
         WHERE source_qualified IN (SELECT qualified_name FROM nodes WHERE file_path = ?1)
            OR target_qualified IN (SELECT qualified_name FROM nodes WHERE file_path = ?1)",
        [path],
    )
    .map_err(db_err)?;
    conn.execute("DELETE FROM nodes WHERE file_path = ?1", [path])
        .map_err(db_err)?;

    // Step 5: upsert the file row.
    conn.execute(
        "INSERT OR REPLACE INTO files (path, language, hash, size, indexed_at)
         VALUES (?1, ?2, ?3, ?4, datetime('now'))",
        params![path, language, hash, size],
    )
    .map_err(db_err)?;

    // Steps 6a + 6b: insert each node then its FTS row.
    for n in nodes {
        let extra = serde_json::to_string(&n.extra_json).map_err(AtlasError::Serde)?;
        conn.execute(
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

        let rowid = conn.last_insert_rowid();
        conn.execute(
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
        conn.execute(
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

    Ok(())
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

    #[test]
    fn impact_radius_disconnected_graph() {
        let mut store = open_in_memory();
        // a.rs and c.rs exist but share no edges; b.rs is connected to a.rs.
        let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
        let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
        let nc = make_node(NodeKind::Function, "c", "c.rs::fn::c", "c.rs", "rust");
        let edge = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
        store
            .replace_file_graph("a.rs", "h", None, None, &[na], &[edge])
            .unwrap();
        store
            .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
            .unwrap();
        store
            .replace_file_graph("c.rs", "h", None, None, &[nc], &[])
            .unwrap();

        let result = store.impact_radius(&["a.rs"], 5, 200).unwrap();
        // b.rs is reachable; c.rs is disconnected and must not appear.
        let all_qns: Vec<&str> = result
            .changed_nodes
            .iter()
            .chain(result.impacted_nodes.iter())
            .map(|n| n.qualified_name.as_str())
            .collect();
        assert!(all_qns.contains(&"a.rs::fn::a"), "seed node must be present");
        assert!(all_qns.contains(&"b.rs::fn::b"), "connected node must be present");
        assert!(
            !all_qns.contains(&"c.rs::fn::c"),
            "disconnected node must not appear"
        );
    }

    #[test]
    fn impact_radius_depth_cap() {
        let mut store = open_in_memory();
        // Chain: a → b → c → d  (each hop is one depth unit)
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

        // max_depth=1: only b should be reachable beyond the seed.
        let result = store.impact_radius(&["a.rs"], 1, 200).unwrap();
        let impacted_qns: Vec<&str> = result
            .impacted_nodes
            .iter()
            .map(|n| n.qualified_name.as_str())
            .collect();
        assert!(
            impacted_qns.contains(&"b.rs::fn::b"),
            "one-hop node must be reachable at depth=1"
        );
        assert!(
            !impacted_qns.contains(&"c.rs::fn::c"),
            "two-hop node must not be reachable at depth=1"
        );
        assert!(
            !impacted_qns.contains(&"d.rs::fn::d"),
            "three-hop node must not be reachable at depth=1"
        );
    }

    #[test]
    fn impact_radius_max_node_cap() {
        let mut store = open_in_memory();
        // Star topology: a.rs is the seed; b, c, d, e each called from a.
        let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
        let nodes_src: Vec<(&str, &str, &str)> = vec![
            ("b", "b.rs::fn::b", "b.rs"),
            ("c", "c.rs::fn::c", "c.rs"),
            ("d", "d.rs::fn::d", "d.rs"),
            ("e", "e.rs::fn::e", "e.rs"),
        ];
        let mut edges: Vec<Edge> = nodes_src
            .iter()
            .map(|(_, qn, fp)| make_edge(EdgeKind::Calls, "a.rs::fn::a", qn, fp))
            .collect();
        store
            .replace_file_graph("a.rs", "h", None, None, &[na], &edges)
            .unwrap();
        // Clear so we can reuse this vec as a per-file edges slice.
        edges.clear();
        for (name, qn, fp) in &nodes_src {
            let n = make_node(NodeKind::Function, name, qn, fp, "rust");
            store
                .replace_file_graph(fp, "h", None, None, &[n], &[])
                .unwrap();
        }

        // Cap at 2 total nodes; seed a.rs has 1 node, so at most 1 impacted node
        // should be returned regardless of star size.
        let result = store.impact_radius(&["a.rs"], 5, 2).unwrap();
        let total = result.changed_nodes.len() + result.impacted_nodes.len();
        assert!(
            total <= 2,
            "total nodes must not exceed max_nodes cap; got {total}"
        );
    }

    #[test]
    fn impact_radius_deleted_seed_file_returns_empty() {
        let mut store = open_in_memory();
        let na = make_node(NodeKind::Function, "a", "a.rs::fn::a", "a.rs", "rust");
        let nb = make_node(NodeKind::Function, "b", "b.rs::fn::b", "b.rs", "rust");
        let edge = make_edge(EdgeKind::Calls, "a.rs::fn::a", "b.rs::fn::b", "a.rs");
        store
            .replace_file_graph("a.rs", "h", None, None, &[na], &[edge])
            .unwrap();
        store
            .replace_file_graph("b.rs", "h", None, None, &[nb], &[])
            .unwrap();

        // Delete the seed file before querying impact.
        store.delete_file_graph("a.rs").unwrap();

        let result = store.impact_radius(&["a.rs"], 5, 200).unwrap();
        assert!(
            result.changed_nodes.is_empty(),
            "deleted seed file must yield no changed nodes"
        );
        assert!(
            result.impacted_nodes.is_empty(),
            "deleted seed file must yield no impacted nodes"
        );
    }

    #[test]
    fn impact_radius_seed_file_with_no_nodes() {
        let mut store = open_in_memory();
        // Index a file record with zero nodes.
        store
            .replace_file_graph("empty.rs", "h", None, None, &[], &[])
            .unwrap();

        let result = store.impact_radius(&["empty.rs"], 5, 200).unwrap();
        assert!(
            result.changed_nodes.is_empty(),
            "file with no nodes must yield no changed nodes"
        );
        assert!(
            result.impacted_nodes.is_empty(),
            "file with no nodes must yield no impacted nodes"
        );
        assert!(result.relevant_edges.is_empty());
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

    // -------------------------------------------------------------------------
    // replace_files_transactional
    // -------------------------------------------------------------------------

    #[test]
    fn replace_files_transactional_inserts_all_files() {
        let mut store = open_in_memory();
        let files = vec![
            ParsedFile {
                path: "src/a.rs".to_string(),
                language: Some("rust".to_string()),
                hash: "h1".to_string(),
                size: Some(100),
                nodes: vec![make_node(
                    NodeKind::Function,
                    "foo",
                    "src/a.rs::fn::foo",
                    "src/a.rs",
                    "rust",
                )],
                edges: vec![],
            },
            ParsedFile {
                path: "src/b.rs".to_string(),
                language: Some("rust".to_string()),
                hash: "h2".to_string(),
                size: Some(200),
                nodes: vec![
                    make_node(
                        NodeKind::Function,
                        "bar",
                        "src/b.rs::fn::bar",
                        "src/b.rs",
                        "rust",
                    ),
                    make_node(
                        NodeKind::Function,
                        "baz",
                        "src/b.rs::fn::baz",
                        "src/b.rs",
                        "rust",
                    ),
                ],
                edges: vec![make_edge(
                    EdgeKind::Calls,
                    "src/b.rs::fn::baz",
                    "src/b.rs::fn::bar",
                    "src/b.rs",
                )],
            },
        ];

        let (total_nodes, total_edges) = store.replace_files_transactional(&files).unwrap();
        assert_eq!(total_nodes, 3);
        assert_eq!(total_edges, 1);

        let stats = store.stats().unwrap();
        assert_eq!(stats.file_count, 2);
        assert_eq!(stats.node_count, 3);
        assert_eq!(stats.edge_count, 1);
    }

    #[test]
    fn replace_files_transactional_empty_is_noop() {
        let mut store = open_in_memory();
        let (n, e) = store.replace_files_transactional(&[]).unwrap();
        assert_eq!(n, 0);
        assert_eq!(e, 0);
        assert_eq!(store.stats().unwrap().file_count, 0);
    }

    #[test]
    fn replace_files_transactional_is_idempotent() {
        let mut store = open_in_memory();
        let files = vec![ParsedFile {
            path: "a.rs".to_string(),
            language: Some("rust".to_string()),
            hash: "h1".to_string(),
            size: None,
            nodes: vec![make_node(
                NodeKind::Function,
                "foo",
                "a.rs::fn::foo",
                "a.rs",
                "rust",
            )],
            edges: vec![],
        }];
        store.replace_files_transactional(&files).unwrap();
        store.replace_files_transactional(&files).unwrap();
        assert_eq!(store.stats().unwrap().node_count, 1);
    }

    // -------------------------------------------------------------------------
    // node_signatures_by_file
    // -------------------------------------------------------------------------

    #[test]
    fn node_signatures_by_file_returns_empty_for_unknown_file() {
        let store = open_in_memory();
        let sigs = store.node_signatures_by_file("nonexistent.rs").unwrap();
        assert!(sigs.is_empty());
    }

    #[test]
    fn node_signatures_by_file_returns_entry_per_node() {
        let mut store = open_in_memory();
        let nodes = vec![
            make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust"),
            make_node(NodeKind::Function, "bar", "a.rs::fn::bar", "a.rs", "rust"),
        ];
        store
            .replace_file_graph("a.rs", "h1", Some("rust"), None, &nodes, &[])
            .unwrap();

        let sigs = store.node_signatures_by_file("a.rs").unwrap();
        assert_eq!(sigs.len(), 2);
        assert!(sigs.contains_key("a.rs::fn::foo"));
        assert!(sigs.contains_key("a.rs::fn::bar"));
    }

    #[test]
    fn node_signatures_stable_across_position_change() {
        let mut store = open_in_memory();
        let mut node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
        node.line_start = 1;
        node.line_end = 5;
        store
            .replace_file_graph("a.rs", "h1", Some("rust"), None, &[node.clone()], &[])
            .unwrap();
        let sigs_before = store.node_signatures_by_file("a.rs").unwrap();

        // Move the function to different lines — signature must be identical.
        let mut moved = node;
        moved.line_start = 100;
        moved.line_end = 110;
        store
            .replace_file_graph("a.rs", "h2", Some("rust"), None, &[moved], &[])
            .unwrap();
        let sigs_after = store.node_signatures_by_file("a.rs").unwrap();

        assert_eq!(
            sigs_before["a.rs::fn::foo"],
            sigs_after["a.rs::fn::foo"],
            "moving a function should not change its signature"
        );
    }

    // -------------------------------------------------------------------------
    // find_dependents_for_qnames
    // -------------------------------------------------------------------------

    #[test]
    fn find_dependents_for_qnames_returns_importers_of_changed_symbols() {
        let mut store = open_in_memory();

        // a.rs defines `foo`
        let node_a = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
        store
            .replace_file_graph("a.rs", "h1", Some("rust"), None, &[node_a], &[])
            .unwrap();

        // b.rs defines `bar` and calls a.rs::fn::foo
        let node_b = make_node(NodeKind::Function, "bar", "b.rs::fn::bar", "b.rs", "rust");
        let edge_b_to_a = make_edge(EdgeKind::Calls, "b.rs::fn::bar", "a.rs::fn::foo", "b.rs");
        store
            .replace_file_graph("b.rs", "h2", Some("rust"), None, &[node_b], &[edge_b_to_a])
            .unwrap();

        // c.rs defines `qux` with no edges
        let node_c = make_node(NodeKind::Function, "qux", "c.rs::fn::qux", "c.rs", "rust");
        store
            .replace_file_graph("c.rs", "h3", Some("rust"), None, &[node_c], &[])
            .unwrap();

        // Changing a.rs::fn::foo should only invalidate b.rs, not c.rs.
        let deps = store
            .find_dependents_for_qnames(&["a.rs::fn::foo"])
            .unwrap();
        assert_eq!(deps, vec!["b.rs"]);
    }

    #[test]
    fn find_dependents_for_qnames_empty_input_returns_empty() {
        let store = open_in_memory();
        let deps = store.find_dependents_for_qnames(&[]).unwrap();
        assert!(deps.is_empty());
    }

    #[test]
    fn find_dependents_for_qnames_no_edges_returns_empty() {
        let mut store = open_in_memory();
        let node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
        store
            .replace_file_graph("a.rs", "h1", Some("rust"), None, &[node], &[])
            .unwrap();
        let deps = store
            .find_dependents_for_qnames(&["a.rs::fn::foo"])
            .unwrap();
        assert!(deps.is_empty());
    }
}
