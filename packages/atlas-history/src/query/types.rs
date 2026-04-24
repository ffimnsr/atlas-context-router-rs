use atlas_store_sqlite::{HistoricalEdge, HistoricalNode, StoredSnapshotFile};
use serde::Serialize;

use crate::reports::HistoryEvidence;

#[derive(Debug, Clone)]
pub(crate) struct SnapshotState {
    pub(crate) snapshot_id: i64,
    pub(crate) commit_sha: String,
    pub(crate) author_time: i64,
    pub(crate) files: Vec<StoredSnapshotFile>,
    pub(crate) nodes: Vec<HistoricalNode>,
    pub(crate) edges: Vec<HistoricalEdge>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeHistorySummary {
    pub first_appearance_snapshot_id: Option<i64>,
    pub last_appearance_snapshot_id: Option<i64>,
    pub first_appearance_commit_sha: Option<String>,
    pub last_appearance_commit_sha: Option<String>,
    pub removal_commit_sha: Option<String>,
    pub change_commit_count: usize,
    pub signature_version_count: usize,
    pub current_file_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeChangeRecord {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub change_kinds: Vec<String>,
    pub file_paths: Vec<String>,
    pub node_identifiers: Vec<String>,
    pub signature_hashes: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeSignatureRecord {
    pub file_path: String,
    pub kind: String,
    pub params: Option<String>,
    pub return_type: Option<String>,
    pub modifiers: Option<String>,
    pub signature_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeSignatureSnapshot {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub signatures: Vec<NodeSignatureRecord>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeFilePathSnapshot {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub file_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct NodeHistoryFindings {
    pub appearances: Vec<NodeChangeRecord>,
    pub commits_where_changed: Vec<NodeChangeRecord>,
    pub signature_evolution: Vec<NodeSignatureSnapshot>,
    pub file_path_changes: Vec<NodeFilePathSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeHistoryReport {
    pub qualified_name: String,
    pub summary: NodeHistorySummary,
    pub findings: NodeHistoryFindings,
    pub evidence: HistoryEvidence,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileHistorySummary {
    pub file_path: String,
    pub first_appearance_snapshot_id: Option<i64>,
    pub last_appearance_snapshot_id: Option<i64>,
    pub first_appearance_commit_sha: Option<String>,
    pub last_appearance_commit_sha: Option<String>,
    pub removal_commit_sha: Option<String>,
    pub commit_touch_count: usize,
    pub timeline_points: usize,
    pub current_file_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileHistoryPoint {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub exists: bool,
    pub file_hash: Option<String>,
    pub node_count: usize,
    pub edge_count: usize,
    pub symbol_additions: Vec<String>,
    pub symbol_removals: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileHistoryFindings {
    pub commits_touched: Vec<FileHistoryPoint>,
    pub timeline: Vec<FileHistoryPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileHistoryReport {
    pub file_path: String,
    pub summary: FileHistorySummary,
    pub findings: FileHistoryFindings,
    pub evidence: HistoryEvidence,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeHistorySummary {
    pub source_qn: String,
    pub target_qn: String,
    pub first_appearance_snapshot_id: Option<i64>,
    pub last_appearance_snapshot_id: Option<i64>,
    pub first_appearance_commit_sha: Option<String>,
    pub last_appearance_commit_sha: Option<String>,
    pub disappearance_commit_sha: Option<String>,
    pub currently_present: bool,
    pub change_commit_count: usize,
    pub persistence_duration_secs: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DependencyHistoryPoint {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub present: bool,
    pub edge_count: usize,
    pub edge_identifiers: Vec<String>,
    pub added_edges: Vec<String>,
    pub removed_edges: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeHistoryFindings {
    pub timeline: Vec<DependencyHistoryPoint>,
    pub commits_where_changed: Vec<DependencyHistoryPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct EdgeHistoryReport {
    pub summary: EdgeHistorySummary,
    pub findings: EdgeHistoryFindings,
    pub evidence: HistoryEvidence,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleHistorySummary {
    pub module: String,
    pub first_appearance_snapshot_id: Option<i64>,
    pub last_appearance_snapshot_id: Option<i64>,
    pub first_appearance_commit_sha: Option<String>,
    pub last_appearance_commit_sha: Option<String>,
    pub removal_commit_sha: Option<String>,
    pub max_node_count: usize,
    pub max_dependency_count: usize,
    pub max_coupling_count: usize,
    pub max_test_adjacency_count: usize,
    pub timeline_points: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleHistoryPoint {
    pub snapshot_id: i64,
    pub commit_sha: String,
    pub node_count: usize,
    pub dependency_count: usize,
    pub coupling_count: usize,
    pub test_adjacency_count: usize,
    pub file_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleHistoryFindings {
    pub timeline: Vec<ModuleHistoryPoint>,
    pub commits_where_changed: Vec<ModuleHistoryPoint>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModuleHistoryReport {
    pub module: String,
    pub summary: ModuleHistorySummary,
    pub findings: ModuleHistoryFindings,
    pub evidence: HistoryEvidence,
}
