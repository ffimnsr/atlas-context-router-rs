//! Atlas MCP (Model Context Protocol) server.
//!
//! Exposes a JSON-RPC 2.0 / MCP stdio transport that agents can connect to.
//! The server implements the following MCP tools:
//!
//! | Tool                      | Description                                              |
//! |---------------------------|----------------------------------------------------------|
//! | `list_graph_stats`        | Node/edge counts and language breakdown                  |
//! | `query_graph`             | FTS5 keyword search, returns compact symbol list only    |
//! | `get_impact_radius`       | Graph traversal from changed files                       |
//! | `get_review_context`      | Review bundle: symbols, neighbors, risk summary          |
//! | `get_context`             | General context engine: symbol, file, review, impact     |
//! | `detect_changes`          | Git diff → changed-file list with per-file node counts   |
//! | `build_or_update_graph`   | Full graph build or incremental graph update             |
//! | `traverse_graph`          | Reachability walk from qualified name                    |
//! | `get_minimal_context`     | Compact auto-detected review context                     |
//! | `explain_change`          | Advanced impact: risk, change kinds, boundary/test gaps  |
//! | `get_session_status`      | CM7: current session identity and event count            |
//! | `resume_session`          | CM7: retrieve and consume the resume snapshot            |
//! | `search_saved_context`    | CM7: FTS + trigram search over saved artifacts           |
//! | `save_context_artifact`   | CM7: index and store a large output                      |
//! | `get_context_stats`       | CM7: storage stats for the current session               |
//! | `purge_saved_context`     | CM7: delete saved artifacts by session or age            |
//! | `symbol_neighbors`        | Immediate callers, callees, tests, and nearby nodes      |
//! | `cross_file_links`        | Files coupled through shared symbol references           |
//! | `concept_clusters`        | Related file groups around seed files                    |

mod context;
mod output;
mod session_tools;
mod tools;
mod transport;

pub use tools::tool_list;
pub use transport::{ServerOptions, run_server, run_server_with_options};

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    #[test]
    fn crate_docs_list_current_mcp_tools() {
        let documented = include_str!("lib.rs")
            .lines()
            .filter(|line| line.starts_with("//! | `"))
            .filter_map(|line| line.split('`').nth(1))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();

        let exported = crate::tool_list()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();

        assert_eq!(documented, exported);
    }
}
