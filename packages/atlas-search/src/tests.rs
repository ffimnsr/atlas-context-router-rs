use super::*;
use atlas_core::SearchQuery;
use atlas_core::{Edge, EdgeKind, Node, NodeId};
use atlas_core::{HybridRankingSource, SearchMatchedField};

fn make_test_node(name: &str, qn: &str, file_path: &str, language: &str) -> ScoredNode {
    use atlas_core::{Node, NodeId, NodeKind};
    ScoredNode::new(
        Node {
            id: NodeId::UNSET,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: qn.to_string(),
            file_path: file_path.to_string(),
            line_start: 1,
            line_end: 10,
            language: language.to_string(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: "h".to_string(),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        },
        1.0,
    )
}

mod explain;
mod fts;
mod fuzzy;
mod merge;
mod ranking;
mod search;
