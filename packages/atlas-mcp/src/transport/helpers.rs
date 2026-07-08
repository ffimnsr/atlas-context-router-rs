//! Helper functions for repo context, parameter parsing, and tool-call utilities.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use atlas_repo::{canonical_filesystem_path, find_repo_root};
use camino::Utf8PathBuf;
use url::Url;

use crate::output::{OutputFormat, resolve_output_format};
use crate::tool_result::{ToolErrorCode, ToolErrorPayload, tool_execution_error_value};

use super::broker::{ReverseRequestBroker, ReverseRequestEmitter};
use super::types::{
    ActiveRepoContext, PendingRequest, RepoResolutionState, RequestLogContext, TraceLevel,
};

// ---------------------------------------------------------------------------
// Repo context resolution
// ---------------------------------------------------------------------------

pub(crate) fn ensure_repo_context(
    method: &str,
    connection_state: &mut super::types::ConnectionState,
    _ctx: &super::types::RequestDispatchContext<'_>,
) -> Result<Option<ActiveRepoContext>> {
    if !method_requires_repo_context(method) {
        return Ok(connection_state.repo_resolution.active.clone());
    }
    if let Some(active) = connection_state.repo_resolution.active.clone() {
        return Ok(Some(active));
    }
    if !connection_state.repo_resolution.dynamic_roots {
        return connection_state
            .repo_resolution
            .startup
            .clone()
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("atlas repo context missing; pass --repo"));
    }
    Err(anyhow::anyhow!(
        "atlas repo context not yet resolved for method `{method}`; use tools/call first or pass --repo"
    ))
}

pub(crate) fn resolve_repo_context_for_tool_call(
    repo_resolution: &RepoResolutionState,
    client_capabilities: &serde_json::Value,
    initialized: bool,
    reverse_broker: &ReverseRequestBroker,
    reverse_emitter: &Arc<dyn ReverseRequestEmitter>,
) -> Result<ActiveRepoContext> {
    if let Some(active) = repo_resolution.active.clone() {
        return Ok(active);
    }
    if !repo_resolution.dynamic_roots {
        return repo_resolution
            .startup
            .clone()
            .ok_or_else(|| anyhow::anyhow!("atlas repo context missing; pass --repo"));
    }
    if !client_supports_roots(client_capabilities) {
        return Err(anyhow::anyhow!(
            "MCP client did not advertise roots capability; pass --repo or launch atlas from inside target repo"
        ));
    }
    if !initialized {
        return Err(anyhow::anyhow!(
            "atlas cannot resolve client roots before initialized notification"
        ));
    }

    const ROOTS_LIST_TIMEOUT_MS: u64 = 5_000;
    let response = reverse_broker.issue_request(
        "stdio:roots",
        reverse_emitter,
        "roots/list",
        serde_json::json!({}),
        Duration::from_millis(ROOTS_LIST_TIMEOUT_MS),
    )?;
    let repo_root = select_repo_root_from_roots(response.get("roots"))?;
    Ok(ActiveRepoContext {
        db_path: atlas_engine::paths::default_db_path(&repo_root),
        repo_root,
    })
}

pub(crate) fn method_requires_repo_context(method: &str) -> bool {
    matches!(
        method,
        "resources/read"
            | "completion/complete"
            | "tools/call"
            | "tasks/list"
            | "tasks/get"
            | "tasks/result"
            | "tasks/cancel"
    )
}

fn client_supports_roots(capabilities: &serde_json::Value) -> bool {
    capabilities.get("roots").is_some()
}

fn select_repo_root_from_roots(roots: Option<&serde_json::Value>) -> Result<String> {
    let roots = roots
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("roots/list response missing result.roots array"))?;
    let mut candidates = Vec::new();
    for root in roots {
        let Some(uri) = root.get("uri").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let url = Url::parse(uri).with_context(|| format!("invalid root URI: {uri}"))?;
        let path = url
            .to_file_path()
            .map_err(|_| anyhow::anyhow!("root URI must use file:// scheme: {uri}"))?;
        let utf8 = Utf8PathBuf::from_path_buf(path)
            .map_err(|path| anyhow::anyhow!("root path is not valid UTF-8: {}", path.display()))?;
        let start = if utf8.is_file() {
            utf8.parent()
                .map(|parent| parent.to_owned())
                .unwrap_or_else(|| utf8.clone())
        } else {
            utf8.clone()
        };
        let repo_root = find_repo_root(start.as_path()).unwrap_or(start);
        let canonical = canonical_filesystem_path(repo_root.as_path())?;
        let candidate = canonical.into_string();
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    match candidates.len() {
        0 => Err(anyhow::anyhow!("roots/list returned no usable file roots")),
        1 => Ok(candidates.remove(0)),
        _ => Err(anyhow::anyhow!(
            "multiple workspace roots available; pass --repo or narrow client roots: {}",
            candidates.join(", ")
        )),
    }
}

// ---------------------------------------------------------------------------
// Parameter parsing helpers
// ---------------------------------------------------------------------------

pub(crate) fn parse_trace_level(params: Option<&serde_json::Value>) -> Result<TraceLevel> {
    let level = params
        .and_then(|value| value.get("value"))
        .and_then(|value| value.as_str())
        .unwrap_or("off");
    match level {
        "off" => Ok(TraceLevel::Off),
        "messages" => Ok(TraceLevel::Messages),
        "verbose" => Ok(TraceLevel::Verbose),
        other => Err(anyhow::anyhow!(
            "invalid $/setTrace value: expected 'off', 'messages', or 'verbose', got '{other}'"
        )),
    }
}

pub(crate) fn parse_client_interaction_capabilities(
    capabilities: &serde_json::Value,
) -> crate::runtime_context::ClientInteractionCapabilities {
    crate::runtime_context::ClientInteractionCapabilities {
        supports_elicitation_form: capabilities
            .get("elicitation")
            .and_then(|value| value.get("form"))
            .is_some(),
        supports_elicitation_url: capabilities
            .get("elicitation")
            .and_then(|value| value.get("url"))
            .is_some(),
    }
}

// ---------------------------------------------------------------------------
// Tool-call helpers
// ---------------------------------------------------------------------------

pub(crate) fn tool_name_from_request(
    method: &str,
    params: Option<&serde_json::Value>,
) -> Option<String> {
    if method != "tools/call" {
        return None;
    }
    params
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_owned)
}

pub(crate) fn log_request_finished(
    request: &RequestLogContext,
    success: bool,
    queue_wait_ms: u128,
    execution_ms: u128,
    total_ms: u128,
) {
    tracing::debug!(
        request_id = %request.request_id,
        method = %request.method,
        tool = request.tool_name.as_deref().unwrap_or("-"),
        queue_wait_ms = queue_wait_ms as u64,
        execution_ms = execution_ms as u64,
        total_ms = total_ms as u64,
        success,
        "completed MCP request"
    );
}

pub(crate) fn resolve_request_timeout(
    options: &super::types::ServerOptions,
    request: &RequestLogContext,
) -> Duration {
    let timeout_ms = request
        .tool_name
        .as_ref()
        .and_then(|tool_name| options.tool_timeout_ms_by_tool.get(tool_name))
        .copied()
        .unwrap_or(options.tool_timeout_ms)
        .clamp(1_000, 3_600_000);
    Duration::from_millis(timeout_ms)
}

pub(crate) fn tool_execution_error_fields(result: &serde_json::Value) -> Option<(String, String)> {
    if result.get("isError").and_then(serde_json::Value::as_bool) != Some(true) {
        return None;
    }
    let structured = result.get("structuredContent")?;
    let code = structured.get("code")?.as_str()?.to_owned();
    let tool = structured
        .get("tool")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            result
                .pointer("/atlas_readiness/tool")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "-".to_owned());
    Some((tool, code))
}

pub(crate) fn log_protocol_error_observation(
    channel: &str,
    request: &RequestLogContext,
    error_code: &str,
    message: &str,
) {
    tracing::warn!(
        request_id = %request.request_id,
        method = %request.method,
        tool = request.tool_name.as_deref().unwrap_or("-"),
        error_code,
        classification = "protocol_error",
        channel,
        message,
        "MCP protocol error"
    );
}

pub(crate) fn log_tool_execution_error_observation(
    channel: &str,
    request: &RequestLogContext,
    result: &serde_json::Value,
) {
    if let Some((tool, error_code)) = tool_execution_error_fields(result) {
        tracing::warn!(
            request_id = %request.request_id,
            method = %request.method,
            tool,
            error_code,
            classification = "tool_execution_error",
            channel,
            "MCP tool execution error"
        );
    }
}

pub(crate) fn tool_call_output_format(params: Option<&serde_json::Value>) -> OutputFormat {
    let args = params.and_then(|value| value.get("arguments"));
    resolve_output_format(args, OutputFormat::Toon).unwrap_or(OutputFormat::Toon)
}

pub(crate) fn timeout_tool_call_result(request: &PendingRequest) -> Result<serde_json::Value> {
    let tool_name = request
        .request
        .tool_name
        .clone()
        .unwrap_or_else(|| "tools/call".to_owned());
    let payload = ToolErrorPayload::new(
        ToolErrorCode::Timeout,
        format!("Tool '{tool_name}' timed out after {} ms", request.timeout_ms),
    )
    .with_tool(tool_name)
    .with_retry_guidance(
        "Reduce request scope, increase timeout if configurable, or retry when dependencies are responsive.",
    )
    .with_details(serde_json::json!({
        "detail": format!(
            "worker pool timed out after {} ms while handling request",
            request.timeout_ms
        ),
        "timeout_ms": request.timeout_ms,
        "request_id": request.request.request_id,
        "method": request.request.method,
    }));
    tool_execution_error_value(request.output_format, &payload)
}

pub(crate) fn progress_token_from_params(
    params: Option<&serde_json::Value>,
) -> Option<serde_json::Value> {
    params
        .and_then(|value| value.get("progressToken"))
        .cloned()
        .or_else(|| {
            params
                .and_then(|value| value.get("_meta"))
                .and_then(|value| value.get("progressToken"))
                .cloned()
        })
        .or_else(|| params.and_then(|value| value.get("workDoneToken")).cloned())
}

// ---------------------------------------------------------------------------
// Error helpers
// ---------------------------------------------------------------------------

pub(crate) fn user_visible_error_message(error: &anyhow::Error) -> String {
    atlas_core::user_facing_error_message(&error.to_string(), &format!("{error:#}"))
}

pub(crate) fn panic_payload_message(payload: &Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_owned()
    }
}
