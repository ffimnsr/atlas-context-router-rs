//! Transport and frontend adapter layer for Atlas context memory integration.
//!
//! Provides thin adapter implementations so CLI, MCP, and other surfaces can
//! funnel session events and large outputs through the content and session
//! stores without coupling to transport details.

pub mod bridge;
pub mod events;
pub mod hooks;
pub mod redact;

pub use atlas_contextsave::{ContentStore, OutputRouting, SearchFilters, SessionId, SourceMeta};
pub use bridge::{
    BRIDGE_DIR, BridgeEvent, DEFAULT_BRIDGE_MAX_AGE_SECS, bridge_file_count,
    cleanup_stale_bridge_files, ingest_pending_bridge_files, list_bridge_files,
    purge_all_bridge_files, write_bridge_file,
};
pub use events::{
    PendingEvent, extract_cli_event, extract_context_event, extract_decision_event,
    extract_graph_event, extract_reasoning_event, extract_rule_event, extract_tool_event,
    extract_user_event, hash_event, normalize_event,
};
pub use hooks::{AdapterHooks, CliAdapter, McpAdapter};
pub use redact::redact_payload;
