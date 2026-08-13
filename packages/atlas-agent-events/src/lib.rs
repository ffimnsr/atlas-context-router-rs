//! Shared agent event service for native hook capture and MCP fallback capture.
//!
//! One event policy and one persistence/action pipeline back both surfaces:
//!
//! - native host hooks (`atlas hook <event>` in `atlas-cli`)
//! - instruction-driven MCP fallback (`record_session_event` in `atlas-mcp`)
//!
//! The service owns:
//!
//! - canonical event names, aliases, priorities, storage mode, and lifecycle mode
//! - payload redaction and sanitization
//! - session event persistence and oversized-payload routing through the content store
//! - lifecycle actions: restore load, handoff persist, restore verify
//! - prompt routing, freshness metadata, graph refresh, and review refresh
//!
//! # Path identity invariant
//!
//! Callers MUST pass canonical repo roots (via `atlas_repo::CanonicalRepoPath` or
//! the helper APIs built on it). Repo roots feed artifact labels, source ids,
//! session derivation, and content-store routing, so non-canonical inputs would
//! break path-derived identity. Do not normalize paths inside this service.

pub mod actions;
pub mod graph_readiness;
pub mod metadata;
pub mod payload;
pub mod policy;
pub mod service;
pub mod wake_pack;

pub use actions::execute_hook_actions;
pub use graph_readiness::{
    FreshnessWarning, compute_freshness_warning, derive_graph_readiness,
    derive_graph_readiness_open_failed, pending_graph_relevant_changes,
};
pub use payload::tool_may_change_files;
pub use policy::{HookEventParts, HookPersistence, HookPolicy, resolve_hook_policy};
pub use service::{
    AgentEventRequest, AgentEventResult, AgentEventSource, build_hook_event, persist_hook_event,
    record_agent_event,
};
pub use wake_pack::{
    DEFAULT_MAX_FEEDBACK_ITEMS, DEFAULT_MAX_ITEMS, DEFAULT_MAX_PENDING_CHANGES,
    HARD_MAX_FEEDBACK_ITEMS, HARD_MAX_ITEMS, HARD_MAX_PENDING_CHANGES, WakeBudget, WakePack,
    WakePackBuild, WakePackOptions, build_wake_pack,
};

#[cfg(test)]
mod tests;
