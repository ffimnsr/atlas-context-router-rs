//! Review-context assembly and change explanation for Atlas.
//!
//! Bridges graph, impact, search, and saved-content signals into bounded review
//! outputs for CLI and MCP callers.
//!
//! Main entry points:
//! - [`build_context`] and [`ContextEngine`] for targeted context retrieval
//! - [`build_explain_change_summary`] for deterministic change-risk summaries

pub mod context;
mod explain_change;
pub mod query_parser;
mod ranking;

pub use context::{
    ContextEngine, ResolvedTarget, build_context, normalize_qn_kind_tokens, resolve_target,
};
pub use explain_change::{
    ExplainChangeSummary, build_explain_change_summary, empty_explain_change_summary,
};
