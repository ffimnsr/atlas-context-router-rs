//! Run-level metric wrappers around the build and update pipelines.
//!
//! Wrapping here (rather than inside `build.rs`/`update.rs`) keeps the large
//! pipeline files unchanged and guarantees every production caller records
//! end-to-end duration and parsed-file metrics for both success and failure.

use std::time::Instant;

use anyhow::Result;
use camino::Utf8Path;

use crate::build::{BuildOptions, BuildSummary};
use crate::update::{UpdateOptions, UpdateSummary};

/// Metric label for full graph builds.
pub const BUILD_RUN_KIND: &str = "build";
/// Metric label for incremental graph updates.
pub const UPDATE_RUN_KIND: &str = "update";

/// Run a full build, recording duration and parsed-file metrics.
pub fn build_graph(
    repo_root: &Utf8Path,
    db_path: &str,
    opts: &BuildOptions,
) -> Result<BuildSummary> {
    let started = Instant::now();
    let result = crate::build::build_graph(repo_root, db_path, opts);
    record_run(
        BUILD_RUN_KIND,
        started.elapsed().as_millis() as u64,
        result.as_ref().ok().map(|summary| summary.parsed as u64),
        result.is_ok(),
    );
    result
}

/// Run an incremental update, recording duration and parsed-file metrics.
pub fn update_graph(
    repo_root: &Utf8Path,
    db_path: &str,
    opts: &UpdateOptions,
) -> Result<UpdateSummary> {
    let started = Instant::now();
    let result = crate::update::update_graph(repo_root, db_path, opts);
    record_run(
        UPDATE_RUN_KIND,
        started.elapsed().as_millis() as u64,
        result.as_ref().ok().map(|summary| summary.parsed as u64),
        result.is_ok(),
    );
    result
}

fn record_run(kind: &str, elapsed_ms: u64, parsed_files: Option<u64>, ok: bool) {
    atlas_metrics::record_build_run(kind, elapsed_ms, parsed_files, ok);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_run_records_failures_without_parsed_observations() {
        let before = atlas_metrics::snapshot();
        record_run("unit_test_failure_kind", 7, None, false);
        let after = atlas_metrics::snapshot();

        let runs_before = before
            .build_runs
            .get("unit_test_failure_kind")
            .copied()
            .unwrap_or(0);
        assert!(after.build_runs["unit_test_failure_kind"] > runs_before);
        assert!(after.build_failures["unit_test_failure_kind"] >= 1);
        assert_eq!(
            after.build_parsed_files.get("unit_test_failure_kind"),
            before.build_parsed_files.get("unit_test_failure_kind"),
            "failed runs must not observe parsed-file counts"
        );
    }

    #[test]
    fn record_run_records_success_with_parsed_observations() {
        let before = atlas_metrics::snapshot();
        record_run("unit_test_success_kind", 12, Some(5), true);
        let after = atlas_metrics::snapshot();

        assert!(after.build_runs["unit_test_success_kind"] >= 1);
        assert_eq!(
            after
                .build_failures
                .get("unit_test_success_kind")
                .copied()
                .unwrap_or(0),
            0,
            "successful runs must not count as failures"
        );
        let files_before = before
            .build_parsed_files
            .get("unit_test_success_kind")
            .map(|histogram| histogram.count)
            .unwrap_or(0);
        assert!(after.build_parsed_files["unit_test_success_kind"].count > files_before);
        assert_eq!(
            after.build_parsed_files["unit_test_success_kind"].max,
            Some(5)
        );
    }
}
