//! JSON-RPC 2.0 / MCP transport layer.
//!
//! Reads newline-delimited JSON from stdin, dispatches each request, and
//! writes newline-delimited JSON responses to stdout.  Follows the MCP
//! 2025-11-25 protocol specification.
//!
//! Also provides Unix socket (daemon) and Windows named-pipe transports.

pub(crate) mod broker;
mod dispatch;
pub(crate) mod helpers;
pub(crate) mod input;
pub(crate) mod io;
mod jsonrpc;
pub(crate) mod notify;
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
    InteractiveStdioTestSession, run_server, run_server_with_dynamic_roots,
    run_server_with_options, run_stdio_jsonrpc_session_for_tests,
};
pub use self::types::ServerOptions;

// ── Re-export pub(crate) API used by transport_http.rs ────────────────────
#[cfg(feature = "http-transport")]
pub(crate) use self::broker::{ReverseRequestBroker, ReverseRequestEmitter};
#[cfg(feature = "http-transport")]
pub(crate) use self::helpers::{
    log_protocol_error_observation, log_tool_execution_error_observation,
    parse_client_interaction_capabilities,
};
#[cfg(feature = "http-transport")]
pub(crate) use self::types::RequestLogContext;
