pub mod context;
pub mod query_parser;

pub use context::{
    ContextEngine, ResolvedTarget, build_context, normalize_qn_kind_tokens, resolve_target,
};

use std::collections::{HashMap, HashSet};

use atlas_core::model::{
    ChangedSymbolSummary, ImpactResult, ReviewContext, ReviewImpactOverview, RiskSummary,
};
use atlas_core::{Edge, EdgeKind, Node, NodeKind, ScoredImpactNode};
use atlas_impact::analyze as analyze_impact;

const LARGE_FUNCTION_LINE_THRESHOLD: u32 = 80;
const MAX_RELATED_NODES_PER_BUCKET: usize = 10;

/// Assemble a [`ReviewContext`] from a completed impact traversal.
///
/// `changed_file_paths` must be the repo-relative paths of the seed files.
pub fn assemble_review_context(
    impact: &ImpactResult,
    changed_file_paths: &[String],
    max_depth: u32,
    max_nodes: usize,
) -> ReviewContext {
    let advanced = analyze_impact(impact.clone());
    let changed_file_set: HashSet<&str> = changed_file_paths.iter().map(String::as_str).collect();

    // Public-API symbol kinds.
    const PUBLIC_KINDS: &[NodeKind] = &[
        NodeKind::Function,
        NodeKind::Method,
        NodeKind::Class,
        NodeKind::Trait,
        NodeKind::Struct,
        NodeKind::Enum,
        NodeKind::Interface,
    ];

    let public_api_changes = advanced
        .base
        .changed_nodes
        .iter()
        .filter(|n| {
            PUBLIC_KINDS.contains(&n.kind)
                && n.modifiers
                    .as_deref()
                    .map(|m| m.contains("pub"))
                    .unwrap_or(false)
        })
        .count();

    let score_by_qn: HashMap<&str, &ScoredImpactNode> = advanced
        .scored_nodes
        .iter()
        .map(|scored| (scored.node.qualified_name.as_str(), scored))
        .collect();
    let node_by_qn: HashMap<&str, &Node> = advanced
        .base
        .changed_nodes
        .iter()
        .chain(advanced.base.impacted_nodes.iter())
        .map(|node| (node.qualified_name.as_str(), node))
        .collect();

    let mut changed_symbols: Vec<Node> = advanced.base.changed_nodes.clone();
    changed_symbols.sort_by(|left, right| compare_nodes_by_relevance(left, right, &score_by_qn));

    let changed_symbol_summaries: Vec<ChangedSymbolSummary> = changed_symbols
        .iter()
        .map(|node| ChangedSymbolSummary {
            node: node.clone(),
            callers: collect_related_nodes(
                node,
                &advanced.base.relevant_edges,
                &node_by_qn,
                &score_by_qn,
                RelatedKind::Callers,
            ),
            callees: collect_related_nodes(
                node,
                &advanced.base.relevant_edges,
                &node_by_qn,
                &score_by_qn,
                RelatedKind::Callees,
            ),
            importers: collect_related_nodes(
                node,
                &advanced.base.relevant_edges,
                &node_by_qn,
                &score_by_qn,
                RelatedKind::Importers,
            ),
            tests: collect_related_nodes(
                node,
                &advanced.base.relevant_edges,
                &node_by_qn,
                &score_by_qn,
                RelatedKind::Tests,
            ),
        })
        .collect();

    let cross_module_impact = advanced
        .base
        .impacted_nodes
        .iter()
        .any(|node| !changed_file_set.contains(node.file_path.as_str()));
    let cross_package_impact = advanced
        .boundary_violations
        .iter()
        .any(|violation| violation.kind == atlas_core::BoundaryKind::CrossPackage);

    let large_function_count = advanced
        .base
        .changed_nodes
        .iter()
        .filter(|node| {
            matches!(node.kind, NodeKind::Function | NodeKind::Method)
                && node.line_end >= node.line_start
                && node.line_end - node.line_start + 1 >= LARGE_FUNCTION_LINE_THRESHOLD
        })
        .count();

    let affected_test_count = advanced.test_impact.affected_tests.len();
    let uncovered_changed_symbol_count = advanced.test_impact.uncovered_changed_nodes.len();
    let test_adjacent = affected_test_count > 0
        || advanced
            .base
            .changed_nodes
            .iter()
            .chain(advanced.base.impacted_nodes.iter())
            .any(|node| node.is_test || node.kind == NodeKind::Test);

    let risk_summary = RiskSummary {
        changed_symbol_count: changed_symbols.len(),
        public_api_changes,
        test_adjacent,
        affected_test_count,
        uncovered_changed_symbol_count,
        large_function_touched: large_function_count > 0,
        large_function_count,
        cross_module_impact,
        cross_package_impact,
    };

    let mut impacted_neighbors: Vec<_> = advanced
        .base
        .impacted_nodes
        .iter()
        .filter(|n| !changed_file_set.contains(n.file_path.as_str()))
        .cloned()
        .collect();
    impacted_neighbors.sort_by(|left, right| compare_nodes_by_relevance(left, right, &score_by_qn));

    let changed_qns: HashSet<&str> = advanced
        .base
        .changed_nodes
        .iter()
        .map(|n| n.qualified_name.as_str())
        .collect();
    let neighbor_qns: HashSet<&str> = impacted_neighbors
        .iter()
        .map(|n| n.qualified_name.as_str())
        .collect();
    let mut critical_edges: Vec<_> = advanced
        .base
        .relevant_edges
        .iter()
        .filter(|e| {
            (changed_qns.contains(e.source_qn.as_str())
                && neighbor_qns.contains(e.target_qn.as_str()))
                || (neighbor_qns.contains(e.source_qn.as_str())
                    && changed_qns.contains(e.target_qn.as_str()))
        })
        .cloned()
        .collect();
    critical_edges.sort_by(|left, right| compare_edges_by_relevance(left, right, &score_by_qn));

    let impact_overview = ReviewImpactOverview {
        max_depth,
        max_nodes,
        impacted_node_count: advanced.base.impacted_nodes.len(),
        impacted_file_count: advanced.base.impacted_files.len(),
        relevant_edge_count: advanced.base.relevant_edges.len(),
        reached_node_limit: advanced.base.impacted_nodes.len() >= max_nodes && max_nodes > 0,
    };

    ReviewContext {
        changed_files: changed_file_paths.to_vec(),
        changed_symbols,
        changed_symbol_summaries,
        impacted_neighbors,
        critical_edges,
        impact_overview,
        risk_summary,
    }
}

#[derive(Clone, Copy)]
enum RelatedKind {
    Callers,
    Callees,
    Importers,
    Tests,
}

fn collect_related_nodes(
    changed_node: &Node,
    edges: &[Edge],
    node_by_qn: &HashMap<&str, &Node>,
    score_by_qn: &HashMap<&str, &ScoredImpactNode>,
    related_kind: RelatedKind,
) -> Vec<Node> {
    let mut related = Vec::new();
    let mut seen = HashSet::new();

    for edge in edges {
        let maybe_qn = match related_kind {
            RelatedKind::Callers
                if edge.kind == EdgeKind::Calls
                    && edge.target_qn == changed_node.qualified_name =>
            {
                Some(edge.source_qn.as_str())
            }
            RelatedKind::Callees
                if edge.kind == EdgeKind::Calls
                    && edge.source_qn == changed_node.qualified_name =>
            {
                Some(edge.target_qn.as_str())
            }
            RelatedKind::Importers
                if edge.kind == EdgeKind::Imports
                    && edge.target_qn == changed_node.qualified_name =>
            {
                Some(edge.source_qn.as_str())
            }
            RelatedKind::Tests if matches!(edge.kind, EdgeKind::Tests | EdgeKind::TestedBy) => {
                if edge.source_qn == changed_node.qualified_name {
                    Some(edge.target_qn.as_str())
                } else if edge.target_qn == changed_node.qualified_name {
                    Some(edge.source_qn.as_str())
                } else {
                    None
                }
            }
            _ => None,
        };

        let Some(qn) = maybe_qn else {
            continue;
        };

        let Some(node) = node_by_qn.get(qn).copied() else {
            continue;
        };

        if matches!(related_kind, RelatedKind::Tests)
            && !(node.is_test || node.kind == NodeKind::Test)
        {
            continue;
        }

        if seen.insert(node.qualified_name.as_str()) {
            related.push(node.clone());
        }
    }

    related.sort_by(|left, right| compare_nodes_by_relevance(left, right, score_by_qn));
    related.truncate(MAX_RELATED_NODES_PER_BUCKET);
    related
}

fn compare_nodes_by_relevance(
    left: &Node,
    right: &Node,
    score_by_qn: &HashMap<&str, &ScoredImpactNode>,
) -> std::cmp::Ordering {
    let left_score = score_by_qn
        .get(left.qualified_name.as_str())
        .map(|scored| scored.impact_score)
        .unwrap_or(0.0);
    let right_score = score_by_qn
        .get(right.qualified_name.as_str())
        .map(|scored| scored.impact_score)
        .unwrap_or(0.0);

    right_score
        .partial_cmp(&left_score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| left.qualified_name.cmp(&right.qualified_name))
}

fn compare_edges_by_relevance(
    left: &Edge,
    right: &Edge,
    score_by_qn: &HashMap<&str, &ScoredImpactNode>,
) -> std::cmp::Ordering {
    let left_score = edge_relevance(left, score_by_qn);
    let right_score = edge_relevance(right, score_by_qn);

    right_score
        .partial_cmp(&left_score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| left.source_qn.cmp(&right.source_qn))
        .then_with(|| left.target_qn.cmp(&right.target_qn))
}

fn edge_relevance(edge: &Edge, score_by_qn: &HashMap<&str, &ScoredImpactNode>) -> f64 {
    score_by_qn
        .get(edge.source_qn.as_str())
        .map(|scored| scored.impact_score)
        .unwrap_or(0.0)
        + score_by_qn
            .get(edge.target_qn.as_str())
            .map(|scored| scored.impact_score)
            .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::model::{Edge, ImpactResult, Node};
    use atlas_core::{EdgeKind, NodeId, NodeKind};

    fn make_node(qn: &str, file: &str, kind: NodeKind) -> Node {
        Node {
            id: NodeId::UNSET,
            kind,
            name: qn.to_string(),
            qualified_name: qn.to_string(),
            file_path: file.to_string(),
            line_start: 1,
            line_end: 10,
            language: "rust".to_string(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: String::new(),
            extra_json: serde_json::Value::Null,
        }
    }

    fn make_edge(src: &str, tgt: &str, kind: EdgeKind) -> Edge {
        Edge {
            id: 0,
            kind,
            source_qn: src.to_string(),
            target_qn: tgt.to_string(),
            file_path: "src/a.rs".to_string(),
            line: None,
            confidence: 1.0,
            confidence_tier: None,
            extra_json: serde_json::Value::Null,
        }
    }

    #[test]
    fn empty_impact_produces_empty_context() {
        let impact = ImpactResult {
            changed_nodes: vec![],
            impacted_nodes: vec![],
            impacted_files: vec![],
            relevant_edges: vec![],
        };
        let ctx = assemble_review_context(&impact, &[], 3, 200);
        assert_eq!(ctx.risk_summary.changed_symbol_count, 0);
        assert!(!ctx.risk_summary.cross_module_impact);
        assert!(!ctx.risk_summary.cross_package_impact);
        assert!(!ctx.risk_summary.large_function_touched);
        assert!(!ctx.risk_summary.test_adjacent);
        assert_eq!(ctx.changed_symbol_summaries.len(), 0);
        assert_eq!(ctx.impact_overview.max_depth, 3);
    }

    #[test]
    fn cross_module_detected() {
        let changed = make_node("src/a.rs::fn::foo", "src/a.rs", NodeKind::Function);
        let impacted = make_node("src/b.rs::fn::bar", "src/b.rs", NodeKind::Function);
        let edge = make_edge("src/a.rs::fn::foo", "src/b.rs::fn::bar", EdgeKind::Calls);
        let impact = ImpactResult {
            changed_nodes: vec![changed],
            impacted_nodes: vec![impacted],
            impacted_files: vec!["src/b.rs".to_string()],
            relevant_edges: vec![edge],
        };
        let ctx = assemble_review_context(&impact, &["src/a.rs".to_string()], 3, 200);
        assert!(ctx.risk_summary.cross_module_impact);
        assert_eq!(ctx.critical_edges.len(), 1);
        assert_eq!(ctx.impacted_neighbors.len(), 1);
    }

    #[test]
    fn test_adjacent_flagged() {
        let mut node = make_node("src/a.rs::fn::test_foo", "src/a.rs", NodeKind::Function);
        node.is_test = true;
        let impact = ImpactResult {
            changed_nodes: vec![node],
            impacted_nodes: vec![],
            impacted_files: vec![],
            relevant_edges: vec![],
        };
        let ctx = assemble_review_context(&impact, &["src/a.rs".to_string()], 3, 200);
        assert!(ctx.risk_summary.test_adjacent);
        assert_eq!(ctx.risk_summary.affected_test_count, 1);
    }

    #[test]
    fn changed_symbol_summaries_collect_related_nodes() {
        let changed = make_node(
            "pkg_a/src/a.rs::fn::foo",
            "pkg_a/src/a.rs",
            NodeKind::Function,
        );
        let caller = make_node(
            "pkg_a/src/b.rs::fn::caller",
            "pkg_a/src/b.rs",
            NodeKind::Function,
        );
        let callee = make_node(
            "pkg_a/src/c.rs::fn::callee",
            "pkg_a/src/c.rs",
            NodeKind::Function,
        );
        let importer = make_node(
            "pkg_a/src/mod.rs::module",
            "pkg_a/src/mod.rs",
            NodeKind::Module,
        );
        let mut test = make_node(
            "pkg_a/tests/foo.rs::fn::test_foo",
            "pkg_a/tests/foo.rs",
            NodeKind::Test,
        );
        test.is_test = true;

        let impact = ImpactResult {
            changed_nodes: vec![changed.clone()],
            impacted_nodes: vec![
                caller.clone(),
                callee.clone(),
                importer.clone(),
                test.clone(),
            ],
            impacted_files: vec![
                "pkg_a/src/b.rs".to_string(),
                "pkg_a/src/c.rs".to_string(),
                "pkg_a/src/mod.rs".to_string(),
                "pkg_a/tests/foo.rs".to_string(),
            ],
            relevant_edges: vec![
                make_edge(
                    &caller.qualified_name,
                    &changed.qualified_name,
                    EdgeKind::Calls,
                ),
                make_edge(
                    &changed.qualified_name,
                    &callee.qualified_name,
                    EdgeKind::Calls,
                ),
                make_edge(
                    &importer.qualified_name,
                    &changed.qualified_name,
                    EdgeKind::Imports,
                ),
                make_edge(
                    &changed.qualified_name,
                    &test.qualified_name,
                    EdgeKind::Tests,
                ),
            ],
        };

        let ctx = assemble_review_context(&impact, &["pkg_a/src/a.rs".to_string()], 4, 25);
        let summary = &ctx.changed_symbol_summaries[0];

        assert_eq!(summary.callers[0].qualified_name, caller.qualified_name);
        assert_eq!(summary.callees[0].qualified_name, callee.qualified_name);
        assert_eq!(summary.importers[0].qualified_name, importer.qualified_name);
        assert_eq!(summary.tests[0].qualified_name, test.qualified_name);
        assert!(ctx.risk_summary.test_adjacent);
        assert_eq!(ctx.risk_summary.affected_test_count, 1);
    }

    #[test]
    fn risk_summary_flags_large_function_and_cross_package_impact() {
        let mut changed = make_node(
            "pkg_a/src/a.rs::fn::foo",
            "pkg_a/src/a.rs",
            NodeKind::Function,
        );
        changed.line_end = changed.line_start + LARGE_FUNCTION_LINE_THRESHOLD;
        changed.modifiers = Some("pub".to_string());
        let impacted = make_node(
            "pkg_b/src/b.rs::fn::bar",
            "pkg_b/src/b.rs",
            NodeKind::Function,
        );

        let impact = ImpactResult {
            changed_nodes: vec![changed.clone()],
            impacted_nodes: vec![impacted.clone()],
            impacted_files: vec!["pkg_b/src/b.rs".to_string()],
            relevant_edges: vec![make_edge(
                &changed.qualified_name,
                &impacted.qualified_name,
                EdgeKind::Calls,
            )],
        };

        let ctx = assemble_review_context(&impact, &["pkg_a/src/a.rs".to_string()], 2, 1);

        assert!(ctx.risk_summary.large_function_touched);
        assert_eq!(ctx.risk_summary.large_function_count, 1);
        assert!(ctx.risk_summary.cross_package_impact);
        assert_eq!(ctx.risk_summary.public_api_changes, 1);
        assert!(ctx.impact_overview.reached_node_limit);
    }
}
