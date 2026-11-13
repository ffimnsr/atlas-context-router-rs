//! Runtime metrics tool: exposes the process-local Atlas metrics snapshot.

use anyhow::Result;
use serde_json::Value;

use crate::output::OutputFormat;
use crate::tool_result::tool_result_value;

/// Return counters and histograms for build, parser, query, and MCP activity
/// accumulated by this process.
///
/// Compact mode drops per-histogram bucket arrays, which dominate the payload
/// without adding operator value. Pass `verbose: true` to retain them.
pub(super) fn tool_get_metrics(
    args: Option<&serde_json::Value>,
    output_format: OutputFormat,
) -> Result<serde_json::Value> {
    let verbose = args
        .and_then(|value| value.get("verbose"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let snapshot = atlas_metrics::snapshot();
    let mut value = serde_json::to_value(&snapshot)?;
    if !verbose {
        strip_histogram_buckets(&mut value);
    }
    // Keep one identity field across tools so consumers can route payloads
    // without inspecting the server's tool registry.
    if let Some(map) = value.as_object_mut() {
        map.insert("tool".to_owned(), serde_json::json!("get_metrics"));
        map.insert("verbose".to_owned(), serde_json::json!(verbose));
    }
    tool_result_value(&value, output_format)
}

/// Remove `buckets` from every histogram summary object.
fn strip_histogram_buckets(value: &mut Value) {
    match value {
        Value::Object(map) => {
            // Histogram summaries carry p50/p99 plus buckets; other objects do
            // not, so this is a stable discriminator.
            if map.contains_key("buckets") && map.contains_key("p99") {
                map.remove("buckets");
            }
            for child in map.values_mut() {
                strip_histogram_buckets(child);
            }
        }
        Value::Array(items) => {
            for item in items {
                strip_histogram_buckets(item);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(structured["verbose"], serde_json::json!(false));

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

    #[test]
    fn compact_snapshot_omits_histogram_buckets() {
        let mut value = serde_json::json!({
            "query_duration_ms": {
                "fts5": {
                    "count": 1,
                    "sum": 2,
                    "min_ms": 2,
                    "max_ms": 2,
                    "mean_ms": 2.0,
                    "p50": 2,
                    "p90": 2,
                    "p99": 2,
                    "buckets": [{ "upper_bound": 2, "count": 1 }]
                }
            }
        });
        strip_histogram_buckets(&mut value);
        assert!(value["query_duration_ms"]["fts5"].get("buckets").is_none());
        assert_eq!(value["query_duration_ms"]["fts5"]["max_ms"], 2);
    }
}
