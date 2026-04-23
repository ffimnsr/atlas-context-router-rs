use atlas_core::NodeKind;
use atlas_core::model::{
    ContextRequest, SavedContextSource, SelectedEdge, SelectedNode, SelectionReason,
};

use crate::context::{DEFAULT_MAX_EDGES, DEFAULT_MAX_FILES, DEFAULT_MAX_NODES};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ContextRankingPrimitives {
    pub direct_target_priority: u8,
    pub caller_priority: u8,
    pub callee_priority: u8,
    pub importer_priority: u8,
    pub importee_priority: u8,
    pub test_adjacent_priority: u8,
    pub containment_sibling_priority: u8,
    pub impact_neighbor_priority: u8,
    pub distance_step_penalty: f64,
    pub max_distance_bonus: f64,
    pub same_file_boost: f64,
    pub public_api_boost: f64,
    pub function_kind_boost: f64,
    pub type_kind_boost: f64,
    pub test_node_penalty: f64,
}

impl Default for ContextRankingPrimitives {
    fn default() -> Self {
        Self {
            direct_target_priority: 100,
            caller_priority: 80,
            callee_priority: 80,
            importer_priority: 60,
            importee_priority: 60,
            test_adjacent_priority: 50,
            containment_sibling_priority: 40,
            impact_neighbor_priority: 30,
            distance_step_penalty: 5.0,
            max_distance_bonus: 10.0,
            same_file_boost: 3.0,
            public_api_boost: 5.0,
            function_kind_boost: 3.0,
            type_kind_boost: 2.0,
            test_node_penalty: 10.0,
        }
    }
}

impl ContextRankingPrimitives {
    pub fn selection_priority(&self, reason: SelectionReason) -> u8 {
        match reason {
            SelectionReason::DirectTarget => self.direct_target_priority,
            SelectionReason::Caller => self.caller_priority,
            SelectionReason::Callee => self.callee_priority,
            SelectionReason::Importer => self.importer_priority,
            SelectionReason::Importee => self.importee_priority,
            SelectionReason::TestAdjacent => self.test_adjacent_priority,
            SelectionReason::ContainmentSibling => self.containment_sibling_priority,
            SelectionReason::ImpactNeighbor => self.impact_neighbor_priority,
        }
    }

    pub fn kind_boost(&self, kind: NodeKind) -> f64 {
        match kind {
            NodeKind::Function | NodeKind::Method => self.function_kind_boost,
            NodeKind::Class | NodeKind::Struct | NodeKind::Trait | NodeKind::Interface => {
                self.type_kind_boost
            }
            _ => 0.0,
        }
    }

    pub fn distance_bonus(&self, distance: u32) -> f64 {
        (self.max_distance_bonus - distance as f64 * self.distance_step_penalty).max(0.0)
    }

    pub fn node_score(&self, node: &SelectedNode, seed_file: &str) -> f64 {
        let mut score = self.selection_priority(node.selection_reason) as f64;
        score += self.distance_bonus(node.distance);

        if node.node.file_path == seed_file {
            score += self.same_file_boost;
        }

        if let Some(mods) = &node.node.modifiers {
            let m = mods.to_lowercase();
            if m.contains("pub") || m.contains("public") || m.contains("export") {
                score += self.public_api_boost;
            }
        }

        score += self.kind_boost(node.node.kind);

        if node.node.is_test || node.node.kind == NodeKind::Test {
            score -= self.test_node_penalty;
        }

        score
    }

    pub fn edge_score(&self, edge: &SelectedEdge) -> f64 {
        self.selection_priority(edge.selection_reason) as f64 + (edge.edge.confidence as f64) * 10.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavedContextRankingPrimitives {
    pub rank_divisor: f32,
    pub recent_source_bonus: f32,
    pub same_session_bonus: f32,
}

impl Default for SavedContextRankingPrimitives {
    fn default() -> Self {
        Self {
            rank_divisor: 10.0,
            recent_source_bonus: 5.0,
            same_session_bonus: 10.0,
        }
    }
}

impl SavedContextRankingPrimitives {
    pub fn rank_score(&self, rank: usize) -> f32 {
        self.rank_divisor / (rank as f32 + 1.0)
    }

    pub fn sort_sources(&self, sources: &mut [SavedContextSource]) {
        sources.sort_by(|left, right| {
            right
                .relevance_score
                .partial_cmp(&left.relevance_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.source_id.cmp(&right.source_id))
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrimmingPrimitives {
    pub max_nodes: usize,
    pub max_edges: usize,
    pub max_files: usize,
    pub max_content_assets: Option<usize>,
    pub max_payload_bytes: Option<usize>,
    pub max_payload_tokens: Option<usize>,
}

impl TrimmingPrimitives {
    pub fn from_request(request: &ContextRequest) -> Self {
        Self {
            max_nodes: request.max_nodes.unwrap_or(DEFAULT_MAX_NODES),
            max_edges: request.max_edges.unwrap_or(DEFAULT_MAX_EDGES),
            max_files: request.max_files.unwrap_or(DEFAULT_MAX_FILES),
            max_content_assets: None,
            max_payload_bytes: None,
            max_payload_tokens: None,
        }
    }
}

pub(crate) fn compare_node_scores(
    left_score: f64,
    right_score: f64,
    left_qn: &str,
    right_qn: &str,
) -> std::cmp::Ordering {
    right_score
        .partial_cmp(&left_score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| left_qn.cmp(right_qn))
}

pub(crate) fn compare_edge_scores(
    left_score: f64,
    right_score: f64,
    left_source_qn: &str,
    right_source_qn: &str,
    left_target_qn: &str,
    right_target_qn: &str,
) -> std::cmp::Ordering {
    right_score
        .partial_cmp(&left_score)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| left_source_qn.cmp(right_source_qn))
        .then_with(|| left_target_qn.cmp(right_target_qn))
}

pub(crate) fn compare_file_priorities(
    left_priority: u8,
    right_priority: u8,
    left_path: &str,
    right_path: &str,
) -> std::cmp::Ordering {
    right_priority
        .cmp(&left_priority)
        .then_with(|| left_path.cmp(right_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::Node;

    fn selected_node(reason: SelectionReason, qn: &str, file: &str) -> SelectedNode {
        SelectedNode {
            node: Node {
                id: atlas_core::NodeId::UNSET,
                kind: NodeKind::Function,
                name: "helper".to_string(),
                qualified_name: qn.to_string(),
                file_path: file.to_string(),
                line_start: 1,
                line_end: 1,
                language: "rust".to_string(),
                parent_name: None,
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: String::new(),
                extra_json: serde_json::Value::Null,
            },
            selection_reason: reason,
            distance: 0,
            relevance_score: 0.0,
        }
    }

    #[test]
    fn context_primitives_prioritize_direct_targets_over_neighbors() {
        let primitives = ContextRankingPrimitives::default();
        assert!(
            primitives.selection_priority(SelectionReason::DirectTarget)
                > primitives.selection_priority(SelectionReason::Caller)
        );
        assert!(
            primitives.selection_priority(SelectionReason::Caller)
                > primitives.selection_priority(SelectionReason::ImpactNeighbor)
        );
    }

    #[test]
    fn compare_node_scores_breaks_ties_by_qualified_name() {
        let ordering = compare_node_scores(10.0, 10.0, "src/a.rs::fn::a", "src/b.rs::fn::b");
        assert_eq!(ordering, std::cmp::Ordering::Less);
    }

    #[test]
    fn compare_edge_scores_breaks_ties_by_source_then_target() {
        let ordering = compare_edge_scores(
            10.0,
            10.0,
            "src/a.rs::fn::a",
            "src/b.rs::fn::a",
            "src/a.rs::fn::target",
            "src/b.rs::fn::target",
        );
        assert_eq!(ordering, std::cmp::Ordering::Less);

        let second_ordering = compare_edge_scores(
            10.0,
            10.0,
            "src/a.rs::fn::a",
            "src/a.rs::fn::a",
            "src/a.rs::fn::alpha",
            "src/a.rs::fn::beta",
        );
        assert_eq!(second_ordering, std::cmp::Ordering::Less);
    }

    #[test]
    fn saved_context_primitives_sort_same_session_first() {
        let primitives = SavedContextRankingPrimitives::default();
        let mut items = vec![
            SavedContextSource {
                source_id: "b".to_string(),
                label: "b".to_string(),
                source_type: "review_context".to_string(),
                session_id: Some("s2".to_string()),
                preview: String::new(),
                retrieval_hint: String::new(),
                relevance_score: primitives.rank_score(0),
            },
            SavedContextSource {
                source_id: "a".to_string(),
                label: "a".to_string(),
                source_type: "review_context".to_string(),
                session_id: Some("s1".to_string()),
                preview: String::new(),
                retrieval_hint: String::new(),
                relevance_score: primitives.rank_score(0) + primitives.same_session_bonus,
            },
        ];

        primitives.sort_sources(&mut items);
        assert_eq!(items[0].source_id, "a");
    }

    #[test]
    fn saved_context_primitives_break_ties_by_source_id() {
        let primitives = SavedContextRankingPrimitives::default();
        let mut items = vec![
            SavedContextSource {
                source_id: "b".to_string(),
                label: "b".to_string(),
                source_type: "review_context".to_string(),
                session_id: Some("s1".to_string()),
                preview: String::new(),
                retrieval_hint: String::new(),
                relevance_score: 10.0,
            },
            SavedContextSource {
                source_id: "a".to_string(),
                label: "a".to_string(),
                source_type: "review_context".to_string(),
                session_id: Some("s2".to_string()),
                preview: String::new(),
                retrieval_hint: String::new(),
                relevance_score: 10.0,
            },
        ];

        primitives.sort_sources(&mut items);
        assert_eq!(items[0].source_id, "a");
        assert_eq!(items[1].source_id, "b");
    }

    #[test]
    fn trimming_primitives_use_context_defaults() {
        let request = ContextRequest::default();
        let limits = TrimmingPrimitives::from_request(&request);
        assert_eq!(limits.max_nodes, DEFAULT_MAX_NODES);
        assert_eq!(limits.max_edges, DEFAULT_MAX_EDGES);
        assert_eq!(limits.max_files, DEFAULT_MAX_FILES);
        assert_eq!(limits.max_content_assets, None);
        assert_eq!(limits.max_payload_bytes, None);
        assert_eq!(limits.max_payload_tokens, None);
    }

    #[test]
    fn compare_file_priorities_breaks_ties_by_path() {
        let ordering = compare_file_priorities(5, 5, "src/a.rs", "src/b.rs");
        assert_eq!(ordering, std::cmp::Ordering::Less);
    }

    #[test]
    fn context_node_score_rewards_same_file() {
        let primitives = ContextRankingPrimitives::default();
        let same_file = selected_node(SelectionReason::DirectTarget, "src/a.rs::fn::a", "src/a.rs");
        let other_file =
            selected_node(SelectionReason::DirectTarget, "src/b.rs::fn::b", "src/b.rs");
        assert!(
            primitives.node_score(&same_file, "src/a.rs")
                > primitives.node_score(&other_file, "src/a.rs")
        );
    }
}
