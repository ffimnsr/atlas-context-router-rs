//! Atlas MCP (Model Context Protocol) server.
//!
//! Exposes a JSON-RPC 2.0 / MCP stdio transport that agents can connect to.
//! The server implements the following MCP tools:
//!
//! | Tool                | Description                                            |
//! |---------------------|--------------------------------------------------------|
//! | `list_graph_stats`  | Node/edge counts and language breakdown                |
//! | `query_graph`       | FTS5 keyword search, returns compact symbol list       |
//! | `get_impact_radius` | Graph traversal from changed files                     |
//! | `get_review_context`| Review bundle: symbols, neighbors, risk summary        |
//! | `get_context`       | General context engine: symbol, file, review, impact   |
//! | `detect_changes`    | Git diff → changed-file list with per-file node counts |
//! | `explain_change`    | Advanced impact: risk, change kinds, boundary/test gaps|

mod context;
mod output;
mod tools;
mod transport;

pub use transport::run_server;
