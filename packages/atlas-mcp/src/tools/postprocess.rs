use anyhow::Result;
use atlas_engine::{PostprocessOptions, postprocess_graph, supported_postprocess_stages};
use atlas_repo::find_repo_root;
use camino::Utf8Path;

use super::shared::{bool_arg, compute_freshness_warning, open_store, str_arg, tool_result_value};

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
    let summary = postprocess_graph(
        repo_root.as_path(),
        db_path,
        &PostprocessOptions {
            changed_only,
            stage,
            dry_run,
        },
    )?;

    let mut response = tool_result_value(&summary, output_format)?;
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

    response["atlas_result_kind"] = serde_json::json!("postprocess_summary");
    response["atlas_readiness"] = serde_json::json!({
        "graph_built": summary.graph_built,
        "build_state": build_state,
        "supported_stages": supported_postprocess_stages(),
        "latest_postprocess_state": latest_postprocess.as_ref().map(|status| status.state.as_str()),
    });
    if let Some(freshness) =
        compute_freshness_warning(repo_root.as_str(), db_path, &summary.changed_files)
    {
        response["atlas_freshness"] = serde_json::json!(freshness);
    }
    Ok(response)
}
