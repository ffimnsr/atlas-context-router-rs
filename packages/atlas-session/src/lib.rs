#![doc = include_str!("../README.md")]

pub mod frontend;
pub mod identity;
mod migrations;
pub mod store;

pub use frontend::{KNOWN_FRONTENDS, normalize_frontend};
pub use identity::SessionId;
pub use store::{
    AgentMemorySummary, AgentPartitionSummary, AgentResponsibilitySummary,
    CONSOLIDATION_OVERFLOW_SOURCE_TYPE, ConsolidatedBody, CurationResult,
    DEFAULT_DEDUP_WINDOW_SECS, DEFAULT_EXPERIENCE_ACTIVATION_COSINE,
    DEFAULT_EXPERIENCE_ACTIVATION_JACCARD, DEFAULT_EXPERIENCE_MAX_EDGES,
    DEFAULT_EXPERIENCE_PRUNE_DAYS, DEFAULT_MAX_SNAPSHOT_BYTES, DEFAULT_RECALL_AUX_MAX_HOPS,
    DEFAULT_RECALL_FRESHNESS_WINDOW_DAYS, DEFAULT_RECALL_PRIMARY_QUOTA_RATIO,
    DEFAULT_RECALL_WEIGHT_BUSINESS, DEFAULT_RECALL_WEIGHT_TIME, DEFAULT_SESSION_DB,
    DEFAULT_SESSION_MAX_EVENTS, DecisionRecord, DecisionSearchHit, DelegatedTaskSummary,
    DurableTaskListPage, DurableTaskRecord, DurableTaskStatus, DurableTaskUpdate,
    EXPERIENCE_QUERY_TOKEN_CAP, EXPERIENCE_TRACE_SUMMARY_MAX_CHARS, EventCategory,
    ExperienceDirection, ExperienceEdgeRecord, ExperienceFreshness, FeedbackRecord,
    FeedbackSearchFilter, FeedbackSearchHit, FeedbackStats, GlobalAccessEntry,
    GlobalWorkflowPattern, MAX_INLINE_EVENT_PAYLOAD_BYTES, MEMOIR_RELATION_TYPES, MemoirConcept,
    MemoirGraph, MemoirInspectQuery, MemoirInspection, MemoirInspectionEdge, MemoirRelation,
    MemoirRelationDirection, MemoryConsolidationGroup, MemoryConsolidationPlan, MemoryDecayPolicy,
    MemoryDecayReport, MemoryDeleteResult, MemoryEntity, MemoryHealthCategory, MemoryHealthFinding,
    MemoryHealthReport, MemoryImportance, MemoryListFilter, MemoryOp, MemoryOpHistory,
    MemoryOpHistoryEntry, MemoryPruneResult, MemoryRecallPolicy, MemoryRecallRanking, MemoryRecord,
    MemoryScope, MemorySearchHit, MemorySupersessionLink, MemoryViewer, NOISY_TOPIC_ENTRIES,
    NewDurableTask, NewExperienceEdge, NewFeedback, NewMemoirConcept, NewMemoirGraph,
    NewMemoirRelation, NewMemory, NewOverviewRecord, NewSessionEvent, OVERSIZED_BODY_CHARS,
    OVERVIEW_DESCRIPTION_PREVIEW_MAX_CHARS, OVERVIEW_INLINE_BODY_BUDGET_BYTES,
    OVERVIEW_L0_MAX_CHARS, OVERVIEW_L1_MAX_CHARS, OVERVIEW_L2_MAX_CHARS, OVERVIEW_SCHEMA_VERSION,
    OpOutcome, OverviewAnalysisInput, OverviewAnalysisPacket, OverviewBudgetMeta,
    OverviewCodeExcerpt, OverviewContextHit, OverviewContextQuery, OverviewDeleteResult,
    OverviewExportRequest, OverviewExportScope, OverviewFreshness, OverviewGraphProvenance,
    OverviewLevel, OverviewLevelView, OverviewListFilter, OverviewPacketBudget,
    OverviewPacketEvidence, OverviewPacketFreshness, OverviewRanking, OverviewRecord,
    OverviewSearchFilter, OverviewSearchHit, OverviewSourceKind, OverviewStaleFilter,
    OverviewSubject, OverviewSubjectKind, PatchApplication, PatchBlock, ResumeSnapshot,
    SessionEventRow, SessionEventType, SessionMeta, SessionStats, SessionStore, SessionStoreConfig,
    apply_patch_block, bound_new_overview_record, canonicalize_subject_path,
    normalize_memoir_relation, overflow_source_id, packet_id, parse_patch_blocks, render_level,
    subject_id, synthesize_consolidated_body, truncate_chars, validate_confidence,
};
