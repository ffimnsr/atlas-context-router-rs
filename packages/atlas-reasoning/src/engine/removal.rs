use std::collections::{HashMap, HashSet, VecDeque};

use atlas_core::{
    BudgetManager, BudgetPolicy, ImpactedNode, Node, NodeKind, ReasoningEvidence, ReasoningWarning,
    RemovalImpactResult, Result,
};

use super::{
    ReasoningEngine,
    helpers::{BfsImpact, EDGE_QUERY_LIMIT, classify_impact, normalize_qn_kind_tokens},
};

impl<'s> ReasoningEngine<'s> {
    /// Analyse the impact of removing `seed_qnames` from the codebase.
    ///
    /// Traverses the graph bidirectionally up to `max_depth` hops (default 3)
    /// and separates impacted nodes by confidence: `Definite` (direct call /
    /// import / test), `Probable` (same-file/package inferred), `Weak` (textual
    /// reference only).
    pub fn analyze_removal(
        &self,
        seed_qnames: &[&str],
        max_depth: Option<u32>,
        max_nodes: Option<usize>,
    ) -> Result<RemovalImpactResult> {
        let policy = BudgetPolicy::default();
        let mut budgets = BudgetManager::new();
        let normalized_seed_qnames: Vec<String> = seed_qnames
            .iter()
            .map(|qname| normalize_qn_kind_tokens(qname))
            .collect();
        let normalized_seed_refs: Vec<&str> =
            normalized_seed_qnames.iter().map(String::as_str).collect();
        let depth = budgets.resolve_limit(
            policy.graph_traversal.depth,
            "graph_traversal.max_depth",
            max_depth.map(|depth| depth as usize),
        ) as u32;
        let cap = budgets.resolve_limit(
            policy.graph_traversal.nodes,
            "graph_traversal.max_nodes",
            max_nodes,
        );

        let seed_nodes = self.load_nodes(&normalized_seed_refs)?;
        if seed_nodes.is_empty() {
            return Ok(RemovalImpactResult {
                seed: vec![],
                impacted_symbols: vec![],
                impacted_files: vec![],
                impacted_tests: vec![],
                relevant_edges: vec![],
                evidence_nodes: vec![],
                warnings: vec![ReasoningWarning {
                    message: format!(
                        "none of {} seed qualified names resolved to graph nodes",
                        normalized_seed_refs.len()
                    ),
                    confidence: atlas_core::ConfidenceTier::High,
                    error_code: Some("seed_not_found".to_owned()),
                    suggestions: vec![
                        "run `atlas build` to populate the graph".to_owned(),
                        "verify the qualified name with `atlas query`".to_owned(),
                    ],
                }],
                evidence: vec![],
                uncertainty_flags: vec![
                    "seed qualified names not found in graph — run `atlas build` first".to_owned(),
                ],
                budget: budgets.summary("graph_traversal.max_nodes", cap, 0),
            });
        }

        let (impacted, relevant_edges) = self.bfs_impact(&normalized_seed_refs, depth, cap)?;

        let seed_set: HashSet<&str> = normalized_seed_refs.iter().copied().collect();

        let impacted_symbols: Vec<ImpactedNode> = impacted
            .into_iter()
            .filter(|(node, _, _)| !seed_set.contains(node.qualified_name.as_str()))
            .map(|(node, depth_val, edge_kind)| ImpactedNode {
                impact_class: classify_impact(&node, depth_val, edge_kind),
                node,
                depth: depth_val,
                via_edge_kind: edge_kind,
            })
            .collect();

        let impacted_tests: Vec<Node> = impacted_symbols
            .iter()
            .filter(|im| im.node.is_test || im.node.kind == NodeKind::Test)
            .map(|im| im.node.clone())
            .collect();

        let mut impacted_files: Vec<String> = impacted_symbols
            .iter()
            .map(|im| im.node.file_path.clone())
            .collect();
        impacted_files.sort();
        impacted_files.dedup();

        let evidence = vec![
            ReasoningEvidence {
                key: "seed_count".to_owned(),
                value: seed_nodes.len().to_string(),
            },
            ReasoningEvidence {
                key: "impacted_symbol_count".to_owned(),
                value: impacted_symbols.len().to_string(),
            },
            ReasoningEvidence {
                key: "max_depth".to_owned(),
                value: depth.to_string(),
            },
        ];

        let observed_nodes = seed_nodes.len() + impacted_symbols.len();
        budgets.record_usage(
            policy.graph_traversal.nodes,
            "graph_traversal.max_nodes",
            cap,
            observed_nodes,
            observed_nodes >= cap,
        );

        Ok(RemovalImpactResult {
            seed: seed_nodes,
            impacted_symbols,
            impacted_files,
            impacted_tests,
            relevant_edges,
            evidence_nodes: vec![],
            warnings: vec![],
            evidence,
            uncertainty_flags: vec![],
            budget: budgets.summary("graph_traversal.max_nodes", cap, observed_nodes),
        })
    }

    /// Load nodes for the given qualified names. Skips any QN that does not
    /// resolve to a node (caller sees them missing from `seed`).
    fn load_nodes(&self, qnames: &[&str]) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();
        for qn in qnames {
            let normalized_qn = normalize_qn_kind_tokens(qn);
            if let Some(node) = self.store.node_by_qname(&normalized_qn)? {
                nodes.push(node);
            }
        }
        Ok(nodes)
    }

    /// Breadth-first traversal seeded from `seed_qnames`, bidirectional,
    /// up to `max_depth` hops, capped at `max_nodes` total.
    ///
    /// Returns `(reachable_nodes_with_depth_and_edge_kind, relevant_edges)`.
    fn bfs_impact(
        &self,
        seed_qnames: &[&str],
        max_depth: u32,
        max_nodes: usize,
    ) -> Result<BfsImpact> {
        let mut visited: HashMap<String, (u32, Option<atlas_core::EdgeKind>)> = HashMap::new();
        for qn in seed_qnames {
            visited.insert((*qn).to_owned(), (0, None));
        }

        let mut queue: VecDeque<(String, u32)> =
            seed_qnames.iter().map(|qn| ((*qn).to_owned(), 0)).collect();
        let mut all_edges: Vec<atlas_core::Edge> = Vec::new();

        while let Some((qname, depth)) = queue.pop_front() {
            if depth >= max_depth || visited.len() >= max_nodes {
                break;
            }

            let inbound = self.store.inbound_edges(&qname, EDGE_QUERY_LIMIT)?;
            let outbound = self.store.outbound_edges(&qname, EDGE_QUERY_LIMIT)?;

            for (neighbor, edge) in inbound.iter().chain(outbound.iter()) {
                let neighbor_qname = &neighbor.qualified_name;
                if !visited.contains_key(neighbor_qname) {
                    visited.insert(neighbor_qname.clone(), (depth + 1, Some(edge.kind)));
                    queue.push_back((neighbor_qname.clone(), depth + 1));
                }
                all_edges.push(edge.clone());
            }
        }

        all_edges.sort_by_key(|edge| edge.id);
        all_edges.dedup_by_key(|edge| edge.id);

        let mut results = Vec::new();
        for (qname, (depth, edge_kind)) in &visited {
            if let Some(node) = self.store.node_by_qname(qname)? {
                results.push((node, *depth, *edge_kind));
            }
        }
        results.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then(a.0.qualified_name.cmp(&b.0.qualified_name))
        });

        Ok((results, all_edges))
    }
}
