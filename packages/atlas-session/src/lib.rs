#![doc = include_str!("../README.md")]

pub mod frontend;
pub mod identity;
mod migrations;
pub mod store;

pub use frontend::{KNOWN_FRONTENDS, normalize_frontend};
pub use identity::SessionId;
pub use store::{
    AgentMemorySummary, AgentPartitionSummary, AgentResponsibilitySummary, CurationResult,
    DEFAULT_DEDUP_WINDOW_SECS, DEFAULT_MAX_SNAPSHOT_BYTES, DEFAULT_SESSION_DB,
    DEFAULT_SESSION_MAX_EVENTS, DecisionRecord, DecisionSearchHit, DelegatedTaskSummary,
    DurableTaskListPage, DurableTaskRecord, DurableTaskStatus, DurableTaskUpdate, EventCategory,
    FeedbackRecord, FeedbackSearchFilter, FeedbackSearchHit, FeedbackStats, GlobalAccessEntry,
    GlobalWorkflowPattern, MAX_INLINE_EVENT_PAYLOAD_BYTES, MemoryConsolidationGroup,
    MemoryConsolidationPlan, MemoryDecayPolicy, MemoryDecayReport, MemoryDeleteResult,
    MemoryHealthCategory, MemoryHealthFinding, MemoryHealthReport, MemoryImportance,
    MemoryListFilter, MemoryPruneResult, MemoryRecord, MemoryScope, MemorySearchHit,
    MemorySupersessionLink, MemoryViewer, NOISY_TOPIC_ENTRIES, NewDurableTask, NewFeedback,
    NewMemory, NewSessionEvent, OVERSIZED_BODY_CHARS, ResumeSnapshot, SessionEventRow,
    SessionEventType, SessionMeta, SessionStats, SessionStore, SessionStoreConfig,
};
