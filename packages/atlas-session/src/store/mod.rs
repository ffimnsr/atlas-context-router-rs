//! SQLite-backed session metadata, event ledger, and resume snapshots.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock};

use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::Deserialize;
use serde_json::Value;
use tracing::info;

use atlas_core::{AtlasError, Clock, Result, SystemClock};
use atlas_db_utils::{
    application_id, apply_atlas_pragmas, migrate_database_to, set_application_id,
};
use atlas_store_markdown::MarkdownStore;

use crate::SessionId;
use crate::migrations::{LATEST_VERSION, MIGRATION_SET};

mod agent_memory;
mod curation;
mod decision_memory;
mod experience_edges;
mod feedback;
mod global_memory;
mod markdown_import;
mod markdown_memory;
mod markdown_memory_sync;
mod markdown_records;
mod markdown_session;
mod memoir;
mod memoir_delete;
mod memoir_schema;
mod memory;
mod memory_consolidate;
mod memory_embeddings;
mod memory_ops;
mod memory_patch;
mod memory_recall;
mod memory_recall_aux;
mod overview;
mod resume;
#[cfg(test)]
mod tests;
mod types;
mod util;

use feedback::recent_feedback;

pub use self::experience_edges::{
    DEFAULT_EXPERIENCE_ACTIVATION_COSINE, DEFAULT_EXPERIENCE_ACTIVATION_JACCARD,
    DEFAULT_EXPERIENCE_MAX_EDGES, DEFAULT_EXPERIENCE_PRUNE_DAYS, EXPERIENCE_QUERY_TOKEN_CAP,
    EXPERIENCE_TRACE_SUMMARY_MAX_CHARS, ExperienceDirection, ExperienceEdgeRecord,
    ExperienceFreshness, NewExperienceEdge, experience_query_tokens,
};

pub use self::memoir::{
    MEMOIR_RELATION_TYPES, MemoirConcept, MemoirGraph, MemoirInspectQuery, MemoirInspection,
    MemoirInspectionEdge, MemoirRelation, MemoirRelationDirection, NewMemoirConcept,
    NewMemoirGraph, NewMemoirRelation, normalize_memoir_relation,
};
pub use self::memoir_delete::MemoirDeleteResult;
pub use self::overview::{
    NewOverviewRecord, OVERVIEW_DESCRIPTION_PREVIEW_MAX_CHARS, OVERVIEW_INLINE_BODY_BUDGET_BYTES,
    OVERVIEW_L0_MAX_CHARS, OVERVIEW_L1_MAX_CHARS, OVERVIEW_L2_MAX_CHARS,
    OVERVIEW_LIST_ITEM_MAX_CHARS, OVERVIEW_LIST_MAX_ITEMS, OVERVIEW_SCHEMA_VERSION,
    OVERVIEW_SUMMARY_MAX_CHARS, OVERVIEW_TITLE_MAX_CHARS, OverviewAnalysisInput,
    OverviewAnalysisPacket, OverviewBudgetMeta, OverviewCodeExcerpt, OverviewContextHit,
    OverviewContextQuery, OverviewDeleteResult, OverviewExportRequest, OverviewExportScope,
    OverviewFreshness, OverviewGraphProvenance, OverviewLevel, OverviewLevelView,
    OverviewListFilter, OverviewPacketBudget, OverviewPacketEvidence, OverviewPacketFreshness,
    OverviewRanking, OverviewRecord, OverviewSearchFilter, OverviewSearchHit, OverviewSourceKind,
    OverviewStaleFilter, OverviewSubject, OverviewSubjectInput, OverviewSubjectKind,
    bound_new_overview_record, canonicalize_subject_path, packet_id, render_level, subject_id,
    truncate_chars, validate_confidence,
};
pub use self::types::{
    AgentMemorySummary, AgentPartitionSummary, AgentResponsibilitySummary, CurationResult,
    DEFAULT_DEDUP_WINDOW_SECS, DEFAULT_HYBRID_WEIGHT_FTS, DEFAULT_HYBRID_WEIGHT_VECTOR,
    DEFAULT_MAX_SNAPSHOT_BYTES, DEFAULT_MEMORY_VECTOR_MIN_SIMILARITY, DEFAULT_RECALL_AUX_MAX_HOPS,
    DEFAULT_RECALL_FRESHNESS_WINDOW_DAYS, DEFAULT_RECALL_PRIMARY_QUOTA_RATIO,
    DEFAULT_RECALL_WEIGHT_BUSINESS, DEFAULT_RECALL_WEIGHT_TIME, DEFAULT_SESSION_DB,
    DEFAULT_SESSION_MAX_EVENTS, DecisionRecord, DecisionSearchHit, DelegatedTaskSummary,
    DurableTaskListPage, DurableTaskRecord, DurableTaskStatus, DurableTaskUpdate, EventCategory,
    FeedbackRecord, FeedbackSearchFilter, FeedbackSearchHit, FeedbackStats, GlobalAccessEntry,
    GlobalWorkflowPattern, MAX_INLINE_EVENT_PAYLOAD_BYTES, MEMORY_HYBRID_RRF_K,
    MEMORY_VECTOR_SCAN_LIMIT, MemoryConsolidationGroup, MemoryConsolidationPlan, MemoryDecayPolicy,
    MemoryDecayReport, MemoryDeleteResult, MemoryEmbeddingCoverage, MemoryEmbeddingPolicy,
    MemoryHealthCategory, MemoryHealthFinding, MemoryHealthReport, MemoryImportance, MemoryLink,
    MemoryListFilter, MemoryPruneResult, MemoryRecallPolicy, MemoryRecallRanking, MemoryRecord,
    MemoryScope, MemorySearchHit, MemorySupersessionLink, MemoryViewer, NOISY_TOPIC_ENTRIES,
    NewDurableTask, NewFeedback, NewMemory, NewSessionEvent, OVERSIZED_BODY_CHARS, ResumeSnapshot,
    SessionEventRow, SessionEventType, SessionMeta, SessionStats, SessionStoreConfig,
};

pub use self::markdown_memory::MarkdownMigrationReport;
pub use self::memory_consolidate::{
    CONSOLIDATION_OVERFLOW_SOURCE_TYPE, ConsolidatedBody, OverflowSink, overflow_source_id,
    route_overflow_body, synthesize_consolidated_body,
};
pub use self::memory_ops::{
    FAILURE_SIGNATURE_CAP, FAILURE_TOPIC_BUILD, FAILURE_TOPIC_TESTS, MAX_OP_HISTORY_ENTRIES,
    MemoryEntity, MemoryOp, MemoryOpHistory, MemoryOpHistoryEntry, OpOutcome, apply_memory_op,
};
pub use self::memory_patch::{PatchApplication, PatchBlock, apply_patch_block, parse_patch_blocks};

use self::agent_memory::{summarize_agent_memory, summarize_agent_memory_from_events};
use self::curation::compact_session_events;
use self::decision_memory::{search_decisions, upsert_decision_from_event};
use self::feedback::{
    feedback_matching, feedback_schema_issues, feedback_stats, search_feedback, store_feedback,
};
use self::markdown_memory::{decay_reports, health_report, prune_memories, stale_memories};
use self::memory::memory_schema_issues;
use self::memory_recall::{recall_memories, recall_memories_with_policy};
use self::resume::build_resume_snapshot;
use self::util::{
    canonical_json, enforce_event_limit, format_days_ago, format_now, format_seconds_ago,
    is_corruption_error, normalize_event_payload_paths, path_to_str, quarantine_db, row_to_event,
};

static SESSION_DB_OPEN_LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();

pub struct SessionStore {
    // One connection per store instance. Keep thread-confined; do not share
    // across worker threads. The `_thread_bound` field explicitly opts out of
    // `Send` and `Sync` auto-traits at the compiler level.
    pub(super) conn: Connection,
    pub(super) config: SessionStoreConfig,
    /// Markdown truth store rooted at the `.atlas/` directory.
    pub(super) markdown: MarkdownStore,
    /// Absolute database path, used by markdown migration backups.
    pub(super) db_path: std::path::PathBuf,
    /// Marker that opts this struct out of `Send` and `Sync`.
    _thread_bound: std::marker::PhantomData<*const ()>,
}

#[derive(Debug, Deserialize)]
struct DurableTaskListCursor {
    updated_at: String,
    created_at: String,
    task_id: String,
}

fn session_db_open_lock(path: &str) -> Arc<Mutex<()>> {
    SESSION_DB_OPEN_LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .expect("session db open lock map poisoned")
        .entry(path.to_owned())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

fn parse_durable_task_list_cursor(cursor: &str) -> Result<DurableTaskListCursor> {
    serde_json::from_str(cursor)
        .map_err(|error| AtlasError::Other(format!("invalid durable task cursor: {error}")))
}

fn encode_durable_task_list_cursor(task: &DurableTaskRecord) -> String {
    serde_json::json!({
        "updated_at": task.updated_at,
        "created_at": task.created_at,
        "task_id": task.task_id,
    })
    .to_string()
}

fn row_to_durable_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<DurableTaskRecord> {
    let status: String = row.get(8)?;
    let progress_json: Option<String> = row.get(10)?;
    let result_json: Option<String> = row.get(11)?;
    let error_json: Option<String> = row.get(12)?;
    let input_requests_json: Option<String> = row.get(13)?;
    Ok(DurableTaskRecord {
        task_id: row.get(0)?,
        originating_method: row.get(1)?,
        request_id: row.get(2)?,
        tool_name: row.get(3)?,
        transport_kind: row.get(4)?,
        session_id: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        status: status.parse().map_err(|error: AtlasError| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    error.to_string(),
                )),
            )
        })?,
        status_message: row.get(9)?,
        progress: progress_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        result: result_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    11,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        error: error_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    12,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        input_requests: input_requests_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    13,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?,
        request_state: row.get(14)?,
        ttl_ms: row.get::<_, Option<i64>>(15)?.map(|value| value as u64),
        cancel_requested: row.get::<_, i32>(16)? != 0,
    })
}

mod store_decision_api;
mod store_experience_api;
mod store_feedback_api;
mod store_global_api;
mod store_memory_api;
mod store_open;
mod store_session_api;
