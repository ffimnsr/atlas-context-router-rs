use atlas_core::model::{ImpactResult, ReviewContext, RiskSummary};
use atlas_core::NodeKind;

/// Assemble a [`ReviewContext`] from a completed impact traversal.
///
/// `changed_file_paths` must be the repo-relative paths of the seed files.
pub fn assemble_review_context(
    impact: &ImpactResult,
    changed_file_paths: &[String],
) -> ReviewContext {
    let changed_file_set: std::collections::HashSet<&str> =
        changed_file_paths.iter().map(String::as_str).collect();

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

    let public_api_changes = impact
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

    let test_adjacent = impact
        .changed_nodes
        .iter()
        .chain(impact.impacted_nodes.iter())
        .any(|n| n.is_test || n.kind == NodeKind::Test);

    let cross_module_impact = impact
        .impacted_nodes
        .iter()
        .any(|n| !changed_file_set.contains(n.file_path.as_str()));

    let risk_summary = RiskSummary {
        changed_symbol_count: impact.changed_nodes.len(),
        public_api_changes,
        test_adjacent,
        cross_module_impact,
    };

    // Impacted neighbors: nodes from outside the changed-file set, capped at 50.
    let impacted_neighbors: Vec<_> = impact
        .impacted_nodes
        .iter()
        .filter(|n| !changed_file_set.contains(n.file_path.as_str()))
        .take(50)
        .cloned()
        .collect();

    // Critical edges: those crossing the boundary between changed and impacted.
    let changed_qns: std::collections::HashSet<&str> =
        impact.changed_nodes.iter().map(|n| n.qualified_name.as_str()).collect();
    let neighbor_qns: std::collections::HashSet<&str> =
        impacted_neighbors.iter().map(|n| n.qualified_name.as_str()).collect();
    let critical_edges: Vec<_> = impact
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

    ReviewContext {
        changed_files: changed_file_paths.to_vec(),
        changed_symbols: impact.changed_nodes.clone(),
        impacted_neighbors,
        critical_edges,
        risk_summary,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::model::{Edge, ImpactResult, Node};
    use atlas_core::{EdgeKind, NodeKind};

    fn make_node(qn: &str, file: &str, kind: NodeKind) -> Node {
        Node {
            id: 0,
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
        let ctx = assemble_review_context(&impact, &[]);
        assert_eq!(ctx.risk_summary.changed_symbol_count, 0);
        assert!(!ctx.risk_summary.cross_module_impact);
        assert!(!ctx.risk_summary.test_adjacent);
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
        let ctx = assemble_review_context(&impact, &["src/a.rs".to_string()]);
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
        let ctx = assemble_review_context(&impact, &["src/a.rs".to_string()]);
        assert!(ctx.risk_summary.test_adjacent);
    }
}
