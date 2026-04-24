#[derive(Debug, Clone, Copy)]
pub struct GraphHealthInput<'a> {
    pub db_exists: bool,
    pub graph_error: Option<&'a str>,
    pub build_state: Option<&'a str>,
    pub stale_index: bool,
    pub retrieval_unavailable: bool,
}

pub fn is_schema_mismatch_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    [
        "no such column",
        "has no column named",
        "duplicate column name",
        "no such table",
        "unknown column",
        "table graph_build_state has",
        "table edges has",
        "table nodes has",
        "table files has",
        "table retrieval_index_state has",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

pub fn select_graph_health_error_code(input: GraphHealthInput<'_>) -> &'static str {
    if !input.db_exists {
        return "missing_graph_db";
    }
    if let Some(error) = input.graph_error {
        if is_schema_mismatch_error(error) {
            return "schema_mismatch";
        }
        return "corrupt_or_inconsistent_graph_rows";
    }
    match input.build_state {
        Some("building") => return "interrupted_build",
        Some("degraded") => return "degraded_build",
        Some("build_failed") => return "failed_build",
        _ => {}
    }
    if input.stale_index {
        return "stale_index";
    }
    if input.retrieval_unavailable {
        return "retrieval_index_unavailable";
    }
    "none"
}

pub fn graph_health_error_message(error_code: &str) -> &'static str {
    match error_code {
        "none" => "Graph is healthy and up-to-date.",
        "missing_graph_db" => "Graph database not found. Run `atlas build` to create it.",
        "noncanonical_path_rows" => {
            "Persisted path rows are not canonical. Rebuild graph/content state from clean canonical inputs."
        }
        "schema_mismatch" => {
            "Graph database schema does not match this Atlas build. Rebuild the graph to refresh the schema."
        }
        "corrupt_or_inconsistent_graph_rows" => {
            "Graph database has integrity issues. Run `atlas build` to rebuild from scratch."
        }
        "interrupted_build" => {
            "Previous build was interrupted and did not complete. Run `atlas build` to restart."
        }
        "degraded_build" => {
            "Last build finished in degraded mode because an operational budget was hit. Check build_status for counters and stop reason, then run `atlas build` with narrower scope or higher safe limits."
        }
        "failed_build" => {
            "Last build failed. Check build_last_error for details, then run `atlas build` to retry."
        }
        "stale_index" => {
            "Graph-backed answers may be stale because graph-relevant files changed after the last index."
        }
        "retrieval_index_unavailable" => {
            "Retrieval/content index is unavailable. Graph queries may still work, but retrieval-backed features are degraded."
        }
        "node_not_found" => "No graph nodes matched this request.",
        "checks_failed" => "One or more health checks failed.",
        _ => "An unknown error occurred.",
    }
}

pub fn graph_health_error_suggestions(error_code: &str) -> &'static [&'static str] {
    match error_code {
        "none" => &[],
        "missing_graph_db" => &[
            "run `atlas build` to create the graph",
            "run `atlas init` if the project is new",
        ],
        "noncanonical_path_rows" => &[
            "run `atlas build` to rebuild canonical graph rows",
            "run `atlas purge-noncanonical` to remove stale repo-local context/session stores",
            "delete stale context/session artifacts that still reference raw repo paths",
        ],
        "schema_mismatch" => &[
            "run `atlas build` to rebuild the graph with the current schema",
            "delete the stale database file if rebuild keeps failing",
        ],
        "corrupt_or_inconsistent_graph_rows" => {
            &["run `atlas build` to rebuild the graph from scratch"]
        }
        "interrupted_build" => &["run `atlas build` to restart the interrupted build"],
        "degraded_build" => &[
            "check the build_status counters and budget_stop_reason fields",
            "rerun `atlas build` or `atlas update` with narrower scope or adjusted safe limits",
        ],
        "failed_build" => &[
            "check the build_last_error field for details",
            "run `atlas build` to retry",
        ],
        "stale_index" => &[
            "run `atlas update` or `build_or_update_graph` to refresh graph facts",
            "run `atlas detect-changes` to inspect pending graph-relevant files",
        ],
        "retrieval_index_unavailable" => &[
            "run `atlas build` or `atlas update` to refresh retrieval index state",
            "run `atlas doctor` to inspect retrieval_index details",
        ],
        "node_not_found" => &[
            "verify the symbol name with query_graph or resolve_symbol",
            "run status to confirm the graph is built",
            "run build_or_update_graph to index the repo first",
        ],
        "checks_failed" => &["inspect the checks array for details"],
        _ => &[],
    }
}

pub fn user_facing_error_message(primary_error: &str, detail: &str) -> String {
    if let Some(error_code) = internal_error_code(detail) {
        return graph_health_error_message(error_code).to_owned();
    }

    primary_error
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("An unknown error occurred.")
        .to_owned()
}

fn internal_error_code(detail: &str) -> Option<&'static str> {
    if detail.trim().is_empty() {
        return None;
    }

    let lower = detail.to_ascii_lowercase();

    if is_schema_mismatch_error(detail) {
        return Some("schema_mismatch");
    }
    if looks_like_missing_graph_db(&lower) {
        return Some("missing_graph_db");
    }
    if looks_like_internal_storage_error(&lower) {
        return Some("corrupt_or_inconsistent_graph_rows");
    }

    None
}

fn looks_like_missing_graph_db(lower: &str) -> bool {
    (lower.contains("cannot open database at")
        || lower.contains("unable to open database file")
        || lower.contains("cannot open database"))
        && (lower.contains("no such file or directory") || lower.contains("os error 2"))
}

fn looks_like_internal_storage_error(lower: &str) -> bool {
    [
        "sqlite",
        "rusqlite",
        "fts5",
        "database disk image is malformed",
        "readonly database",
        "constraint failed",
        "failed to prepare",
        "failed to execute",
        "cannot execute",
        "execute returned results",
        "no such table",
        "no such column",
        "has no column named",
        "duplicate column name",
        "table graph_build_state has",
        "table edges has",
        "table nodes has",
        "table files has",
        "table retrieval_index_state has",
        "worldtree.db",
        "context.db",
        "session.db",
        "wal",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
        || contains_sql_statement(lower)
}

fn contains_sql_statement(lower: &str) -> bool {
    [
        "select ",
        "insert into ",
        "delete from ",
        "create table ",
        "drop table ",
        "alter table ",
        "pragma ",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::{graph_health_error_message, user_facing_error_message};

    #[test]
    fn user_facing_error_message_redacts_schema_mismatch_details() {
        let primary = "cannot open database at /repo/.atlas/worldtree.db";
        let detail = concat!(
            "cannot open database at /repo/.atlas/worldtree.db: no such table: nodes\n",
            "SELECT qualified_name FROM nodes"
        );

        assert_eq!(
            user_facing_error_message(primary, detail),
            graph_health_error_message("schema_mismatch")
        );
    }

    #[test]
    fn user_facing_error_message_redacts_internal_sql_details() {
        let primary = "atlas update failed";
        let detail = concat!(
            "rusqlite error: database disk image is malformed\n",
            "pragma integrity_check"
        );

        assert_eq!(
            user_facing_error_message(primary, detail),
            graph_health_error_message("corrupt_or_inconsistent_graph_rows")
        );
    }

    #[test]
    fn user_facing_error_message_preserves_plain_validation_errors() {
        assert_eq!(
            user_facing_error_message("missing tool name", "missing tool name"),
            "missing tool name"
        );
    }
}
