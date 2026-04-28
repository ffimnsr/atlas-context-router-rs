#![doc = include_str!("../README.md")]

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
