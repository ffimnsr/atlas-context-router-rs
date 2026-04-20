//! Transport and frontend adapter layer for Atlas context-mode integration.
//!
//! Provides thin adapter implementations so CLI, MCP, and other surfaces can
//! funnel session events and large outputs through the content and session
//! stores without coupling to transport details.

pub mod events;
pub mod hooks;

pub use atlas_contextsave::{ContentStore, OutputRouting, SearchFilters, SessionId, SourceMeta};
pub use events::{
    PendingEvent, extract_cli_event, extract_context_event, extract_decision_event,
    extract_graph_event, extract_reasoning_event, extract_rule_event, extract_tool_event,
    extract_user_event, hash_event, normalize_event,
};
pub use hooks::{AdapterHooks, CliAdapter, McpAdapter};
