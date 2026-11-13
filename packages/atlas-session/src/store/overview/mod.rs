//! ICM-G — code overview memory domain and persistence.
//!
//! Overview records are evidence-linked code documentation stored in the
//! continuity-owned session database. They never merge into the code graph.

mod budget;
mod context;
mod levels;
mod store;
mod types;

#[cfg(test)]
mod tests;

pub use budget::{
    OVERVIEW_DESCRIPTION_PREVIEW_MAX_CHARS, OVERVIEW_INLINE_BODY_BUDGET_BYTES,
    OVERVIEW_LIST_ITEM_MAX_CHARS, OVERVIEW_LIST_MAX_ITEMS, OVERVIEW_SUMMARY_MAX_CHARS,
    OVERVIEW_TITLE_MAX_CHARS, OverviewBudgetMeta, bound_new_overview_record, truncate_chars,
    validate_confidence,
};

pub use levels::{
    OVERVIEW_L0_MAX_CHARS, OVERVIEW_L1_MAX_CHARS, OVERVIEW_L2_MAX_CHARS, OverviewLevel,
    OverviewLevelView, render_level,
};

pub use types::{
    NewOverviewRecord, OVERVIEW_SCHEMA_VERSION, OverviewAnalysisInput, OverviewAnalysisPacket,
    OverviewCodeExcerpt, OverviewContextHit, OverviewContextQuery, OverviewDeleteResult,
    OverviewExportRequest, OverviewExportScope, OverviewFreshness, OverviewGraphProvenance,
    OverviewListFilter, OverviewPacketBudget, OverviewPacketEvidence, OverviewPacketFreshness,
    OverviewRanking, OverviewRecord, OverviewSearchFilter, OverviewSearchHit, OverviewSourceKind,
    OverviewStaleFilter, OverviewSubject, OverviewSubjectInput, OverviewSubjectKind,
    canonicalize_subject_path, packet_id, subject_id,
};
