use serde::{Deserialize, Serialize};
use serde_json::Value;

use atlas_core::AtlasError;

use crate::SessionId;

pub const DEFAULT_SESSION_DB: &str = "session.db";
pub const DEFAULT_SESSION_MAX_EVENTS: usize = 256;
pub const MAX_INLINE_EVENT_PAYLOAD_BYTES: usize = 8 * 1024;
pub const DEFAULT_MAX_SNAPSHOT_BYTES: usize = 64 * 1024;
pub const DEFAULT_DEDUP_WINDOW_SECS: u64 = 0;

#[derive(Debug, Clone)]
pub struct SessionStoreConfig {
    pub max_events_per_session: usize,
    pub max_inline_payload_bytes: usize,
    pub max_snapshot_bytes: usize,
    pub dedup_window_secs: u64,
}

impl Default for SessionStoreConfig {
    fn default() -> Self {
        Self {
            max_events_per_session: DEFAULT_SESSION_MAX_EVENTS,
            max_inline_payload_bytes: MAX_INLINE_EVENT_PAYLOAD_BYTES,
            max_snapshot_bytes: DEFAULT_MAX_SNAPSHOT_BYTES,
            dedup_window_secs: DEFAULT_DEDUP_WINDOW_SECS,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionStats {
    pub session_count: usize,
    pub total_events: usize,
    pub snapshot_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMeta {
    pub session_id: SessionId,
    pub repo_root: String,
    pub frontend: String,
    pub worktree_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_resume_at: Option<String>,
    pub last_compaction_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventCategory {
    UserIntent,
    Command,
    GraphState,
    Context,
    Reasoning,
    Error,
    FileOperation,
    SessionLifecycle,
}

impl EventCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserIntent => "USER_INTENT",
            Self::Command => "COMMAND",
            Self::GraphState => "GRAPH_STATE",
            Self::Context => "CONTEXT",
            Self::Reasoning => "REASONING",
            Self::Error => "ERROR",
            Self::FileOperation => "FILE_OPERATION",
            Self::SessionLifecycle => "SESSION_LIFECYCLE",
        }
    }
}

impl std::fmt::Display for EventCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionEventType {
    FileRead,
    FileWrite,
    CommandRun,
    CommandFail,
    GraphBuild,
    GraphUpdate,
    ReviewContext,
    ImpactAnalysis,
    ContextRequest,
    ReasoningResult,
    UserIntent,
    Decision,
    RuleInstruction,
    Error,
    SessionStart,
    SessionResume,
}

impl SessionEventType {
    pub fn category(&self) -> EventCategory {
        match self {
            Self::UserIntent | Self::Decision | Self::RuleInstruction => EventCategory::UserIntent,
            Self::CommandRun | Self::CommandFail => EventCategory::Command,
            Self::GraphBuild | Self::GraphUpdate => EventCategory::GraphState,
            Self::ReviewContext | Self::ImpactAnalysis | Self::ContextRequest => {
                EventCategory::Context
            }
            Self::ReasoningResult => EventCategory::Reasoning,
            Self::Error => EventCategory::Error,
            Self::FileRead | Self::FileWrite => EventCategory::FileOperation,
            Self::SessionStart | Self::SessionResume => EventCategory::SessionLifecycle,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FileRead => "FILE_READ",
            Self::FileWrite => "FILE_WRITE",
            Self::CommandRun => "COMMAND_RUN",
            Self::CommandFail => "COMMAND_FAIL",
            Self::GraphBuild => "GRAPH_BUILD",
            Self::GraphUpdate => "GRAPH_UPDATE",
            Self::ReviewContext => "REVIEW_CONTEXT",
            Self::ImpactAnalysis => "IMPACT_ANALYSIS",
            Self::ContextRequest => "CONTEXT_REQUEST",
            Self::ReasoningResult => "REASONING_RESULT",
            Self::UserIntent => "USER_INTENT",
            Self::Decision => "DECISION",
            Self::RuleInstruction => "RULE_INSTRUCTION",
            Self::Error => "ERROR",
            Self::SessionStart => "SESSION_START",
            Self::SessionResume => "SESSION_RESUME",
        }
    }
}

impl std::fmt::Display for SessionEventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for SessionEventType {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "FILE_READ" => Ok(Self::FileRead),
            "FILE_WRITE" => Ok(Self::FileWrite),
            "COMMAND_RUN" => Ok(Self::CommandRun),
            "COMMAND_FAIL" => Ok(Self::CommandFail),
            "GRAPH_BUILD" => Ok(Self::GraphBuild),
            "GRAPH_UPDATE" => Ok(Self::GraphUpdate),
            "REVIEW_CONTEXT" => Ok(Self::ReviewContext),
            "IMPACT_ANALYSIS" => Ok(Self::ImpactAnalysis),
            "CONTEXT_REQUEST" => Ok(Self::ContextRequest),
            "REASONING_RESULT" => Ok(Self::ReasoningResult),
            "USER_INTENT" => Ok(Self::UserIntent),
            "DECISION" => Ok(Self::Decision),
            "RULE_INSTRUCTION" => Ok(Self::RuleInstruction),
            "ERROR" => Ok(Self::Error),
            "SESSION_START" => Ok(Self::SessionStart),
            "SESSION_RESUME" => Ok(Self::SessionResume),
            other => Err(AtlasError::Other(format!(
                "unknown session event type: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct NewSessionEvent {
    pub session_id: SessionId,
    pub event_type: SessionEventType,
    pub priority: i32,
    pub payload: Value,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEventRow {
    pub id: i64,
    pub session_id: SessionId,
    pub event_type: SessionEventType,
    pub priority: i32,
    pub payload_json: String,
    pub event_hash: String,
    pub created_at: String,
}

/// Result returned by `SessionStore::compact_session()`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CurationResult {
    /// Number of events in the session before compaction.
    pub events_before: usize,
    /// Number of events remaining after compaction.
    pub events_after: usize,
    /// Events removed by merging repeated actions (e.g., duplicate COMMAND_RUN).
    pub merged_count: usize,
    /// Events removed by decay (FILE_READ excess, old GRAPH_STATE, old CONTEXT_REQUEST).
    pub decayed_count: usize,
    /// Events removed by deduplication (REASONING_RESULT with same source_id).
    pub deduplicated_count: usize,
    /// Events whose priority was raised to survive future eviction.
    pub promoted_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSnapshot {
    pub session_id: SessionId,
    pub snapshot: String,
    pub event_count: i64,
    pub consumed: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurableTaskStatus {
    Cancelled,
    Completed,
    Failed,
    InputRequired,
    Working,
}

impl DurableTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::InputRequired => "input_required",
            Self::Working => "working",
        }
    }
}

impl std::fmt::Display for DurableTaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for DurableTaskStatus {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "cancelled" => Ok(Self::Cancelled),
            "completed" => Ok(Self::Completed),
            "failed" => Ok(Self::Failed),
            "input_required" => Ok(Self::InputRequired),
            "working" => Ok(Self::Working),
            other => Err(AtlasError::Other(format!(
                "unknown durable task status: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DurableTaskRecord {
    pub task_id: String,
    pub originating_method: String,
    pub request_id: Option<String>,
    pub tool_name: Option<String>,
    pub transport_kind: Option<String>,
    pub session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub status: DurableTaskStatus,
    pub status_message: Option<String>,
    pub progress: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub ttl_ms: Option<u64>,
    pub cancel_requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDurableTask {
    pub task_id: String,
    pub originating_method: String,
    pub request_id: Option<String>,
    pub tool_name: Option<String>,
    pub transport_kind: Option<String>,
    pub session_id: Option<String>,
    pub status: DurableTaskStatus,
    pub status_message: Option<String>,
    pub ttl_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DurableTaskUpdate {
    pub status: Option<DurableTaskStatus>,
    pub status_message: Option<String>,
    pub progress: Option<Value>,
    pub result: Option<Value>,
    pub error: Option<Value>,
    pub cancel_requested: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DurableTaskListPage {
    pub tasks: Vec<DurableTaskRecord>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPartitionSummary {
    pub agent_id: Option<String>,
    pub event_count: usize,
    pub last_event_at: Option<String>,
    pub active_task_count: usize,
    pub completed_task_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DelegatedTaskSummary {
    pub task_id: String,
    pub title: String,
    pub status: String,
    pub agent_id: Option<String>,
    pub delegated_by: Option<String>,
    pub responsibility: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentResponsibilitySummary {
    pub agent_id: String,
    pub responsibilities: Vec<String>,
    pub active_task_count: usize,
    pub completed_task_count: usize,
    pub last_event_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentMemorySummary {
    pub merged_view: bool,
    pub requested_agent_id: Option<String>,
    pub partitions: Vec<AgentPartitionSummary>,
    pub delegated_tasks: Vec<DelegatedTaskSummary>,
    pub responsibilities: Vec<AgentResponsibilitySummary>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DecisionRecord {
    pub decision_id: String,
    pub session_id: String,
    pub repo_root: String,
    pub summary: String,
    pub rationale: Option<String>,
    pub conclusion: Option<String>,
    pub query_text: Option<String>,
    pub source_ids: Vec<String>,
    pub evidence: Vec<Value>,
    pub related_files: Vec<String>,
    pub related_symbols: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionSearchHit {
    pub decision: DecisionRecord,
    pub relevance_score: f32,
    pub matched_terms: Vec<String>,
}

/// A frequently-accessed symbol or file aggregated across all sessions.
///
/// Used by the global memory layer (CM11) to surface recurring access patterns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalAccessEntry {
    /// Stable ID: hex-encoded SHA-256 of `{repo_root}:{value}`.
    pub id: String,
    pub repo_root: String,
    /// Symbol qualified name (for symbol entries) or canonical file path (for file entries).
    pub value: String,
    pub access_count: u64,
    pub last_accessed: String,
    pub first_accessed: String,
}

/// A recurring workflow pattern detected across sessions in a single repo.
///
/// `pattern` is an ordered list of command strings or event-type tokens that
/// appear together repeatedly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalWorkflowPattern {
    /// Stable ID: hex-encoded SHA-256 of `{repo_root}:{pattern_json}`.
    pub id: String,
    pub repo_root: String,
    /// Ordered sequence of command strings or event-type tokens.
    pub pattern: Vec<String>,
    pub occurrence_count: u64,
    pub last_seen: String,
    pub first_seen: String,
}

// ── ICM-A — Shared memory model ───────────────────────────────────────────────
// These types form the single memory record shape shared by CLI and MCP so the
// two surfaces cannot drift on defaults, validation, or visibility semantics.

/// Importance of a memory record. Exact values: `critical`, `high`, `normal`, `low`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryImportance {
    Critical,
    High,
    #[default]
    Normal,
    Low,
}

impl MemoryImportance {
    pub const ALL: [Self; 4] = [Self::Critical, Self::High, Self::Normal, Self::Low];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Critical => "critical",
            Self::High => "high",
            Self::Normal => "normal",
            Self::Low => "low",
        }
    }
}

impl std::fmt::Display for MemoryImportance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MemoryImportance {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "critical" => Ok(Self::Critical),
            "high" => Ok(Self::High),
            "normal" => Ok(Self::Normal),
            "low" => Ok(Self::Low),
            other => Err(AtlasError::Other(format!(
                "unknown memory importance: {other}; expected one of critical, high, normal, low"
            ))),
        }
    }
}

/// Visibility scope of a memory record. Exact values: `project`, `session`,
/// `frontend`, `global`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    #[default]
    Project,
    Session,
    Frontend,
    Global,
}

impl MemoryScope {
    pub const ALL: [Self; 4] = [Self::Project, Self::Session, Self::Frontend, Self::Global];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Session => "session",
            Self::Frontend => "frontend",
            Self::Global => "global",
        }
    }
}

impl std::fmt::Display for MemoryScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for MemoryScope {
    type Err = AtlasError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "project" => Ok(Self::Project),
            "session" => Ok(Self::Session),
            "frontend" => Ok(Self::Frontend),
            "global" => Ok(Self::Global),
            other => Err(AtlasError::Other(format!(
                "unknown memory scope: {other}; expected one of project, session, frontend, global"
            ))),
        }
    }
}

/// A stored memory record as persisted in the `memories` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecord {
    pub id: String,
    pub repo_root: String,
    pub session_id: Option<String>,
    pub frontend: Option<String>,
    pub scope: MemoryScope,
    pub topic: String,
    pub title: String,
    pub body: String,
    pub importance: MemoryImportance,
    pub created_at: String,
    pub updated_at: String,
    pub last_accessed_at: String,
    pub decay_score: f64,
    pub source_id: Option<String>,
    /// Free-form JSON metadata (column `metadata_json`).
    pub metadata: Value,
}

/// Input shape for a manual memory write shared by CLI and MCP surfaces.
///
/// Defaults: `importance` is `normal`, `scope` is `project`. Call
/// [`NewMemory::validate`] before persisting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewMemory {
    pub repo_root: String,
    pub session_id: Option<String>,
    pub frontend: Option<String>,
    #[serde(default)]
    pub scope: MemoryScope,
    #[serde(default)]
    pub topic: String,
    #[serde(default)]
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub importance: MemoryImportance,
    pub source_id: Option<String>,
    #[serde(default = "default_memory_metadata")]
    pub metadata: Value,
}

fn default_memory_metadata() -> Value {
    Value::Object(Default::default())
}

impl Default for NewMemory {
    fn default() -> Self {
        Self {
            repo_root: String::new(),
            session_id: None,
            frontend: None,
            scope: MemoryScope::default(),
            topic: String::new(),
            title: String::new(),
            body: String::new(),
            importance: MemoryImportance::default(),
            source_id: None,
            metadata: default_memory_metadata(),
        }
    }
}

impl NewMemory {
    /// Rejects invalid memory writes before they reach storage: `frontend`
    /// scoped memories require a frontend identifier, `session` scoped
    /// memories require a session id, and the body must not be empty.
    pub fn validate(&self) -> atlas_core::Result<()> {
        if self.body.trim().is_empty() {
            return Err(AtlasError::Other(
                "memory body must not be empty".to_owned(),
            ));
        }
        if self.scope == MemoryScope::Frontend
            && self
                .frontend
                .as_deref()
                .map(str::trim)
                .is_none_or(str::is_empty)
        {
            return Err(AtlasError::Other(
                "scope 'frontend' requires a frontend identifier".to_owned(),
            ));
        }
        if self.scope == MemoryScope::Session
            && self
                .session_id
                .as_deref()
                .map(str::trim)
                .is_none_or(str::is_empty)
        {
            return Err(AtlasError::Other(
                "scope 'session' requires a session_id".to_owned(),
            ));
        }
        Ok(())
    }
}

/// A recall hit pairing a memory record with its lexical match tier.
///
/// Lower `relevance_score` ranks higher: `0` = exact topic match, `1` =
/// topic/title contains match, `2` = body-only match.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemorySearchHit {
    pub memory: MemoryRecord,
    pub relevance_score: i32,
}

/// Filters shared by memory recall and list surfaces.
///
/// Timestamps are normalized RFC 3339 strings (second precision) so string
/// comparison equals chronological comparison.
#[derive(Debug, Clone, Default)]
pub struct MemoryListFilter {
    /// Case-insensitive exact topic match.
    pub topic: Option<String>,
    pub importance: Option<MemoryImportance>,
    pub scope: Option<MemoryScope>,
    /// Only memories updated before this timestamp.
    pub older_than: Option<String>,
    /// Only memories updated after this timestamp.
    pub newer_than: Option<String>,
}

/// Outcome of a memory delete, including dry-run inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDeleteResult {
    pub memory_id: String,
    /// Whether a memory with the exact id exists in this repo.
    pub found: bool,
    /// Whether a row was actually removed (false for dry-run).
    pub deleted: bool,
    pub dry_run: bool,
}

/// Identifies who is viewing memories so recall can enforce visibility rules.
///
/// Visibility (ICM-A3): `global` visible everywhere, `project` visible to all
/// frontends in the repo, `session` visible only to the same session, and
/// `frontend` visible only to the same repo plus the same frontend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryViewer {
    /// Canonical frontend identity (`claude`, `codex`, `copilot`, `cli`, `mcp`).
    pub frontend: String,
    /// Viewer session id; only `session`-scoped memories with the same id are visible.
    pub session_id: String,
}
