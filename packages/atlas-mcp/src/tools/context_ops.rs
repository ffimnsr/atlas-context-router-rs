use anyhow::{Context, Result};
use atlas_adapters::derive_content_db_path;
use atlas_core::SearchQuery;
use atlas_core::model::{ChangeType, ChangedFile, ContextIntent, ContextRequest, ContextTarget};
use atlas_engine::{BuildOptions, UpdateOptions, UpdateTarget, build_graph, update_graph};
use atlas_repo::{
    CanonicalRepoPath, DiffTarget, RepoRegistration, changed_files, find_repo_root, stable_repo_id,
};
use atlas_review::ContextEngine;
use atlas_search::semantic as sem;
use atlas_store_sqlite::{BuildFinishStats, GraphBuildState, Store};
use camino::Utf8Path;
use serde::Serialize;
use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

use super::shared::{
    ResolvedChangeSourceKind, bool_arg, error_code_docs, error_message, error_suggestions,
    inject_budget_metadata, inject_deprecated_input_fields, load_budget_policy,
    mcp_query_looks_like_unstructured_description, mcp_supported_query_grammar_examples,
    open_store, parse_mcp_intent, parse_mcp_query_grammar, repo_aliases_by_id,
    resolve_change_source_selection, resolve_repo_scope_selection, str_arg, u64_arg,
};
use crate::context::{enforce_mcp_response_budget, package_context_result, package_impact};
use crate::session_tools::{
    decision_hits_json, record_mcp_decision_best_effort, search_decisions_best_effort,
};
use crate::tool_result::{
    InputShapeErrorSpec, ToolErrorPayload, ToolSuccessEnvelope, input_shape_error_payload,
    normalized_tool_result_value, tool_execution_error_value,
};

fn context_ranking_evidence_legend_json() -> serde_json::Value {
    atlas_core::context_ranking_evidence_legend()
}

fn context_decision_lookup_query(request: &ContextRequest) -> Option<String> {
    match &request.target {
        ContextTarget::QualifiedName { qname } => Some(qname.clone()),
        ContextTarget::SymbolName { name } => Some(name.clone()),
        ContextTarget::FilePath { path } => Some(path.clone()),
        ContextTarget::ChangedFiles { paths } => {
            let joined = paths.iter().take(3).cloned().collect::<Vec<_>>().join(" ");
            (!joined.is_empty()).then_some(joined)
        }
        ContextTarget::ChangedSymbols { qnames } => {
            let joined = qnames.iter().take(3).cloned().collect::<Vec<_>>().join(" ");
            (!joined.is_empty()).then_some(joined)
        }
        ContextTarget::EdgeQuerySeed { source_qname, .. } => Some(source_qname.clone()),
    }
}

mod build;
mod changes;
mod explain;
mod get_context;
mod impact;
mod minimal;
mod request;
mod review;
mod target;
mod types;

pub(super) use build::tool_build_or_update_graph;
pub(super) use changes::tool_detect_changes;
pub(super) use explain::tool_explain_change;
pub(super) use get_context::tool_get_context;
pub(super) use impact::tool_get_impact_radius;
pub(super) use minimal::tool_get_minimal_context;
use request::*;
pub(super) use review::tool_get_review_context;
use target::*;
use types::*;

#[cfg(test)]
mod tests {
    use super::context_query_looks_like_unstructured_description;

    #[test]
    fn natural_language_query_detection_rejects_plain_descriptions() {
        assert!(context_query_looks_like_unstructured_description(
            "please show me authentication flow"
        ));
    }

    #[test]
    fn natural_language_query_detection_allows_code_like_queries() {
        assert!(!context_query_looks_like_unstructured_description(
            "who calls handle_request"
        ));
        assert!(!context_query_looks_like_unstructured_description(
            "src/lib.rs::fn::handle_request"
        ));
        assert!(!context_query_looks_like_unstructured_description(
            "handle_request"
        ));
    }
}
