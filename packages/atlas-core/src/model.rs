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

impl Node {
    /// Build a concise text representation suitable for semantic embedding.
    ///
    /// Format: `{kind} {name}[({params})][ -> {return_type}]  [{qualified_name}]`
    /// This provides a dense, symbol-level chunk for vector retrieval.
    pub fn chunk_text(&self) -> String {
        let mut out = format!("{} {}", self.kind.as_str(), self.name);
        if let Some(p) = &self.params
            && !p.is_empty()
        {
            out.push('(');
            out.push_str(p);
            out.push(')');
        }
        if let Some(r) = &self.return_type
            && !r.is_empty()
        {
            out.push_str(" -> ");
            out.push_str(r);
        }
        out.push_str("  [");
        out.push_str(&self.qualified_name);
        out.push(']');
        out
    }
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
    pub changed_symbol_summaries: Vec<ChangedSymbolSummary>,
    pub impacted_neighbors: Vec<Node>,
    pub critical_edges: Vec<Edge>,
    pub impact_overview: ReviewImpactOverview,
    pub risk_summary: RiskSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedSymbolSummary {
    pub node: Node,
    pub callers: Vec<Node>,
    pub callees: Vec<Node>,
    pub importers: Vec<Node>,
    pub tests: Vec<Node>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewImpactOverview {
    pub max_depth: u32,
    pub max_nodes: usize,
    pub impacted_node_count: usize,
    pub impacted_file_count: usize,
    pub relevant_edge_count: usize,
    pub reached_node_limit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskSummary {
    pub changed_symbol_count: usize,
    pub public_api_changes: usize,
    pub test_adjacent: bool,
    pub affected_test_count: usize,
    pub uncovered_changed_symbol_count: usize,
    pub large_function_touched: bool,
    pub large_function_count: usize,
    pub cross_module_impact: bool,
    pub cross_package_impact: bool,
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
    /// Reference file path for same-directory boost. When set, results in the
    /// same directory as this file receive a ranking bonus.
    pub reference_file: Option<String>,
    /// Reference language for same-language boost. When set, results in the
    /// same language receive a ranking bonus.
    pub reference_language: Option<String>,
    /// Enable fuzzy (edit-distance) name matching boost (+4). Off by default
    /// because it adds O(results) edit-distance work.
    pub fuzzy_match: bool,
    /// Boost nodes whose file was among the most recently indexed (+4). Requires
    /// one extra DB read inside `atlas_search::search`; off by default.
    pub recent_file_boost: bool,
    /// Boost nodes whose file appears in this set of changed file paths (+5).
    /// Caller populates this with the paths from the current git diff.
    /// Empty vec disables the boost.
    pub changed_files: Vec<String>,
    /// Enable hybrid (FTS + vector) retrieval.
    ///
    /// When `true` and `ATLAS_EMBED_URL` is set, the search layer runs both
    /// FTS and vector retrieval and merges results with Reciprocal Rank Fusion.
    /// Falls back to FTS-only when no embedding backend is configured.
    pub hybrid: bool,
    /// FTS candidate pool size before RRF merge (default: 60).
    pub top_k_fts: usize,
    /// Vector candidate pool size before RRF merge (default: 60).
    pub top_k_vector: usize,
    /// Reciprocal Rank Fusion k constant (default: 60).
    pub rrf_k: u32,
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
            reference_file: None,
            reference_language: None,
            fuzzy_match: false,
            recent_file_boost: false,
            changed_files: vec![],
            hybrid: false,
            top_k_fts: 60,
            top_k_vector: 60,
            rrf_k: 60,
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

// ---------------------------------------------------------------------------
// Advanced impact types (Slice 16)
// ---------------------------------------------------------------------------

/// Coarse risk level assigned to a change set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskLevel {
    Low,
    Medium,
    High,
    Critical,
}

impl std::fmt::Display for RiskLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            RiskLevel::Low => "low",
            RiskLevel::Medium => "medium",
            RiskLevel::High => "high",
            RiskLevel::Critical => "critical",
        };
        f.write_str(s)
    }
}

/// How a symbol was changed in the current diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// Public/visibility modifier changed — externally visible interface break.
    ApiChange,
    /// Parameter list or return type changed.
    SignatureChange,
    /// Body-only change; public interface is stable.
    InternalChange,
}

impl std::fmt::Display for ChangeKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            ChangeKind::ApiChange => "api_change",
            ChangeKind::SignatureChange => "signature_change",
            ChangeKind::InternalChange => "internal_change",
        };
        f.write_str(s)
    }
}

/// A node enriched with a weighted impact score and optional change classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredImpactNode {
    pub node: Node,
    /// Higher = more impacted. Based on weighted edge traversal from seed nodes.
    pub impact_score: f64,
    /// Set only for nodes that are direct seed (changed) nodes.
    pub change_kind: Option<ChangeKind>,
}

/// Test-coverage impact: which tests are affected and which changed symbols
/// have no adjacent test.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestImpactResult {
    /// Test nodes (`is_test = true` or `NodeKind::Test`) within the impact set.
    pub affected_tests: Vec<Node>,
    /// Changed nodes that have no `Tests`/`TestedBy` edge to any test node.
    pub uncovered_changed_nodes: Vec<Node>,
}

/// A detected cross-boundary impact.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryViolation {
    pub kind: BoundaryKind,
    pub description: String,
    /// Representative qualified names crossing the boundary.
    pub nodes: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryKind {
    CrossModule,
    CrossPackage,
}

impl std::fmt::Display for BoundaryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            BoundaryKind::CrossModule => "cross_module",
            BoundaryKind::CrossPackage => "cross_package",
        };
        f.write_str(s)
    }
}

/// Full advanced impact analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedImpactResult {
    /// Base impact radius (unchanged nodes + impacted nodes + edges).
    pub base: ImpactResult,
    /// Impacted nodes ranked by weighted traversal score (highest first).
    pub scored_nodes: Vec<ScoredImpactNode>,
    /// Overall risk level for this change set.
    pub risk_level: RiskLevel,
    /// Test-coverage impact.
    pub test_impact: TestImpactResult,
    /// Architecture boundary violations detected.
    pub boundary_violations: Vec<BoundaryViolation>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::{EdgeKind, NodeKind};

    // -------------------------------------------------------------------------
    // NodeId
    // -------------------------------------------------------------------------

    #[test]
    fn node_id_serde_round_trip() {
        let id = NodeId(42);
        let json = serde_json::to_string(&id).unwrap();
        // transparent: serialises as a plain number
        assert_eq!(json, "42");
        let back: NodeId = serde_json::from_str(&json).unwrap();
        assert_eq!(back, id);
    }

    #[test]
    fn node_id_unset_sentinel() {
        assert_eq!(NodeId::UNSET.0, 0);
    }

    #[test]
    fn node_id_from_i64() {
        assert_eq!(NodeId::from(7_i64), NodeId(7));
        let raw: i64 = NodeId(99).into();
        assert_eq!(raw, 99);
    }

    // -------------------------------------------------------------------------
    // Node serialization
    // -------------------------------------------------------------------------

    fn sample_node() -> Node {
        Node {
            id: NodeId(1),
            kind: NodeKind::Function,
            name: "my_func".to_string(),
            qualified_name: "src/lib.rs::fn::my_func".to_string(),
            file_path: "src/lib.rs".to_string(),
            line_start: 10,
            line_end: 20,
            language: "rust".to_string(),
            parent_name: None,
            params: Some("(x: i32)".to_string()),
            return_type: Some("i32".to_string()),
            modifiers: None,
            is_test: false,
            file_hash: "abc123".to_string(),
            extra_json: serde_json::Value::Null,
        }
    }

    #[test]
    fn node_serde_round_trip() {
        let n = sample_node();
        let json = serde_json::to_string(&n).unwrap();
        let back: Node = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, n.id);
        assert_eq!(back.kind, n.kind);
        assert_eq!(back.name, n.name);
        assert_eq!(back.qualified_name, n.qualified_name);
        assert_eq!(back.file_path, n.file_path);
        assert_eq!(back.line_start, n.line_start);
        assert_eq!(back.line_end, n.line_end);
        assert_eq!(back.language, n.language);
        assert_eq!(back.params, n.params);
        assert_eq!(back.return_type, n.return_type);
        assert_eq!(back.is_test, n.is_test);
        assert_eq!(back.file_hash, n.file_hash);
    }

    #[test]
    fn node_optional_fields_null_round_trip() {
        let mut n = sample_node();
        n.parent_name = None;
        n.params = None;
        n.return_type = None;
        n.modifiers = None;
        let json = serde_json::to_string(&n).unwrap();
        let back: Node = serde_json::from_str(&json).unwrap();
        assert!(back.parent_name.is_none());
        assert!(back.params.is_none());
        assert!(back.return_type.is_none());
        assert!(back.modifiers.is_none());
    }

    #[test]
    fn node_is_test_flag_preserved() {
        let mut n = sample_node();
        n.is_test = true;
        n.kind = NodeKind::Test;
        let json = serde_json::to_string(&n).unwrap();
        let back: Node = serde_json::from_str(&json).unwrap();
        assert!(back.is_test);
        assert_eq!(back.kind, NodeKind::Test);
    }

    // -------------------------------------------------------------------------
    // Edge serialization
    // -------------------------------------------------------------------------

    fn sample_edge() -> Edge {
        Edge {
            id: 0,
            kind: EdgeKind::Calls,
            source_qn: "src/a.rs::fn::caller".to_string(),
            target_qn: "src/b.rs::fn::callee".to_string(),
            file_path: "src/a.rs".to_string(),
            line: Some(15),
            confidence: 1.0,
            confidence_tier: Some("high".to_string()),
            extra_json: serde_json::Value::Null,
        }
    }

    #[test]
    fn edge_serde_round_trip() {
        let e = sample_edge();
        let json = serde_json::to_string(&e).unwrap();
        let back: Edge = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, e.id);
        assert_eq!(back.kind, e.kind);
        assert_eq!(back.source_qn, e.source_qn);
        assert_eq!(back.target_qn, e.target_qn);
        assert_eq!(back.file_path, e.file_path);
        assert_eq!(back.line, e.line);
        assert_eq!(back.confidence, e.confidence);
        assert_eq!(back.confidence_tier, e.confidence_tier);
    }

    #[test]
    fn edge_optional_line_none_round_trip() {
        let mut e = sample_edge();
        e.line = None;
        let json = serde_json::to_string(&e).unwrap();
        let back: Edge = serde_json::from_str(&json).unwrap();
        assert!(back.line.is_none());
    }

    #[test]
    fn edge_optional_confidence_tier_none_round_trip() {
        let mut e = sample_edge();
        e.confidence_tier = None;
        let json = serde_json::to_string(&e).unwrap();
        let back: Edge = serde_json::from_str(&json).unwrap();
        assert!(back.confidence_tier.is_none());
    }
}
