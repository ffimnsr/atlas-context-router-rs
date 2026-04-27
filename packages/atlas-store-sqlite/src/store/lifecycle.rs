use atlas_core::{AtlasError, GraphStats, Node, Result};
use atlas_db_utils::{application_id, apply_atlas_pragmas, migrate_database_to, set_application_id};
use atlas_repo::CanonicalRepoPath;
use rusqlite::{Connection, OpenFlags, params};
use tracing::{debug, info};

use crate::migrations::{LATEST_VERSION, MIGRATION_SET};

use super::{DanglingEdge, Store, helpers::row_to_node};

impl Store {
    const NONCANONICAL_PATH_LIMIT: usize = 100;

    pub fn open(path: &str) -> Result<Self> {
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                // Safe because `Store` owns one thread-confined connection and
                // Atlas keeps DB work out of Rayon closures.
                | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| AtlasError::Db(e.to_string()))?;

        apply_atlas_pragmas(&conn)?;
        set_application_id(&conn, application_id::WORLDTREE)?;
        Self::register_regexp_udf(&conn)?;

        let mut store = Self {
            conn,
            _thread_bound: std::marker::PhantomData,
        };
        store.migrate()?;
        Ok(store)
    }

    /// Register a permanent two-arg `atlas_regexp(pattern, value)` UDF on `conn`.
    ///
    /// Uses a thread-local `(pattern, Regex)` cache so the regex is only
    /// recompiled when the pattern changes between successive SQLite evaluations.
    pub(super) fn register_regexp_udf(conn: &Connection) -> Result<()> {
        use rusqlite::functions::FunctionFlags;
        use rusqlite::types::ValueRef;
        use std::cell::RefCell;

        conn.create_scalar_function(
            "atlas_regexp",
            2,
            FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DETERMINISTIC,
            |ctx| {
                // Zero-alloc: borrow bytes directly from SQLite without a String copy.
                let pat = match ctx.get_raw(0) {
                    ValueRef::Text(b) => std::str::from_utf8(b).unwrap_or(""),
                    _ => return Ok(false),
                };
                let val = match ctx.get_raw(1) {
                    ValueRef::Text(b) => std::str::from_utf8(b).unwrap_or(""),
                    _ => return Ok(false),
                };

                thread_local! {
                    static CACHE: RefCell<Option<(String, regex::Regex)>> = const { RefCell::new(None) };
                }
                CACHE.with(|c| {
                    let mut slot = c.borrow_mut();
                    if !matches!(slot.as_ref(), Some((p, _)) if p == pat) {
                        let re = regex::Regex::new(pat).map_err(|e| {
                            rusqlite::Error::UserFunctionError(e.to_string().into())
                        })?;
                        *slot = Some((pat.to_owned(), re));
                    }
                    Ok(slot.as_ref().unwrap().1.is_match(val))
                })
            },
        )
        .map_err(|e| AtlasError::Db(e.to_string()))
    }

    /// Execute the same SQLite-backed regex UDF path used by query-time
    /// structural scans and regex post-filters.
    pub fn eval_regexp_udf(pattern: &str, value: &str) -> Result<bool> {
        let conn = Connection::open_in_memory().map_err(|e| AtlasError::Db(e.to_string()))?;
        Self::register_regexp_udf(&conn)?;
        conn.query_row(
            "SELECT atlas_regexp(?1, ?2)",
            params![pattern, value],
            |row| row.get(0),
        )
        .map_err(|e| AtlasError::Db(e.to_string()))
    }

    /// Apply any migrations that have not yet been applied to this database.
    pub fn migrate(&mut self) -> Result<()> {
        self.migrate_to(LATEST_VERSION)
    }

    pub fn migrate_to(&mut self, target_version: i32) -> Result<()> {
        let current_version: i32 = self
            .conn
            .query_row(
                "SELECT CAST(value AS INTEGER) FROM metadata WHERE key = 'schema_version'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0);
        debug!(current_version, target_version, "checking migrations");
        if target_version >= current_version {
            for migration in MIGRATION_SET
                .migrations
                .iter()
                .filter(|migration| migration.version > current_version && migration.version <= target_version)
            {
                info!(version = migration.version, name = migration.name, "applying migration");
            }
        } else {
            info!(current_version, target_version, "rebuilding schema for downgrade");
        }
        migrate_database_to(&mut self.conn, &MIGRATION_SET, target_version)
    }

    /// Return high-level statistics about the stored graph.
    /// Return a minimal provenance snapshot: only the two cheapest queries.
    ///
    /// Used by MCP7 to attach compact metadata to every tool response without
    /// the overhead of the full `stats()` call (which groups by kind/language).
    pub fn provenance_meta(&self) -> Result<atlas_core::ProvenanceMeta> {
        let indexed_file_count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM files", [], |r| r.get(0))
            .map_err(|e| AtlasError::Db(e.to_string()))?;

        let last_indexed_at: Option<String> = self
            .conn
            .query_row("SELECT MAX(indexed_at) FROM files", [], |r| r.get(0))
            .unwrap_or(None);

        Ok(atlas_core::ProvenanceMeta {
            indexed_file_count,
            last_indexed_at,
        })
    }

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

        issues.extend(self.noncanonical_path_rows(Self::NONCANONICAL_PATH_LIMIT)?);

        Ok(issues)
    }

    /// Return persisted graph path rows that are not in canonical repo-relative form.
    pub fn noncanonical_path_rows(&self, limit: usize) -> Result<Vec<String>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut issues = Vec::new();

        for (table, column) in [
            ("files", "path"),
            ("nodes", "file_path"),
            ("edges", "file_path"),
        ] {
            if issues.len() >= limit {
                break;
            }
            let remaining = (limit - issues.len()) as i64;
            let sql = format!(
                "SELECT rowid, {column}
                 FROM {table}
                 WHERE instr({column}, char(92)) > 0
                    OR {column} LIKE './%'
                    OR {column} LIKE '../%'
                    OR {column} LIKE '%/./%'
                    OR {column} LIKE '%/../%'
                    OR {column} LIKE '/%'
                    OR {column} LIKE '%//%'
                    OR {column} LIKE '%/'
                 LIMIT ?1"
            );
            let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
            let mut rows = stmt.query(params![remaining]).map_err(db_err)?;
            while let Some(row) = rows.next().map_err(db_err)? {
                let rowid: i64 = row.get(0).map_err(db_err)?;
                let path: String = row.get(1).map_err(db_err)?;
                match CanonicalRepoPath::from_repo_relative(&path) {
                    Ok(canonical) if canonical.as_str() == path => {}
                    Ok(canonical) => issues.push(format!(
                        "noncanonical_path: table={table} rowid={rowid} path={path} canonical={}",
                        canonical.as_str()
                    )),
                    Err(error) => issues.push(format!(
                        "noncanonical_path: table={table} rowid={rowid} path={path} error={error}"
                    )),
                }
            }
        }

        Ok(issues)
    }

    // -------------------------------------------------------------------------
    // Observability — data integrity & graph debug
    // -------------------------------------------------------------------------

    /// Return nodes that have no edges (neither as source nor target).
    ///
    /// These are isolated nodes that may indicate parse gaps or stale data.
    /// `limit` caps the result set; pass `usize::MAX` for all.
    pub fn orphan_nodes(&self, limit: usize) -> Result<Vec<Node>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let sql = "
            SELECT n.id, n.kind, n.name, n.qualified_name, n.file_path,
                   n.line_start, n.line_end, n.language, n.parent_name,
                   n.params, n.return_type, n.modifiers, n.is_test,
                   n.file_hash, n.extra_json
            FROM nodes n
            WHERE NOT EXISTS (
                SELECT 1 FROM edges e
                WHERE e.source_qualified = n.qualified_name
                   OR e.target_qualified = n.qualified_name
            )
            LIMIT ?1
        ";
        let mut stmt = self.conn.prepare(sql).map_err(db_err)?;
        let nodes = stmt
            .query_map(params![limit as i64], row_to_node)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(nodes)
    }

    /// Return edges whose `source_qn` or `target_qn` do not match any node in the graph.
    ///
    /// Each returned tuple is `(edge_id, source_qn, target_qn, kind, side)` where
    /// `side` is `"source"` or `"target"`.
    pub fn dangling_edges(&self, limit: usize) -> Result<Vec<DanglingEdge>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut results = Vec::new();

        // Dangling source
        {
            let sql = "
                SELECT e.id, e.source_qualified, e.target_qualified, e.kind
                FROM edges e
                WHERE NOT EXISTS (SELECT 1 FROM nodes n WHERE n.qualified_name = e.source_qualified)
                LIMIT ?1
            ";
            let mut stmt = self.conn.prepare(sql).map_err(db_err)?;
            let rows: Vec<_> = stmt
                .query_map(params![limit as i64], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(db_err)?
                .filter_map(|r| r.ok())
                .collect();
            for (id, src, tgt, kind) in rows {
                results.push((id, src, tgt, kind, "source"));
            }
        }

        // Dangling target
        if results.len() < limit {
            let remaining = limit - results.len();
            let sql = "
                SELECT e.id, e.source_qualified, e.target_qualified, e.kind
                FROM edges e
                WHERE NOT EXISTS (SELECT 1 FROM nodes n WHERE n.qualified_name = e.target_qualified)
                LIMIT ?1
            ";
            let mut stmt = self.conn.prepare(sql).map_err(db_err)?;
            let rows: Vec<_> = stmt
                .query_map(params![remaining as i64], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })
                .map_err(db_err)?
                .filter_map(|r| r.ok())
                .collect();
            for (id, src, tgt, kind) in rows {
                results.push((id, src, tgt, kind, "target"));
            }
        }

        Ok(results)
    }

    /// Return edge counts grouped by kind, ordered descending.
    pub fn edge_kind_stats(&self) -> Result<Vec<(String, i64)>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare("SELECT kind, COUNT(*) FROM edges GROUP BY kind ORDER BY COUNT(*) DESC")
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// Return the top `n` files by node count.
    pub fn top_files_by_node_count(&self, n: usize) -> Result<Vec<(String, i64)>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT file_path, COUNT(*) AS cnt FROM nodes
                 GROUP BY file_path ORDER BY cnt DESC LIMIT ?1",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![n as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }
}
