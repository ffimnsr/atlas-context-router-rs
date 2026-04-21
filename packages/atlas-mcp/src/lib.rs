//! Atlas MCP (Model Context Protocol) server.
//!
//! Exposes a JSON-RPC 2.0 / MCP stdio transport that agents can connect to.
//! The server implements the following MCP tools:
//!
//! | Tool                      | Description                                              |
//! |---------------------------|----------------------------------------------------------|
//! | `list_graph_stats`        | Node/edge counts and language breakdown                  |
//! | `query_graph`             | FTS5 keyword search, returns compact symbol list         |
//! | `get_impact_radius`       | Graph traversal from changed files                       |
//! | `get_review_context`      | Review bundle: symbols, neighbors, risk summary          |
//! | `get_context`             | General context engine: symbol, file, review, impact     |
//! | `detect_changes`          | Git diff → changed-file list with per-file node counts   |
//! | `explain_change`          | Advanced impact: risk, change kinds, boundary/test gaps  |
//! | `get_session_status`      | CM7: current session identity and event count            |
//! | `resume_session`          | CM7: retrieve and consume the resume snapshot            |
//! | `search_saved_context`    | CM7: FTS + trigram search over saved artifacts           |
//! | `save_context_artifact`   | CM7: index and store a large output                      |
//! | `get_context_stats`       | CM7: storage stats for the current session               |
//! | `purge_saved_context`     | CM7: delete saved artifacts by session or age            |

mod context;
mod output;
mod session_tools;
mod tools;
mod transport;

pub use transport::{ServerOptions, run_server, run_server_with_options};
