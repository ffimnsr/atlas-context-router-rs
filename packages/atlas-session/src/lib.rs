#![doc = include_str!("../README.md")]

pub mod identity;
mod migrations;
pub mod store;

pub use identity::SessionId;
pub use store::{
    AgentMemorySummary, AgentPartitionSummary, AgentResponsibilitySummary, CurationResult,
    DEFAULT_DEDUP_WINDOW_SECS, DEFAULT_MAX_SNAPSHOT_BYTES, DEFAULT_SESSION_DB,
    DEFAULT_SESSION_MAX_EVENTS, DecisionRecord, DecisionSearchHit, DelegatedTaskSummary,
    EventCategory, GlobalAccessEntry, GlobalWorkflowPattern, MAX_INLINE_EVENT_PAYLOAD_BYTES,
    NewSessionEvent, ResumeSnapshot, SessionEventRow, SessionEventType, SessionMeta, SessionStats,
    SessionStore, SessionStoreConfig,
};
