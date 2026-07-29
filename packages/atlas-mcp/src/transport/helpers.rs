//! Helper functions for repo context, parameter parsing, and tool-call utilities.

use std::time::Duration;

use anyhow::Result;
use serde_json::json;

use crate::output::{OutputFormat, resolve_output_format};
use crate::tool_result::{ToolErrorCode, ToolErrorPayload, tool_execution_error_value};

use super::repo_selection::{
    RepoSelectionOutcome, RepoSelectionSource, explicit_repo_context_from_tool_args,
};
use super::types::{
    ActiveRepoContext, PendingRequest, RepoResolutionState, RequestLogContext, TraceLevel,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RepoSelectionFailureKind {
    NoRepoContextAvailable,
    InvalidExplicitRepoSelector,
}

impl RepoSelectionFailureKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoRepoContextAvailable => "no_repo_context_available",
            Self::InvalidExplicitRepoSelector => "invalid_explicit_repo_selector",
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct RepoSelectionError {
    pub(crate) kind: RepoSelectionFailureKind,
    pub(crate) message: String,
    pub(crate) candidate_roots: Vec<String>,
    pub(crate) selection_attempts: Vec<String>,
    pub(crate) selection_source: Option<RepoSelectionSource>,
    pub(crate) tool_name: String,
    pub(crate) recommended_fix: String,
    pub(crate) session_mode: &'static str,
    pub(crate) active_repo_root: Option<String>,
}

impl RepoSelectionError {
    pub(crate) fn message(&self) -> String {
        self.message.clone()
    }

    pub(crate) fn error_data(&self) -> serde_json::Value {
        json!({
            "atlas_repo_selection": {
                "failure_kind": self.kind.as_str(),
                "candidate_roots": self.candidate_roots,
                "selection_attempts": self.selection_attempts,
                "selection_source": self.selection_source.map(RepoSelectionSource::as_str),
                "tool_name": self.tool_name,
                "recommended_fix": self.recommended_fix,
                "session_mode": self.session_mode,
                "active_repo_root": self.active_repo_root,
                "canonical_repo_path_guidance": "Use canonical absolute repo root in arguments.repo_root; use canonical repo-relative paths inside tool arguments.",
            }
        })
    }
}

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
    connection_state
        .repo_resolution
        .startup
        .clone()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("atlas repo context missing; pass --repo"))
}

pub(crate) struct ToolRepoResolutionContext;

pub(crate) fn resolve_repo_context_for_tool_call(
    repo_resolution: &RepoResolutionState,
    tool_name: Option<&str>,
    tool_args: Option<&serde_json::Value>,
    _ctx: ToolRepoResolutionContext,
) -> std::result::Result<RepoSelectionOutcome, Box<RepoSelectionError>> {
    let tool_name = tool_name.unwrap_or("tools/call");
    let active_repo_root = repo_resolution
        .active
        .as_ref()
        .or(repo_resolution.startup.as_ref())
        .map(|repo| repo.repo_root.clone());
    if let Some(repo_context) = explicit_repo_context_from_tool_args(tool_args).map_err(|error| {
        Box::new(RepoSelectionError {
            kind: RepoSelectionFailureKind::InvalidExplicitRepoSelector,
            message: error.to_string(),
            candidate_roots: Vec::new(),
            selection_attempts: vec!["explicit_request_repo_root".to_owned()],
            selection_source: Some(RepoSelectionSource::ExplicitRequest),
            tool_name: tool_name.to_owned(),
            recommended_fix: "Pass arguments.repo_root as canonical repo path, or start Atlas with --repo for fixed-mode MCP.".to_owned(),
            session_mode: "fixed",
            active_repo_root: active_repo_root.clone(),
        })
    })? {
        return Ok(RepoSelectionOutcome {
            repo_context,
            selection_source: RepoSelectionSource::ExplicitRequest,
            candidate_roots: None,
        });
    }
    if let Some(active) = repo_resolution.active.clone() {
        return Ok(RepoSelectionOutcome {
            repo_context: active,
            selection_source: repo_resolution
                .active_selection_source
                .unwrap_or(RepoSelectionSource::CachedActiveRoot),
            candidate_roots: None,
        });
    }
    if let Some(repo_context) = repo_resolution.startup.clone() {
        return Ok(RepoSelectionOutcome {
            repo_context,
            selection_source: RepoSelectionSource::ExplicitCli,
            candidate_roots: None,
        });
    }
    Err(Box::new(RepoSelectionError {
        kind: RepoSelectionFailureKind::NoRepoContextAvailable,
        message: "atlas repo context missing; pass --repo or include arguments.repo_root".to_owned(),
        candidate_roots: Vec::new(),
        selection_attempts: vec!["explicit_cli".to_owned(), "explicit_request_repo_root".to_owned()],
        selection_source: None,
        tool_name: tool_name.to_owned(),
        recommended_fix: "Start Atlas with --repo, or pass arguments.repo_root using canonical repo path for repo-bound tools.".to_owned(),
        session_mode: "fixed",
        active_repo_root,
    }))
}

pub(crate) fn annotate_tool_result_with_repo_selection(
    result: &mut serde_json::Value,
    repo_root: &str,
    selection_source: RepoSelectionSource,
    dynamic_mode: bool,
) {
    let Some(object) = result.as_object_mut() else {
        return;
    };
    let meta_value = object.entry("_meta").or_insert_with(|| json!({}));
    let Some(meta) = meta_value.as_object_mut() else {
        return;
    };
    let used_cached_selection = selection_source == RepoSelectionSource::CachedActiveRoot;
    let resolved_this_request = dynamic_mode && !used_cached_selection;
    meta.insert("atlas:repoRoot".to_owned(), json!(repo_root));
    meta.insert(
        "atlas:repoSelection".to_owned(),
        json!({
            "repoRoot": repo_root,
            "selectionSource": selection_source.as_str(),
            "usedCachedSelection": used_cached_selection,
            "resolvedThisRequest": resolved_this_request,
            "sessionMode": if dynamic_mode { "dynamic" } else { "fixed" },
        }),
    );
}

pub(crate) fn method_requires_repo_context(method: &str) -> bool {
    matches!(
        method,
        "resources/read" | "completion/complete" | "tools/call" | "tasks/get" | "tasks/update"
    )
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
