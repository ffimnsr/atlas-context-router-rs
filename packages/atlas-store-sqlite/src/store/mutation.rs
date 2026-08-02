use std::collections::HashMap;

use atlas_core::{
    AtlasError, Node, PackageOwner, PackageOwnerKind, ParsedFile, RepoProvenance, Result,
};
use rusqlite::{Connection, params};
use tracing::info;

const LEGACY_SOURCE_REPO_ID: &str = "legacy";

use super::{
    Store,
    helpers::{
        canonicalize_graph_slice, canonicalize_parsed_file, canonicalize_repo_path, row_to_node,
    },
};

#[allow(clippy::too_many_arguments)]
fn upsert_repo_provenance_json(
    extra_json: &serde_json::Value,
    provenance: &RepoProvenance,
) -> serde_json::Value {
    let mut extra = extra_json.as_object().cloned().unwrap_or_default();
    extra
        .entry("repo_id".to_owned())
        .or_insert_with(|| serde_json::Value::String(provenance.repo_id.clone()));
    extra
        .entry("repo_provenance".to_owned())
        .or_insert_with(|| serde_json::to_value(provenance).unwrap_or(serde_json::Value::Null));
    serde_json::Value::Object(extra)
}

#[allow(clippy::too_many_arguments)]
fn do_replace_file_graph(
    conn: &Connection,
    source_repo_id: &str,
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
                 FROM nodes WHERE source_repo_id = ?1 AND file_path = ?2",
            )
            .map_err(db_err)?;
        let rows: Vec<Node> = stmt
            .query_map(params![source_repo_id, path], row_to_node)
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
    conn.execute(
        "DELETE FROM edges WHERE source_repo_id = ?1 AND file_path = ?2",
        params![source_repo_id, path],
    )
    .map_err(db_err)?;
    // Remove dangling cross-file edges referencing old nodes from this file.
    conn.execute(
        "DELETE FROM edges
         WHERE source_repo_id = ?1
           AND (source_qualified IN (SELECT qualified_name FROM nodes WHERE source_repo_id = ?1 AND file_path = ?2)
             OR target_qualified IN (SELECT qualified_name FROM nodes WHERE source_repo_id = ?1 AND file_path = ?2))",
        params![source_repo_id, path],
    )
    .map_err(db_err)?;
    conn.execute(
        "DELETE FROM nodes WHERE source_repo_id = ?1 AND file_path = ?2",
        params![source_repo_id, path],
    )
    .map_err(db_err)?;

    // Step 5: upsert the file row.
    conn.execute(
        "INSERT OR REPLACE INTO files
             (path, language, hash, size, indexed_at, owner_id, owner_kind,
              owner_root, owner_manifest_path, owner_name, source_repo_id)
         VALUES (?1, ?2, ?3, ?4, datetime('now'),
                 (SELECT owner_id FROM files WHERE source_repo_id = ?5 AND path = ?1),
                 (SELECT owner_kind FROM files WHERE source_repo_id = ?5 AND path = ?1),
                 (SELECT owner_root FROM files WHERE source_repo_id = ?5 AND path = ?1),
                 (SELECT owner_manifest_path FROM files WHERE source_repo_id = ?5 AND path = ?1),
                 (SELECT owner_name FROM files WHERE source_repo_id = ?5 AND path = ?1),
                 ?5)",
        params![path, language, hash, size, source_repo_id],
    )
    .map_err(db_err)?;

    let default_repo_provenance = RepoProvenance::new(source_repo_id.to_owned())
        .with_repo_fingerprint(format!("repo_fp_legacy_{source_repo_id}"));

    // Steps 6a + 6b: insert each node then its FTS row.
    for n in nodes {
        let node_extra_json = upsert_repo_provenance_json(
            &n.extra_json,
            n.repo_provenance
                .as_ref()
                .unwrap_or(&default_repo_provenance),
        );
        let extra = serde_json::to_string(&node_extra_json).map_err(AtlasError::Serde)?;
        conn.execute(
            "INSERT OR REPLACE INTO nodes
                 (kind, name, qualified_name, file_path, line_start, line_end,
                  language, parent_name, params, return_type, modifiers,
                  is_test, file_hash, extra_json, source_repo_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",
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
                source_repo_id,
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
        let edge_extra_json = upsert_repo_provenance_json(
            &e.extra_json,
            e.repo_provenance
                .as_ref()
                .unwrap_or(&default_repo_provenance),
        );
        let extra = serde_json::to_string(&edge_extra_json).map_err(AtlasError::Serde)?;
        conn.execute(
            "INSERT INTO edges
                 (kind, source_qualified, target_qualified, file_path,
                  line, confidence, confidence_tier, extra_json, source_repo_id)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            params![
                e.kind.as_str(),
                e.source_qn,
                e.target_qn,
                e.file_path,
                e.line,
                e.confidence,
                e.confidence_tier,
                extra,
                source_repo_id,
            ],
        )
        .map_err(db_err)?;
    }

    Ok(())
}

impl Store {
    pub fn replace_file_graph(
        &mut self,
        path: &str,
        hash: &str,
        language: Option<&str>,
        size: Option<i64>,
        nodes: &[Node],
        edges: &[atlas_core::Edge],
    ) -> Result<()> {
        let normalized = canonicalize_graph_slice(path, nodes, edges)?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;
        match do_replace_file_graph(
            &self.conn,
            LEGACY_SOURCE_REPO_ID,
            &normalized.path,
            hash,
            language,
            size,
            &normalized.nodes,
            &normalized.edges,
        ) {
            Ok(()) => {
                self.conn.execute_batch("COMMIT").map_err(db_err)?;
                info!(
                    path = normalized.path.as_str(),
                    nodes = normalized.nodes.len(),
                    edges = normalized.edges.len(),
                    "replaced file graph"
                );
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
    pub fn replace_files_transactional(&mut self, files: &[ParsedFile]) -> Result<(usize, usize)> {
        self.replace_files_transactional_for_repo(LEGACY_SOURCE_REPO_ID, files)
    }

    pub fn replace_files_transactional_for_repo(
        &mut self,
        source_repo_id: &str,
        files: &[ParsedFile],
    ) -> Result<(usize, usize)> {
        if files.is_empty() {
            return Ok((0, 0));
        }
        let normalized_files = files
            .iter()
            .map(canonicalize_parsed_file)
            .collect::<Result<Vec<_>>>()?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;

        let mut total_nodes = 0usize;
        let mut total_edges = 0usize;
        for f in &normalized_files {
            match do_replace_file_graph(
                &self.conn,
                source_repo_id,
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
        let path = canonicalize_repo_path(path)?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT qualified_name, kind, params, return_type, modifiers, is_test
                 FROM nodes WHERE file_path = ?1",
            )
            .map_err(db_err)?;
        let map = stmt
            .query_map([path.as_str()], |row| {
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
        self.delete_file_graph_for_repo(LEGACY_SOURCE_REPO_ID, path)
    }

    pub fn delete_file_graph_for_repo(&mut self, source_repo_id: &str, path: &str) -> Result<()> {
        let path = canonicalize_repo_path(path)?;
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
                     FROM nodes WHERE source_repo_id = ?1 AND file_path = ?2",
                )
                .map_err(db_err)?;
            let rows: Vec<Node> = stmt
                .query_map(params![source_repo_id, path.as_str()], row_to_node)
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
            .execute(
                "DELETE FROM edges WHERE source_repo_id = ?1 AND file_path = ?2",
                params![source_repo_id, path.as_str()],
            )
            .map_err(db_err)?;
        // Also remove dangling cross-file edges whose source or target
        // qualified name belongs to a node in the deleted file.  These edges
        // originate from other files and would otherwise linger as stale
        // references after the target nodes are gone.
        self.conn
            .execute(
                "DELETE FROM edges
                 WHERE source_repo_id = ?1
                   AND (source_qualified IN (SELECT qualified_name FROM nodes WHERE source_repo_id = ?1 AND file_path = ?2)
                     OR target_qualified IN (SELECT qualified_name FROM nodes WHERE source_repo_id = ?1 AND file_path = ?2))",
                params![source_repo_id, path.as_str()],
            )
            .map_err(db_err)?;
        self.conn
            .execute(
                "DELETE FROM nodes WHERE source_repo_id = ?1 AND file_path = ?2",
                params![source_repo_id, path.as_str()],
            )
            .map_err(db_err)?;
        self.conn
            .execute(
                "DELETE FROM files WHERE source_repo_id = ?1 AND path = ?2",
                params![source_repo_id, path.as_str()],
            )
            .map_err(db_err)?;

        self.conn.execute_batch("COMMIT").map_err(db_err)?;

        info!(path = path.as_str(), "deleted file graph");
        Ok(())
    }

    /// Returns the stored content hash for `path`, or `None` if the file has
    /// not been indexed yet.
    pub fn file_hash(&self, path: &str) -> Result<Option<String>> {
        let path = canonicalize_repo_path(path)?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        use rusqlite::OptionalExtension;
        let result = self
            .conn
            .query_row(
                "SELECT hash FROM files WHERE path = ?1",
                [path.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(db_err)?;
        Ok(result)
    }

    pub fn file_hash_for_repo(&self, source_repo_id: &str, path: &str) -> Result<Option<String>> {
        let path = canonicalize_repo_path(path)?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        use rusqlite::OptionalExtension;
        self.conn
            .query_row(
                "SELECT hash FROM files WHERE source_repo_id = ?1 AND path = ?2",
                params![source_repo_id, path.as_str()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(db_err)
    }

    /// Returns the stored owner metadata for `path`, if present.
    pub fn file_owner(&self, path: &str) -> Result<Option<PackageOwner>> {
        if let Some(owner) = self.file_owner_for_repo(LEGACY_SOURCE_REPO_ID, path)? {
            return Ok(Some(owner));
        }

        let path = canonicalize_repo_path(path)?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        use rusqlite::OptionalExtension;
        let source_repo_id: Option<String> = self
            .conn
            .query_row(
                "SELECT source_repo_id
                 FROM files
                 WHERE path = ?1 AND owner_id IS NOT NULL
                 ORDER BY CASE
                     WHEN source_repo_id = 'legacy' THEN 0
                     WHEN source_repo_id = 'registry' THEN 2
                     ELSE 1
                 END
                 LIMIT 1",
                params![path.as_str()],
                |row| row.get(0),
            )
            .optional()
            .map_err(db_err)?;

        match source_repo_id {
            Some(source_repo_id) => self.file_owner_for_repo(&source_repo_id, path.as_str()),
            None => Ok(None),
        }
    }

    pub fn file_owner_for_repo(
        &self,
        source_repo_id: &str,
        path: &str,
    ) -> Result<Option<PackageOwner>> {
        let path = canonicalize_repo_path(path)?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        use rusqlite::OptionalExtension;
        let result = self
            .conn
            .query_row(
                "SELECT owner_id, owner_kind, owner_root, owner_manifest_path, owner_name
                 FROM files WHERE source_repo_id = ?1 AND path = ?2",
                params![source_repo_id, path.as_str()],
                |row| {
                    let owner_id: Option<String> = row.get(0)?;
                    let owner_kind: Option<String> = row.get(1)?;
                    let owner_root: Option<String> = row.get(2)?;
                    let owner_manifest_path: Option<String> = row.get(3)?;
                    let owner_name: Option<String> = row.get(4)?;
                    Ok(
                        match (owner_id, owner_kind, owner_root, owner_manifest_path) {
                            (Some(owner_id), Some(owner_kind), Some(root), Some(manifest_path)) => {
                                let kind = match owner_kind.as_str() {
                                    "cargo" => PackageOwnerKind::Cargo,
                                    "npm" => PackageOwnerKind::Npm,
                                    "go" => PackageOwnerKind::Go,
                                    _ => return Ok(None),
                                };
                                Some(PackageOwner {
                                    owner_id,
                                    kind,
                                    root,
                                    manifest_path,
                                    package_name: owner_name,
                                })
                            }
                            _ => None,
                        },
                    )
                },
            )
            .optional()
            .map_err(db_err)?
            .flatten();
        Ok(result)
    }

    pub fn file_owner_id(&self, path: &str) -> Result<Option<String>> {
        Ok(self.file_owner(path)?.map(|owner| owner.owner_id))
    }

    pub fn file_owner_id_for_repo(
        &self,
        source_repo_id: &str,
        path: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .file_owner_for_repo(source_repo_id, path)?
            .map(|owner| owner.owner_id))
    }

    pub fn file_paths_with_prefix(&self, prefix: &str) -> Result<Vec<String>> {
        self.file_paths_with_prefix_for_repo(LEGACY_SOURCE_REPO_ID, prefix)
    }

    pub fn file_paths_with_prefix_for_repo(
        &self,
        source_repo_id: &str,
        prefix: &str,
    ) -> Result<Vec<String>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let like = format!("{prefix}%");
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path FROM files WHERE source_repo_id = ?1 AND path LIKE ?2 ORDER BY path",
            )
            .map_err(db_err)?;
        let paths = stmt
            .query_map(params![source_repo_id, like], |row| row.get::<_, String>(0))
            .map_err(db_err)?
            .filter_map(|row| row.ok())
            .collect();
        Ok(paths)
    }

    /// Upsert owner metadata for a stored file row.
    pub fn upsert_file_owner(&mut self, path: &str, owner: Option<&PackageOwner>) -> Result<()> {
        let path = canonicalize_repo_path(path)?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let (owner_id, owner_kind, owner_root, owner_manifest_path, owner_name) = match owner {
            Some(owner) => (
                Some(owner.owner_id.as_str()),
                Some(owner.kind.as_str()),
                Some(owner.root.as_str()),
                Some(owner.manifest_path.as_str()),
                owner.package_name.as_deref(),
            ),
            None => (None, None, None, None, None),
        };
        self.conn
            .execute(
                "UPDATE files
                 SET owner_id = ?2,
                     owner_kind = ?3,
                     owner_root = ?4,
                     owner_manifest_path = ?5,
                     owner_name = ?6
                 WHERE path = ?1",
                params![
                    path.as_str(),
                    owner_id,
                    owner_kind,
                    owner_root,
                    owner_manifest_path,
                    owner_name,
                ],
            )
            .map_err(db_err)?;
        Ok(())
    }

    /// Rename a file in the graph, preserving every node's primary-key `id`.
    ///
    /// Updates `file_path` on all nodes and edges owned by `old_path`, moves
    /// the `files` row, and keeps the FTS index consistent.  Used when a git
    /// rename is detected but the content hash is unchanged — the node graph
    /// can simply be retargeted to the new path instead of being deleted and
    /// rebuilt from scratch.
    pub fn rename_file_graph(&mut self, old_path: &str, new_path: &str) -> Result<()> {
        let old_path = canonicalize_repo_path(old_path)?;
        let new_path = canonicalize_repo_path(new_path)?;
        if old_path == new_path {
            return Ok(());
        }
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());

        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;

        // Read existing nodes so we can update FTS.
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
                .query_map([old_path.as_str()], row_to_node)
                .map_err(db_err)?
                .filter_map(|r| r.ok())
                .collect();
            rows
        };

        // FTS-unindex old file_path entries.
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

        // Update node file_path references.
        self.conn
            .execute(
                "UPDATE nodes SET file_path = ?1 WHERE file_path = ?2",
                [new_path.as_str(), old_path.as_str()],
            )
            .map_err(db_err)?;

        // Update edge file_path references.
        self.conn
            .execute(
                "UPDATE edges SET file_path = ?1 WHERE file_path = ?2",
                [new_path.as_str(), old_path.as_str()],
            )
            .map_err(db_err)?;

        // Move the files row (path is the PK so we delete + re-insert).
        self.conn
            .execute(
                "INSERT OR REPLACE INTO files
                     (path, language, hash, size, indexed_at, owner_id, owner_kind,
                      owner_root, owner_manifest_path, owner_name)
                 SELECT ?1, language, hash, size, datetime('now'), owner_id, owner_kind,
                        owner_root, owner_manifest_path, owner_name
                 FROM files WHERE path = ?2",
                [new_path.as_str(), old_path.as_str()],
            )
            .map_err(db_err)?;
        self.conn
            .execute("DELETE FROM files WHERE path = ?1", [old_path.as_str()])
            .map_err(db_err)?;

        // FTS-reindex with the new file_path.
        for n in &old_nodes {
            self.conn
                .execute(
                    "INSERT INTO nodes_fts (rowid,
                             qualified_name, name, kind, file_path, language,
                             params, return_type, modifiers)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        n.id.0,
                        n.qualified_name,
                        n.name,
                        n.kind.as_str(),
                        new_path.as_str(),
                        n.language,
                        n.params,
                        n.return_type,
                        n.modifiers,
                    ],
                )
                .map_err(db_err)?;
        }

        self.conn.execute_batch("COMMIT").map_err(db_err)?;

        info!(
            old_path = old_path.as_str(),
            new_path = new_path.as_str(),
            nodes = old_nodes.len(),
            "renamed file graph"
        );
        Ok(())
    }
}
