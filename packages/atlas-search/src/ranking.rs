use atlas_core::{NodeKind, ScoredNode};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GraphSearchRankingPrimitives {
    pub exact_name_boost: f64,
    pub prefix_name_boost: f64,
    pub exact_qualified_name_boost: f64,
    pub fuzzy_distance_0_boost: f64,
    pub fuzzy_distance_1_boost: f64,
    pub fuzzy_distance_2_boost: f64,
    pub fuzzy_distance_3_boost: f64,
    pub function_kind_boost: f64,
    pub type_kind_boost: f64,
    pub enum_kind_boost: f64,
    pub public_api_boost: f64,
    pub same_directory_boost: f64,
    pub same_language_boost: f64,
    pub recent_file_boost: f64,
    pub changed_file_boost: f64,
}

impl Default for GraphSearchRankingPrimitives {
    fn default() -> Self {
        Self {
            exact_name_boost: 20.0,
            prefix_name_boost: 5.0,
            exact_qualified_name_boost: 15.0,
            fuzzy_distance_0_boost: 24.0,
            fuzzy_distance_1_boost: 18.0,
            fuzzy_distance_2_boost: 12.0,
            fuzzy_distance_3_boost: 8.0,
            function_kind_boost: 3.0,
            type_kind_boost: 2.0,
            enum_kind_boost: 1.0,
            public_api_boost: 2.0,
            same_directory_boost: 3.0,
            same_language_boost: 2.0,
            recent_file_boost: 4.0,
            changed_file_boost: 5.0,
        }
    }
}

impl GraphSearchRankingPrimitives {
    pub fn fuzzy_distance_bonus(&self, distance: usize) -> f64 {
        match distance {
            0 => self.fuzzy_distance_0_boost,
            1 => self.fuzzy_distance_1_boost,
            2 => self.fuzzy_distance_2_boost,
            _ => self.fuzzy_distance_3_boost,
        }
    }

    pub fn kind_boost(&self, kind: NodeKind) -> f64 {
        match kind {
            NodeKind::Function | NodeKind::Method => self.function_kind_boost,
            NodeKind::Class | NodeKind::Struct | NodeKind::Trait | NodeKind::Interface => {
                self.type_kind_boost
            }
            NodeKind::Enum => self.enum_kind_boost,
            _ => 0.0,
        }
    }
}

pub(crate) fn sort_scored_nodes(results: &mut [ScoredNode]) {
    results.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.node.qualified_name.cmp(&right.node.qualified_name))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::{Node, NodeId};

    fn scored_node(name: &str, qn: &str) -> ScoredNode {
        ScoredNode::new(
            Node {
                id: NodeId::UNSET,
                kind: NodeKind::Function,
                name: name.to_string(),
                qualified_name: qn.to_string(),
                file_path: "src/lib.rs".to_string(),
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
            1.0,
        )
    }

    #[test]
    fn graph_search_primitives_expose_expected_defaults() {
        let primitives = GraphSearchRankingPrimitives::default();
        assert_eq!(primitives.exact_name_boost, 20.0);
        assert_eq!(primitives.exact_qualified_name_boost, 15.0);
        assert_eq!(primitives.changed_file_boost, 5.0);
        assert_eq!(primitives.kind_boost(NodeKind::Function), 3.0);
        assert_eq!(primitives.kind_boost(NodeKind::Struct), 2.0);
        assert_eq!(primitives.kind_boost(NodeKind::Enum), 1.0);
    }

    #[test]
    fn fuzzy_distance_bonus_decreases_by_distance() {
        let primitives = GraphSearchRankingPrimitives::default();
        assert!(
            primitives.fuzzy_distance_bonus(0) > primitives.fuzzy_distance_bonus(1)
                && primitives.fuzzy_distance_bonus(1) > primitives.fuzzy_distance_bonus(2)
                && primitives.fuzzy_distance_bonus(2) > primitives.fuzzy_distance_bonus(3)
        );
    }

    #[test]
    fn sort_scored_nodes_breaks_ties_by_qualified_name() {
        let mut nodes = vec![
            scored_node("helper", "src/z.rs::fn::helper"),
            scored_node("helper", "src/a.rs::fn::helper"),
        ];

        sort_scored_nodes(&mut nodes);

        assert_eq!(nodes[0].node.qualified_name, "src/a.rs::fn::helper");
        assert_eq!(nodes[1].node.qualified_name, "src/z.rs::fn::helper");
    }
}
