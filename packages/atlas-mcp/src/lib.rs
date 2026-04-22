#![recursion_limit = "512"]
//! Atlas MCP (Model Context Protocol) server.
//!
//! Exposes a JSON-RPC 2.0 / MCP stdio transport that agents can connect to.
//! The server implements the following MCP tools:
//!
//! | Tool                      | Description                                              |
//! |---------------------------|----------------------------------------------------------|
//! | `list_graph_stats`        | Node/edge counts and language breakdown                  |
//! | `query_graph`             | FTS5 keyword search, returns compact symbol list only    |
//! | `batch_query_graph`       | Run up to 20 query_graph searches in one round-trip     |
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
//! | `search_files`            | File-path discovery by glob pattern outside graph lookup |
//! | `search_content`          | Content search by literal string or regex outside graph  |
//! | `search_templates`        | Template file discovery by engine kind (HTML, Jinja, …)  |
//! | `search_text_assets`      | SQL, config, env, and prompt file discovery              |
//! | `status`                  | Compact graph health summary with machine-readable state |
//! | `doctor`                  | Full repo health checks: git, config, DB, build, index   |
//! | `db_check`                | SQLite integrity + orphan/dangling edge scan             |
//! | `debug_graph`             | Graph internals: node/edge kinds, top files, anomalies   |
//! | `explain_query`           | Explain how query_graph resolves a given search input    |
//! | `resolve_symbol`          | Resolve symbol name to exact qualified_name with aliases |
//! | `analyze_safety`          | Refactor-safety score: fan-in, fan-out, test adjacency   |
//! | `analyze_remove`          | Removal-impact analysis with confidence-tiered results   |
//! | `analyze_dead_code`       | Dead-code candidates with certainty tiers and blockers   |
//! | `analyze_dependency`      | Dependency-removal validation: removable verdict and refs |
//!
//! MCP prompt templates:
//! - `review_change`: review-flow guidance for changed files
//! - `inspect_symbol`: symbol lookup and usage-exploration guidance
//! - `plan_refactor`: refactor-safety and blast-radius guidance
//! - `resume_prior_session`: continuity and saved-context retrieval guidance

mod context;
mod discovery_tools;
mod output;
mod prompts;
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

    #[test]
    fn crate_docs_list_current_mcp_prompts() {
        let documented = include_str!("lib.rs")
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("//! - `"))
            .filter_map(|line| line.split('`').next())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();

        let exported = crate::prompts::prompt_list()["prompts"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|prompt| prompt["name"].as_str())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();

        assert_eq!(documented, exported);
    }
}
