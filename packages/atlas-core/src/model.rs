use serde::{Deserialize, Serialize};

use crate::kinds::{EdgeKind, NodeKind};

/// Opaque primary key for a graph node.
///
/// `NodeId(0)` is the sentinel value used before a database ID is assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub i64);

impl NodeId {
    /// Sentinel used before a real database ID has been assigned.
    pub const UNSET: NodeId = NodeId(0);
}

impl From<i64> for NodeId {
    fn from(v: i64) -> Self {
        NodeId(v)
    }
}

impl From<NodeId> for i64 {
    fn from(id: NodeId) -> Self {
        id.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub file_path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub language: String,
    pub parent_name: Option<String>,
    pub params: Option<String>,
    pub return_type: Option<String>,
    pub modifiers: Option<String>,
    pub is_test: bool,
    pub file_hash: String,
    pub extra_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: i64,
    pub kind: EdgeKind,
    pub source_qn: String,
    pub target_qn: String,
    pub file_path: String,
    pub line: Option<u32>,
    pub confidence: f32,
    pub confidence_tier: Option<String>,
    pub extra_json: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileRecord {
    pub path: String,
    pub language: Option<String>,
    pub hash: String,
    pub size: Option<i64>,
    pub indexed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphStats {
    pub file_count: i64,
    pub node_count: i64,
    pub edge_count: i64,
    pub nodes_by_kind: Vec<(String, i64)>,
    pub languages: Vec<String>,
    pub last_indexed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub change_type: ChangeType,
    pub old_path: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Added,
    Modified,
    Deleted,
    Renamed,
    Copied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactResult {
    pub changed_nodes: Vec<Node>,
    pub impacted_nodes: Vec<Node>,
    pub impacted_files: Vec<String>,
    pub relevant_edges: Vec<Edge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewContext {
    pub changed_files: Vec<String>,
    pub changed_symbols: Vec<Node>,
    pub impacted_neighbors: Vec<Node>,
    pub critical_edges: Vec<Edge>,
    pub risk_summary: RiskSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskSummary {
    pub changed_symbol_count: usize,
    pub public_api_changes: usize,
    pub test_adjacent: bool,
    pub cross_module_impact: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub text: String,
    pub kind: Option<String>,
    pub language: Option<String>,
    pub file_path: Option<String>,
    /// Filter results whose `file_path` starts with this subpath prefix.
    pub subpath: Option<String>,
    pub is_test: Option<bool>,
    pub limit: usize,
    /// Expand FTS seed results through graph edges.
    pub graph_expand: bool,
    /// Maximum edge hops when `graph_expand` is true (default: 1).
    pub graph_max_hops: u32,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: String::new(),
            kind: None,
            language: None,
            file_path: None,
            subpath: None,
            is_test: None,
            limit: 20,
            graph_expand: false,
            graph_max_hops: 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredNode {
    pub node: Node,
    pub score: f64,
}

/// All data produced by parsing one file, ready to be persisted.
///
/// `nodes` and `edges` carry `id = 0`; the store assigns real database IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParsedFile {
    pub path: String,
    pub language: Option<String>,
    pub hash: String,
    pub size: Option<i64>,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}
