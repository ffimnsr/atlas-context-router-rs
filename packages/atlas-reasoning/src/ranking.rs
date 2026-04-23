use atlas_core::{
    ConfidenceTier, DeadCodeCandidate, DependencyRemovalResult, Edge, ImpactClass, ImpactedNode,
    Node, RefactorSafetyResult, RemovalImpactResult,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisRankingPrimitives {
    pub definite_impact_priority: u8,
    pub probable_impact_priority: u8,
    pub weak_impact_priority: u8,
    pub high_confidence_priority: u8,
    pub medium_confidence_priority: u8,
    pub low_confidence_priority: u8,
}

impl Default for AnalysisRankingPrimitives {
    fn default() -> Self {
        Self {
            definite_impact_priority: 3,
            probable_impact_priority: 2,
            weak_impact_priority: 1,
            high_confidence_priority: 3,
            medium_confidence_priority: 2,
            low_confidence_priority: 1,
        }
    }
}

impl AnalysisRankingPrimitives {
    pub fn impact_priority(&self, class: ImpactClass) -> u8 {
        match class {
            ImpactClass::Definite => self.definite_impact_priority,
            ImpactClass::Probable => self.probable_impact_priority,
            ImpactClass::Weak => self.weak_impact_priority,
        }
    }

    pub fn confidence_priority(&self, tier: ConfidenceTier) -> u8 {
        match tier {
            ConfidenceTier::High => self.high_confidence_priority,
            ConfidenceTier::Medium => self.medium_confidence_priority,
            ConfidenceTier::Low => self.low_confidence_priority,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisTrimmingPrimitives {
    pub removal_symbol_preview_limit: usize,
    pub removal_primary_preview_limit: usize,
    pub removal_containment_preview_limit: usize,
    pub dead_code_candidate_preview_limit: usize,
    pub dependency_blocker_preview_limit: usize,
}

impl Default for AnalysisTrimmingPrimitives {
    fn default() -> Self {
        Self {
            removal_symbol_preview_limit: 50,
            removal_primary_preview_limit: 20,
            removal_containment_preview_limit: 10,
            dead_code_candidate_preview_limit: 50,
            dependency_blocker_preview_limit: 20,
        }
    }
}

pub fn sort_removal_result(
    result: &mut RemovalImpactResult,
    primitives: &AnalysisRankingPrimitives,
) {
    result.seed.sort_by(compare_nodes);
    result
        .impacted_symbols
        .sort_by(|left, right| compare_impacted_nodes(primitives, left, right));
    result.impacted_files.sort();
    result.impacted_files.dedup();
    result.impacted_tests.sort_by(compare_nodes);
    result.relevant_edges.sort_by(compare_edges);
    result.evidence_nodes.sort_by(compare_nodes);
    result.warnings.sort_by(|left, right| {
        right
            .confidence
            .cmp(&left.confidence)
            .then_with(|| left.message.cmp(&right.message))
    });
    result.evidence.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.value.cmp(&right.value))
    });
    result.uncertainty_flags.sort();
}

pub fn sort_dead_code_candidates(
    candidates: &mut [DeadCodeCandidate],
    primitives: &AnalysisRankingPrimitives,
) {
    candidates.sort_by(|left, right| {
        compare_confidence(primitives, left.certainty, right.certainty)
            .then_with(|| compare_nodes(&left.node, &right.node))
    });
}

pub fn sort_refactor_safety_result(result: &mut RefactorSafetyResult) {
    result.evidence.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.value.cmp(&right.value))
    });
}

pub fn sort_dependency_result(
    result: &mut DependencyRemovalResult,
    _primitives: &AnalysisRankingPrimitives,
) {
    result.blocking_references.sort_by(compare_nodes);
    result.evidence_edges.sort_by(compare_edges);
    result.suggested_cleanups.sort();
    result.uncertainty_flags.sort();
    result.evidence.sort_by(|left, right| {
        left.key
            .cmp(&right.key)
            .then_with(|| left.value.cmp(&right.value))
    });
}

fn compare_impacted_nodes(
    primitives: &AnalysisRankingPrimitives,
    left: &ImpactedNode,
    right: &ImpactedNode,
) -> std::cmp::Ordering {
    compare_impact_class(primitives, left.impact_class, right.impact_class)
        .then_with(|| left.depth.cmp(&right.depth))
        .then_with(|| compare_nodes(&left.node, &right.node))
}

fn compare_impact_class(
    primitives: &AnalysisRankingPrimitives,
    left: ImpactClass,
    right: ImpactClass,
) -> std::cmp::Ordering {
    primitives
        .impact_priority(right)
        .cmp(&primitives.impact_priority(left))
}

fn compare_confidence(
    primitives: &AnalysisRankingPrimitives,
    left: ConfidenceTier,
    right: ConfidenceTier,
) -> std::cmp::Ordering {
    primitives
        .confidence_priority(right)
        .cmp(&primitives.confidence_priority(left))
}

fn compare_nodes(left: &Node, right: &Node) -> std::cmp::Ordering {
    left.file_path
        .cmp(&right.file_path)
        .then_with(|| left.line_start.cmp(&right.line_start))
        .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
        .then_with(|| left.qualified_name.cmp(&right.qualified_name))
}

fn compare_edges(left: &Edge, right: &Edge) -> std::cmp::Ordering {
    left.file_path
        .cmp(&right.file_path)
        .then_with(|| left.line.cmp(&right.line))
        .then_with(|| left.kind.as_str().cmp(right.kind.as_str()))
        .then_with(|| left.source_qn.cmp(&right.source_qn))
        .then_with(|| left.target_qn.cmp(&right.target_qn))
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::BudgetReport;
    use atlas_core::{
        EdgeKind, ImpactedNode, Node, NodeId, NodeKind, ReasoningEvidence, ReasoningWarning,
    };

    fn node(qn: &str, file: &str, line_start: u32) -> Node {
        Node {
            id: NodeId::UNSET,
            kind: NodeKind::Function,
            name: qn.rsplit("::").next().unwrap_or(qn).to_owned(),
            qualified_name: qn.to_owned(),
            file_path: file.to_owned(),
            line_start,
            line_end: line_start,
            language: "rust".to_owned(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: String::new(),
            extra_json: serde_json::Value::Null,
        }
    }

    fn impacted(qn: &str, file: &str, class: ImpactClass, depth: u32) -> ImpactedNode {
        ImpactedNode {
            node: node(qn, file, 1),
            depth,
            impact_class: class,
            via_edge_kind: Some(EdgeKind::Calls),
        }
    }

    #[test]
    fn removal_sort_prefers_stronger_impact_then_depth_then_qname() {
        let mut result = RemovalImpactResult {
            seed: vec![node("src/lib.rs::fn::seed", "src/lib.rs", 10)],
            impacted_symbols: vec![
                impacted("src/z.rs::fn::weak", "src/z.rs", ImpactClass::Weak, 1),
                impacted(
                    "src/b.rs::fn::probable",
                    "src/b.rs",
                    ImpactClass::Probable,
                    1,
                ),
                impacted(
                    "src/c.rs::fn::definite_deep",
                    "src/c.rs",
                    ImpactClass::Definite,
                    2,
                ),
                impacted(
                    "src/a.rs::fn::definite_shallow",
                    "src/a.rs",
                    ImpactClass::Definite,
                    1,
                ),
            ],
            impacted_files: vec!["src/z.rs".to_owned(), "src/a.rs".to_owned()],
            impacted_tests: vec![],
            relevant_edges: vec![],
            evidence_nodes: vec![],
            warnings: vec![ReasoningWarning {
                message: "warn".to_owned(),
                confidence: ConfidenceTier::Low,
                error_code: None,
                suggestions: vec![],
            }],
            evidence: vec![
                ReasoningEvidence {
                    key: "b".to_owned(),
                    value: "2".to_owned(),
                },
                ReasoningEvidence {
                    key: "a".to_owned(),
                    value: "1".to_owned(),
                },
            ],
            uncertainty_flags: vec![],
            budget: BudgetReport::not_applicable(),
        };

        sort_removal_result(&mut result, &AnalysisRankingPrimitives::default());

        let ordered: Vec<_> = result
            .impacted_symbols
            .iter()
            .map(|item| item.node.qualified_name.as_str())
            .collect();
        assert_eq!(
            ordered,
            vec![
                "src/a.rs::fn::definite_shallow",
                "src/c.rs::fn::definite_deep",
                "src/b.rs::fn::probable",
                "src/z.rs::fn::weak",
            ]
        );
        assert_eq!(result.impacted_files, vec!["src/a.rs", "src/z.rs"]);
        assert_eq!(result.evidence[0].key, "a");
    }

    #[test]
    fn dead_code_sort_prefers_higher_certainty_then_path() {
        let mut candidates = vec![
            DeadCodeCandidate {
                node: node("src/z.rs::fn::candidate_z", "src/z.rs", 1),
                reasons: vec![],
                certainty: ConfidenceTier::Medium,
                blockers: vec![],
            },
            DeadCodeCandidate {
                node: node("src/a.rs::fn::candidate_a", "src/a.rs", 1),
                reasons: vec![],
                certainty: ConfidenceTier::High,
                blockers: vec![],
            },
        ];

        sort_dead_code_candidates(&mut candidates, &AnalysisRankingPrimitives::default());

        assert_eq!(
            candidates[0].node.qualified_name,
            "src/a.rs::fn::candidate_a"
        );
        assert_eq!(
            candidates[1].node.qualified_name,
            "src/z.rs::fn::candidate_z"
        );
    }

    #[test]
    fn dependency_sort_stabilizes_blocking_references() {
        let mut result = DependencyRemovalResult {
            target_qname: "src/lib.rs::fn::target".to_owned(),
            removable: false,
            blocking_references: vec![
                node("src/z.rs::fn::caller_z", "src/z.rs", 1),
                node("src/a.rs::fn::caller_a", "src/a.rs", 1),
            ],
            evidence_edges: vec![
                Edge {
                    id: 0,
                    kind: EdgeKind::Calls,
                    source_qn: "src/z.rs::fn::caller_z".to_owned(),
                    target_qn: "src/lib.rs::fn::target".to_owned(),
                    file_path: "src/z.rs".to_owned(),
                    line: Some(8),
                    confidence: 1.0,
                    confidence_tier: Some("high".to_owned()),
                    extra_json: serde_json::Value::Null,
                },
                Edge {
                    id: 1,
                    kind: EdgeKind::Calls,
                    source_qn: "src/a.rs::fn::caller_a".to_owned(),
                    target_qn: "src/lib.rs::fn::target".to_owned(),
                    file_path: "src/a.rs".to_owned(),
                    line: Some(3),
                    confidence: 1.0,
                    confidence_tier: Some("high".to_owned()),
                    extra_json: serde_json::Value::Null,
                },
            ],
            confidence: ConfidenceTier::Low,
            suggested_cleanups: vec!["z".to_owned(), "a".to_owned()],
            evidence: vec![],
            uncertainty_flags: vec!["b".to_owned(), "a".to_owned()],
            budget: BudgetReport::not_applicable(),
        };

        sort_dependency_result(&mut result, &AnalysisRankingPrimitives::default());

        assert_eq!(
            result.blocking_references[0].qualified_name,
            "src/a.rs::fn::caller_a"
        );
        assert_eq!(result.evidence_edges[0].file_path, "src/a.rs");
        assert_eq!(result.uncertainty_flags, vec!["a", "b"]);
    }

    #[test]
    fn analysis_trimming_primitives_expose_shared_preview_defaults() {
        let trimming = AnalysisTrimmingPrimitives::default();
        assert_eq!(trimming.removal_symbol_preview_limit, 50);
        assert_eq!(trimming.removal_primary_preview_limit, 20);
        assert_eq!(trimming.removal_containment_preview_limit, 10);
        assert_eq!(trimming.dead_code_candidate_preview_limit, 50);
        assert_eq!(trimming.dependency_blocker_preview_limit, 20);
    }
}
