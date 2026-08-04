//! Per-tool annotations, categories, and test-support registry consts.

use crate::descriptors::ToolDescriptorAnnotations;

pub(crate) fn tool_annotations(name: &str) -> ToolDescriptorAnnotations {
    let destructive = matches!(name, "purge_saved_context");
    let state_changing = matches!(
        name,
        "build_or_update_graph"
            | "postprocess_graph"
            | "compact_session"
            | "record_session_event"
            | "wake_up"
            | "purge_saved_context"
    );
    ToolDescriptorAnnotations::new()
        .read_only(!state_changing)
        .destructive(destructive)
}

pub(crate) fn tool_category(name: &str) -> &'static str {
    match name {
        "build_or_update_graph" | "postprocess_graph" => "maintenance",
        "compact_session"
        | "purge_saved_context"
        | "resume_session"
        | "save_context_artifact"
        | "read_saved_context"
        | "search_saved_context"
        | "search_decisions"
        | "get_context_stats"
        | "get_session_status"
        | "record_session_event"
        | "wake_up"
        | "cross_session_search"
        | "get_global_memory"
        | "memory_store"
        | "memory_recall" => "memory",
        "tool_list" | "tool_search" | "tool_help" | "man" | "repo_registry" => "introspection",
        "status" | "doctor" | "db_check" | "debug_graph" | "broker_status" => "health",
        name if name.starts_with("analyze_")
            || name.starts_with("assess_")
            || name.starts_with("find_")
            || name == "infer_modules"
            || name == "label_components" =>
        {
            "analysis"
        }
        name if name.starts_with("search_") || name.starts_with("read_") => "content",
        _ => "graph",
    }
}

#[cfg(test)]
pub(crate) const ALLOWED_TEXT_ONLY_TOOLS: &[&str] = &[];

#[cfg(test)]
pub(crate) const TYPED_HEALTH_SCHEMA_TOOLS: &[&str] = &[
    "broker_status",
    "status",
    "doctor",
    "db_check",
    "debug_graph",
];

#[cfg(test)]
pub(crate) const TYPED_DISCOVERY_SCHEMA_TOOLS: &[&str] = &[
    "search_files",
    "search_content",
    "read_file_excerpt",
    "get_docs_section",
    "read_file_around_match",
    "search_templates",
    "search_text_assets",
];

#[cfg(test)]
pub(crate) const TYPED_GRAPH_SCHEMA_TOOLS: &[&str] = &[
    "query_graph",
    "batch_query_graph",
    "resolve_symbol",
    "symbol_neighbors",
    "traverse_graph",
    "cross_file_links",
    "concept_clusters",
    "explain_query",
];

#[cfg(test)]
pub(crate) const TYPED_CONTEXT_REVIEW_SCHEMA_TOOLS: &[&str] = &[
    "detect_changes",
    "get_context",
    "get_review_context",
    "get_minimal_context",
    "get_impact_radius",
    "explain_change",
    "build_or_update_graph",
    "postprocess_graph",
];

#[cfg(test)]
pub(crate) const TYPED_ANALYSIS_SCHEMA_TOOLS: &[&str] = &[
    "analyze_safety",
    "analyze_remove",
    "analyze_dead_code",
    "analyze_dependency",
    "find_large_functions",
    "find_complex_functions",
    "find_similar_functions",
    "find_duplicates",
    "infer_modules",
    "label_components",
];

#[cfg(test)]
pub(crate) const TYPED_SESSION_MEMORY_SCHEMA_TOOLS: &[&str] = &[
    "get_session_status",
    "compact_session",
    "resume_session",
    "search_saved_context",
    "search_decisions",
    "read_saved_context",
    "save_context_artifact",
    "purge_saved_context",
    "cross_session_search",
    "get_global_memory",
    "memory_store",
    "memory_recall",
    "record_session_event",
    "wake_up",
];

#[cfg(test)]
pub(crate) const ALLOWED_TOOL_DESCRIPTOR_FIELDS: &[&str] = &[
    "name",
    "title",
    "description",
    "inputSchema",
    "outputSchema",
    "annotations",
    "icons",
    "_meta",
];
