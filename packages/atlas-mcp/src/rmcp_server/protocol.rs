//! Free helper fns for the rmcp server surface: progress forwarding,
//! timeouts, logging/trace mapping, repo-root canonicalization, completion
//! shims, and error mapping.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use anyhow::Result;
use rmcp::ErrorData as McpError;
use rmcp::model::{
    ArgumentInfo, CacheScope, CallToolResponse, CompleteRequestParams, LoggingLevel,
    LoggingMessageNotificationParam, ProgressNotificationParam, Reference, Root,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value, json};

use crate::output::OutputFormat;
use crate::runtime_context::ClientInteractionCapabilities;
use crate::spec;
use crate::tool_result::{ToolErrorCode, ToolErrorPayload, tool_execution_error_value};
use crate::transport::{ServerOptions, TraceLevel, TraceThreshold};

use super::{AtlasRmcpProgressContext, AuthenticatedPrincipal};

pub(super) fn start_progress_forwarder(
    context: &RequestContext<RoleServer>,
    cancel_flag: Arc<AtomicBool>,
) -> Option<(AtlasRmcpProgressContext, tokio::task::JoinHandle<()>)> {
    let progress_token = context.meta.get_progress_token()?;
    let peer = context.peer.clone();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<(String, Option<u32>)>();
    let seq = Arc::new(AtomicU64::new(0));
    let seq_for_task = Arc::clone(&seq);
    let handle = tokio::spawn(async move {
        while let Some((message, percentage)) = rx.recv().await {
            let progress = percentage
                .map(|value| value as f64)
                .unwrap_or_else(|| seq_for_task.fetch_add(1, Ordering::Relaxed) as f64);
            let mut notification = ProgressNotificationParam::new(progress_token.clone(), progress)
                .with_message(message);
            if percentage.is_some() {
                notification = notification.with_total(100.0);
            }
            let _ = peer.notify_progress(notification).await;
        }
    });
    Some((AtlasRmcpProgressContext { tx, cancel_flag }, handle))
}

pub(super) fn tool_call_timeout(options: &ServerOptions, tool_name: &str) -> Duration {
    let timeout_ms = options
        .tool_timeout_ms_by_tool
        .get(tool_name)
        .copied()
        .unwrap_or(options.tool_timeout_ms)
        .clamp(1_000, 3_600_000);
    Duration::from_millis(timeout_ms)
}

pub(super) fn timeout_call_tool_response(
    tool_name: &str,
    timeout: Duration,
) -> Result<CallToolResponse, McpError> {
    let timeout_ms = timeout.as_millis();
    let payload = ToolErrorPayload::new(
        ToolErrorCode::Timeout,
        format!("Tool '{tool_name}' timed out after {timeout_ms} ms"),
    )
    .with_tool(tool_name)
    .with_retry_guidance(
        "Reduce request scope, increase timeout if configurable, or retry when dependencies are responsive.",
    )
    .with_details(json!({
        "detail": format!("rmcp stdio timed out after {timeout_ms} ms while handling tool call"),
        "timeout_ms": timeout_ms,
        "tool": tool_name,
    }));
    let atlas_value = tool_execution_error_value(OutputFormat::Json, &payload)
        .map_err(crate::rmcp_error::internal_error)?;
    crate::rmcp_types::call_tool_response_from_atlas_value(atlas_value)
        .map_err(crate::rmcp_error::internal_error)
}

pub(super) fn tool_response_is_error(response: &CallToolResponse) -> bool {
    match response {
        CallToolResponse::Complete(result) => result.is_error == Some(true),
        CallToolResponse::InputRequired(_) | CallToolResponse::Task(_) => false,
        _ => false,
    }
}

pub(super) fn tool_call_log_level(response: &CallToolResponse) -> crate::logging::LogLevel {
    if tool_response_is_error(response) {
        crate::logging::LogLevel::Error
    } else {
        crate::logging::LogLevel::Info
    }
}

pub(super) fn trace_enabled(level: TraceLevel, threshold: TraceThreshold) -> bool {
    match (level, threshold) {
        (TraceLevel::Off, _) => false,
        (TraceLevel::Messages, TraceThreshold::Messages) => true,
        (TraceLevel::Messages, TraceThreshold::Verbose) => false,
        (TraceLevel::Verbose, _) => true,
    }
}

pub(super) fn logging_message_notification_param(
    level: crate::logging::LogLevel,
    message: &str,
) -> LoggingMessageNotificationParam {
    LoggingMessageNotificationParam::new(
        logging_level_to_rmcp(level),
        Value::String(message.to_owned()),
    )
    .with_logger("atlas-mcp")
}

pub(super) fn trace_level_from_value(params: Option<&Value>) -> anyhow::Result<TraceLevel> {
    let level = params
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
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

pub(super) fn logging_level_to_rmcp(level: crate::logging::LogLevel) -> LoggingLevel {
    match level {
        crate::logging::LogLevel::Debug => LoggingLevel::Debug,
        crate::logging::LogLevel::Info => LoggingLevel::Info,
        crate::logging::LogLevel::Notice => LoggingLevel::Notice,
        crate::logging::LogLevel::Warning => LoggingLevel::Warning,
        crate::logging::LogLevel::Error => LoggingLevel::Error,
    }
}

pub(super) fn logging_level_from_rmcp(level: LoggingLevel) -> crate::logging::LogLevel {
    match level {
        LoggingLevel::Debug => crate::logging::LogLevel::Debug,
        LoggingLevel::Info => crate::logging::LogLevel::Info,
        LoggingLevel::Notice => crate::logging::LogLevel::Notice,
        LoggingLevel::Warning => crate::logging::LogLevel::Warning,
        LoggingLevel::Error
        | LoggingLevel::Critical
        | LoggingLevel::Alert
        | LoggingLevel::Emergency => crate::logging::LogLevel::Error,
    }
}

pub(super) fn authenticated_principal(context: &RequestContext<RoleServer>) -> Option<String> {
    context
        .extensions
        .get::<AuthenticatedPrincipal>()
        .map(|principal| principal.0.clone())
}

pub(super) fn canonical_repo_roots_from_roots(roots: &[Root]) -> anyhow::Result<Vec<String>> {
    use atlas_repo::{canonical_filesystem_path, find_repo_root};
    use camino::Utf8PathBuf;

    let mut canonical = Vec::new();
    for root in roots {
        let parsed = url::Url::parse(&root.uri)
            .map_err(|error| anyhow::anyhow!("invalid roots/list URI '{}': {error}", root.uri))?;
        anyhow::ensure!(
            parsed.scheme() == "file",
            "unsupported roots/list URI scheme '{}' for '{}'",
            parsed.scheme(),
            root.uri
        );
        let file_path = parsed.to_file_path().map_err(|_| {
            anyhow::anyhow!("roots/list URI is not a valid file path: {}", root.uri)
        })?;
        let utf8 = Utf8PathBuf::from_path_buf(file_path)
            .map_err(|_| anyhow::anyhow!("roots/list URI path is not valid UTF-8: {}", root.uri))?;
        let repo_root = find_repo_root(utf8.as_path()).unwrap_or(utf8);
        canonical.push(
            canonical_filesystem_path(repo_root.as_path())
                .map_err(|error| {
                    anyhow::anyhow!("invalid roots/list repo root '{}': {error}", repo_root)
                })?
                .into_string(),
        );
    }
    canonical.sort();
    canonical.dedup();
    Ok(canonical)
}

pub(super) fn cache_scope_from_str(scope: &str) -> CacheScope {
    match scope {
        spec::CACHE_SCOPE_PRIVATE => CacheScope::Private,
        _ => CacheScope::Public,
    }
}

pub(super) fn client_interaction_capabilities(
    client_capabilities: Option<&Value>,
) -> ClientInteractionCapabilities {
    let elicitation = client_capabilities.and_then(|value| value.get("elicitation"));
    ClientInteractionCapabilities {
        supports_elicitation_form: elicitation.and_then(|value| value.get("form")).is_some(),
        supports_elicitation_url: elicitation.and_then(|value| value.get("url")).is_some(),
    }
}

pub(super) fn request_id_string(context: &RequestContext<RoleServer>) -> String {
    match serde_json::to_value(&context.id) {
        Ok(Value::String(value)) => value,
        Ok(other) => other.to_string(),
        Err(_) => context.id.to_string(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct BootstrapNotificationPlan {
    pub(super) send_tool_list_changed: bool,
    pub(super) send_prompt_list_changed: bool,
    pub(super) send_resource_list_changed: bool,
    pub(super) resource_updates: Vec<String>,
}

pub(super) fn legacy_completion_request_value(
    request: &CompleteRequestParams,
) -> anyhow::Result<Value> {
    let mut object = Map::new();
    object.insert(
        "argument".to_owned(),
        serde_json::to_value(&request.argument)?,
    );
    if let Some(context) = &request.context {
        object.insert("context".to_owned(), serde_json::to_value(context)?);
    }
    object.insert(
        "ref".to_owned(),
        legacy_completion_reference_value(&request.r#ref, &request.argument)?,
    );
    Ok(Value::Object(object))
}

pub(super) fn legacy_completion_reference_value(
    reference: &Reference,
    argument: &ArgumentInfo,
) -> anyhow::Result<Value> {
    match reference {
        Reference::Prompt(prompt) => Ok(json!({ "name": prompt.name })),
        Reference::Resource(resource) => {
            if argument.name == "uri" {
                Ok(json!({ "name": "resources/read" }))
            } else {
                Ok(json!({ "uriTemplate": resource.uri }))
            }
        }
        _ => Err(anyhow::anyhow!(
            "unsupported completion reference type: {}",
            reference.reference_type()
        )),
    }
}

pub(super) fn map_prompt_error(error: anyhow::Error) -> McpError {
    let detail = error.to_string();
    if detail.starts_with("unknown prompt:")
        || detail.starts_with("missing ")
        || detail.starts_with("argument '")
        || detail.contains("missing required argument:")
        || detail.contains("requires non-empty")
        || detail.contains("must be ")
        || detail.contains("invalid regex pattern")
    {
        crate::rmcp_error::invalid_params(detail, None)
    } else {
        crate::rmcp_error::internal_error(error)
    }
}

pub(super) fn map_invalid_params_error(error: anyhow::Error) -> McpError {
    crate::rmcp_error::invalid_params(error.to_string(), None)
}

pub(super) fn map_task_api_error(error: crate::tasks::TaskApiError) -> McpError {
    match error.kind() {
        crate::tasks::TaskApiErrorKind::InvalidParams => {
            crate::rmcp_error::invalid_params(error.message(), None)
        }
        crate::tasks::TaskApiErrorKind::NotFound => {
            crate::rmcp_error::invalid_params(error.message(), None)
        }
        crate::tasks::TaskApiErrorKind::Cancelled
        | crate::tasks::TaskApiErrorKind::Failed
        | crate::tasks::TaskApiErrorKind::Internal
        | crate::tasks::TaskApiErrorKind::NotReady => {
            crate::rmcp_error::internal_error(error.into_anyhow())
        }
    }
}
