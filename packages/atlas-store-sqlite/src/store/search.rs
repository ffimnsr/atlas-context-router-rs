use atlas_core::{
    AtlasError, BudgetManager, BudgetPolicy, ImpactResult, Node, Result, ScoredNode, SearchQuery,
};

use super::{
    Store,
    helpers::{fts5_escape, repeat_placeholders, row_to_edge, row_to_node},
};

// Search and traversal queries live here. Retrieval chunk persistence and
// vector similarity live in retrieval.rs.

impl Store {
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<ScoredNode>> {
        let text_empty = query.text.trim().is_empty();
        let has_regex = query.regex_pattern.is_some();

        // Validate regex pattern early for a clear error message before hitting SQLite.
        if let Some(pat) = query.regex_pattern.as_deref() {
            regex::Regex::new(pat)
                .map_err(|e| AtlasError::Other(format!("invalid regex pattern: {e}")))?;
        }

        // Nothing to search on: no text and no regex.
        if text_empty && !has_regex {
            return Err(AtlasError::Other(
                "search requires a non-empty text or regex pattern".to_string(),
            ));
        }

        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());

        // Build a LIKE pattern from the subpath (escape SQLite LIKE wildcards).
        let subpath_like = query.subpath.as_deref().map(|sp| {
            let escaped = sp
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_");
            format!("{escaped}%")
        });

        let results: Vec<ScoredNode> = if text_empty {
            // Structural scan path: no FTS, apply field filters directly on nodes table.
            let mut filters: Vec<String> = Vec::new();
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(kind) = &query.kind {
                filters.push("n.kind = ?".to_string());
                params.push(Box::new(kind.clone()));
            } else if !query.include_files {
                filters.push("n.kind != 'file'".to_string());
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

            // Regex is always Some in this branch; pass pattern as SQL params so the
            // static atlas_regexp(pat, val) UDF can use its thread-local cache.
            let pat = query.regex_pattern.as_deref().unwrap_or("").to_owned();
            filters
                .push("(atlas_regexp(?, n.name) OR atlas_regexp(?, n.qualified_name))".to_string());
            params.push(Box::new(pat.clone()));
            params.push(Box::new(pat));

            let where_clause = if filters.is_empty() {
                "1=1".to_string()
            } else {
                filters.join(" AND ")
            };

            let fetch_cap = query.limit as i64;
            params.push(Box::new(fetch_cap));

            let sql = format!(
                "SELECT n.id, n.kind, n.name, n.qualified_name, n.file_path,
                        n.line_start, n.line_end, n.language, n.parent_name,
                        n.params, n.return_type, n.modifiers, n.is_test,
                        n.file_hash, n.extra_json
                 FROM   nodes n
                 WHERE  {where_clause}
                 ORDER  BY n.rowid
                 LIMIT  ?"
            );

            let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(|b| b.as_ref()).collect();
            let mut stmt = self.conn.prepare_cached(&sql).map_err(db_err)?;
            stmt.query_map(params_ref.as_slice(), |row| {
                let node = row_to_node(row)?;
                Ok(ScoredNode { node, score: 1.0 })
            })
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect()
        } else {
            // FTS5 path: full-text search with BM25 ranking.
            let fts_query = fts5_escape(&query.text);

            let mut filters: Vec<String> = vec!["nodes_fts MATCH ?".to_string()];
            let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(fts_query)];

            if let Some(kind) = &query.kind {
                filters.push("n.kind = ?".to_string());
                params.push(Box::new(kind.clone()));
            } else if !query.include_files {
                filters.push("n.kind != 'file'".to_string());
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

            if has_regex {
                let pat = query.regex_pattern.as_deref().unwrap_or("").to_owned();
                filters.push(
                    "(atlas_regexp(?, n.name) OR atlas_regexp(?, n.qualified_name))".to_string(),
                );
                params.push(Box::new(pat.clone()));
                params.push(Box::new(pat));
            }

            let fetch_limit = query.limit as i64;
            params.push(Box::new(fetch_limit));

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

            let mut stmt = self.conn.prepare_cached(&sql).map_err(db_err)?;
            stmt.query_map(params_ref.as_slice(), |row| {
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
            .collect()
        };

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

    /// Bi-directional impact radius seeded from explicit qualified names rather
    /// than file paths.
    ///
    /// Identical traversal semantics to `impact_radius`, but the seed set is
    /// the provided `seed_qnames` instead of every node in a set of files.
    /// The seeds appear in `ImpactResult::changed_nodes`; all other reachable
    /// nodes appear in `ImpactResult::impacted_nodes`.
    pub fn traverse_from_qnames(
        &self,
        seed_qnames: &[&str],
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
        if seed_qnames.is_empty() {
            return Ok(ImpactResult {
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
            });
        }
        let db_err = |e: rusqlite::Error| AtlasError::Db(e.to_string());
        let ph = repeat_placeholders(seed_qnames.len());

        // Load seed nodes.
        let seed_sql = format!(
            "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                    language, parent_name, params, return_type, modifiers,
                    is_test, file_hash, extra_json
             FROM nodes WHERE qualified_name IN ({ph})"
        );
        let mut stmt = self.conn.prepare(&seed_sql).map_err(db_err)?;
        let params_seed: Vec<&dyn rusqlite::types::ToSql> = seed_qnames
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let changed_nodes: Vec<Node> = stmt
            .query_map(params_seed.as_slice(), row_to_node)
            .map_err(db_err)?
            .filter_map(|r| r.ok())
            .collect();

        // Recursive CTE: bidirectional traversal starting from seed QNs.
        let cte_sql = format!(
            "WITH RECURSIVE impact(qn, depth) AS (
               SELECT qualified_name, 0 FROM nodes WHERE qualified_name IN ({ph})
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

        let mut all_params: Vec<Box<dyn rusqlite::types::ToSql>> = seed_qnames
            .iter()
            .map(|s| Box::new(s.to_string()) as Box<dyn rusqlite::types::ToSql>)
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

        let seed_set: std::collections::HashSet<&str> = seed_qnames.iter().copied().collect();
        let impacted_qns: Vec<&str> = all_qns
            .iter()
            .filter(|qn| !seed_set.contains(qn.as_str()))
            .map(|s| s.as_str())
            .collect();

        let impacted_nodes = if impacted_qns.is_empty() {
            vec![]
        } else {
            let iph = repeat_placeholders(impacted_qns.len());
            let sql = format!(
                "SELECT id, kind, name, qualified_name, file_path, line_start, line_end,
                        language, parent_name, params, return_type, modifiers,
                        is_test, file_hash, extra_json
                 FROM nodes WHERE qualified_name IN ({iph})"
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

        let mut relevant_edges = if all_qns.is_empty() {
            vec![]
        } else {
            let eph = repeat_placeholders(all_qns.len());
            let sql = format!(
                "SELECT id, kind, source_qualified, target_qualified, file_path,
                        line, confidence, confidence_tier, extra_json
                 FROM edges
                 WHERE source_qualified IN ({eph}) AND target_qualified IN ({eph})"
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

        Ok(ImpactResult {
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
                        "reduce traversal scope or depth so edge count stays within {}",
                        max_edges
                    )
                }),
            }),
            budget: budgets.summary("graph_traversal.max_nodes", max_nodes, observed_nodes),
        })
    }
}
