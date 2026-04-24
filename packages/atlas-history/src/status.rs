//! `atlas history status` output type and builder.

use atlas_store_sqlite::HistoryStatusSummary;
use serde::Serialize;

/// Human-readable and JSON-serializable status for `atlas history status`.
#[derive(Debug, Serialize)]
pub struct HistoryStatus {
    pub indexed_commit_count: i64,
    pub snapshot_count: i64,
    pub partial_snapshot_count: i64,
    pub parse_error_snapshot_count: i64,
    pub latest_commit_sha: Option<String>,
    pub latest_commit_subject: Option<String>,
    pub latest_author_time: Option<i64>,
    pub latest_indexed_ref: Option<String>,
    /// Non-empty when the repo has not been registered yet.
    pub warnings: Vec<String>,
}

impl HistoryStatus {
    /// Build from the raw store summary, adding any contextual warnings.
    pub fn from_summary(summary: HistoryStatusSummary, is_shallow: bool) -> Self {
        let mut warnings = Vec::new();
        if summary.repo_id.is_none() {
            warnings.push(
                "repo not yet registered; run `atlas history build` to start indexing".into(),
            );
        }
        if is_shallow {
            warnings.push("shallow clone detected; history may be incomplete".into());
        }
        HistoryStatus {
            indexed_commit_count: summary.indexed_commit_count,
            snapshot_count: summary.snapshot_count,
            partial_snapshot_count: summary.partial_snapshot_count,
            parse_error_snapshot_count: summary.parse_error_snapshot_count,
            latest_commit_sha: summary.latest_commit_sha,
            latest_commit_subject: summary.latest_commit_subject,
            latest_author_time: summary.latest_author_time,
            latest_indexed_ref: summary.latest_indexed_ref,
            warnings,
        }
    }
}
