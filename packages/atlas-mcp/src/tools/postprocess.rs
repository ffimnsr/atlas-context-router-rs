use anyhow::Result;
use atlas_engine::{PostprocessOptions, postprocess_graph, supported_postprocess_stages};
use atlas_repo::find_repo_root;
use camino::Utf8Path;
use serde_json::{Value, json};

use crate::tool_result::{ToolSuccessEnvelope, normalized_tool_result_value};

use super::shared::{bool_arg, compute_freshness_warning, error_code_docs, open_store, str_arg};

pub(super) fn tool_postprocess_graph(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let changed_only = bool_arg(args, "changed_only").unwrap_or(false);
    let dry_run = bool_arg(args, "dry_run").unwrap_or(false);
    let stage = str_arg(args, "stage")?.map(str::to_owned);
    let repo_root = find_repo_root(Utf8Path::new(repo_root))?;
    crate::progress::report("running postprocess stages", None);
    if crate::progress::is_canceled() {
        return Err(anyhow::anyhow!("canceled"));
    }
    let summary = postprocess_graph(
        repo_root.as_path(),
        db_path,
        &PostprocessOptions {
            changed_only,
            stage,
            dry_run,
        },
    )?;
    crate::progress::report("postprocess complete", Some(100));

    let store = open_store(db_path)?;
    let build_state =
        store
            .get_build_status(repo_root.as_str())?
            .map(|status| match status.state {
                atlas_store_sqlite::GraphBuildState::Building => "building",
                atlas_store_sqlite::GraphBuildState::Built => "built",
                atlas_store_sqlite::GraphBuildState::Degraded => "degraded",
                atlas_store_sqlite::GraphBuildState::BuildFailed => "build_failed",
            });
    let latest_postprocess = store.get_postprocess_status(repo_root.as_str())?;

    let normalize_stage = |stage: &atlas_core::model::PostprocessStageSummary| {
        json!({
            "stage": &stage.stage,
            "status": stage.status.as_str(),
            "mode": stage.mode.as_str(),
            "affected_file_count": stage.affected_file_count,
            "item_count": stage.item_count,
            "elapsed_ms": stage.elapsed_ms,
            "error_code": stage.error_code,
            "message": stage.message,
            "details": stage.details,
        })
    };

    let planned_stages = summary
        .stages
        .iter()
        .filter(|item| {
            dry_run
                || matches!(
                    item.status,
                    atlas_core::model::PostprocessStageStatus::Planned
                )
        })
        .map(normalize_stage)
        .collect::<Vec<_>>();
    let executed_stages = summary
        .stages
        .iter()
        .filter(|item| {
            !dry_run
                && !matches!(
                    item.status,
                    atlas_core::model::PostprocessStageStatus::Planned
                )
        })
        .map(normalize_stage)
        .collect::<Vec<_>>();

    let mut warnings = Vec::new();
    if let Some(reason) = summary.noop_reason.clone() {
        warnings.push(reason);
    }
    if !summary.ok && !summary.message.is_empty() {
        warnings.push(summary.message.clone());
    }

    let payload = json!({
        "mode": summary.requested_mode.as_str(),
        "scope": {
            "changed_only": changed_only,
            "stage_filter": summary.stage_filter,
            "changed_file_count": summary.changed_files.len(),
            "changed_files": &summary.changed_files,
        },
        "dry_run": summary.dry_run,
        "planned_stages": planned_stages,
        "executed_stages": executed_stages,
        "summary": {
            "ok": summary.ok,
            "noop": summary.noop,
            "noop_reason": summary.noop_reason,
            "error_code": &summary.error_code,
            "error_code_docs": error_code_docs(&summary.error_code),
            "message": &summary.message,
            "suggestions": &summary.suggestions,
            "graph_built": summary.graph_built,
            "state": summary.state.as_str(),
            "started_at_ms": summary.started_at_ms,
            "finished_at_ms": summary.finished_at_ms,
            "duration_ms": summary.total_elapsed_ms,
            "stage_count": summary.stages.len(),
            "supported_stage_count": summary.supported_stages.len(),
        },
        "warnings": warnings,
    });

    let envelope = ToolSuccessEnvelope::new("postprocess_graph", payload);
    let mut response = normalized_tool_result_value(&envelope, output_format)?;
    response["atlas_readiness"] = json!({
        "graph_built": summary.graph_built,
        "build_state": build_state,
        "supported_stages": supported_postprocess_stages(),
        "latest_postprocess_state": latest_postprocess.as_ref().map(|status| status.state.as_str()),
    });
    if let Some(freshness) =
        compute_freshness_warning(repo_root.as_str(), db_path, &summary.changed_files)
    {
        response["atlas_freshness"] = Value::Object(
            serde_json::to_value(freshness)?
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
    }
    Ok(response)
}
