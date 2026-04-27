use atlas_core::{AtlasError, BudgetManager, BudgetPolicy, ImpactResult, Node, Result};
use rusqlite::params;

use super::{
    Store,
    helpers::{
        canonicalize_graph_slice, canonicalize_repo_path, repeat_placeholders, row_to_edge,
        row_to_node,
    },
};

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

    /// Replace only the stored edges for `path`, leaving nodes and file
    /// metadata untouched.
    pub fn rewrite_file_edges(&mut self, path: &str, edges: &[atlas_core::Edge]) -> Result<()> {
        let normalized = canonicalize_graph_slice(path, &[], edges)?;
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        self.conn.execute_batch("BEGIN IMMEDIATE").map_err(db_err)?;
        self.conn
            .execute("DELETE FROM edges WHERE file_path = ?1", [&normalized.path])
            .map_err(db_err)?;
        for edge in &normalized.edges {
            let extra = serde_json::to_string(&edge.extra_json).map_err(AtlasError::Serde)?;
            self.conn
                .execute(
                    "INSERT INTO edges
                         (kind, source_qualified, target_qualified, file_path,
                          line, confidence, confidence_tier, extra_json)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                    params![
                        edge.kind.as_str(),
                        edge.source_qn,
                        edge.target_qn,
                        edge.file_path,
                        edge.line,
                        edge.confidence,
                        edge.confidence_tier,
                        extra,
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

    /// Returns a map of `file_path → stored_hash` for all indexed files.
    ///
    /// Used by the build command to skip re-parsing files whose content has not
    /// changed since the last indexed pass. The map keys are the canonical
    /// `files.path` identities stored in the graph DB, so file-hash reuse and
    /// later historical snapshot keys operate on the same path spelling.
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
    /// More targeted than `Store::find_dependents()` which operates on file
    /// paths:
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
