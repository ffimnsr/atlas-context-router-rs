//! rmcp-backed MCP transport layer.
//!
//! Provides stdio, Unix socket, and Windows named-pipe wrappers around the
//! official rmcp server implementation.

#[cfg(test)]
pub(crate) mod broker;
pub(crate) mod helpers;
pub(crate) mod repo_selection;
mod socket;
pub(crate) mod stdio;
mod types;
mod worker;

#[cfg(test)]
mod tests;

// ── Re-export public API ──────────────────────────────────────────────────
pub use self::socket::run_socket_server_with_options;
pub use self::stdio::{
    InteractiveStdioTestSession, run_server, run_server_with_options,
    run_stdio_jsonrpc_session_for_tests,
};

pub use self::types::ServerOptions;
pub(crate) use self::types::{ActiveRepoContext, RepoResolutionState, TraceLevel, TraceThreshold};
