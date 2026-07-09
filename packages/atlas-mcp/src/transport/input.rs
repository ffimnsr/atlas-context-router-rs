//! Per-request input handling: JSON-RPC parsing, routing, and dispatch.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use anyhow::Result;

use crate::logging;
use crate::spec;

use super::dispatch::dispatch;
use super::helpers::{
    ToolRepoResolutionContext, annotate_tool_result_with_repo_selection, ensure_repo_context,
    log_protocol_error_observation, log_request_finished, panic_payload_message,
    parse_client_interaction_capabilities, parse_trace_level, progress_token_from_params,
    resolve_repo_context_for_tool_call, resolve_request_timeout, tool_call_output_format,
    tool_name_from_request,
};
use super::jsonrpc::{
    JsonRpcErrorKind, jsonrpc_dispatch_error, jsonrpc_error, jsonrpc_error_with_data, jsonrpc_ok,
};
use super::notify::{
    emit_mcp_log_notification, emit_progress_notification, emit_trace_log, write_response,
};
use super::repo_selection::{preferred_root_hint_uri, request_active_root_hint_uri};
use super::types::{
    ConnectionState, PendingRequest, ProgressEventKind, RequestCompletion, RequestDispatchContext,
    RequestLogContext, StdioReverseEmitter, TraceThreshold, TransportEvent, TransportStats,
    request_id_string,
};

pub(crate) fn handle_input_line<W: Write>(
    writer: &mut W,
    line: String,
    ctx: &RequestDispatchContext<'_>,
    pending: &mut HashMap<u64, PendingRequest>,
    next_token: &mut u64,
    stats: &mut TransportStats,
    connection_state: &mut ConnectionState,
) -> Result<()> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(());
    }

    stats.received += 1;

    let request: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(error) => {
            stats.parse_errors += 1;
            stats.protocol_errors += 1;
            let request = RequestLogContext {
                request_id: "null".to_owned(),
                method: "<parse>".to_owned(),
                tool_name: None,
            };
            log_protocol_error_observation(
                "stdio",
                &request,
                JsonRpcErrorKind::ParseError.atlas_error_code(),
                &format!("invalid JSON-RPC message: {error}"),
            );
            let response = jsonrpc_error(
                serde_json::Value::Null,
                JsonRpcErrorKind::ParseError,
                format!("invalid JSON-RPC message: {error}"),
            );
            write_response(writer, &response)?;
            return Ok(());
        }
    };

    // Reject batch requests
    if request.is_array() {
        let response = jsonrpc_error(
            serde_json::Value::Null,
            JsonRpcErrorKind::InvalidRequest,
            "JSON-RPC batch requests are not supported".to_owned(),
        );
        write_response(writer, &response)?;
        return Ok(());
    }

    // Try to resolve as reverse-request response
    if request.get("jsonrpc").and_then(|value| value.as_str()) == Some("2.0")
        && request.get("id").is_some()
        && request.get("method").is_none()
        && connection_state
            .reverse_broker
            .try_resolve_response(&request)
    {
        return Ok(());
    }

    // Validate basic JSON-RPC envelope
    if !request.is_object()
        || request.get("jsonrpc").and_then(|value| value.as_str()) != Some("2.0")
        || request
            .get("method")
            .and_then(|value| value.as_str())
            .is_none()
    {
        let id = request
            .get("id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let response = jsonrpc_error(
            id,
            JsonRpcErrorKind::InvalidRequest,
            "invalid request: expected jsonrpc='2.0' and string method".to_owned(),
        );
        write_response(writer, &response)?;
        return Ok(());
    }

    let is_notification = request.get("id").is_none();
    let id = request
        .get("id")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let method = request
        .get("method")
        .and_then(|m| m.as_str())
        .unwrap_or("")
        .to_owned();
    let params = request.get("params");
    let request_log = RequestLogContext {
        request_id: request_id_string(&id),
        method: method.clone(),
        tool_name: tool_name_from_request(&method, params),
    };

    if method == "initialize"
        && let Ok(parsed) = spec::parse_initialize_request(params)
    {
        connection_state.client_capabilities = parsed.capabilities;
        connection_state.repo_resolution.preferred_root_hint_uri =
            preferred_root_hint_uri(parsed.meta.as_ref());
    }

    if method == "initialized" || method == "notifications/initialized" {
        stats.notifications += 1;
        connection_state.initialized = true;
        return Ok(());
    }

    if method == "notifications/roots/list_changed" {
        stats.notifications += 1;
        if connection_state.repo_resolution.dynamic_roots {
            connection_state.repo_resolution.active = None;
            connection_state.repo_resolution.active_selection_source = None;
            connection_state.repo_resolution.candidate_roots = None;
        }
        return Ok(());
    }

    if method.starts_with("notifications/") {
        stats.notifications += 1;
        return Ok(());
    }

    if method == "$/setTrace" {
        stats.notifications += u64::from(is_notification);
        match parse_trace_level(params) {
            Ok(level) => connection_state.trace = level,
            Err(error) => {
                if !is_notification {
                    write_response(
                        writer,
                        &jsonrpc_error(id, JsonRpcErrorKind::InvalidParams, error.to_string()),
                    )?;
                }
                return Ok(());
            }
        }
        if !is_notification {
            write_response(writer, &jsonrpc_ok(id, serde_json::Value::Null))?;
        }
        return Ok(());
    }

    if method == "$/cancelRequest" {
        stats.notifications += 1;
        let _ = cancel_request(writer, pending, params, stats, connection_state);
        return Ok(());
    }

    if method == "logging/setLevel" {
        let level = match logging::parse_set_level_params(params) {
            Ok(level) => level,
            Err(error) => {
                if !is_notification {
                    write_response(
                        writer,
                        &jsonrpc_error(id, JsonRpcErrorKind::InvalidParams, error.to_string()),
                    )?;
                }
                return Ok(());
            }
        };
        logging::set_level(level);
        connection_state.log_level = Some(level);
        logging::write_stdio_log(
            level,
            &format!("client set log level to {}", level.as_str()),
        );
        if !is_notification {
            write_response(
                writer,
                &jsonrpc_ok(id.clone(), serde_json::json!({ "level": level.as_str() })),
            )?;
        }
        emit_mcp_log_notification(
            writer,
            connection_state,
            level,
            "transport",
            format!("log level set to {}", level.as_str()),
        )?;
        return Ok(());
    }

    // tools/call — async dispatch via worker pool
    if method == "tools/call" {
        stats.async_dispatched += 1;
        *next_token += 1;
        let token = *next_token;
        let request_id = id.clone();
        let params = params.cloned();
        let event_tx = ctx.event_tx.clone();
        let method_name = method.clone();
        let timeout = resolve_request_timeout(ctx.server_options, &request_log);
        let queued_at = Instant::now();
        let request_log_for_worker = request_log.clone();
        let canceled_tokens = Arc::clone(&ctx.canceled_tokens);
        let progress_token = progress_token_from_params(params.as_ref());
        let request_active_root_hint_uri = request_active_root_hint_uri(&request);
        let reverse_broker = connection_state.reverse_broker.clone();
        let repo_resolution = connection_state.repo_resolution.clone();
        let client_capabilities = connection_state.client_capabilities.clone();
        let initialized = connection_state.initialized;
        let client_interactions =
            parse_client_interaction_capabilities(&connection_state.client_capabilities);
        tracing::debug!(
            request_id = %request_log.request_id,
            method = %request_log.method,
            tool = request_log.tool_name.as_deref().unwrap_or("-"),
            timeout_ms = timeout.as_millis() as u64,
            "queued MCP request"
        );
        let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let output_format = tool_call_output_format(params.as_ref());
        pending.insert(
            token,
            PendingRequest {
                id: id.clone(),
                request: request_log,
                queued_at,
                deadline: Instant::now() + timeout,
                timeout_ms: timeout.as_millis(),
                output_format,
                progress_token: progress_token.clone(),
                cancel_flag: Arc::clone(&cancel_flag),
            },
        );
        emit_progress_notification(
            writer,
            progress_token.as_ref(),
            ProgressEventKind::Begin,
            "queued",
        )?;
        emit_trace_log(
            writer,
            connection_state.trace,
            TraceThreshold::Messages,
            "info",
            format!(
                "queued request method={} tool={} timeout_ms={}",
                method_name,
                request_log_for_worker.tool_name.as_deref().unwrap_or("-"),
                timeout.as_millis()
            ),
        )?;
        let submit_result = ctx.worker_pool.submit(move || {
            let reverse_emitter: Arc<dyn super::broker::ReverseRequestEmitter> =
                Arc::new(StdioReverseEmitter {
                    event_tx: event_tx.clone(),
                });
            let reverse_scope_id = format!("stdio:{token}");
            let reverse_client = crate::runtime_context::ReverseRequestClient::new(
                Arc::new({
                    let reverse_broker = reverse_broker.clone();
                    let reverse_emitter = Arc::clone(&reverse_emitter);
                    let reverse_scope_id = reverse_scope_id.clone();
                    move |method, params, timeout| {
                        reverse_broker.issue_request(
                            &reverse_scope_id,
                            &reverse_emitter,
                            method,
                            params,
                            timeout,
                        )
                    }
                }),
                Arc::new({
                    let reverse_emitter = Arc::clone(&reverse_emitter);
                    move |params| reverse_emitter.emit_task_status(params)
                }),
                client_interactions.clone(),
                "stdio",
                None,
                request_id_string(&request_id),
            );
            if canceled_tokens
                .lock()
                .expect("canceled token set lock poisoned")
                .remove(&token)
            {
                return;
            }
            let started_at = Instant::now();
            let queue_wait_ms = started_at.duration_since(queued_at).as_millis();
            let _ = event_tx.send(TransportEvent::WorkerStarted {
                token,
                queue_wait_ms,
            });
            tracing::debug!(
                request_id = %request_log_for_worker.request_id,
                method = %request_log_for_worker.method,
                tool = request_log_for_worker.tool_name.as_deref().unwrap_or("-"),
                queue_wait_ms = queue_wait_ms as u64,
                "started MCP request"
            );
            let progress_event_tx = event_tx.clone();
            crate::runtime_context::install(reverse_client);
            crate::tasks::install_tool_call_request_params(params.as_ref());
            crate::progress::install(
                move |msg, pct| {
                    let _ = progress_event_tx.send(TransportEvent::ProgressReport {
                        token,
                        message: msg.to_owned(),
                        percentage: pct,
                    });
                },
                cancel_flag,
            );
            let tool_args = params.as_ref().and_then(|value| value.get("arguments"));
            let selection = match resolve_repo_context_for_tool_call(
                &repo_resolution,
                request_log_for_worker.tool_name.as_deref(),
                tool_args,
                ToolRepoResolutionContext {
                    request_active_root_hint_uri: request_active_root_hint_uri.as_deref(),
                    client_capabilities: &client_capabilities,
                    initialized,
                    reverse_broker: &reverse_broker,
                    reverse_emitter: &reverse_emitter,
                },
            ) {
                Ok(selection) => selection,
                Err(error) => {
                    let _ = event_tx.send(TransportEvent::Response {
                        token,
                        response: jsonrpc_error_with_data(
                            request_id.clone(),
                            JsonRpcErrorKind::InvalidParams,
                            error.message(),
                            Some(error.error_data()),
                        ),
                        completion: RequestCompletion {
                            request: request_log_for_worker.clone(),
                            queue_wait_ms,
                            execution_ms: 0,
                            success: false,
                        },
                    });
                    return;
                }
            };
            let repo_context = selection.repo_context.clone();
            if repo_resolution.dynamic_roots {
                let _ = event_tx.send(TransportEvent::RepoContextResolved {
                    repo_context: selection.repo_context,
                    selection_source: selection.selection_source,
                    candidate_roots: selection.candidate_roots,
                });
            }
            let dispatch_started_at = Instant::now();
            let dispatch_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                dispatch(
                    &method_name,
                    params.as_ref(),
                    &repo_context.repo_root,
                    &repo_context.db_path,
                )
            }));
            crate::progress::uninstall();
            crate::tasks::uninstall_tool_call_request_params();
            crate::runtime_context::uninstall();
            let execution_ms = dispatch_started_at.elapsed().as_millis();
            let (success, response) = match dispatch_result {
                Ok(Ok(mut result)) => {
                    annotate_tool_result_with_repo_selection(
                        &mut result,
                        &repo_context.repo_root,
                        selection.selection_source,
                        repo_resolution.dynamic_roots,
                    );
                    (true, jsonrpc_ok(request_id, result))
                }
                Ok(Err(error)) => (false, jsonrpc_dispatch_error(request_id, &error)),
                Err(payload) => {
                    tracing::error!(
                        method = %method_name,
                        message = %panic_payload_message(&payload),
                        "MCP tool dispatch panicked; returning internal error to client"
                    );
                    (
                        false,
                        jsonrpc_error(
                            request_id,
                            JsonRpcErrorKind::InternalError,
                            "tool panicked; server recovered".to_owned(),
                        ),
                    )
                }
            };
            let _ = event_tx.send(TransportEvent::Response {
                token,
                response,
                completion: RequestCompletion {
                    request: request_log_for_worker,
                    queue_wait_ms,
                    execution_ms,
                    success,
                },
            });
        });
        if let Err(error) = submit_result {
            pending.remove(&token);
            stats.async_dispatched = stats.async_dispatched.saturating_sub(1);
            let response = jsonrpc_error(id, error.kind, error.message);
            write_response(writer, &response)?;
        }
        return Ok(());
    }

    // Synchronous dispatch path (initialize, tools/list, prompts/get, etc.)
    let active_repo = match ensure_repo_context(&method, connection_state, ctx) {
        Ok(active_repo) => active_repo,
        Err(error) => {
            if !is_notification {
                write_response(
                    writer,
                    &jsonrpc_error(id, JsonRpcErrorKind::InvalidParams, error.to_string()),
                )?;
            }
            return Ok(());
        }
    };

    let dispatch_started_at = Instant::now();
    let sync_dispatch_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch(
            &method,
            params,
            active_repo
                .as_ref()
                .map(|ctx| ctx.repo_root.as_str())
                .unwrap_or(""),
            active_repo
                .as_ref()
                .map(|ctx| ctx.db_path.as_str())
                .unwrap_or(""),
        )
    }));
    let elapsed_ms = dispatch_started_at.elapsed().as_millis();
    let response = match sync_dispatch_result {
        Ok(Ok(result)) => {
            log_request_finished(&request_log, true, 0, elapsed_ms, elapsed_ms);
            jsonrpc_ok(id, result)
        }
        Ok(Err(error)) => {
            log_request_finished(&request_log, false, 0, elapsed_ms, elapsed_ms);
            stats.protocol_errors += 1;
            log_protocol_error_observation(
                "stdio",
                &request_log,
                error.kind.atlas_error_code(),
                &error.message(),
            );
            tracing::warn!(
                request_id = %request_log.request_id,
                method = %request_log.method,
                tool = request_log.tool_name.as_deref().unwrap_or("-"),
                error = %error.source,
                "MCP method failed"
            );
            jsonrpc_dispatch_error(id, &error)
        }
        Err(payload) => {
            log_request_finished(&request_log, false, 0, elapsed_ms, elapsed_ms);
            tracing::error!(
                method = %method,
                message = %panic_payload_message(&payload),
                "MCP sync dispatch panicked; returning internal error to client"
            );
            jsonrpc_error(
                id,
                JsonRpcErrorKind::InternalError,
                "method panicked; server recovered".to_owned(),
            )
        }
    };
    write_response(writer, &response)
}

fn cancel_request<W: Write>(
    writer: &mut W,
    pending: &mut HashMap<u64, PendingRequest>,
    params: Option<&serde_json::Value>,
    stats: &mut TransportStats,
    connection_state: &ConnectionState,
) -> Result<()> {
    let target_id = params
        .and_then(|value| value.get("id"))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("$/cancelRequest requires 'id'"))?;
    let Some(token) = pending
        .iter()
        .find_map(|(token, request)| (request.id == target_id).then_some(*token))
    else {
        return Ok(());
    };
    let request = pending
        .remove(&token)
        .expect("pending request must exist while canceling");
    connection_state
        .reverse_broker
        .cancel_scope(&format!("stdio:{token}"), "parent request cancelled");
    request.cancel_flag.store(true, Ordering::Relaxed);
    connection_state
        .canceled_tokens
        .lock()
        .expect("canceled token set lock poisoned")
        .insert(token);
    stats.canceled += 1;
    emit_progress_notification(
        writer,
        request.progress_token.as_ref(),
        ProgressEventKind::End,
        "canceled",
    )?;
    emit_trace_log(
        writer,
        connection_state.trace,
        TraceThreshold::Messages,
        "warning",
        format!(
            "canceled request method={} tool={}",
            request.request.method,
            request.request.tool_name.as_deref().unwrap_or("-")
        ),
    )?;
    Ok(())
}
