//! ICM-G — shared overview service result types.

use serde::{Deserialize, Serialize};

use atlas_session::{
    OverviewAnalysisPacket, OverviewFreshness, OverviewGraphProvenance, OverviewPacketBudget,
    OverviewPacketFreshness, OverviewRecord, OverviewSubjectKind,
};

/// Result of `atlas overview export` / MCP `overview_export`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewExportResult {
    pub schema_version: u32,
    pub repo_root: String,
    pub commit_sha: String,
    pub graph: OverviewGraphProvenance,
    pub freshness: OverviewPacketFreshness,
    pub packets: Vec<OverviewAnalysisPacket>,
    pub subject_count: usize,
    pub omitted_subject_count: usize,
    pub budget: OverviewPacketBudget,
}

/// Result of `atlas overview ingest` / MCP `overview_ingest`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewIngestResult {
    pub record: OverviewRecord,
    pub subject_resolved: bool,
    pub packet_validated: bool,
    pub body_routed: bool,
    pub redacted: bool,
    pub status: String,
}

/// One refresh-plan entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewRefreshItem {
    pub overview_id: String,
    pub subject_id: String,
    pub subject_kind: OverviewSubjectKind,
    pub subject_label: String,
    pub freshness: OverviewFreshness,
    pub stale_reason: String,
    pub suggested_export_command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub affected_dependents: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_record_id: Option<String>,
    pub priority: u32,
}

/// Read-only refresh plan unless the caller passes an explicit apply flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OverviewRefreshPlan {
    pub schema_version: u32,
    pub repo_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    pub changed_files: Vec<String>,
    pub apply: bool,
    pub applied_count: usize,
    pub direct_stale: Vec<OverviewRefreshItem>,
    pub possibly_stale: Vec<OverviewRefreshItem>,
    pub orphaned: Vec<OverviewRefreshItem>,
    pub unchanged_count: usize,
}

/// Markdown docs export grouping project, package, module, file, then symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OverviewDocsExport {
    pub markdown: String,
    pub record_count: usize,
    pub stale_count: usize,
}
