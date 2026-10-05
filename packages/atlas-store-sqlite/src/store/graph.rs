use atlas_core::{
    AtlasError, BudgetManager, BudgetPolicy, Edge, FileRecord, ImpactResult, Node, Result,
    kinds::normalize_kind_alias,
};
use rusqlite::params;

use super::{
    Store,
    helpers::{
        canonicalize_graph_slice, canonicalize_repo_path, repeat_placeholders, row_to_edge,
        row_to_node,
    },
};

/// Maximum page size accepted by [`Store::list_nodes`].
///
/// Larger requests clamp so a single call cannot materialize an unbounded
/// slice of the graph.
pub const MAX_NODE_LIST_LIMIT: usize = 10_000;

/// Optional filters for [`Store::list_nodes`].
///
/// Non-`None` values combine with `AND`. `kind` accepts canonical names and
/// aliases (`fn`, `record`, ...); the store normalizes it internally. `subpath`
/// is a canonical repo-relative path prefix (for example `packages/atlas-core`);
/// callers normalize it at the consumer boundary per the path identity
/// invariant. `repo_id` matches `nodes.source_repo_id` for multi-repo graphs.
#[derive(Clone, Copy, Debug, Default)]
pub struct NodeListFilter<'a> {
    pub kind: Option<&'a str>,
    pub language: Option<&'a str>,
    pub subpath: Option<&'a str>,
    pub repo_id: Option<&'a str>,
}

/// One deterministic page of nodes plus the total number of matching rows.
#[derive(Clone, Debug)]
pub struct NodeListPage {
    pub nodes: Vec<Node>,
    pub total: u64,
    pub limit: usize,
    pub offset: usize,
}

impl NodeListPage {
    /// Offset for the next page, or `None` when this page reaches the end.
    pub fn next_offset(&self) -> Option<usize> {
        let consumed = self.offset.saturating_add(self.nodes.len());
        if (consumed as u64) < self.total {
            Some(consumed)
        } else {
            None
        }
    }

    pub fn has_more(&self) -> bool {
        self.next_offset().is_some()
    }
}

/// Smallest string strictly greater than every string starting with `prefix`.
///
/// Turns a path-prefix filter into a half-open range
/// (`file_path >= prefix AND file_path < upper`) so SQLite can use the
/// `file_path` index instead of a full-table `substr()` scan. `None` means no
/// upper bound exists because the prefix is all maximal scalar values
/// (practically unreachable for repo paths); callers fall back to the exact
/// `substr()` comparison in that case.
pub(super) fn prefix_upper_bound(prefix: &str) -> Option<String> {
    let mut chars: Vec<char> = prefix.chars().collect();
    while let Some(last) = chars.pop() {
        let mut candidate = last as u32 + 1;
        while candidate <= 0x10FFFF {
            if let Some(next) = char::from_u32(candidate) {
                chars.push(next);
                return Some(chars.into_iter().collect());
            }
            candidate += 1;
        }
    }
    None
}

impl Store {
    fn sort_impact_result(result: &mut ImpactResult) {
        result
            .changed_nodes
            .sort_by(|left, right| left.qualified_name.cmp(&right.qualified_name));
        result
            .impacted_nodes
            .sort_by(|left, right| left.qualified_name.cmp(&right.qualified_name));
        result.impacted_files.sort();
        result.impacted_files.dedup();
        result.relevant_edges.sort_by(|left, right| {
            left.source_qn
                .cmp(&right.source_qn)
                .then_with(|| left.target_qn.cmp(&right.target_qn))
                .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
                .then_with(|| left.file_path.cmp(&right.file_path))
                .then_with(|| left.line.cmp(&right.line))
        });
    }

    pub fn nodes_by_file(&self, path: &str) -> Result<Vec<Node>> {
        let path = canonicalize_repo_path(path)?;
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

    pub fn nodes_by_file_for_repo(&self, source_repo_id: &str, path: &str) -> Result<Vec<Node>> {
        let path = canonicalize_repo_path(path)?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                        language, parent_name, params, return_type, modifiers,
                        is_test, file_hash, extra_json
                 FROM nodes WHERE source_repo_id = ?1 AND file_path = ?2
                 ORDER BY line_start",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![source_repo_id, path.as_str()], row_to_node)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// All file records in the graph, ordered by canonical repo-relative path.
    ///
    /// Used by docs generation and other whole-graph consumers that need the
    /// complete file inventory in one deterministic pass.
    pub fn list_files(&self) -> Result<Vec<FileRecord>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT path, language, hash, size, indexed_at,
                        owner_id, owner_kind, owner_root, owner_manifest_path, owner_name
                 FROM files ORDER BY path",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(FileRecord {
                    path: row.get(0)?,
                    language: row.get(1)?,
                    hash: row.get(2)?,
                    size: row.get(3)?,
                    indexed_at: row.get(4)?,
                    owner_id: row.get(5)?,
                    owner_kind: row.get(6)?,
                    owner_root: row.get(7)?,
                    owner_manifest_path: row.get(8)?,
                    owner_name: row.get(9)?,
                    repo_provenance: None,
                })
            })
            .map_err(db_err)?;
        let mut files: Vec<FileRecord> = rows.filter_map(|r| r.ok()).collect();
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    /// All nodes in the graph, ordered by file path, line start, then qualified
    /// name, so whole-graph consumers render deterministically.
    pub fn list_all_nodes(&self) -> Result<Vec<Node>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                        language, parent_name, params, return_type, modifiers,
                        is_test, file_hash, extra_json
                 FROM nodes
                 ORDER BY file_path, line_start, qualified_name",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], row_to_node)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// One deterministic page of nodes matching `filter`, plus the total
    /// match count for the same filter.
    ///
    /// Pagination is offset-based over a stable `(file_path, line_start,
    /// qualified_name)` ordering, so repeated calls with the same offset see
    /// the same slice while the graph is unchanged. `limit` clamps to
    /// `1..=`[`MAX_NODE_LIST_LIMIT`]. A `subpath` filter compiles to an indexed
    /// half-open range on `file_path`.
    pub fn list_nodes(
        &self,
        filter: &NodeListFilter<'_>,
        limit: usize,
        offset: usize,
    ) -> Result<NodeListPage> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let limit = limit.clamp(1, MAX_NODE_LIST_LIMIT);
        // Alias normalization lives here as well as at consumer boundaries so a
        // raw `fn`/`record` filter cannot silently match nothing.
        let kind = filter.kind.map(normalize_kind_alias);
        let subpath_upper = filter.subpath.and_then(prefix_upper_bound);
        let where_clause = "WHERE (?1 IS NULL OR kind = ?1)
                   AND (?2 IS NULL OR language = ?2)
                   AND (?3 IS NULL OR ((?4 IS NOT NULL AND file_path >= ?3 AND file_path < ?4)
                        OR (?4 IS NULL AND substr(file_path, 1, length(?3)) = ?3)))
                   AND (?5 IS NULL OR source_repo_id = ?5)";
        let count_sql = format!("SELECT COUNT(*) FROM nodes {where_clause}");
        let total: u64 = self
            .conn
            .query_row(
                &count_sql,
                params![
                    kind,
                    filter.language,
                    filter.subpath,
                    subpath_upper,
                    filter.repo_id
                ],
                |row| row.get(0),
            )
            .map_err(db_err)?;
        let page_sql = format!(
            "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                    language, parent_name, params, return_type, modifiers,
                    is_test, file_hash, extra_json
             FROM nodes {where_clause}
             ORDER BY file_path, line_start, qualified_name
             LIMIT ?6 OFFSET ?7"
        );
        let mut stmt = self.conn.prepare(&page_sql).map_err(db_err)?;
        let rows = stmt
            .query_map(
                params![
                    kind,
                    filter.language,
                    filter.subpath,
                    subpath_upper,
                    filter.repo_id,
                    limit as i64,
                    offset as i64
                ],
                row_to_node,
            )
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(NodeListPage {
            nodes: rows,
            total,
            limit,
            offset,
        })
    }

    /// All edges in the graph, ordered by source, target, kind, then file path.
    ///
    /// Used by whole-graph consumers (docs generation, diagram export) that
    /// derive callers/callees and dependency summaries in one pass.
    pub fn list_all_edges(&self) -> Result<Vec<Edge>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, source_qualified, target_qualified, file_path,
                        line, confidence, confidence_tier, extra_json
                 FROM edges
                 ORDER BY source_qualified, target_qualified, kind, file_path",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([], row_to_edge)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    /// All edges whose `file_path` column matches `path`.
    pub fn edges_by_file(&self, path: &str) -> Result<Vec<atlas_core::Edge>> {
        let path = canonicalize_repo_path(path)?;
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

    pub fn edges_by_file_for_repo(
        &self,
        source_repo_id: &str,
        path: &str,
    ) -> Result<Vec<atlas_core::Edge>> {
        let path = canonicalize_repo_path(path)?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, source_qualified, target_qualified, file_path,
                        line, confidence, confidence_tier, extra_json
                 FROM edges WHERE source_repo_id = ?1 AND file_path = ?2",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map(params![source_repo_id, path.as_str()], row_to_edge)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn rewrite_file_edges_for_repo(
        &mut self,
        source_repo_id: &str,
        path: &str,
        edges: &[atlas_core::Edge],
    ) -> Result<()> {
        let normalized = canonicalize_graph_slice(path, &[], edges)?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;
        self.conn
            .execute(
                "DELETE FROM edges WHERE source_repo_id = ?1 AND file_path = ?2",
                params![source_repo_id, normalized.path],
            )
            .map_err(db_err)?;
        for edge in &normalized.edges {
            let extra = serde_json::to_string(&edge.extra_json).map_err(AtlasError::Serde)?;
            self.conn
                .execute(
                    "INSERT INTO edges
                         (kind, source_qualified, target_qualified, file_path,
                          line, confidence, confidence_tier, extra_json, source_repo_id)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                    params![
                        edge.kind.as_str(),
                        edge.source_qn,
                        edge.target_qn,
                        edge.file_path,
                        edge.line,
                        edge.confidence,
                        edge.confidence_tier,
                        extra,
                        source_repo_id,
                    ],
                )
                .map_err(db_err)?;
        }
        self.conn.execute_batch("COMMIT").map_err(db_err)?;
        Ok(())
    }

    /// Return callable nodes with the given simple `name` and `language`.
    pub fn callable_nodes_by_name(&self, language: &str, name: &str) -> Result<Vec<Node>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                        language, parent_name, params, return_type, modifiers,
                        is_test, file_hash, extra_json
                 FROM nodes
                 WHERE language = ?1
                   AND name = ?2
                   AND kind IN ('function', 'method', 'test')
                 ORDER BY file_path, line_start",
            )
            .map_err(db_err)?;
        let rows = stmt
            .query_map([language, name], row_to_node)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(rows)
    }

    pub fn file_hashes_for_repo(
        &self,
        source_repo_id: &str,
    ) -> Result<std::collections::HashMap<String, String>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare("SELECT path, hash FROM files WHERE source_repo_id = ?1")
            .map_err(db_err)?;
        let map = stmt
            .query_map([source_repo_id], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(map)
    }

    /// Returns the paths of the `n` most recently indexed files (ordered by
    /// `indexed_at` descending). Used by the search layer when
    /// `SearchQuery::recent_file_boost` is enabled.
    pub fn recently_indexed_files(&self, n: usize) -> Result<Vec<String>> {
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let mut stmt = self
            .conn
            .prepare("SELECT path FROM files ORDER BY indexed_at DESC LIMIT ?1")
            .map_err(db_err)?;
        let paths = stmt
            .query_map([n as i64], |r| r.get::<_, String>(0))
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();
        Ok(paths)
    }

    /// Files that have at least one edge pointing into any of `changed_qnames`.
    ///
    /// More targeted than path-based invalidation: this accepts specific
    /// qualified names so the caller can restrict invalidation to symbols whose
    /// signatures actually changed, avoiding unnecessary reparsing of files
    /// that only depend on stable symbols.
    pub fn find_dependents_for_qnames_for_repo(
        &self,
        source_repo_id: &str,
        changed_qnames: &[&str],
    ) -> Result<Vec<String>> {
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
             FROM edges e
             JOIN nodes ns
               ON ns.source_repo_id = e.source_repo_id
              AND ns.qualified_name = e.source_qualified
             WHERE e.source_repo_id = ?
               AND e.target_qualified IN ({placeholders})
               AND e.source_qualified NOT IN (
                   SELECT qualified_name FROM nodes
                   WHERE source_repo_id = ? AND qualified_name IN ({placeholders})
               )
             ORDER BY ns.file_path"
        );

        let params = std::iter::once(source_repo_id)
            .chain(changed_qnames.iter().copied())
            .chain(std::iter::once(source_repo_id))
            .chain(changed_qnames.iter().copied());

        let mut stmt = self.conn.prepare(&sql).map_err(db_err)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params), |r| r.get(0))
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
        max_edges: usize,
    ) -> Result<ImpactResult> {
        let policy = BudgetPolicy::default();
        let mut budgets = BudgetManager::new();
        let requested_depth = max_depth;
        let requested_nodes = max_nodes;
        let requested_edges = max_edges;
        let max_depth = budgets.resolve_limit(
            policy.graph_traversal.depth,
            "graph_traversal.max_depth",
            Some(max_depth as usize),
        ) as u32;
        let max_nodes = budgets.resolve_limit(
            policy.graph_traversal.nodes,
            "graph_traversal.max_nodes",
            Some(max_nodes),
        );
        let max_edges = budgets.resolve_limit(
            policy.graph_traversal.edges,
            "graph_traversal.max_edges",
            Some(max_edges),
        );
        if changed_paths.is_empty() {
            let mut result = ImpactResult {
                changed_nodes: vec![],
                impacted_nodes: vec![],
                impacted_files: vec![],
                relevant_edges: vec![],
                seed_budgets: vec![],
                traversal_budget: Some(atlas_core::model::TraversalBudgetMeta {
                    requested_depth,
                    accepted_depth: max_depth,
                    requested_node_budget: requested_nodes,
                    accepted_node_budget: max_nodes,
                    requested_edge_budget: requested_edges,
                    accepted_edge_budget: max_edges,
                    emitted_node_count: 0,
                    emitted_edge_count: 0,
                    omitted_edge_count: 0,
                    budget_hit: false,
                    suggested_narrower_query: None,
                }),
                budget: budgets.summary("graph_traversal.max_nodes", max_nodes, 0),
            };
            Self::sort_impact_result(&mut result);
            return Ok(result);
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
        let mut relevant_edges = if all_qns.is_empty() {
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

        let observed_nodes = changed_nodes.len() + impacted_nodes.len();
        budgets.record_usage(
            policy.graph_traversal.nodes,
            "graph_traversal.max_nodes",
            max_nodes,
            observed_nodes,
            observed_nodes >= max_nodes,
        );

        let original_edge_count = relevant_edges.len();
        if original_edge_count > max_edges {
            budgets.record_usage(
                policy.graph_traversal.edges,
                "graph_traversal.max_edges",
                max_edges,
                original_edge_count,
                true,
            );
            relevant_edges.truncate(max_edges);
        }

        let mut result = ImpactResult {
            changed_nodes,
            impacted_nodes,
            impacted_files,
            relevant_edges,
            seed_budgets: vec![],
            traversal_budget: Some(atlas_core::model::TraversalBudgetMeta {
                requested_depth,
                accepted_depth: max_depth,
                requested_node_budget: requested_nodes,
                accepted_node_budget: max_nodes,
                requested_edge_budget: requested_edges,
                accepted_edge_budget: max_edges,
                emitted_node_count: observed_nodes,
                emitted_edge_count: original_edge_count.min(max_edges),
                omitted_edge_count: original_edge_count.saturating_sub(max_edges),
                budget_hit: requested_depth != max_depth
                    || requested_nodes != max_nodes
                    || requested_edges != max_edges
                    || original_edge_count > max_edges,
                suggested_narrower_query: (original_edge_count > max_edges).then(|| {
                    format!(
                        "reduce changed-file seed set or traversal depth so edge count stays within {}",
                        max_edges
                    )
                }),
            }),
            budget: budgets.summary("graph_traversal.max_nodes", max_nodes, observed_nodes),
        };
        Self::sort_impact_result(&mut result);
        Ok(result)
    }
}
