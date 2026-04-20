//! Transport and frontend adapter layer for Atlas context-mode integration.
//!
//! Provides thin adapter implementations so CLI, MCP, and other surfaces can
//! funnel session events and large outputs through the content and session
//! stores without coupling to transport details.

pub use atlas_contextsave::{ContentStore, OutputRouting, SearchFilters, SessionId, SourceMeta};
