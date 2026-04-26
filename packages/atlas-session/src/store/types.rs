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
