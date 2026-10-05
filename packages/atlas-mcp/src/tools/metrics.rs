//! Runtime metrics tool: exposes the process-local Atlas metrics snapshot.

use anyhow::Result;

use crate::output::OutputFormat;
use crate::tool_result::tool_result_value;

/// Return counters and histograms for build, parser, query, and MCP activity
/// accumulated by this process.
pub(super) fn tool_get_metrics(output_format: OutputFormat) -> Result<serde_json::Value> {
    let snapshot = atlas_metrics::snapshot();
    tool_result_value(&snapshot, output_format)
}

#[cfg(test)]
mod tests {
    #[test]
    fn get_metrics_returns_snapshot_and_counts_its_own_call() {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db").to_string_lossy().to_string();
        let _ = atlas_store_sqlite::Store::open(&db_path).expect("open store");

        let before = atlas_metrics::snapshot();
        let response = crate::tools::call("get_metrics", None, "/ignored", &db_path)
            .expect("get_metrics call");
        assert_ne!(response["isError"], serde_json::json!(true));

        let structured = &response["structuredContent"];
        assert!(
            structured["parser_parses_total"].is_number(),
            "snapshot must expose parser counters"
        );
        assert!(
            structured["parser_cache_reuse_ratio"].is_number(),
            "snapshot must expose the derived reuse ratio"
        );
        assert!(
            structured["build_runs"].is_object()
                && structured["query_calls"].is_object()
                && structured["mcp_tool_calls"].is_object(),
            "snapshot must expose build, query, and MCP families"
        );

        let after = atlas_metrics::snapshot();
        let before_calls = before
            .mcp_tool_calls
            .get("get_metrics")
            .map(|counts| counts.ok)
            .unwrap_or(0);
        let after_calls = after
            .mcp_tool_calls
            .get("get_metrics")
            .map(|counts| counts.ok)
            .unwrap_or(0);
        assert!(
            after_calls > before_calls,
            "tool dispatch must count the get_metrics call"
        );
    }
}
