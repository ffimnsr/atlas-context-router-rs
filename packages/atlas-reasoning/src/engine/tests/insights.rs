//! `InsightsEngine` report-filtering and sort/limit tests.

use super::super::InsightsEngine;
use super::super::insights::InsightsGraphSummary;
use super::sample_insight;
use atlas_core::{FreshnessWarning, GraphStats, InsightSeverity, ProvenanceMeta};

#[test]
fn insights_engine_filters_ignored_findings_and_keeps_metadata() {
    let summary = InsightsGraphSummary {
        graph_stats: GraphStats {
            file_count: 1,
            node_count: 2,
            edge_count: 3,
            nodes_by_kind: vec![("function".to_owned(), 2)],
            languages: vec!["rust".to_owned()],
            last_indexed_at: Some("2026-05-11T00:00:00Z".to_owned()),
        },
        atlas_provenance: ProvenanceMeta {
            indexed_file_count: 1,
            last_indexed_at: Some("2026-05-11T00:00:00Z".to_owned()),
        },
        atlas_freshness: Some(FreshnessWarning {
            stale: true,
            changed_files: vec!["src/lib.rs".to_owned()],
            stale_result_files: vec!["src/lib.rs".to_owned()],
            warning: "stale".to_owned(),
            suggested_recovery: vec!["refresh".to_owned()],
        }),
    };
    let config = atlas_engine::config::InsightsConfig {
        ignore_files: vec!["tests".to_owned()],
        ignore_modules: vec!["crate::ignored".to_owned()],
        ..Default::default()
    };

    let engine =
        InsightsEngine::from_summary(summary, config).with_generated_at("2026-05-11T12:00:00Z");
    let report = engine.metrics_report(vec![
        sample_insight(
            "keep",
            "src/lib.rs",
            "crate::kept::compute",
            InsightSeverity::High,
        ),
        sample_insight(
            "drop-file",
            "tests/lib.rs",
            "crate::tests::helper",
            InsightSeverity::High,
        ),
        sample_insight(
            "drop-module",
            "src/ignored.rs",
            "crate::ignored::helper",
            InsightSeverity::High,
        ),
    ]);

    assert_eq!(report.summary.total_findings, 1);
    assert_eq!(report.findings[0].id, "keep");
    assert_eq!(report.summary.generated_at, "2026-05-11T12:00:00Z");
    assert!(report.atlas_freshness.is_some());
    assert_eq!(report.atlas_provenance.indexed_file_count, 1);
}

#[test]
fn insights_engine_sorts_and_limits_findings() {
    let summary = InsightsGraphSummary {
        graph_stats: GraphStats {
            file_count: 1,
            node_count: 1,
            edge_count: 0,
            nodes_by_kind: vec![],
            languages: vec!["rust".to_owned()],
            last_indexed_at: None,
        },
        atlas_provenance: ProvenanceMeta {
            indexed_file_count: 1,
            last_indexed_at: None,
        },
        atlas_freshness: None,
    };
    let config = atlas_engine::config::InsightsConfig {
        max_findings: 2,
        ..Default::default()
    };

    let mut high = sample_insight("high", "src/a.rs", "crate::alpha", InsightSeverity::High);
    high.score = 50.0;
    let mut low = sample_insight("low", "src/c.rs", "crate::gamma", InsightSeverity::Low);
    low.score = 99.0;
    let mut high_later = sample_insight(
        "high-later",
        "src/b.rs",
        "crate::beta",
        InsightSeverity::High,
    );
    high_later.score = 40.0;

    let engine = InsightsEngine::from_summary(summary, config);
    let report = engine.pattern_report(vec![low, high_later, high]);

    assert_eq!(report.findings.len(), 2);
    assert_eq!(report.findings[0].id, "high");
    assert_eq!(report.findings[1].id, "high-later");
}
