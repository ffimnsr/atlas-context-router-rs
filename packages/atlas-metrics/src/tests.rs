use super::*;

#[test]
fn counter_accumulates() {
    let counter = Counter::new();
    assert_eq!(counter.value(), 0);
    counter.increment();
    counter.add(4);
    assert_eq!(counter.value(), 5);
}

#[test]
fn histogram_snapshot_computes_bounds_and_percentiles() {
    let histogram = Histogram::new(LATENCY_MS_BUCKETS);
    histogram.observe(1);
    histogram.observe(3);
    histogram.observe(7);

    let snapshot = histogram.snapshot();
    assert_eq!(snapshot.count, 3);
    assert_eq!(snapshot.sum, 11);
    assert_eq!(snapshot.min, Some(1));
    assert_eq!(snapshot.max, Some(7));
    assert!((snapshot.mean - 11.0 / 3.0).abs() < f64::EPSILON);
    // Buckets are [1, 2, 5, 10, ...]: 1 → 1, 3 → 5, 7 → 10.
    assert_eq!(snapshot.p50, 5);
    assert_eq!(snapshot.p90, 10);
    assert_eq!(snapshot.p99, 10);
    assert_eq!(
        snapshot.buckets,
        vec![
            HistogramBucketSnapshot {
                upper_bound: Some(1),
                count: 1
            },
            HistogramBucketSnapshot {
                upper_bound: Some(5),
                count: 1
            },
            HistogramBucketSnapshot {
                upper_bound: Some(10),
                count: 1
            },
        ]
    );
}

#[test]
fn histogram_snapshot_handles_overflow_bucket_and_empty_state() {
    let histogram = Histogram::new(LATENCY_MS_BUCKETS);
    let empty = histogram.snapshot();
    assert_eq!(empty.count, 0);
    assert_eq!(empty.min, None);
    assert_eq!(empty.max, None);
    assert_eq!(empty.mean, 0.0);
    assert_eq!(empty.p50, 0);
    assert!(empty.buckets.is_empty());

    histogram.observe(120_000);
    let overflow = histogram.snapshot();
    assert_eq!(overflow.p99, 120_000);
    assert_eq!(
        overflow.buckets,
        vec![HistogramBucketSnapshot {
            upper_bound: None,
            count: 1
        }]
    );
}

#[test]
fn labeled_families_partition_by_label() {
    let counters = LabeledCounter::new();
    counters.increment("build");
    counters.add("update", 3);
    counters.increment("build");

    let snapshot = counters.snapshot();
    assert_eq!(snapshot.get("build"), Some(&2));
    assert_eq!(snapshot.get("update"), Some(&3));

    let histograms = LabeledHistogram::new(FILE_COUNT_BUCKETS);
    histograms.observe("build", 10);
    histograms.observe("build", 20);
    histograms.observe("update", 2);

    let snapshot = histograms.snapshot();
    assert_eq!(snapshot["build"].count, 2);
    assert_eq!(snapshot["build"].sum, 30);
    assert_eq!(snapshot["update"].count, 1);
    assert_eq!(snapshot["update"].max, Some(2));
}

#[test]
fn metrics_snapshot_derives_ratios_and_outcomes() {
    let metrics = Metrics::new();
    metrics.record_build_run("build", 120, Some(10), true);
    metrics.record_build_run("update", 30, Some(4), true);
    metrics.record_build_run("build", 5, None, false);
    metrics.record_parse_attempt(false);
    metrics.record_parse_attempt(true);
    metrics.record_parse_attempt(true);
    metrics.record_query("fts5", 4);
    metrics.record_query("fts5_vector_hybrid", 40);
    metrics.record_mcp_tool_call("query_graph", 12, true);
    metrics.record_mcp_tool_call("query_graph", 8, false);
    metrics.record_mcp_tool_call("status", 1, true);

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.build_runs.get("build"), Some(&2));
    assert_eq!(snapshot.build_runs.get("update"), Some(&1));
    assert_eq!(snapshot.build_failures.get("build"), Some(&1));
    assert_eq!(snapshot.build_duration_ms["build"].count, 2);
    assert_eq!(snapshot.build_duration_ms["build"].max, Some(120));
    assert_eq!(snapshot.build_parsed_files["build"].count, 1);
    assert_eq!(snapshot.build_parsed_files["build"].max, Some(10));
    assert_eq!(snapshot.build_parsed_files["update"].count, 1);

    assert_eq!(snapshot.parser_parses_total, 3);
    assert_eq!(snapshot.parser_tree_reuses_total, 2);
    assert!((snapshot.parser_cache_reuse_ratio - 2.0 / 3.0).abs() < f64::EPSILON);

    assert_eq!(snapshot.query_calls.get("fts5"), Some(&1));
    assert_eq!(snapshot.query_calls.get("fts5_vector_hybrid"), Some(&1));
    assert_eq!(snapshot.query_duration_ms["fts5"].count, 1);

    let query_graph = &snapshot.mcp_tool_calls["query_graph"];
    assert_eq!(query_graph.ok, 1);
    assert_eq!(query_graph.error, 1);
    let status = &snapshot.mcp_tool_calls["status"];
    assert_eq!(status.ok, 1);
    assert_eq!(status.error, 0);
    assert_eq!(snapshot.mcp_tool_duration_ms["query_graph"].count, 2);
}

#[test]
fn metrics_snapshot_round_trips_through_json() {
    let metrics = Metrics::new();
    metrics.record_build_run("build", 42, Some(2), true);
    metrics.record_parse_attempt(true);
    metrics.record_query("regex_structural_scan", 3);
    metrics.record_mcp_tool_call("doctor", 7, true);

    let snapshot = metrics.snapshot();
    let json = serde_json::to_string(&snapshot).expect("snapshot serializes");
    let decoded: MetricsSnapshot = serde_json::from_str(&json).expect("snapshot deserializes");
    assert_eq!(decoded, snapshot);
}

#[test]
fn empty_metrics_snapshot_has_zeroed_families() {
    let snapshot = Metrics::new().snapshot();
    assert!(snapshot.build_runs.is_empty());
    assert!(snapshot.build_failures.is_empty());
    assert!(snapshot.build_duration_ms.is_empty());
    assert!(snapshot.build_parsed_files.is_empty());
    assert_eq!(snapshot.parser_parses_total, 0);
    assert_eq!(snapshot.parser_cache_reuse_ratio, 0.0);
    assert!(snapshot.query_calls.is_empty());
    assert!(snapshot.mcp_tool_calls.is_empty());
    assert!(snapshot.mcp_tool_duration_ms.is_empty());
}

#[test]
fn global_registry_records_and_snapshots() {
    record_parse_attempt(true);
    record_query("fts5", 1);
    record_mcp_tool_call("get_metrics", 0, true);

    let snapshot = snapshot();
    assert!(snapshot.parser_parses_total >= 1);
    assert!(snapshot.parser_tree_reuses_total >= 1);
    assert!(snapshot.query_calls.get("fts5").copied().unwrap_or(0) >= 1);
    assert!(snapshot.mcp_tool_calls["get_metrics"].ok >= 1);
}
