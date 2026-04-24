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
    pub owner_id: Option<String>,
    pub owner_kind: Option<String>,
    pub owner_root: Option<String>,
    pub owner_manifest_path: Option<String>,
    pub owner_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PackageOwnerKind {
    Cargo,
    Npm,
    Go,
}

impl PackageOwnerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cargo => "cargo",
            Self::Npm => "npm",
            Self::Go => "go",
        }
    }
}

impl std::fmt::Display for PackageOwnerKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageOwner {
    pub owner_id: String,
    pub kind: PackageOwnerKind,
    pub root: String,
    pub manifest_path: String,
    pub package_name: Option<String>,
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

/// Compact provenance snapshot attached to every MCP tool response (MCP7).
///
/// Intentionally kept minimal: two SQL queries, no breakdown tables.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceMeta {
    pub indexed_file_count: i64,
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
