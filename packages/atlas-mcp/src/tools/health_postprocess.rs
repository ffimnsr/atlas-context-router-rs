//! Postprocess freshness signal embedded in the `status` tool payload.
//!
//! Split out of `health.rs` because that file already exceeds the 1K-LOC
//! threshold from the AGENTS.md file-size rule. Everything here is read-only:
//! it turns the persisted `postprocess_state` row plus the build timestamp
//! into a bounded JSON block so agents can detect stale derived analytics
//! (communities) without calling `postprocess_graph` speculatively.

use atlas_core::PostprocessStageStatus;
use atlas_store_sqlite::Store;
use serde_json::{Value, json};

/// Build the `status.postprocess` block and its companion warnings.
///
/// Returns `Value::Null` (not `None`) when no postprocess row exists, so the
/// payload key stays present and the output schema stays exact. Query errors
/// degrade into warnings instead of failing the whole `status` call: graph
/// health must not depend on the postprocess ledger being readable.
pub(super) fn status_postprocess_block(store: &Store, repo_root: &str) -> (Value, Vec<String>) {
    let mut warnings = Vec::new();

    let postprocess = match store.get_postprocess_status(repo_root) {
        Ok(status) => status,
        Err(error) => {
            warnings.push(format!("postprocess status query failed: {error}"));
            return (Value::Null, warnings);
        }
    };
    let Some(postprocess) = postprocess else {
        return (Value::Null, warnings);
    };

    let last_built_ms = match store.last_built_at_ms(repo_root) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("build timestamp query failed: {error}"));
            None
        }
    };

    // A run is only comparable to the latest build when both timestamps are
    // known; a running pass or a never-built repo yields null instead of a
    // guess.
    let finished_after_last_build = match (postprocess.finished_at_ms, last_built_ms) {
        (Some(finished), Some(built)) => Some(finished >= built),
        _ => None,
    };
    // Only the communities stage persists derived data consumed by
    // `infer_modules`/`label_components`; a stage-filtered run that skipped it
    // cannot vouch for freshness, so report null rather than false.
    let communities_in_last_run = postprocess.stages.iter().any(|stage| {
        stage.stage == "communities" && stage.status == PostprocessStageStatus::Completed
    });
    let communities_refreshed_after_last_build = if communities_in_last_run {
        finished_after_last_build
    } else {
        None
    };
    if communities_refreshed_after_last_build == Some(false) {
        warnings.push(
            "persisted communities predate the latest build; run postprocess_graph with \
             stage communities before trusting community-derived module buckets in \
             infer_modules and label_components"
                .to_owned(),
        );
    }

    let stages = postprocess
        .stages
        .iter()
        .map(|stage| {
            json!({
                "stage": &stage.stage,
                "status": stage.status.as_str(),
                "mode": stage.mode.as_str(),
                "affected_file_count": stage.affected_file_count,
                "item_count": stage.item_count,
            })
        })
        .collect::<Vec<_>>();

    let block = json!({
        "state": postprocess.state.as_str(),
        "mode": postprocess.mode.as_str(),
        "stage_filter": &postprocess.stage_filter,
        "changed_file_count": postprocess.changed_file_count,
        "started_at_ms": postprocess.started_at_ms,
        "finished_at_ms": postprocess.finished_at_ms,
        "last_error_code": &postprocess.last_error_code,
        "last_error": &postprocess.last_error,
        "stages": stages,
        "finished_after_last_build": finished_after_last_build,
        "communities_refreshed_after_last_build": communities_refreshed_after_last_build,
    });
    (block, warnings)
}
