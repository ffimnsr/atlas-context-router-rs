use serde::{Deserialize, Serialize};
use serde_json::Value;

use atlas_core::AtlasError;

use crate::SessionId;

fn default_memory_metadata() -> Value {
    Value::Object(Default::default())
}

mod feedback;
mod memory;
mod session;

pub use self::feedback::FeedbackRecord;
pub use self::feedback::FeedbackSearchFilter;
pub use self::feedback::FeedbackSearchHit;
pub use self::feedback::FeedbackStats;
pub use self::feedback::NewFeedback;
pub use self::memory::DEFAULT_HYBRID_WEIGHT_FTS;
pub use self::memory::DEFAULT_HYBRID_WEIGHT_VECTOR;
pub use self::memory::DEFAULT_MEMORY_VECTOR_MIN_SIMILARITY;
pub use self::memory::DEFAULT_RECALL_AUX_MAX_HOPS;
pub use self::memory::DEFAULT_RECALL_FRESHNESS_WINDOW_DAYS;
pub use self::memory::DEFAULT_RECALL_PRIMARY_QUOTA_RATIO;
pub use self::memory::DEFAULT_RECALL_WEIGHT_BUSINESS;
pub use self::memory::DEFAULT_RECALL_WEIGHT_TIME;
pub use self::memory::MEMORY_HYBRID_RRF_K;
pub use self::memory::MEMORY_VECTOR_SCAN_LIMIT;
pub use self::memory::MemoryConsolidationGroup;
pub use self::memory::MemoryConsolidationPlan;
pub use self::memory::MemoryDecayPolicy;
pub use self::memory::MemoryDecayReport;
pub use self::memory::MemoryDeleteResult;
pub use self::memory::MemoryEmbeddingCoverage;
pub use self::memory::MemoryEmbeddingPolicy;
pub use self::memory::MemoryHealthCategory;
pub use self::memory::MemoryHealthFinding;
pub use self::memory::MemoryHealthReport;
pub use self::memory::MemoryImportance;
pub use self::memory::MemoryLink;
pub use self::memory::MemoryListFilter;
pub use self::memory::MemoryPruneResult;
pub use self::memory::MemoryRecallPolicy;
pub use self::memory::MemoryRecallRanking;
pub use self::memory::MemoryRecord;
pub use self::memory::MemoryScope;
pub use self::memory::MemorySearchHit;
pub use self::memory::MemorySupersessionLink;
pub use self::memory::MemoryViewer;
pub use self::memory::NOISY_TOPIC_ENTRIES;
pub use self::memory::NewMemory;
pub use self::memory::OVERSIZED_BODY_CHARS;
pub use self::session::AgentMemorySummary;
pub use self::session::AgentPartitionSummary;
pub use self::session::AgentResponsibilitySummary;
pub use self::session::CurationResult;
pub use self::session::DEFAULT_DEDUP_WINDOW_SECS;
pub use self::session::DEFAULT_MAX_SNAPSHOT_BYTES;
pub use self::session::DEFAULT_SESSION_DB;
pub use self::session::DEFAULT_SESSION_MAX_EVENTS;
pub use self::session::DecisionRecord;
pub use self::session::DecisionSearchHit;
pub use self::session::DelegatedTaskSummary;
pub use self::session::DurableTaskListPage;
pub use self::session::DurableTaskRecord;
pub use self::session::DurableTaskStatus;
pub use self::session::DurableTaskUpdate;
pub use self::session::EventCategory;
pub use self::session::GlobalAccessEntry;
pub use self::session::GlobalWorkflowPattern;
pub use self::session::MAX_INLINE_EVENT_PAYLOAD_BYTES;
pub use self::session::NewDurableTask;
pub use self::session::NewSessionEvent;
pub use self::session::ResumeSnapshot;
pub use self::session::SessionEventRow;
pub use self::session::SessionEventType;
pub use self::session::SessionMeta;
pub use self::session::SessionStats;
pub use self::session::SessionStoreConfig;
