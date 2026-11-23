//! M1 — markdown-truth storage for the shared memory surface.
//!
//! Memory records live as `.atlas/memories/<topic-slug>/<id>.md`. SQLite keeps
//! only derived state: `memory_index`/`memory_index_fts` for lexical recall,
//! `memory_access` for read-path bookkeeping, and `memory_embeddings` for
//! optional vectors. Reads never rewrite truth files; access bookkeeping is
//! recorded in `memory_access` so recall leaves file bytes and mtimes intact.
//!
//! The active set is derived from `supersedes` links at read time, so
//! consolidation only writes the new consolidated file and never rewrites the
//! members it replaces.

use super::markdown_memory_sync;
use super::memory_consolidate;
use super::types;

mod consolidate;
mod decay;
mod encoding;
mod health;
mod index;
mod integrity;
mod migration;
mod read;
mod scan;
mod write;

/// Durable memory directory, relative to the `.atlas/` store root.
pub(super) const MEMORIES_DIR: &str = "memories";

/// Relation vocabulary accepted by memory links: the memoir controlled
/// vocabulary plus `supersedes`, which marks the target as replaced.
pub(super) const MEMORY_LINK_RELATIONS: &[&str] = &[
    "depends_on",
    "part_of",
    "contradicts",
    "refines",
    "replaces",
    "caused_by",
    "fixed_by",
    "blocked_by",
    "decided_by",
    "related_to",
    "supersedes",
];

const RELATION_SUPERSEDES: &str = "supersedes";

pub(crate) use super::types::{
    MemoryConsolidationGroup, MemoryConsolidationPlan, MemoryDecayPolicy, MemoryDecayReport,
    MemoryDeleteResult, MemoryHealthCategory, MemoryHealthFinding, MemoryHealthReport,
    MemoryImportance, MemoryLink, MemoryListFilter, MemoryPruneResult, MemoryRecord,
    NOISY_TOPIC_ENTRIES, NewMemory, OVERSIZED_BODY_CHARS,
};

pub use self::migration::MarkdownMigrationReport;

pub(in crate::store) use self::consolidate::consolidation_plan;
pub(in crate::store) use self::decay::decay_reports;
pub(in crate::store) use self::decay::prune_memories;
pub(in crate::store) use self::decay::stale_memories;
pub(in crate::store) use self::encoding::*;
pub(in crate::store) use self::health::health_report;
pub(in crate::store) use self::index::ensure_index;
pub(in crate::store) use self::index::index_delete;
pub(in crate::store) use self::index::index_upsert;
pub(in crate::store) use self::index::record_recall_access;
pub(in crate::store) use self::index::refresh_supersession_index;
pub(in crate::store) use self::integrity::tree_integrity;
pub(in crate::store) use self::migration::export_pending;
pub(in crate::store) use self::migration::finalize_migration;
pub(in crate::store) use self::migration::pending_export_count;
pub(in crate::store) use self::migration::reconcile_on_open;
pub(in crate::store) use self::migration::write_migration_backup;
pub(in crate::store) use self::read::delete_memory;
pub(in crate::store) use self::read::get_memory;
pub(in crate::store) use self::read::list_memories;
pub(in crate::store) use self::scan::*;
pub(in crate::store) use self::write::add_memory_link;
pub(in crate::store) use self::write::store_memory;
pub(in crate::store) use self::write::store_memory_at;
pub(in crate::store) use self::write::write_existing_record;
