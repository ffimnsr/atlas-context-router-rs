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
    GlobalWorkflowPattern, MAX_INLINE_EVENT_PAYLOAD_BYTES, MEMOIR_RELATION_TYPES, MemoirConcept,
    MemoirGraph, MemoirInspectQuery, MemoirInspection, MemoirInspectionEdge, MemoirRelation,
    MemoirRelationDirection, MemoryConsolidationGroup, MemoryConsolidationPlan, MemoryDecayPolicy,
    MemoryDecayReport, MemoryDeleteResult, MemoryHealthCategory, MemoryHealthFinding,
    MemoryHealthReport, MemoryImportance, MemoryListFilter, MemoryPruneResult, MemoryRecallRanking,
    MemoryRecord, MemoryScope, MemorySearchHit, MemorySupersessionLink, MemoryViewer,
    NOISY_TOPIC_ENTRIES, NewDurableTask, NewFeedback, NewMemoirConcept, NewMemoirGraph,
    NewMemoirRelation, NewMemory, NewOverviewRecord, NewSessionEvent, OVERSIZED_BODY_CHARS,
    OVERVIEW_DESCRIPTION_PREVIEW_MAX_CHARS, OVERVIEW_INLINE_BODY_BUDGET_BYTES,
    OVERVIEW_SCHEMA_VERSION, OverviewAnalysisInput, OverviewAnalysisPacket, OverviewBudgetMeta,
    OverviewCodeExcerpt, OverviewContextHit, OverviewContextQuery, OverviewDeleteResult,
    OverviewExportRequest, OverviewExportScope, OverviewFreshness, OverviewGraphProvenance,
    OverviewListFilter, OverviewPacketBudget, OverviewPacketEvidence, OverviewPacketFreshness,
    OverviewRanking, OverviewRecord, OverviewSearchFilter, OverviewSearchHit, OverviewSourceKind,
    OverviewStaleFilter, OverviewSubject, OverviewSubjectKind, ResumeSnapshot, SessionEventRow,
    SessionEventType, SessionMeta, SessionStats, SessionStore, SessionStoreConfig,
    bound_new_overview_record, canonicalize_subject_path, normalize_memoir_relation, packet_id,
    subject_id, truncate_chars, validate_confidence,
};
