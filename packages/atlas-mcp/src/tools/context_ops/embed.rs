use anyhow::Result;
use serde_json::Value;

use crate::output::OutputFormat;
use crate::tool_result::{
    ToolErrorCode, ToolErrorPayload, tool_execution_error_value, tool_result_value,
};

use crate::tools::shared::{bool_arg, u64_arg};

/// Embed retrieval chunks that do not yet have a vector.
///
/// Thin MCP wrapper over the shared `atlas_engine` pipeline so CLI and MCP
/// share provider resolution, dimension freezing, and error accounting.
pub(crate) fn tool_embed_chunks(
    args: Option<&Value>,
    repo_root: &str,
    db_path: &str,
    output_format: OutputFormat,
) -> Result<Value> {
    let limit = u64_arg(args, "limit")
        .map(|value| value as usize)
        .unwrap_or(atlas_engine::DEFAULT_EMBED_CHUNK_LIMIT);
    let dry_run = bool_arg(args, "dry_run").unwrap_or(false);

    let report = match atlas_engine::embed_pending_chunks(
        repo_root,
        db_path,
        atlas_engine::EmbedChunkOptions { limit, dry_run },
    ) {
        Ok(report) => report,
        Err(error) => {
            let payload = ToolErrorPayload::new(ToolErrorCode::InvalidInput, error.to_string())
                .with_tool("embed_chunks")
                .with_retry_guidance(
                    "Set [search.embedding] url in .atlas/config.toml, then retry.",
                );
            return tool_execution_error_value(output_format, &payload);
        }
    };

    tool_result_value(&report, output_format)
}
