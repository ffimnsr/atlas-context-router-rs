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
    /// Deterministic rollup written before curation deletes low-level events
    /// (VM2 `TIME_COMPRESS`).
    SessionSummary,
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
            Self::SessionSummary => EventCategory::Reasoning,
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
            Self::SessionSummary => "SESSION_SUMMARY",
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
            "SESSION_SUMMARY" => Ok(Self::SessionSummary),
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
    pub input_requests: Option<Value>,
    pub request_state: Option<String>,
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
    pub input_requests: Option<Value>,
    pub request_state: Option<String>,
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
    /// Id of the consolidated memory that replaced this row (ICM-B3), or
    /// `None` when the row is still active. Set by consolidation apply.
    pub superseded_by: Option<String>,
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

/// A recall hit pairing a memory record with its lexical match tier and
/// cross-session ranking evidence.
///
/// Lower `relevance_score` ranks higher: `0` = exact topic match, `1` =
/// topic/title contains match, `2` = body-only match, `3` = vector-only match
/// added by hybrid recall (ICM-E2). `recall_score` is the higher-is-better
/// composite score described by [`MemoryRecallRanking`]; the lexical tier
/// always dominates so exact keyword evidence is never buried.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemorySearchHit {
    pub memory: MemoryRecord,
    pub relevance_score: i32,
    /// Raw FTS5 BM25 value for primary-path hits; `0.0` for auxiliary hits.
    #[serde(default)]
    pub bm25_score: f64,
    pub recall_score: f64,
    pub ranking: MemoryRecallRanking,
}

/// Deterministic ranking evidence for one recall hit (ICM-E1).
///
/// Ranking is lexical-first: `topic_match_tier` dominates, then importance,
/// broad scope visibility, source-backed evidence, and recency. Lower ranks
/// are stronger for every `*_rank` field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryRecallRanking {
    /// Human-readable match tier: `exact_topic`, `topic_or_title`, or `body`.
    pub topic_match: String,
    /// Numeric match tier (0 = exact topic, 1 = topic/title, 2 = body).
    pub topic_match_tier: u8,
    /// Importance rank (0 = critical, 1 = high, 2 = normal, 3 = low).
    pub importance_rank: u8,
    /// Scope-visibility rank (0 = global, 1 = project, 2 = frontend, 3 = session).
    pub scope_rank: u8,
    /// Whether the memory carries a non-empty `source_id`.
    pub source_backed: bool,
    /// Whether consolidation superseded this memory.
    pub superseded: bool,
    /// Whole days since `updated_at` (0 when unknown or in the future).
    pub age_days: u32,
    /// Normalized origin score combining lexical evidence, importance, scope,
    /// and source-backed evidence (VM3).
    #[serde(default)]
    pub origin_score: f64,
    /// Freshness score: `1.0` inside the freshness window, then
    /// `1 / (1 + age_days / 30)`.
    #[serde(default)]
    pub time_score: f64,
    /// Business-importance score (`critical = 1.0`, `high = 0.75`,
    /// `normal = 0.5`, `low = 0.25`).
    #[serde(default)]
    pub importance_score: f64,
    /// Which recall path produced this hit: `primary`, `auxiliary`, or
    /// `vector` (ICM-E2 hybrid recall only).
    #[serde(default)]
    pub recall_path: String,
    /// Cosine similarity against the configured embedding model when the
    /// hybrid path scored this memory; `None` for lexical-only recall.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vector_score: Option<f64>,
    /// Reciprocal-rank-fusion score for hybrid recall; `None` for lexical-only
    /// recall. Exact lexical tiers still outrank vector-only hits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hybrid_score: Option<f64>,
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
    /// Include rows already superseded by consolidation (ICM-B3). Default
    /// `false`: superseded rows are hidden from normal listing.
    pub include_superseded: bool,
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

/// Recall scoring and quota policy mirrored from `[memory.recall]` (VM3).
///
/// Lexical recall stays the default; the auxiliary memoir path only widens the
/// candidate set and never pools its raw scores with the primary path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemoryRecallPolicy {
    /// Memories updated within this many days keep `time_score = 1.0`.
    pub freshness_window_days: u32,
    /// Composite weight for the freshness signal (`0..=1`).
    pub weight_time: f64,
    /// Composite weight for the business-importance signal (`0..=1`).
    pub weight_business: f64,
    /// Fraction of `limit` reserved for the primary lexical path (`0..=1`).
    pub primary_quota_ratio: f64,
    /// Maximum memoir relation hops for the auxiliary path (`1..=2`).
    pub aux_max_hops: u32,
}

impl Default for MemoryRecallPolicy {
    fn default() -> Self {
        Self {
            freshness_window_days: DEFAULT_RECALL_FRESHNESS_WINDOW_DAYS,
            weight_time: DEFAULT_RECALL_WEIGHT_TIME,
            weight_business: DEFAULT_RECALL_WEIGHT_BUSINESS,
            primary_quota_ratio: DEFAULT_RECALL_PRIMARY_QUOTA_RATIO,
            aux_max_hops: DEFAULT_RECALL_AUX_MAX_HOPS,
        }
    }
}

impl MemoryRecallPolicy {
    /// Validates weights, quota, and hop bounds; used by `Config::load`.
    pub fn validate(&self) -> atlas_core::Result<()> {
        if !self.weight_time.is_finite() || !(0.0..=1.0).contains(&self.weight_time) {
            return Err(AtlasError::Other(
                "memory.recall.weight_time must be between 0.0 and 1.0".to_owned(),
            ));
        }
        if !self.weight_business.is_finite() || !(0.0..=1.0).contains(&self.weight_business) {
            return Err(AtlasError::Other(
                "memory.recall.weight_business must be between 0.0 and 1.0".to_owned(),
            ));
        }
        if self.weight_time + self.weight_business >= 1.0 {
            return Err(AtlasError::Other(
                "memory.recall.weight_time + memory.recall.weight_business must be less than 1.0"
                    .to_owned(),
            ));
        }
        if !self.primary_quota_ratio.is_finite() || !(0.0..=1.0).contains(&self.primary_quota_ratio)
        {
            return Err(AtlasError::Other(
                "memory.recall.primary_quota_ratio must be between 0.0 and 1.0".to_owned(),
            ));
        }
        if !(1..=2).contains(&self.aux_max_hops) {
            return Err(AtlasError::Other(
                "memory.recall.aux_max_hops must be 1 or 2".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Default freshness tolerance window in days (`[memory.recall]`).
pub const DEFAULT_RECALL_FRESHNESS_WINDOW_DAYS: u32 = 1;
/// Default composite weight for freshness.
pub const DEFAULT_RECALL_WEIGHT_TIME: f64 = 0.25;
/// Default composite weight for business importance.
pub const DEFAULT_RECALL_WEIGHT_BUSINESS: f64 = 0.15;
/// Default quota reserved for the primary lexical path.
pub const DEFAULT_RECALL_PRIMARY_QUOTA_RATIO: f64 = 0.75;
/// Default maximum memoir hops for the auxiliary path.
pub const DEFAULT_RECALL_AUX_MAX_HOPS: u32 = 2;
/// Default reciprocal-rank-fusion weight for the lexical path.
pub const DEFAULT_HYBRID_WEIGHT_FTS: f64 = 0.7;
/// Default reciprocal-rank-fusion weight for the vector path.
pub const DEFAULT_HYBRID_WEIGHT_VECTOR: f64 = 0.3;
/// RRF k constant shared with hybrid graph retrieval.
pub const MEMORY_HYBRID_RRF_K: u32 = 60;
/// Minimum cosine similarity for a vector-only recall candidate.
pub const DEFAULT_MEMORY_VECTOR_MIN_SIMILARITY: f64 = 0.30;
/// Upper bound on memories scanned by the vector path per recall call.
pub const MEMORY_VECTOR_SCAN_LIMIT: usize = 4096;

/// Hybrid recall fusion policy mirrored from `[memory.search]` (ICM-E2).
///
/// The lexical path stays authoritative: fusion only reranks lexical hits and
/// appends vector-only matches after them. Raw lexical and vector scores are
/// never pooled into one number.
#[derive(Debug, Clone, PartialEq)]
pub struct MemoryEmbeddingPolicy {
    /// RRF weight for the lexical ranking (`0..=1`).
    pub weight_fts: f64,
    /// RRF weight for the vector ranking (`0..=1`).
    pub weight_vector: f64,
    /// RRF k constant; higher values flatten rank-position sensitivity.
    pub rrf_k: u32,
    /// Minimum cosine similarity before a vector-only hit is emitted.
    pub min_similarity: f64,
}

impl Default for MemoryEmbeddingPolicy {
    fn default() -> Self {
        Self {
            weight_fts: DEFAULT_HYBRID_WEIGHT_FTS,
            weight_vector: DEFAULT_HYBRID_WEIGHT_VECTOR,
            rrf_k: MEMORY_HYBRID_RRF_K,
            min_similarity: DEFAULT_MEMORY_VECTOR_MIN_SIMILARITY,
        }
    }
}

impl MemoryEmbeddingPolicy {
    /// Validates weights and the RRF constant.
    pub fn validate(&self) -> atlas_core::Result<()> {
        if !self.weight_fts.is_finite() || !(0.0..=1.0).contains(&self.weight_fts) {
            return Err(AtlasError::Other(
                "memory.search.hybrid_weight_fts must be between 0.0 and 1.0".to_owned(),
            ));
        }
        if !self.weight_vector.is_finite() || !(0.0..=1.0).contains(&self.weight_vector) {
            return Err(AtlasError::Other(
                "memory.search.hybrid_weight_vector must be between 0.0 and 1.0".to_owned(),
            ));
        }
        if self.weight_fts + self.weight_vector <= 0.0 {
            return Err(AtlasError::Other(
                "memory.search.hybrid_weight_fts + memory.search.hybrid_weight_vector must be greater than 0.0".to_owned(),
            ));
        }
        if self.rrf_k == 0 {
            return Err(AtlasError::Other(
                "memory.search.rrf_k must be greater than 0".to_owned(),
            ));
        }
        if !self.min_similarity.is_finite() || !(0.0..=1.0).contains(&self.min_similarity) {
            return Err(AtlasError::Other(
                "memory.search.min_vector_similarity must be between 0.0 and 1.0".to_owned(),
            ));
        }
        Ok(())
    }
}

/// Embedding coverage for one repo and model (ICM-E2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEmbeddingCoverage {
    /// Embedding model the coverage was measured for.
    pub embedding_model: String,
    /// Vector dimension stored for that model.
    pub dimension: usize,
    /// Number of recallable memories with a compatible stored vector.
    pub embedded: usize,
    /// Number of recallable memories without a compatible stored vector.
    pub missing: usize,
}

// ── ICM-B — memory curation (decay, stale, prune, health, consolidation) ──────

/// Default retention (days) per importance when `memory.decay` config is absent.
pub const DEFAULT_DECAY_LOW_DAYS: u32 = 30;
pub const DEFAULT_DECAY_NORMAL_DAYS: u32 = 90;
pub const DEFAULT_DECAY_HIGH_DAYS: u32 = 365;

/// Topic entry count above which `atlas memory health` reports the topic as noisy.
pub const NOISY_TOPIC_ENTRIES: usize = 10;

/// Body length (characters) above which `atlas memory health` reports the
/// memory as oversized.
pub const OVERSIZED_BODY_CHARS: usize = 2000;

/// Decay policy mirrored from `[memory.decay]` in `.atlas/config.toml`.
///
/// `decay_score` grows from `0.0` (fresh) toward `1.0` (pruneable) as a
/// memory ages past its retention window. `critical` memories are protected:
/// they never decay and are never auto-prune candidates unless
/// `critical_never_prune` is explicitly disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryDecayPolicy {
    pub enabled: bool,
    pub low_days: u32,
    pub normal_days: u32,
    pub high_days: u32,
    pub critical_never_prune: bool,
}

impl Default for MemoryDecayPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            low_days: DEFAULT_DECAY_LOW_DAYS,
            normal_days: DEFAULT_DECAY_NORMAL_DAYS,
            high_days: DEFAULT_DECAY_HIGH_DAYS,
            critical_never_prune: true,
        }
    }
}

impl MemoryDecayPolicy {
    /// Retention window in days for an importance level; `None` means the row
    /// never decays (critical + never-prune).
    pub fn retention_days(&self, importance: MemoryImportance) -> Option<u32> {
        match importance {
            MemoryImportance::Critical if self.critical_never_prune => None,
            MemoryImportance::Critical | MemoryImportance::High => Some(self.high_days),
            MemoryImportance::Normal => Some(self.normal_days),
            MemoryImportance::Low => Some(self.low_days),
        }
    }

    /// Whether the row is protected from auto-pruning by policy.
    pub fn is_protected(&self, importance: MemoryImportance) -> bool {
        importance == MemoryImportance::Critical && self.critical_never_prune
    }

    /// Updated decay score for a memory of `importance` that is `age_days` old.
    ///
    /// Protected critical memories always score `0.0`; everything else decays
    /// linearly to `1.0` at the end of its retention window.
    pub fn score(&self, importance: MemoryImportance, age_days: f64) -> f64 {
        let Some(retention) = self.retention_days(importance) else {
            return 0.0;
        };
        (age_days / f64::from(retention)).clamp(0.0, 1.0)
    }
}

/// Per-memory decay computation from `atlas memory decay`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryDecayReport {
    pub memory: MemoryRecord,
    /// Age of the memory in days, derived from `updated_at`.
    pub age_days: f64,
    /// Retention window in days for this memory's importance; `None` when
    /// policy protects the row from ever decaying (critical + never-prune).
    pub retention_days: Option<u32>,
    /// Score that would be (or was) written back when not dry-running.
    pub updated_decay_score: f64,
    /// Whether policy protects this row from auto-pruning.
    pub protected: bool,
    /// Whether the row is past its retention window and not protected.
    pub stale: bool,
}

/// Outcome of `atlas memory prune` (dry-run or applied).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryPruneResult {
    pub dry_run: bool,
    /// Rows past their retention window and selected by filters.
    pub candidate_count: usize,
    /// Rows actually deleted (0 for dry-run).
    pub deleted_count: usize,
    /// Critical rows skipped because `critical_never_prune` is on and no
    /// explicit override was given.
    pub protected_count: usize,
    pub candidates: Vec<MemoryRecord>,
}

/// Deterministic health categories from `atlas memory health`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryHealthCategory {
    Healthy,
    Stale,
    Noisy,
    Duplicated,
    Orphaned,
    Oversized,
}

impl MemoryHealthCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Stale => "stale",
            Self::Noisy => "noisy",
            Self::Duplicated => "duplicated",
            Self::Orphaned => "orphaned",
            Self::Oversized => "oversized",
        }
    }
}

impl std::fmt::Display for MemoryHealthCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One deterministic health finding with an actionable follow-up command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryHealthFinding {
    pub category: MemoryHealthCategory,
    /// Stable sub-kind: `stale_memory`, `duplicated_memory`, `orphaned_source`,
    /// `oversized_memory`, `noisy_topic`, `topic_without_critical`.
    pub kind: String,
    /// Memory id the finding applies to; `None` for topic-level findings.
    pub memory_id: Option<String>,
    /// Topic the finding applies to.
    pub topic: Option<String>,
    pub detail: String,
    pub suggestion: String,
    /// Exact follow-up command for the human output.
    pub command: String,
}

/// Deterministic health report from `atlas memory health`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryHealthReport {
    pub total_memories: usize,
    pub findings: Vec<MemoryHealthFinding>,
    /// Count of findings per category (healthy rows are never findings).
    pub by_category: std::collections::BTreeMap<String, usize>,
}

/// One deterministic consolidation group from `atlas memory consolidate`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryConsolidationGroup {
    pub topic: String,
    /// Deterministic grouping reason: `same_title`, `same_body`, `same_source`,
    /// or `same_category`.
    pub group_key: String,
    /// Representative memory kept as the consolidated row.
    pub kept_memory_id: String,
    /// Other rows merged into the consolidated row.
    pub merged_memory_ids: Vec<String>,
    /// Every distinct non-null `source_id` in the group (preserved in the
    /// consolidated record metadata).
    pub source_ids: Vec<String>,
    /// Deterministic synthesized body containing every distinct fact from all
    /// members in chronological order (truncated on overflow).
    pub body: String,
    /// Full merged metadata JSON written to the consolidated row.
    pub metadata_json: String,
    /// Every distinct member `source_id` preserved by consolidation (same
    /// value set as [`Self::source_ids`], kept for JSON consumers).
    pub merged_source_ids: Vec<String>,
    /// Set when the synthesized body exceeded `OVERSIZED_BODY_CHARS` and the
    /// full text was routed to `context.db`.
    pub overflow_source_id: Option<String>,
    /// Id of the consolidated memory; `Some` only after apply mode ran.
    pub consolidated_id: Option<String>,
}

/// Consolidation plan from `atlas memory consolidate` (dry-run or applied).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryConsolidationPlan {
    pub dry_run: bool,
    pub groups: Vec<MemoryConsolidationGroup>,
    pub kept_ids: Vec<String>,
    pub merged_ids: Vec<String>,
}

/// One persisted supersession link (table `memory_supersessions`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct MemorySupersessionLink {
    pub old_memory_id: String,
    pub new_memory_id: String,
    pub reason: String,
    pub created_at: String,
}

// ── ICM-C — feedback records (predicted vs actual corrections) ───────────────

/// Input shape for a manual feedback write shared by CLI and MCP surfaces.
///
/// `predicted` and `actual` are required; everything else is optional
/// enrichment. Call [`NewFeedback::validate`] before persisting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct NewFeedback {
    pub repo_root: String,
    pub session_id: Option<String>,
    /// Tool that produced the prediction (e.g. `cli`, `mcp`).
    #[serde(default)]
    pub tool_name: String,
    /// Analysis kind the prediction came from (e.g. `dead_code`, `remove`,
    /// `safety`, `remove_dead`).
    #[serde(default)]
    pub analysis_kind: String,
    /// What the analysis predicted; required.
    pub predicted: String,
    /// What actually happened; required. `actual != predicted` marks the
    /// record as false-positive evidence for confidence adjustment.
    pub actual: String,
    /// Optional correction text explaining the right answer.
    #[serde(default)]
    pub correction: String,
    /// Symbol the prediction was about (exact qualified name).
    pub related_symbol: Option<String>,
    /// File the prediction was about (repo-relative path).
    pub related_file: Option<String>,
    /// Optional link to a saved-context artifact.
    pub source_id: Option<String>,
    #[serde(default = "default_memory_metadata")]
    pub metadata: Value,
}

impl NewFeedback {
    /// Rejects invalid feedback writes before they reach storage: both
    /// `predicted` and `actual` must be non-empty.
    pub fn validate(&self) -> atlas_core::Result<()> {
        if self.predicted.trim().is_empty() {
            return Err(AtlasError::Other(
                "feedback predicted must not be empty".to_owned(),
            ));
        }
        if self.actual.trim().is_empty() {
            return Err(AtlasError::Other(
                "feedback actual must not be empty".to_owned(),
            ));
        }
        Ok(())
    }
}

/// A stored feedback record as persisted in the `feedback_records` table.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FeedbackRecord {
    pub id: String,
    pub repo_root: String,
    pub session_id: Option<String>,
    pub tool_name: String,
    pub analysis_kind: String,
    pub predicted: String,
    pub actual: String,
    pub correction: String,
    pub related_symbol: Option<String>,
    pub related_file: Option<String>,
    pub source_id: Option<String>,
    pub created_at: String,
    pub metadata: Value,
}

impl FeedbackRecord {
    /// Whether this record is false-positive evidence for confidence
    /// adjustment: the actual outcome differs from the prediction, or the
    /// metadata explicitly marks `false_positive`.
    pub fn is_false_positive_evidence(&self) -> bool {
        self.metadata.get("false_positive") == Some(&Value::Bool(true))
            || normalize_feedback_text(&self.predicted) != normalize_feedback_text(&self.actual)
    }
}

/// Collapse whitespace and lowercase for deterministic text comparison.
fn normalize_feedback_text(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// A feedback search hit pairing a record with its relevance score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FeedbackSearchHit {
    pub feedback: FeedbackRecord,
    /// Higher is more relevant; `0.0` for LIKE-fallback matches.
    pub relevance_score: f32,
}

/// Exact-match filters applied on top of feedback search.
#[derive(Debug, Clone, Default)]
pub struct FeedbackSearchFilter {
    pub tool_name: Option<String>,
    pub analysis_kind: Option<String>,
    pub related_symbol: Option<String>,
    pub related_file: Option<String>,
}

/// Deterministic feedback statistics (ICM-C2 `atlas feedback stats`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct FeedbackStats {
    pub total_count: usize,
    /// Records carrying a correction text.
    pub correction_count: usize,
    /// Records where `actual != predicted` (or metadata marks false positive).
    pub false_positive_count: usize,
    pub by_analysis_kind: std::collections::BTreeMap<String, usize>,
    pub by_tool: std::collections::BTreeMap<String, usize>,
}
