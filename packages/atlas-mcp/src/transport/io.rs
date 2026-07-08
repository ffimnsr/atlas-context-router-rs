//! Core I/O processing loop and event handlers for the JSON-RPC transport.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Instant;

use anyhow::{Context, Result};

use crate::logging;

use super::helpers::{
    log_request_finished, log_tool_execution_error_observation, timeout_tool_call_result,
    user_visible_error_message,
};
use super::input::handle_input_line;
use super::jsonrpc::{JsonRpcErrorKind, jsonrpc_error, jsonrpc_ok};
use super::notify::{
    emit_mcp_log_notification, emit_progress_notification, emit_trace_log, write_response,
};
use super::types::{
    ConnectionState, PendingRequest, ProgressEventKind, RequestCompletion, TransportEvent,
    TransportStats, connection_state,
};
use super::worker::WorkerPool;

// ---------------------------------------------------------------------------
// I/O entry points
// ---------------------------------------------------------------------------

pub(crate) fn run_server_io<R: BufRead + Send, W: Write>(
    reader: R,
    writer: &mut W,
    repo_root: &str,
    db_path: &str,
    options: super::types::ServerOptions,
) -> Result<()> {
    run_server_io_with_state(
        reader,
        writer,
        Some(repo_root),
        Some(db_path),
        false,
        options,
    )
}

pub(crate) fn run_server_io_with_state<R: BufRead + Send, W: Write>(
    reader: R,
    writer: &mut W,
    repo_root: Option<&str>,
    db_path: Option<&str>,
    dynamic_roots: bool,
    options: super::types::ServerOptions,
) -> Result<()> {
    let worker_pool = Arc::new(WorkerPool::from_env(
        "atlas-mcp:tool-worker",
        options.clone(),
    )?);
    serve_connection(
        reader,
        writer,
        repo_root,
        db_path,
        dynamic_roots,
        worker_pool,
        options,
    )
}

pub(crate) fn serve_connection<R: BufRead + Send, W: Write>(
    reader: R,
    writer: &mut W,
    repo_root: Option<&str>,
    db_path: Option<&str>,
    dynamic_roots: bool,
    worker_pool: Arc<WorkerPool>,
    server_options: super::types::ServerOptions,
) -> Result<()> {
    let (event_tx, event_rx) = mpsc::channel::<TransportEvent>();
    let connection_state = connection_state(repo_root, db_path, dynamic_roots);

    std::thread::scope(|scope| -> Result<()> {
        let reader_tx = event_tx.clone();
        scope.spawn(move || read_requests(reader, reader_tx));
        process_requests(
            writer,
            worker_pool.as_ref(),
            &server_options,
            event_tx,
            event_rx,
            connection_state,
        )
    })
}

// ---------------------------------------------------------------------------
// Handshake (used by both Unix and Windows socket transports)
// ---------------------------------------------------------------------------

pub(crate) fn perform_socket_handshake<R: BufRead, W: Write>(
    reader: &mut R,
    writer: &mut W,
    repo_root: &str,
    db_path: &str,
) -> Result<()> {
    let mut line = String::new();
    let bytes = reader.read_line(&mut line)?;
    if bytes == 0 {
        return Err(anyhow::anyhow!("daemon handshake missing"));
    }

    use super::dispatch::{DaemonHandshakeRequest, DaemonHandshakeResponse};
    use crate::MCP_PROTOCOL_VERSION;
    use crate::spec;

    let request: DaemonHandshakeRequest =
        serde_json::from_str(line.trim()).context("invalid daemon handshake")?;
    let response = if request.protocol_version != spec::MCP_PROTOCOL_VERSION {
        DaemonHandshakeResponse::err(
            spec::MCP_PROTOCOL_VERSION,
            repo_root,
            db_path,
            format!(
                "protocol mismatch: client={} server={}",
                request.protocol_version,
                spec::MCP_PROTOCOL_VERSION
            ),
        )
    } else if request.repo_root != repo_root {
        DaemonHandshakeResponse::err(
            MCP_PROTOCOL_VERSION,
            repo_root,
            db_path,
            format!(
                "repo mismatch: client={} server={repo_root}",
                request.repo_root
            ),
        )
    } else if request.db_path != db_path {
        DaemonHandshakeResponse::err(
            MCP_PROTOCOL_VERSION,
            repo_root,
            db_path,
            format!("db mismatch: client={} server={db_path}", request.db_path),
        )
    } else {
        DaemonHandshakeResponse::ok(MCP_PROTOCOL_VERSION, repo_root, db_path)
    };

    writeln!(writer, "{}", serde_json::to_string(&response)?)?;
    writer.flush()?;

    if response.ok {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            response
                .error
                .unwrap_or_else(|| "daemon handshake failed".to_owned())
        ))
    }
}

// ---------------------------------------------------------------------------
// Main processing loop
// ---------------------------------------------------------------------------

pub(crate) fn process_requests<W: Write>(
    writer: &mut W,
    worker_pool: &WorkerPool,
    server_options: &super::types::ServerOptions,
    event_tx: mpsc::Sender<TransportEvent>,
    event_rx: mpsc::Receiver<TransportEvent>,
    mut connection_state: ConnectionState,
) -> Result<()> {
    let mut input_closed = false;
    let mut next_token = 0_u64;
    let mut pending = HashMap::<u64, PendingRequest>::new();
    let mut stats = TransportStats::default();
    let request_ctx = super::types::RequestDispatchContext {
        worker_pool,
        server_options,
        canceled_tokens: Arc::clone(&connection_state.canceled_tokens),
        event_tx: &event_tx,
    };

    loop {
        drain_expired_requests(writer, &mut pending, &mut stats, &connection_state)?;
        if input_closed && pending.is_empty() {
            break;
        }

        let recv_result = match next_deadline(&pending) {
            Some(deadline) => {
                let now = Instant::now();
                if deadline <= now {
                    continue;
                }
                event_rx.recv_timeout(deadline.saturating_duration_since(now))
            }
            None => event_rx
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };

        match recv_result {
            Ok(TransportEvent::InputLine(line)) => handle_input_line(
                writer,
                line,
                &request_ctx,
                &mut pending,
                &mut next_token,
                &mut stats,
                &mut connection_state,
            )?,
            Ok(TransportEvent::WorkerStarted {
                token,
                queue_wait_ms,
            }) => handle_worker_started(
                writer,
                &mut pending,
                token,
                queue_wait_ms,
                &connection_state,
            )?,
            Ok(TransportEvent::ProgressReport {
                token,
                message,
                percentage,
            }) => handle_progress_report(writer, &pending, token, &message, percentage)?,
            Ok(TransportEvent::Response {
                token,
                response,
                completion,
            }) => handle_completion_event(
                writer,
                &mut pending,
                token,
                response,
                completion,
                &mut stats,
                &connection_state,
            )?,
            Ok(TransportEvent::OutboundJson(message)) => write_response(writer, &message)?,
            Ok(TransportEvent::InputClosed) => {
                input_closed = true;
            }
            Ok(TransportEvent::InputError(message)) => {
                return Err(anyhow::anyhow!(message));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                if input_closed || pending.is_empty() {
                    break;
                }
                return Err(anyhow::anyhow!(
                    "transport event channel disconnected unexpectedly"
                ));
            }
        }
    }

    tracing::info!(
        received = stats.received,
        notifications = stats.notifications,
        parse_errors = stats.parse_errors,
        async_dispatched = stats.async_dispatched,
        completed = stats.completed,
        completed_ok = stats.completed_ok,
        completed_err = stats.completed_err,
        protocol_errors = stats.protocol_errors,
        tool_execution_errors = stats.tool_execution_errors,
        timed_out = stats.timed_out,
        canceled = stats.canceled,
        dropped_late = stats.dropped_late,
        "MCP transport summary"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Input reading
// ---------------------------------------------------------------------------

const MAX_INPUT_LINE_BYTES: usize = 16 * 1024 * 1024; // 16 MiB

fn read_requests<R: BufRead>(reader: R, event_tx: mpsc::Sender<TransportEvent>) {
    let mut reader = reader;
    let mut raw: Vec<u8> = Vec::with_capacity(4096);
    loop {
        raw.clear();
        match read_limited_line(&mut reader, &mut raw, MAX_INPUT_LINE_BYTES) {
            Ok(false) => break,
            Ok(true) => {
                if raw.last() == Some(&b'\n') {
                    raw.pop();
                }
                if raw.last() == Some(&b'\r') {
                    raw.pop();
                }
                let line = match String::from_utf8(raw.clone()) {
                    Ok(s) => s,
                    Err(_) => {
                        let _ = event_tx.send(TransportEvent::InputError(
                            "invalid UTF-8 in input line".to_owned(),
                        ));
                        return;
                    }
                };
                if event_tx.send(TransportEvent::InputLine(line)).is_err() {
                    return;
                }
            }
            Err(error) => {
                let _ = event_tx.send(TransportEvent::InputError(format!(
                    "stdin read error: {error}"
                )));
                return;
            }
        }
    }

    let _ = event_tx.send(TransportEvent::InputClosed);
}

fn read_limited_line<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    limit: usize,
) -> std::io::Result<bool> {
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(!buf.is_empty());
        }
        match available.iter().position(|&b| b == b'\n') {
            Some(i) => {
                let end = i + 1;
                if buf.len() + end > limit {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("input line exceeds {limit} byte limit"),
                    ));
                }
                buf.extend_from_slice(&available[..end]);
                reader.consume(end);
                return Ok(true);
            }
            None => {
                let n = available.len();
                if buf.len() + n > limit {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("input line exceeds {limit} byte limit"),
                    ));
                }
                buf.extend_from_slice(available);
                reader.consume(n);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Request lifecycle helpers
// ---------------------------------------------------------------------------

fn drain_expired_requests<W: Write>(
    writer: &mut W,
    pending: &mut HashMap<u64, PendingRequest>,
    stats: &mut TransportStats,
    connection_state: &ConnectionState,
) -> Result<()> {
    let now = Instant::now();
    let mut expired_tokens = Vec::new();
    for (token, request) in pending.iter() {
        if request.deadline <= now {
            expired_tokens.push(*token);
        }
    }

    for token in expired_tokens {
        if let Some(request) = pending.remove(&token) {
            connection_state
                .reverse_broker
                .cancel_scope(&format!("stdio:{token}"), "parent request timed out");
            stats.timed_out += 1;
            tracing::warn!(
                request_id = %request.request.request_id,
                method = %request.request.method,
                tool = request.request.tool_name.as_deref().unwrap_or("-"),
                total_ms = request.queued_at.elapsed().as_millis() as u64,
                timeout_ms = request.timeout_ms as u64,
                "MCP request timed out"
            );
            let response = match timeout_tool_call_result(&request) {
                Ok(result) => jsonrpc_ok(request.id.clone(), result),
                Err(error) => jsonrpc_error(
                    request.id.clone(),
                    JsonRpcErrorKind::InternalError,
                    user_visible_error_message(&error),
                ),
            };
            emit_progress_notification(
                writer,
                request.progress_token.as_ref(),
                ProgressEventKind::End,
                "timed out",
            )?;
            emit_trace_log(
                writer,
                connection_state.trace,
                super::types::TraceThreshold::Messages,
                "error",
                format!(
                    "timed out request method={} tool={} timeout_ms={}",
                    request.request.method,
                    request.request.tool_name.as_deref().unwrap_or("-"),
                    request.timeout_ms
                ),
            )?;
            write_response(writer, &response)?;
        }
    }

    Ok(())
}

fn handle_completion_event<W: Write>(
    writer: &mut W,
    pending: &mut HashMap<u64, PendingRequest>,
    token: u64,
    response: String,
    completion: RequestCompletion,
    stats: &mut TransportStats,
    connection_state: &ConnectionState,
) -> Result<()> {
    if let Some(request) = pending.remove(&token) {
        stats.completed += 1;
        if completion.success {
            stats.completed_ok += 1;
        } else {
            stats.completed_err += 1;
        }
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&response)
            && let Some(result) = value.get("result")
            && result.get("isError").and_then(serde_json::Value::as_bool) == Some(true)
        {
            stats.tool_execution_errors += 1;
            log_tool_execution_error_observation("stdio", &completion.request, result);
        }
        log_request_finished(
            &completion.request,
            completion.success,
            completion.queue_wait_ms,
            completion.execution_ms,
            request.queued_at.elapsed().as_millis(),
        );
        emit_progress_notification(
            writer,
            request.progress_token.as_ref(),
            ProgressEventKind::End,
            if completion.success {
                "completed"
            } else {
                "failed"
            },
        )?;
        emit_trace_log(
            writer,
            connection_state.trace,
            super::types::TraceThreshold::Messages,
            if completion.success { "info" } else { "error" },
            format!(
                "completed request method={} tool={} success={} total_ms={}",
                completion.request.method,
                completion.request.tool_name.as_deref().unwrap_or("-"),
                completion.success,
                request.queued_at.elapsed().as_millis()
            ),
        )?;
        emit_mcp_log_notification(
            writer,
            connection_state,
            if completion.success {
                logging::LogLevel::Info
            } else {
                logging::LogLevel::Error
            },
            "tools/call",
            format!(
                "tool={} success={} total_ms={}",
                completion.request.tool_name.as_deref().unwrap_or("-"),
                completion.success,
                request.queued_at.elapsed().as_millis()
            ),
        )?;
        write_response(writer, &response)?;
    } else {
        stats.dropped_late += 1;
        tracing::debug!(
            request_id = %completion.request.request_id,
            method = %completion.request.method,
            tool = completion.request.tool_name.as_deref().unwrap_or("-"),
            queue_wait_ms = completion.queue_wait_ms as u64,
            execution_ms = completion.execution_ms as u64,
            success = completion.success,
            "dropping late MCP response"
        );
    }

    Ok(())
}

fn next_deadline(pending: &HashMap<u64, PendingRequest>) -> Option<Instant> {
    pending.values().map(|request| request.deadline).min()
}

fn handle_worker_started<W: Write>(
    writer: &mut W,
    pending: &mut HashMap<u64, PendingRequest>,
    token: u64,
    queue_wait_ms: u128,
    connection_state: &ConnectionState,
) -> Result<()> {
    let Some(request) = pending.get(&token) else {
        return Ok(());
    };
    emit_progress_notification(
        writer,
        request.progress_token.as_ref(),
        ProgressEventKind::Report,
        "running",
    )?;
    emit_trace_log(
        writer,
        connection_state.trace,
        super::types::TraceThreshold::Verbose,
        "debug",
        format!(
            "started request method={} tool={} queue_wait_ms={queue_wait_ms}",
            request.request.method,
            request.request.tool_name.as_deref().unwrap_or("-")
        ),
    )?;
    Ok(())
}

fn handle_progress_report<W: Write>(
    writer: &mut W,
    pending: &HashMap<u64, PendingRequest>,
    token: u64,
    message: &str,
    percentage: Option<u32>,
) -> Result<()> {
    let Some(request) = pending.get(&token) else {
        return Ok(());
    };
    let msg = match percentage {
        Some(pct) => format!("{message} ({pct}%)"),
        None => message.to_owned(),
    };
    emit_progress_notification(
        writer,
        request.progress_token.as_ref(),
        ProgressEventKind::Report,
        &msg,
    )
}
