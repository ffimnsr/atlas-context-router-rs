//! Exact column sets for the memoir tables (migration 011).
//!
//! Kept in a separate module so `memoir.rs` stays below the 1,000-line limit.

/// Exact column set of the `memoir_graphs` table (migration 011).
pub(super) const MEMOIR_GRAPH_COLUMNS: &[&str] = &[
    "id",
    "repo_root",
    "scope",
    "name",
    "description",
    "created_at",
    "updated_at",
    "metadata_json",
];

/// Exact column set of the `memoir_concepts` table (migration 011).
pub(super) const MEMOIR_CONCEPT_COLUMNS: &[&str] = &[
    "id",
    "graph_id",
    "repo_root",
    "name",
    "description",
    "kind",
    "source_id",
    "created_at",
    "updated_at",
    "metadata_json",
];

/// Exact column set of the `memoir_relations` table (migration 011).
pub(super) const MEMOIR_RELATION_COLUMNS: &[&str] = &[
    "id",
    "graph_id",
    "repo_root",
    "source_concept_id",
    "target_concept_id",
    "relation_type",
    "confidence",
    "source_id",
    "created_at",
    "metadata_json",
];
