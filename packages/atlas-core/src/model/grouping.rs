use serde::{Deserialize, Serialize};

/// A named ordered sequence of graph nodes.
///
/// Flows represent user-defined traversal paths or call chains. Membership
/// is stored with soft references so it survives graph rebuilds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    pub id: i64,
    pub name: String,
    pub kind: Option<String>,
    pub description: Option<String>,
    pub extra_json: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

/// One node's participation in a flow with optional ordering and role.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowMembership {
    pub flow_id: i64,
    /// Qualified name of the node. Soft reference — survives node rebuild.
    pub node_qualified_name: String,
    pub position: Option<i64>,
    pub role: Option<String>,
    pub extra_json: serde_json::Value,
}

/// A named partition of graph nodes produced by a clustering algorithm.
///
/// Communities may nest: `parent_community_id` links child to parent.
/// The actual node members are stored in the `community_nodes` join table.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Community {
    pub id: i64,
    pub name: String,
    pub algorithm: Option<String>,
    pub level: Option<i64>,
    pub parent_community_id: Option<i64>,
    pub extra_json: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

/// One node's membership in a community. Soft reference on qualified name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommunityNode {
    pub community_id: i64,
    /// Qualified name of the node. Soft reference — survives node rebuild.
    pub node_qualified_name: String,
}

/// Requested scope for a postprocess orchestration run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostprocessExecutionMode {
    Full,
    ChangedOnly,
}

impl PostprocessExecutionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::ChangedOnly => "changed_only",
        }
    }
}

/// Lifecycle state recorded for the latest postprocess run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostprocessRunState {
    Running,
    Succeeded,
    Failed,
}

impl PostprocessRunState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

/// Per-stage status within a postprocess run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostprocessStageStatus {
    Planned,
    Completed,
    Skipped,
    Failed,
}

impl PostprocessStageStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Planned => "planned",
            Self::Completed => "completed",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }
}

/// Compact machine-readable per-stage summary for derived analytics refresh.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostprocessStageSummary {
    pub stage: String,
    pub status: PostprocessStageStatus,
    pub mode: PostprocessExecutionMode,
    pub affected_file_count: usize,
    pub item_count: usize,
    pub elapsed_ms: u64,
    pub error_code: Option<String>,
    pub message: Option<String>,
    pub details: serde_json::Value,
}

/// Public run summary shared by CLI and MCP postprocess surfaces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostprocessRunSummary {
    pub repo_root: String,
    pub ok: bool,
    pub noop: bool,
    pub noop_reason: Option<String>,
    pub error_code: String,
    pub message: String,
    pub suggestions: Vec<String>,
    pub graph_built: bool,
    pub state: PostprocessRunState,
    pub requested_mode: PostprocessExecutionMode,
    pub stage_filter: Option<String>,
    pub dry_run: bool,
    pub changed_files: Vec<String>,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub total_elapsed_ms: u64,
    pub stages: Vec<PostprocessStageSummary>,
    pub supported_stages: Vec<String>,
}

/// Latest persisted postprocess lifecycle record for one repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PostprocessStatus {
    pub repo_root: String,
    pub state: PostprocessRunState,
    pub mode: PostprocessExecutionMode,
    pub stage_filter: Option<String>,
    pub changed_file_count: usize,
    pub stages: Vec<PostprocessStageSummary>,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
    pub last_error_code: Option<String>,
    pub last_error: Option<String>,
    pub updated_at_ms: i64,
}
