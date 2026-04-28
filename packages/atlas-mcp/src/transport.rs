//! JSON-RPC 2.0 / MCP stdio transport loop.
//!
//! Reads newline-delimited JSON from stdin, dispatches each request, and
//! writes newline-delimited JSON responses to stdout.  Follows the MCP
//! 2024-11-05 protocol specification.

use std::collections::{HashMap, HashSet};
use std::io::BufReader;
use std::io::{BufRead, Write};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use atlas_core::user_facing_error_message;
use serde::{Deserialize, Serialize};

use crate::{prompts, tools};

#[cfg(unix)]
use std::net::Shutdown;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
#[cfg(unix)]
use std::sync::Mutex;
#[cfg(unix)]
use std::sync::atomic::AtomicBool;

#[cfg(windows)]
use std::os::windows::io::FromRawHandle;
#[cfg(windows)]
use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, FALSE, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE,
};
#[cfg(windows)]
use windows_sys::Win32::Security::{
    CopySid, EqualSid, GetLengthSid, GetTokenInformation, OpenProcessToken, TOKEN_QUERY,
    TOKEN_USER, TokenUser,
};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_ACCESS_DUPLEX,
    PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
#[cfg(windows)]
use windows_sys::Win32::System::Threading::{
    DUPLICATE_SAME_ACCESS, GetCurrentProcess, GetCurrentProcessId, OpenProcess,
    PROCESS_QUERY_LIMITED_INFORMATION,
};

const MCP_WORKER_THREADS_ENV: &str = "ATLAS_MCP_WORKER_THREADS";
const MCP_TOOL_TIMEOUT_MS_ENV: &str = "ATLAS_MCP_TOOL_TIMEOUT_MS";
const DEFAULT_WORKER_THREADS: usize = 2;
const DEFAULT_TOOL_TIMEOUT_MS: u64 = 300_000;
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const JSONRPC_PARSE_ERROR: i32 = -32700;
const JSONRPC_INVALID_REQUEST: i32 = -32600;
const JSONRPC_METHOD_NOT_FOUND: i32 = -32601;
const JSONRPC_INVALID_PARAMS: i32 = -32602;
const JSONRPC_INTERNAL_ERROR: i32 = -32603;
const JSONRPC_TOOL_EXECUTION_FAILED: i32 = -32001;
const JSONRPC_WORKER_UNAVAILABLE: i32 = -32002;
const JSONRPC_REQUEST_TIMEOUT: i32 = -32003;
const JSONRPC_RATE_LIMITED: i32 = -32004;

#[derive(Clone, Debug)]
pub struct ServerOptions {
    pub worker_threads: usize,
    pub tool_timeout_ms: u64,
    pub tool_timeout_ms_by_tool: HashMap<String, u64>,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            worker_threads: DEFAULT_WORKER_THREADS,
            tool_timeout_ms: DEFAULT_TOOL_TIMEOUT_MS,
            tool_timeout_ms_by_tool: HashMap::new(),
        }
    }
}

/// Run the MCP server until stdin closes.
///
/// * `repo_root` – absolute path to the git repository root; used by tools
///   that need git context (e.g. `detect_changes`).
/// * `db_path`   – path to the Atlas SQLite database.
pub fn run_server(repo_root: &str, db_path: &str) -> Result<()> {
    run_server_with_options(repo_root, db_path, ServerOptions::default())
}

pub fn run_server_with_options(
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
) -> Result<()> {
    crate::tools::health::mark_server_started();
    eprintln!("atlas-mcp: server ready (repo={repo_root}, db={db_path})");
    eprintln!("atlas-mcp: reading JSON-RPC requests from stdin");

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let reader = BufReader::new(stdin);
    let mut writer = std::io::BufWriter::new(stdout.lock());
    #[cfg(unix)]
    let _shutdown_guard = install_stdio_shutdown_handler()?;

    run_server_io(reader, &mut writer, repo_root, db_path, options)
}

#[cfg(unix)]
pub fn run_socket_server_with_options(
    socket_path: &Path,
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
) -> Result<()> {
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    if socket_path.exists() {
        std::fs::remove_file(socket_path)
            .with_context(|| format!("cannot remove stale {}", socket_path.display()))?;
    }

    // Set umask to 0o177 before bind so the socket is created as 0600 directly,
    // eliminating the race window between bind and the chmod below.
    let old_mask = unsafe { libc::umask(0o177) };
    let bind_result = UnixListener::bind(socket_path);
    unsafe { libc::umask(old_mask) };
    let listener = bind_result.with_context(|| format!("cannot bind {}", socket_path.display()))?;
    // Defense-in-depth: re-apply 0600 in case the umask was ineffective.
    secure_socket_path(socket_path)?;
    listener
        .set_nonblocking(true)
        .with_context(|| format!("cannot set nonblocking {}", socket_path.display()))?;
    crate::tools::health::mark_server_started();
    let worker_pool = Arc::new(WorkerPool::from_env(
        "atlas-mcp:tool-worker",
        options.clone(),
    )?);
    let next_connection = AtomicUsize::new(0);
    let shutdown = Arc::new(AtomicBool::new(false));
    let active_streams = Arc::new(Mutex::new(Vec::<UnixStream>::new()));
    let _shutdown_guard =
        install_socket_shutdown_handler(Arc::clone(&shutdown), Arc::clone(&active_streams))?;

    eprintln!(
        "atlas-mcp: daemon ready (repo={repo_root}, db={db_path}, socket={})",
        socket_path.display()
    );

    while !shutdown.load(Ordering::Relaxed) {
        let stream = match listener.accept() {
            Ok((stream, _addr)) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(25));
                continue;
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_error) if shutdown.load(Ordering::Relaxed) => break,
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("cannot accept connection on {}", socket_path.display())
                });
            }
        };
        // On macOS (and other BSDs), an accepted socket inherits the
        // non-blocking flag from the listening socket. Reset to blocking so
        // that serve_socket_connection can use normal blocking I/O without
        // getting WouldBlock errors on reads.
        if let Err(error) = stream.set_nonblocking(false) {
            tracing::warn!(error = %error, "cannot set accepted socket to blocking; skipping connection");
            continue;
        }
        let repo_root = repo_root.to_owned();
        let db_path = db_path.to_owned();
        let worker_pool = Arc::clone(&worker_pool);
        let server_options = options.clone();
        let active_streams = Arc::clone(&active_streams);
        let connection_id = next_connection.fetch_add(1, Ordering::Relaxed) + 1;
        let thread_name = format!("atlas-mcp:daemon-client-{connection_id}");
        let log_thread_name = thread_name.clone();
        thread::Builder::new()
            .name(thread_name.clone())
            .spawn(move || {
                if let Err(error) = serve_socket_connection(
                    stream,
                    &repo_root,
                    &db_path,
                    worker_pool,
                    server_options,
                    active_streams,
                ) {
                    tracing::warn!(error = %error, client = %log_thread_name, "daemon client failed");
                }
            })
            .with_context(|| format!("cannot spawn '{thread_name}'"))?;
    }

    Ok(())
}

#[cfg(windows)]
pub fn run_socket_server_with_options(
    pipe_path: &Path,
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;

    let pipe_name: Vec<u16> = pipe_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    let worker_pool = Arc::new(WorkerPool::from_env(
        "atlas-mcp:tool-worker",
        options.clone(),
    )?);
    let next_connection = AtomicUsize::new(0);

    eprintln!(
        "atlas-mcp: daemon ready (repo={repo_root}, db={db_path}, pipe={})",
        pipe_path.display()
    );

    loop {
        let pipe_handle = win_create_named_pipe_server(&pipe_name)?;

        // ConnectNamedPipe blocks until a client connects; ERROR_PIPE_CONNECTED
        // means a client connected between CreateNamedPipe and ConnectNamedPipe.
        let connected = unsafe {
            ConnectNamedPipe(pipe_handle, std::ptr::null_mut()) != 0
                || GetLastError() == ERROR_PIPE_CONNECTED
        };
        if !connected {
            let err_code = unsafe { GetLastError() };
            unsafe { CloseHandle(pipe_handle) };
            if err_code == ERROR_NO_DATA {
                // Client connected and disconnected before we serviced it; continue.
                continue;
            }
            return Err(std::io::Error::from_raw_os_error(err_code as i32))
                .context("ConnectNamedPipe failed");
        }

        let repo_root = repo_root.to_owned();
        let db_path = db_path.to_owned();
        let worker_pool = Arc::clone(&worker_pool);
        let server_options = options.clone();
        let connection_id = next_connection.fetch_add(1, Ordering::Relaxed) + 1;
        let thread_name = format!("atlas-mcp:daemon-client-{connection_id}");
        let log_thread_name = thread_name.clone();

        thread::Builder::new()
            .name(thread_name.clone())
            .spawn(move || {
                if let Err(error) = win_serve_pipe_connection(
                    pipe_handle,
                    &repo_root,
                    &db_path,
                    worker_pool,
                    server_options,
                ) {
                    tracing::warn!(
                        error = %error,
                        client = %log_thread_name,
                        "daemon pipe client failed"
                    );
                }
            })
            .with_context(|| format!("cannot spawn '{thread_name}'"))?;
    }
}

#[cfg(not(any(unix, windows)))]
pub fn run_socket_server_with_options(
    _socket_path: &Path,
    _repo_root: &str,
    _db_path: &str,
    _options: ServerOptions,
) -> Result<()> {
    Err(anyhow::anyhow!(
        "MCP daemon transport is unsupported on this platform"
    ))
}

fn run_server_io<R: BufRead + Send, W: Write>(
    reader: R,
    writer: &mut W,
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
) -> Result<()> {
    let worker_pool = Arc::new(WorkerPool::from_env(
        "atlas-mcp:tool-worker",
        options.clone(),
    )?);
    serve_connection(reader, writer, repo_root, db_path, worker_pool, options)
}

fn serve_connection<R: BufRead + Send, W: Write>(
    reader: R,
    writer: &mut W,
    repo_root: &str,
    db_path: &str,
    worker_pool: Arc<WorkerPool>,
    server_options: ServerOptions,
) -> Result<()> {
    let (event_tx, event_rx) = mpsc::channel::<TransportEvent>();
    let connection_state = ConnectionState {
        trace: TraceLevel::Off,
        canceled_tokens: Arc::new(Mutex::new(HashSet::new())),
    };

    thread::scope(|scope| -> Result<()> {
        let reader_tx = event_tx.clone();
        scope.spawn(move || read_requests(reader, reader_tx));
        process_requests(
            writer,
            repo_root,
            db_path,
            worker_pool.as_ref(),
            &server_options,
            event_tx,
            event_rx,
            connection_state,
        )
    })
}

#[cfg(unix)]
fn serve_socket_connection(
    stream: UnixStream,
    repo_root: &str,
    db_path: &str,
    worker_pool: Arc<WorkerPool>,
    server_options: ServerOptions,
    active_streams: Arc<Mutex<Vec<UnixStream>>>,
) -> Result<()> {
    ensure_peer_uid_allowed(&stream)?;
    let registered_stream = stream
        .try_clone()
        .context("cannot clone daemon stream for shutdown")?;
    let registered_fd = registered_stream.as_raw_fd();
    active_streams
        .lock()
        .expect("active daemon streams lock poisoned")
        .push(registered_stream);
    let _stream_guard = ActiveStreamGuard::new(registered_fd, active_streams);
    let reader_stream = stream.try_clone().context("cannot clone daemon stream")?;
    let mut reader = BufReader::new(reader_stream);
    let mut writer = std::io::BufWriter::new(stream);

    perform_socket_handshake(&mut reader, &mut writer, repo_root, db_path)?;
    serve_connection(
        reader,
        &mut writer,
        repo_root,
        db_path,
        worker_pool,
        server_options,
    )
}

fn perform_socket_handshake<R: BufRead, W: Write>(
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

    let request: DaemonHandshakeRequest =
        serde_json::from_str(line.trim()).context("invalid daemon handshake")?;
    let response = if request.protocol_version != MCP_PROTOCOL_VERSION {
        DaemonHandshakeResponse::err(
            MCP_PROTOCOL_VERSION,
            repo_root,
            db_path,
            format!(
                "protocol mismatch: client={} server={}",
                request.protocol_version, MCP_PROTOCOL_VERSION
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

#[allow(clippy::too_many_arguments)]
fn process_requests<W: Write>(
    writer: &mut W,
    repo_root: &str,
    db_path: &str,
    worker_pool: &WorkerPool,
    server_options: &ServerOptions,
    event_tx: mpsc::Sender<TransportEvent>,
    event_rx: mpsc::Receiver<TransportEvent>,
    mut connection_state: ConnectionState,
) -> Result<()> {
    let mut input_closed = false;
    let mut next_token = 0_u64;
    let mut pending = HashMap::<u64, PendingRequest>::new();
    let mut stats = TransportStats::default();
    let request_ctx = RequestDispatchContext {
        repo_root,
        db_path,
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
        timed_out = stats.timed_out,
        canceled = stats.canceled,
        dropped_late = stats.dropped_late,
        "MCP transport summary"
    );

    Ok(())
}

fn read_requests<R: BufRead>(reader: R, event_tx: mpsc::Sender<TransportEvent>) {
    for line in reader.lines() {
        match line {
            Ok(line) => {
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

fn handle_input_line<W: Write>(
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
            let response = jsonrpc_error(
                serde_json::Value::Null,
                JsonRpcErrorKind::ParseError,
                format!("parse error: {error}"),
            );
            write_response(writer, &response)?;
            return Ok(());
        }
    };

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

    if method == "initialized" || method.starts_with("notifications/") {
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

    if method == "tools/call" {
        stats.async_dispatched += 1;
        *next_token += 1;
        let token = *next_token;
        let request_id = id.clone();
        let params = params.cloned();
        let repo_root = ctx.repo_root.to_owned();
        let db_path = ctx.db_path.to_owned();
        let event_tx = ctx.event_tx.clone();
        let method_name = method.clone();
        let timeout = resolve_request_timeout(ctx.server_options, &request_log);
        let queued_at = Instant::now();
        let request_log_for_worker = request_log.clone();
        let canceled_tokens = Arc::clone(&ctx.canceled_tokens);
        let progress_token = progress_token_from_params(params.as_ref());
        tracing::debug!(
            request_id = %request_log.request_id,
            method = %request_log.method,
            tool = request_log.tool_name.as_deref().unwrap_or("-"),
            timeout_ms = timeout.as_millis() as u64,
            "queued MCP request"
        );
        let cancel_flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        pending.insert(
            token,
            PendingRequest {
                id: id.clone(),
                request: request_log,
                queued_at,
                deadline: Instant::now() + timeout,
                timeout_ms: timeout.as_millis(),
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
            // Install per-request progress reporter; tools call
            // crate::progress::report() / is_canceled() on this thread.
            let progress_event_tx = event_tx.clone();
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
            let dispatch_started_at = Instant::now();
            let result = dispatch(&method_name, params.as_ref(), &repo_root, &db_path);
            crate::progress::uninstall();
            let execution_ms = dispatch_started_at.elapsed().as_millis();
            let success = result.is_ok();
            let response = match result {
                Ok(result) => jsonrpc_ok(request_id, result),
                Err(error) => jsonrpc_dispatch_error(request_id, &error),
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

    let dispatch_started_at = Instant::now();
    let response = match dispatch(&method, params, ctx.repo_root, ctx.db_path) {
        Ok(result) => {
            log_request_finished(
                &request_log,
                true,
                0,
                dispatch_started_at.elapsed().as_millis(),
                dispatch_started_at.elapsed().as_millis(),
            );
            jsonrpc_ok(id, result)
        }
        Err(error) => {
            log_request_finished(
                &request_log,
                false,
                0,
                dispatch_started_at.elapsed().as_millis(),
                dispatch_started_at.elapsed().as_millis(),
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
    };
    write_response(writer, &response)
}

fn write_response<W: Write>(writer: &mut W, response: &str) -> Result<()> {
    writeln!(writer, "{response}")?;
    writer.flush()?;
    Ok(())
}

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
            stats.timed_out += 1;
            tracing::warn!(
                request_id = %request.request.request_id,
                method = %request.request.method,
                tool = request.request.tool_name.as_deref().unwrap_or("-"),
                total_ms = request.queued_at.elapsed().as_millis() as u64,
                timeout_ms = request.timeout_ms as u64,
                "MCP request timed out"
            );
            let response = jsonrpc_error(
                request.id,
                JsonRpcErrorKind::RequestTimedOut,
                format!(
                    "worker pool timed out after {} ms while handling request",
                    request.timeout_ms
                ),
            );
            emit_progress_notification(
                writer,
                request.progress_token.as_ref(),
                ProgressEventKind::End,
                "timed out",
            )?;
            emit_trace_log(
                writer,
                connection_state.trace,
                TraceThreshold::Messages,
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
            TraceThreshold::Messages,
            if completion.success { "info" } else { "error" },
            format!(
                "completed request method={} tool={} success={} total_ms={}",
                completion.request.method,
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

struct PendingRequest {
    id: serde_json::Value,
    request: RequestLogContext,
    queued_at: Instant,
    deadline: Instant,
    timeout_ms: u128,
    progress_token: Option<serde_json::Value>,
    /// Shared flag set to `true` when the client cancels this request after it
    /// has already started executing.  Long-running tools poll
    /// [`crate::progress::is_canceled`] which reads this flag.
    cancel_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

struct RequestDispatchContext<'a> {
    repo_root: &'a str,
    db_path: &'a str,
    worker_pool: &'a WorkerPool,
    server_options: &'a ServerOptions,
    canceled_tokens: Arc<Mutex<HashSet<u64>>>,
    event_tx: &'a mpsc::Sender<TransportEvent>,
}

#[derive(Default)]
struct TransportStats {
    received: u64,
    notifications: u64,
    parse_errors: u64,
    async_dispatched: u64,
    completed: u64,
    completed_ok: u64,
    completed_err: u64,
    timed_out: u64,
    canceled: u64,
    dropped_late: u64,
}

enum TransportEvent {
    InputLine(String),
    InputClosed,
    InputError(String),
    WorkerStarted {
        token: u64,
        queue_wait_ms: u128,
    },
    Response {
        token: u64,
        response: String,
        completion: RequestCompletion,
    },
    /// Mid-execution progress emitted by a running tool via [`crate::progress`].
    ProgressReport {
        token: u64,
        message: String,
        percentage: Option<u32>,
    },
}

#[derive(Clone)]
struct RequestLogContext {
    request_id: String,
    method: String,
    tool_name: Option<String>,
}

struct RequestCompletion {
    request: RequestLogContext,
    queue_wait_ms: u128,
    execution_ms: u128,
    success: bool,
}

struct ConnectionState {
    trace: TraceLevel,
    canceled_tokens: Arc<Mutex<HashSet<u64>>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TraceLevel {
    Off,
    Messages,
    Verbose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TraceThreshold {
    Messages,
    Verbose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ProgressEventKind {
    Begin,
    Report,
    End,
}

fn request_id_string(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::String(value) => value.clone(),
        _ => id.to_string(),
    }
}

fn tool_name_from_request(method: &str, params: Option<&serde_json::Value>) -> Option<String> {
    if method != "tools/call" {
        return None;
    }

    params
        .and_then(|p| p.get("name"))
        .and_then(|n| n.as_str())
        .map(str::to_owned)
}

fn log_request_finished(
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

fn resolve_request_timeout(options: &ServerOptions, request: &RequestLogContext) -> Duration {
    let timeout_ms = request
        .tool_name
        .as_ref()
        .and_then(|tool_name| options.tool_timeout_ms_by_tool.get(tool_name))
        .copied()
        .unwrap_or(options.tool_timeout_ms)
        .clamp(1_000, 3_600_000);
    Duration::from_millis(timeout_ms)
}

fn progress_token_from_params(params: Option<&serde_json::Value>) -> Option<serde_json::Value> {
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

fn parse_trace_level(params: Option<&serde_json::Value>) -> Result<TraceLevel> {
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

fn emit_progress_notification<W: Write>(
    writer: &mut W,
    token: Option<&serde_json::Value>,
    kind: ProgressEventKind,
    message: &str,
) -> Result<()> {
    let Some(token) = token else {
        return Ok(());
    };
    let value = match kind {
        ProgressEventKind::Begin => serde_json::json!({
            "kind": "begin",
            "title": "Atlas MCP tool call",
            "message": message,
        }),
        ProgressEventKind::Report => serde_json::json!({
            "kind": "report",
            "message": message,
        }),
        ProgressEventKind::End => serde_json::json!({
            "kind": "end",
            "message": message,
        }),
    };
    write_notification(
        writer,
        &jsonrpc_notification(
            "$/progress",
            serde_json::json!({
                "token": token,
                "value": value,
            }),
        ),
    )
}

fn emit_trace_log<W: Write>(
    writer: &mut W,
    trace: TraceLevel,
    threshold: TraceThreshold,
    level: &str,
    message: String,
) -> Result<()> {
    let enabled = match (trace, threshold) {
        (TraceLevel::Off, _) => false,
        (TraceLevel::Messages, TraceThreshold::Messages) => true,
        (TraceLevel::Messages, TraceThreshold::Verbose) => false,
        (TraceLevel::Verbose, _) => true,
    };
    if !enabled {
        return Ok(());
    }

    write_notification(
        writer,
        &jsonrpc_notification(
            "$/logMessage",
            serde_json::json!({
                "level": level,
                "message": message,
            }),
        ),
    )
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
    // Signal any already-running tool to stop at its next cancellation
    // checkpoint (tools poll crate::progress::is_canceled()).
    request
        .cancel_flag
        .store(true, std::sync::atomic::Ordering::Relaxed);
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
        TraceThreshold::Verbose,
        "debug",
        format!(
            "started request method={} tool={} queue_wait_ms={queue_wait_ms}",
            request.request.method,
            request.request.tool_name.as_deref().unwrap_or("-")
        ),
    )?;
    Ok(())
}

/// Forward a mid-execution progress update from a running tool to the client.
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

// ---------------------------------------------------------------------------
// Method dispatch
// ---------------------------------------------------------------------------

fn dispatch(
    method: &str,
    params: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> std::result::Result<serde_json::Value, DispatchError> {
    match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": {},
                "prompts": { "listChanged": false },
                "logging": {},
                "experimental": {
                    "cancelRequest": true,
                    "progressNotifications": true,
                    "setTrace": true
                }
            },
            "serverInfo": {
                "name": "atlas",
                "version": env!("CARGO_PKG_VERSION")
            }
        })),

        "tools/list" => Ok(tools::tool_list()),

        "prompts/list" => Ok(prompts::prompt_list()),

        "prompts/get" => {
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .ok_or_else(|| {
                    DispatchError::new(
                        JsonRpcErrorKind::InvalidParams,
                        anyhow::anyhow!("missing prompt name"),
                    )
                })?;
            let args = params.and_then(|p| p.get("arguments"));
            prompts::prompt_get(name, args).map_err(classify_prompt_error)
        }

        "tools/call" => {
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .ok_or_else(|| {
                    DispatchError::new(
                        JsonRpcErrorKind::InvalidParams,
                        anyhow::anyhow!("missing tool name"),
                    )
                })?;
            let args = params.and_then(|p| p.get("arguments"));
            tools::call(name, args, repo_root, db_path).map_err(classify_tool_error)
        }

        other => Err(DispatchError::new(
            JsonRpcErrorKind::MethodNotFound,
            anyhow::anyhow!("method not found: {other}"),
        )),
    }
}

type WorkerTask = Box<dyn RunWorkerTask>;

trait RunWorkerTask: Send {
    fn run(self: Box<Self>);
}

impl<F> RunWorkerTask for F
where
    F: FnOnce() + Send + 'static,
{
    fn run(self: Box<Self>) {
        (*self)()
    }
}

enum WorkerMessage {
    Run(WorkerTask),
    Shutdown,
}

#[derive(Debug)]
struct WorkerSubmitError {
    kind: JsonRpcErrorKind,
    message: String,
}

#[cfg(unix)]
struct ShutdownHandlerGuard(signal_hook::iterator::Handle);

#[cfg(unix)]
impl Drop for ShutdownHandlerGuard {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[cfg(unix)]
fn install_stdio_shutdown_handler() -> Result<ShutdownHandlerGuard> {
    spawn_signal_handler(move || unsafe {
        let _ = libc::close(libc::STDIN_FILENO);
    })
}

#[cfg(unix)]
fn install_socket_shutdown_handler(
    shutdown: Arc<AtomicBool>,
    active_streams: Arc<Mutex<Vec<UnixStream>>>,
) -> Result<ShutdownHandlerGuard> {
    spawn_signal_handler(move || {
        shutdown.store(true, Ordering::Relaxed);
        let streams = active_streams
            .lock()
            .expect("active daemon streams lock poisoned");
        for stream in streams.iter() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    })
}

#[cfg(unix)]
fn spawn_signal_handler(action: impl Fn() + Send + 'static) -> Result<ShutdownHandlerGuard> {
    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
    ])
    .context("cannot register shutdown signals")?;
    let handle = signals.handle();
    thread::Builder::new()
        .name("atlas-mcp:signal-handler".to_owned())
        .spawn(move || {
            if signals.forever().next().is_some() {
                action();
            }
        })
        .context("cannot spawn shutdown signal handler")?;
    Ok(ShutdownHandlerGuard(handle))
}

#[cfg(unix)]
struct ActiveStreamGuard {
    raw_fd: i32,
    active_streams: Arc<Mutex<Vec<UnixStream>>>,
}

#[cfg(unix)]
impl ActiveStreamGuard {
    fn new(raw_fd: i32, active_streams: Arc<Mutex<Vec<UnixStream>>>) -> Self {
        Self {
            raw_fd,
            active_streams,
        }
    }
}

#[cfg(unix)]
impl Drop for ActiveStreamGuard {
    fn drop(&mut self) {
        let mut streams = self
            .active_streams
            .lock()
            .expect("active daemon streams lock poisoned");
        if let Some(index) = streams
            .iter()
            .position(|stream| stream.as_raw_fd() == self.raw_fd)
        {
            streams.swap_remove(index);
        }
    }
}

struct WorkerPool {
    thread_name_prefix: String,
    #[cfg(test)]
    timeout: Duration,
    sender: mpsc::SyncSender<WorkerMessage>,
    handles: Vec<thread::JoinHandle<()>>,
}

impl WorkerPool {
    fn from_env(thread_name_prefix: &str, options: ServerOptions) -> Result<Self> {
        let worker_count = parse_env_usize(MCP_WORKER_THREADS_ENV, options.worker_threads)?;
        let timeout_ms = parse_env_u64(MCP_TOOL_TIMEOUT_MS_ENV, options.tool_timeout_ms)?;
        Self::new(
            thread_name_prefix,
            worker_count,
            Duration::from_millis(timeout_ms),
        )
    }

    #[cfg_attr(not(test), allow(unused_variables))]
    fn new(thread_name_prefix: &str, worker_count: usize, timeout: Duration) -> Result<Self> {
        let worker_count = worker_count.max(1);
        let queue_bound = worker_count.saturating_mul(4).max(1);
        let (sender, receiver) = mpsc::sync_channel::<WorkerMessage>(queue_bound);
        let receiver = std::sync::Arc::new(std::sync::Mutex::new(receiver));
        let mut handles = Vec::with_capacity(worker_count);
        let thread_name_prefix = thread_name_prefix.to_owned();

        for index in 0..worker_count {
            let receiver = std::sync::Arc::clone(&receiver);
            let thread_name = format!("{}-{}", thread_name_prefix, index + 1);
            let handle = thread::Builder::new()
                .name(thread_name.clone())
                .spawn(move || {
                    loop {
                        let message = {
                            let guard = receiver.lock().expect("worker receiver lock poisoned");
                            guard.recv()
                        };
                        match message {
                            Ok(WorkerMessage::Run(task)) => task.run(),
                            Ok(WorkerMessage::Shutdown) | Err(_) => break,
                        }
                    }
                })
                .with_context(|| format!("cannot spawn worker thread '{thread_name}'"))?;
            handles.push(handle);
        }

        Ok(Self {
            thread_name_prefix,
            #[cfg(test)]
            timeout,
            sender,
            handles,
        })
    }

    #[cfg(test)]
    fn run<T, F>(&self, task: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        let (reply_tx, reply_rx) = mpsc::channel::<Result<T>>();
        let worker_name = self.thread_name_prefix.clone();
        self.sender
            .send(WorkerMessage::Run(Box::new(move || {
                let result = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(task)) {
                    Ok(result) => result,
                    Err(payload) => Err(anyhow::anyhow!(
                        "worker thread '{worker_name}' panicked: {}",
                        panic_payload_message(&payload)
                    )),
                };
                let _ = reply_tx.send(result);
            })))
            .map_err(|_| {
                anyhow::anyhow!("worker pool '{}' is unavailable", self.thread_name_prefix)
            })?;

        reply_rx
            .recv_timeout(self.timeout)
            .map_err(|err| match err {
                mpsc::RecvTimeoutError::Timeout => anyhow::anyhow!(
                    "worker pool '{}' timed out after {} ms while handling task",
                    self.thread_name_prefix,
                    self.timeout.as_millis()
                ),
                mpsc::RecvTimeoutError::Disconnected => {
                    anyhow::anyhow!("worker pool '{}' dropped response", self.thread_name_prefix)
                }
            })?
    }

    fn submit<F>(&self, task: F) -> std::result::Result<(), WorkerSubmitError>
    where
        F: FnOnce() + Send + 'static,
    {
        match self.sender.try_send(WorkerMessage::Run(Box::new(task))) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(_)) => Err(WorkerSubmitError {
                kind: JsonRpcErrorKind::RateLimited,
                message: format!(
                    "worker pool '{}' is saturated; retry later",
                    self.thread_name_prefix
                ),
            }),
            Err(mpsc::TrySendError::Disconnected(_)) => Err(WorkerSubmitError {
                kind: JsonRpcErrorKind::WorkerUnavailable,
                message: format!("worker pool '{}' is unavailable", self.thread_name_prefix),
            }),
        }
    }

    #[cfg(test)]
    fn timeout(&self) -> Duration {
        self.timeout
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        for _ in 0..self.handles.len() {
            let _ = self.sender.try_send(WorkerMessage::Shutdown);
        }
    }
}

fn parse_env_usize(var: &str, default: usize) -> Result<usize> {
    match std::env::var(var) {
        Ok(raw) if !raw.trim().is_empty() => raw
            .trim()
            .parse::<usize>()
            .with_context(|| format!("{var} must be positive integer")),
        _ => Ok(default),
    }
}

fn parse_env_u64(var: &str, default: u64) -> Result<u64> {
    match std::env::var(var) {
        Ok(raw) if !raw.trim().is_empty() => raw
            .trim()
            .parse::<u64>()
            .with_context(|| format!("{var} must be non-negative integer")),
        _ => Ok(default),
    }
}

#[cfg(test)]
fn panic_payload_message(payload: &Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_owned()
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC helpers
// ---------------------------------------------------------------------------

fn jsonrpc_ok(id: serde_json::Value, result: serde_json::Value) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn jsonrpc_notification(method: &str, params: serde_json::Value) -> String {
    serde_json::json!({ "jsonrpc": "2.0", "method": method, "params": params }).to_string()
}

fn user_visible_error_message(error: &anyhow::Error) -> String {
    user_facing_error_message(&error.to_string(), &format!("{error:#}"))
}

fn jsonrpc_error(id: serde_json::Value, kind: JsonRpcErrorKind, message: String) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": {
            "code": kind.code(),
            "message": message,
            "data": { "atlas_error_code": kind.atlas_error_code() }
        }
    })
    .to_string()
}

fn jsonrpc_dispatch_error(id: serde_json::Value, error: &DispatchError) -> String {
    jsonrpc_error(id, error.kind, error.message())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonRpcErrorKind {
    ParseError,
    InvalidRequest,
    MethodNotFound,
    InvalidParams,
    InternalError,
    ToolExecutionFailed,
    WorkerUnavailable,
    RequestTimedOut,
    RateLimited,
}

impl JsonRpcErrorKind {
    fn code(self) -> i32 {
        match self {
            Self::ParseError => JSONRPC_PARSE_ERROR,
            Self::InvalidRequest => JSONRPC_INVALID_REQUEST,
            Self::MethodNotFound => JSONRPC_METHOD_NOT_FOUND,
            Self::InvalidParams => JSONRPC_INVALID_PARAMS,
            Self::InternalError => JSONRPC_INTERNAL_ERROR,
            Self::ToolExecutionFailed => JSONRPC_TOOL_EXECUTION_FAILED,
            Self::WorkerUnavailable => JSONRPC_WORKER_UNAVAILABLE,
            Self::RequestTimedOut => JSONRPC_REQUEST_TIMEOUT,
            Self::RateLimited => JSONRPC_RATE_LIMITED,
        }
    }

    fn atlas_error_code(self) -> &'static str {
        match self {
            Self::ParseError => "parse_error",
            Self::InvalidRequest => "invalid_request",
            Self::MethodNotFound => "method_not_found",
            Self::InvalidParams => "invalid_params",
            Self::InternalError => "internal_error",
            Self::ToolExecutionFailed => "tool_execution_failed",
            Self::WorkerUnavailable => "worker_unavailable",
            Self::RequestTimedOut => "request_timed_out",
            Self::RateLimited => "rate_limited",
        }
    }
}

#[derive(Debug)]
struct DispatchError {
    kind: JsonRpcErrorKind,
    source: anyhow::Error,
}

impl DispatchError {
    fn new(kind: JsonRpcErrorKind, source: anyhow::Error) -> Self {
        Self { kind, source }
    }

    fn message(&self) -> String {
        match self.kind {
            JsonRpcErrorKind::ToolExecutionFailed | JsonRpcErrorKind::InternalError => {
                user_visible_error_message(&self.source)
            }
            _ => self.source.to_string(),
        }
    }
}

fn classify_prompt_error(error: anyhow::Error) -> DispatchError {
    let detail = error.to_string();
    let kind = if detail.starts_with("unknown prompt:") || is_invalid_params_message(&detail) {
        JsonRpcErrorKind::InvalidParams
    } else {
        JsonRpcErrorKind::InternalError
    };
    DispatchError::new(kind, error)
}

fn classify_tool_error(error: anyhow::Error) -> DispatchError {
    let detail = error.to_string();
    let kind = if detail.starts_with("unknown tool:") || is_invalid_params_message(&detail) {
        JsonRpcErrorKind::InvalidParams
    } else {
        JsonRpcErrorKind::ToolExecutionFailed
    };
    DispatchError::new(kind, error)
}

fn is_invalid_params_message(detail: &str) -> bool {
    detail.starts_with("missing ")
        || detail.starts_with("argument '")
        || detail.contains("missing required argument:")
        || detail.contains("requires non-empty")
        || detail.contains("must be ")
        || detail.contains("invalid regex pattern")
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DaemonHandshakeRequest {
    protocol_version: String,
    repo_root: String,
    db_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct DaemonHandshakeResponse {
    ok: bool,
    protocol_version: String,
    repo_root: String,
    db_path: String,
    error: Option<String>,
}

impl DaemonHandshakeResponse {
    fn ok(protocol_version: &str, repo_root: &str, db_path: &str) -> Self {
        Self {
            ok: true,
            protocol_version: protocol_version.to_owned(),
            repo_root: repo_root.to_owned(),
            db_path: db_path.to_owned(),
            error: None,
        }
    }

    fn err(protocol_version: &str, repo_root: &str, db_path: &str, error: String) -> Self {
        Self {
            ok: false,
            protocol_version: protocol_version.to_owned(),
            repo_root: repo_root.to_owned(),
            db_path: db_path.to_owned(),
            error: Some(error),
        }
    }
}

fn write_notification<W: Write>(writer: &mut W, notification: &str) -> Result<()> {
    writeln!(writer, "{notification}")?;
    writer.flush()?;
    Ok(())
}

#[cfg(unix)]
fn secure_socket_path(socket_path: &Path) -> Result<()> {
    std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o600))
        .with_context(|| format!("cannot secure {}", socket_path.display()))
}

#[cfg(unix)]
fn ensure_peer_uid_allowed(stream: &UnixStream) -> Result<()> {
    let peer_uid = peer_uid(stream)?;
    let process_uid = unsafe { libc::geteuid() };
    if peer_uid == process_uid || peer_uid == 0 {
        return Ok(());
    }

    Err(anyhow::anyhow!(
        "rejecting daemon client from uid {peer_uid}; expected uid {process_uid}"
    ))
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_uid(stream: &UnixStream) -> Result<libc::uid_t> {
    let fd = stream.as_raw_fd();
    let mut creds = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let result = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut creds as *mut libc::ucred as *mut libc::c_void,
            &mut len,
        )
    };
    if result == 0 {
        Ok(creds.uid)
    } else {
        Err(std::io::Error::last_os_error()).context("cannot read peer credentials")
    }
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd"
))]
fn peer_uid(stream: &UnixStream) -> Result<libc::uid_t> {
    let fd = stream.as_raw_fd();
    let mut uid = 0;
    let mut gid = 0;
    let result = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
    if result == 0 {
        Ok(uid)
    } else {
        Err(std::io::Error::last_os_error()).context("cannot read peer credentials")
    }
}

// Fail-closed fallback for exotic Unix platforms not covered by the cfg blocks above.
// Rejecting all connections is safer than allowing unknown users to attach.
#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "android",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "openbsd",
        target_os = "netbsd",
    ))
))]
fn peer_uid(_stream: &UnixStream) -> Result<libc::uid_t> {
    Err(anyhow::anyhow!(
        "peer credential check not supported on this Unix platform; daemon rejected"
    ))
}

// ──────────────────────────────────────────────────────────────────────────────
// Windows named-pipe helpers
// ──────────────────────────────────────────────────────────────────────────────

/// Create a new named-pipe server instance.  The default DACL (inherited from
/// the process token) already restricts access to the creating user, so we do
/// not need an explicit security descriptor here.  Peer-credential verification
/// happens after `ConnectNamedPipe` via `win_ensure_pipe_client_is_same_user`.
#[cfg(windows)]
fn win_create_named_pipe_server(pipe_name: &[u16]) -> Result<HANDLE> {
    let handle = unsafe {
        CreateNamedPipeW(
            pipe_name.as_ptr(),
            PIPE_ACCESS_DUPLEX,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            PIPE_UNLIMITED_INSTANCES,
            65536,
            65536,
            0,
            std::ptr::null(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error()).context("cannot create named pipe server");
    }
    Ok(handle)
}

/// Duplicate a Windows HANDLE so that reader and writer threads each own an
/// independent handle to the same pipe endpoint.
#[cfg(windows)]
fn win_dup_handle(handle: HANDLE) -> Result<HANDLE> {
    let process = unsafe { GetCurrentProcess() };
    let mut new_handle = INVALID_HANDLE_VALUE;
    let ok = unsafe {
        DuplicateHandle(
            process,
            handle,
            process,
            &mut new_handle,
            0,
            FALSE,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error()).context("cannot duplicate pipe handle");
    }
    Ok(new_handle)
}

/// RAII guard that closes a Windows HANDLE on drop.
#[cfg(windows)]
struct WinHandleGuard(HANDLE);

#[cfg(windows)]
impl Drop for WinHandleGuard {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Retrieve the binary SID of the owner of the given process.
#[cfg(windows)]
fn win_process_owner_sid(pid: u32) -> Result<Vec<u8>> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, FALSE, pid) };
    if process == 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("cannot open process {pid}"));
    }
    let _process_guard = WinHandleGuard(process);

    let mut token: HANDLE = 0;
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) } == 0 {
        return Err(std::io::Error::last_os_error()).context("cannot open process token");
    }
    let _token_guard = WinHandleGuard(token);

    // First call: determine the required buffer size.
    let mut len: u32 = 0;
    unsafe { GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len) };
    if len == 0 {
        return Err(std::io::Error::last_os_error())
            .context("GetTokenInformation size query failed");
    }

    let mut buf = vec![0u8; len as usize];
    if unsafe { GetTokenInformation(token, TokenUser, buf.as_mut_ptr() as *mut _, len, &mut len) }
        == 0
    {
        return Err(std::io::Error::last_os_error()).context("cannot get token user SID");
    }

    // TOKEN_USER.User.Sid is a pointer into `buf`; copy the SID bytes out before
    // `buf` is dropped.
    let token_user = unsafe { &*(buf.as_ptr() as *const TOKEN_USER) };
    let sid_ptr = token_user.User.Sid;
    let sid_len = unsafe { GetLengthSid(sid_ptr) } as usize;
    let mut sid_bytes = vec![0u8; sid_len];
    if unsafe { CopySid(sid_len as u32, sid_bytes.as_mut_ptr() as *mut _, sid_ptr) } == 0 {
        return Err(std::io::Error::last_os_error()).context("cannot copy SID");
    }
    Ok(sid_bytes)
}

/// Reject daemon connections from a different Windows user account.
///
/// Uses `GetNamedPipeClientProcessId` to identify the connecting process, then
/// compares its owner SID with the server process owner SID.
#[cfg(windows)]
fn win_ensure_pipe_client_is_same_user(pipe_handle: HANDLE) -> Result<()> {
    let mut client_pid: u32 = 0;
    if unsafe { GetNamedPipeClientProcessId(pipe_handle, &mut client_pid) } == 0 {
        return Err(std::io::Error::last_os_error())
            .context("cannot get named pipe client process id");
    }
    let server_pid = unsafe { GetCurrentProcessId() };

    let client_sid = win_process_owner_sid(client_pid)?;
    let server_sid = win_process_owner_sid(server_pid)?;

    if unsafe { EqualSid(client_sid.as_ptr() as *mut _, server_sid.as_ptr() as *mut _) } == 0 {
        return Err(anyhow::anyhow!(
            "rejecting daemon pipe client from a different user account (client pid {client_pid})"
        ));
    }
    Ok(())
}

/// Serve one named-pipe client connection.
#[cfg(windows)]
fn win_serve_pipe_connection(
    pipe_handle: HANDLE,
    repo_root: &str,
    db_path: &str,
    worker_pool: Arc<WorkerPool>,
    server_options: ServerOptions,
) -> Result<()> {
    win_ensure_pipe_client_is_same_user(pipe_handle)?;

    // Duplicate the handle so the reader and writer each own an independent
    // HANDLE; dropping one does not close the other.
    let reader_handle = win_dup_handle(pipe_handle)?;
    let reader_file = unsafe { std::fs::File::from_raw_handle(reader_handle as _) };
    let writer_file = unsafe { std::fs::File::from_raw_handle(pipe_handle as _) };

    let mut reader = BufReader::new(reader_file);
    let mut writer = std::io::BufWriter::new(writer_file);

    perform_socket_handshake(&mut reader, &mut writer, repo_root, db_path)?;
    serve_connection(
        reader,
        &mut writer,
        repo_root,
        db_path,
        worker_pool,
        server_options,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use atlas_core::EdgeKind;
    use atlas_core::kinds::NodeKind;
    use atlas_core::model::{Edge, Node, NodeId};
    use atlas_store_sqlite::Store;
    use rusqlite::Connection;
    use std::io::{Cursor, Read, Write};
    #[cfg(unix)]
    use std::net::Shutdown;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tempfile::TempDir;

    #[cfg(unix)]
    use std::os::unix::net::UnixStream;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct TransportFixture {
        _dir: TempDir,
        db_path: String,
    }

    fn make_node(kind: NodeKind, name: &str, qualified_name: &str, file_path: &str) -> Node {
        Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_owned(),
            qualified_name: qualified_name.to_owned(),
            file_path: file_path.to_owned(),
            line_start: 1,
            line_end: 5,
            language: "rust".to_owned(),
            parent_name: None,
            params: Some("()".to_owned()),
            return_type: None,
            modifiers: Some("pub".to_owned()),
            is_test: kind == NodeKind::Test,
            file_hash: format!("hash:{file_path}"),
            extra_json: serde_json::json!({}),
        }
    }

    fn make_edge(kind: EdgeKind, source_qn: &str, target_qn: &str, file_path: &str) -> Edge {
        Edge {
            id: 0,
            kind,
            source_qn: source_qn.to_owned(),
            target_qn: target_qn.to_owned(),
            file_path: file_path.to_owned(),
            line: Some(1),
            confidence: 1.0,
            confidence_tier: Some("high".to_owned()),
            extra_json: serde_json::json!({}),
        }
    }

    fn setup_fixture() -> TransportFixture {
        let dir = tempfile::tempdir().expect("tempdir");
        let db_path = dir.path().join("atlas.db");
        let db_path = db_path.to_string_lossy().to_string();

        let mut store = Store::open(&db_path).expect("open store");

        let compute = make_node(
            NodeKind::Function,
            "compute",
            "src/service.rs::fn::compute",
            "src/service.rs",
        );
        store
            .replace_file_graph(
                "src/service.rs",
                "hash:src/service.rs",
                Some("rust"),
                Some(5),
                std::slice::from_ref(&compute),
                &[],
            )
            .expect("replace service graph");

        let handle = make_node(
            NodeKind::Function,
            "handle_request",
            "src/api.rs::fn::handle_request",
            "src/api.rs",
        );
        let handle_calls_compute = make_edge(
            EdgeKind::Calls,
            "src/api.rs::fn::handle_request",
            "src/service.rs::fn::compute",
            "src/api.rs",
        );
        store
            .replace_file_graph(
                "src/api.rs",
                "hash:src/api.rs",
                Some("rust"),
                Some(5),
                std::slice::from_ref(&handle),
                &[handle_calls_compute],
            )
            .expect("replace api graph");

        TransportFixture { _dir: dir, db_path }
    }

    fn parse_output_lines(output: Vec<u8>) -> Vec<serde_json::Value> {
        String::from_utf8(output)
            .expect("utf8 output")
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(|line| serde_json::from_str(line).expect("jsonrpc response line"))
            .collect()
    }

    fn test_connection_state() -> ConnectionState {
        ConnectionState {
            trace: TraceLevel::Off,
            canceled_tokens: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    #[test]
    fn stdio_transport_handles_initialize_list_and_tool_calls() {
        let fixture = setup_fixture();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"prompts/list\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"prompts/get\",\"params\":{\"name\":\"inspect_symbol\",\"arguments\":{\"symbol\":\"compute\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"compute\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":6,\"method\":\"tools/call\",\"params\":{\"name\":\"get_context\",\"arguments\":{\"query\":\"compute\"}}}\n"
        );
        let reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut writer = Vec::new();

        run_server_io(
            reader,
            &mut writer,
            "/ignored",
            &fixture.db_path,
            ServerOptions::default(),
        )
        .expect("run server io");

        let responses = parse_output_lines(writer);
        assert_eq!(
            responses.len(),
            6,
            "initialized notification must not emit a response"
        );

        let by_id: std::collections::HashMap<_, _> = responses
            .into_iter()
            .map(|response| (response["id"].clone(), response))
            .collect();

        assert_eq!(
            by_id[&serde_json::json!(1)]["result"]["protocolVersion"],
            MCP_PROTOCOL_VERSION
        );
        assert!(
            by_id[&serde_json::json!(1)]["result"]["capabilities"]["prompts"].is_object(),
            "initialize must advertise prompts capability"
        );

        let tools = by_id[&serde_json::json!(2)]["result"]["tools"]
            .as_array()
            .expect("tools/list result tools array");
        assert!(
            tools.iter().any(|tool| tool["name"] == "get_context"),
            "tools/list must expose get_context"
        );

        let prompts = by_id[&serde_json::json!(3)]["result"]["prompts"]
            .as_array()
            .expect("prompts/list result prompts array");
        assert!(
            prompts
                .iter()
                .any(|prompt| prompt["name"] == "inspect_symbol"),
            "prompts/list must expose inspect_symbol"
        );

        let prompt_text = by_id[&serde_json::json!(4)]["result"]["messages"][0]["content"]["text"]
            .as_str()
            .expect("prompt text");
        assert!(prompt_text.contains("compute"));
        assert!(prompt_text.contains("query_graph"));
        assert!(prompt_text.contains("symbol_neighbors"));

        assert_eq!(
            by_id[&serde_json::json!(5)]["result"]["atlas_output_format"],
            "json",
            "query_graph transport response must preserve JSON default"
        );
        let query_text = by_id[&serde_json::json!(5)]["result"]["content"][0]["text"]
            .as_str()
            .expect("query_graph text content");
        let query_value: serde_json::Value =
            serde_json::from_str(query_text).expect("query_graph payload json");
        assert_eq!(query_value[0]["qn"], "src/service.rs::fn::compute");

        assert_eq!(
            by_id[&serde_json::json!(6)]["result"]["atlas_output_format"],
            "toon",
            "get_context transport response must preserve TOON default"
        );
        let context_text = by_id[&serde_json::json!(6)]["result"]["content"][0]["text"]
            .as_str()
            .expect("get_context text content");
        assert!(context_text.contains("intent: symbol"));
        assert!(context_text.contains("src/service.rs::fn::compute"));
    }

    #[test]
    fn stdio_transport_returns_jsonrpc_errors_for_parse_and_method_failures() {
        let fixture = setup_fixture();
        let input = concat!(
            "not-json\n",
            "{\"id\":6,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"missing/method\",\"params\":{}}\n"
        );
        let reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut writer = Vec::new();

        run_server_io(
            reader,
            &mut writer,
            "/ignored",
            &fixture.db_path,
            ServerOptions::default(),
        )
        .expect("run server io");

        let responses = parse_output_lines(writer);
        assert_eq!(responses.len(), 4);
        let by_id: std::collections::HashMap<_, _> = responses
            .into_iter()
            .map(|response| (response["id"].clone(), response))
            .collect();

        assert_eq!(by_id[&serde_json::Value::Null]["error"]["code"], -32700);
        assert_eq!(
            by_id[&serde_json::Value::Null]["error"]["data"]["atlas_error_code"],
            serde_json::json!("parse_error")
        );
        assert_eq!(by_id[&serde_json::json!(6)]["id"], 6);
        assert_eq!(by_id[&serde_json::json!(6)]["error"]["code"], -32600);
        assert_eq!(
            by_id[&serde_json::json!(6)]["error"]["data"]["atlas_error_code"],
            serde_json::json!("invalid_request")
        );
        assert_eq!(by_id[&serde_json::json!(8)]["id"], 8);
        assert_eq!(by_id[&serde_json::json!(8)]["error"]["code"], -32602);
        assert_eq!(
            by_id[&serde_json::json!(8)]["error"]["data"]["atlas_error_code"],
            serde_json::json!("invalid_params")
        );
        assert_eq!(by_id[&serde_json::json!(7)]["id"], 7);
        assert_eq!(by_id[&serde_json::json!(7)]["error"]["code"], -32601);
        assert_eq!(
            by_id[&serde_json::json!(7)]["error"]["data"]["atlas_error_code"],
            serde_json::json!("method_not_found")
        );
        assert!(
            by_id[&serde_json::json!(7)]["error"]["message"]
                .as_str()
                .expect("error message")
                .contains("method not found")
        );
    }

    #[test]
    fn stdio_transport_redacts_internal_sql_errors_from_tool_failures() {
        let fixture = setup_fixture();
        let conn = Connection::open(&fixture.db_path).expect("open fixture db");
        conn.execute_batch("DROP TABLE nodes;")
            .expect("drop nodes table to force internal db error");
        drop(conn);

        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"compute\"}}}\n"
        );
        let reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut writer = Vec::new();

        run_server_io(
            reader,
            &mut writer,
            "/ignored",
            &fixture.db_path,
            ServerOptions::default(),
        )
        .expect("run server io");

        let responses = parse_output_lines(writer);
        let response = responses
            .into_iter()
            .find(|value| value["id"] == serde_json::json!(2))
            .expect("query_graph response");
        let message = response["error"]["message"]
            .as_str()
            .expect("error message");

        assert_eq!(response["error"]["code"], -32001);
        assert_eq!(
            response["error"]["data"]["atlas_error_code"],
            serde_json::json!("tool_execution_failed")
        );
        assert!(
            message.contains("Graph database schema does not match this Atlas build."),
            "message should be user friendly: {message}"
        );
        assert!(
            !message.to_ascii_lowercase().contains("sqlite"),
            "message must not leak sqlite internals: {message}"
        );
        assert!(
            !message.to_ascii_lowercase().contains("sql"),
            "message must not leak sql internals: {message}"
        );
        assert!(
            !message.contains("no such table"),
            "message must not leak raw schema failure: {message}"
        );
    }

    #[test]
    fn stdio_transport_emits_progress_and_log_notifications() {
        let fixture = setup_fixture();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"$/setTrace\",\"params\":{\"value\":\"messages\"}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"progressToken\":\"tok-1\",\"arguments\":{\"text\":\"compute\"}}}\n"
        );
        let reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut writer = Vec::new();

        run_server_io(
            reader,
            &mut writer,
            "/ignored",
            &fixture.db_path,
            ServerOptions::default(),
        )
        .expect("run server io");

        let responses = parse_output_lines(writer);
        assert!(
            responses
                .iter()
                .any(|value| value["method"] == serde_json::json!("$/logMessage")),
            "setTrace=messages should emit log notifications"
        );
        assert!(
            responses.iter().any(|value| {
                value["method"] == serde_json::json!("$/progress")
                    && value["params"]["token"] == serde_json::json!("tok-1")
                    && value["params"]["value"]["kind"] == serde_json::json!("begin")
            }),
            "tool call should emit progress begin"
        );
        assert!(
            responses.iter().any(|value| {
                value["method"] == serde_json::json!("$/progress")
                    && value["params"]["token"] == serde_json::json!("tok-1")
                    && value["params"]["value"]["kind"] == serde_json::json!("end")
            }),
            "tool call should emit progress end"
        );
        assert!(
            responses
                .iter()
                .any(|value| value["id"] == serde_json::json!(2)),
            "tool response must still be returned"
        );
    }

    #[test]
    fn stdio_transport_cancels_queued_request_without_response() {
        let fixture = setup_fixture();
        let input = concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"__test_sleep\",\"arguments\":{\"sleep_ms\":200}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"__test_sleep\",\"progressToken\":\"cancel-me\",\"arguments\":{\"sleep_ms\":200}}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"$/cancelRequest\",\"params\":{\"id\":2}}\n"
        );
        let reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut writer = Vec::new();

        run_server_io(
            reader,
            &mut writer,
            "/ignored",
            &fixture.db_path,
            ServerOptions {
                worker_threads: 1,
                tool_timeout_ms: 1_000,
                tool_timeout_ms_by_tool: HashMap::from([("__test_sleep".to_owned(), 1_000)]),
            },
        )
        .expect("run server io");

        let responses = parse_output_lines(writer);
        assert!(
            responses
                .iter()
                .any(|value| value["id"] == serde_json::json!(1)),
            "first slow request should complete"
        );
        assert!(
            responses
                .iter()
                .all(|value| value["id"] != serde_json::json!(2)),
            "canceled queued request must not emit a response"
        );
        assert!(
            responses.iter().any(|value| {
                value["method"] == serde_json::json!("$/progress")
                    && value["params"]["token"] == serde_json::json!("cancel-me")
                    && value["params"]["value"]["message"] == serde_json::json!("canceled")
            }),
            "canceled request should close progress stream"
        );
    }

    #[test]
    #[cfg(unix)]
    fn socket_transport_handles_handshake_and_tool_calls() {
        let fixture = setup_fixture();
        let (server_stream, mut client_stream) = UnixStream::pair().expect("unix pair");
        let worker_pool = Arc::new(
            WorkerPool::new("atlas-mcp:socket-test", 2, Duration::from_secs(1))
                .expect("spawn workers"),
        );
        let active_streams = Arc::new(Mutex::new(Vec::new()));
        let db_path = fixture.db_path.clone();
        let expected_db_path = fixture.db_path.clone();
        let server = std::thread::spawn(move || {
            serve_socket_connection(
                server_stream,
                "/repo",
                &db_path,
                worker_pool,
                ServerOptions::default(),
                active_streams,
            )
        });

        writeln!(
            client_stream,
            "{}",
            serde_json::to_string(&DaemonHandshakeRequest {
                protocol_version: MCP_PROTOCOL_VERSION.to_owned(),
                repo_root: "/repo".to_owned(),
                db_path: fixture.db_path.clone(),
            })
            .expect("serialize handshake")
        )
        .expect("write handshake");
        client_stream
            .write_all(
            concat!(
                "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"compute\"}}}\n",
                "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"get_context\",\"arguments\":{\"query\":\"compute\"}}}\n"
            )
            .as_bytes(),
        )
        .expect("write requests");
        client_stream.flush().expect("flush requests");
        client_stream
            .shutdown(Shutdown::Write)
            .expect("shutdown write");

        let mut output = String::new();
        client_stream
            .read_to_string(&mut output)
            .expect("read responses");
        let server_result = server.join().expect("server join");
        assert!(
            server_result.is_ok(),
            "socket serve failed: {server_result:?}"
        );

        let mut lines = output.lines();
        let handshake: DaemonHandshakeResponse =
            serde_json::from_str(lines.next().expect("handshake line"))
                .expect("parse handshake response");
        assert_eq!(
            handshake,
            DaemonHandshakeResponse::ok(MCP_PROTOCOL_VERSION, "/repo", &expected_db_path)
        );

        let responses = parse_output_lines(lines.collect::<Vec<_>>().join("\n").into_bytes());
        assert_eq!(responses.len(), 4);

        let by_id: std::collections::HashMap<_, _> = responses
            .into_iter()
            .map(|response| (response["id"].clone(), response))
            .collect();

        assert_eq!(
            by_id[&serde_json::json!(1)]["result"]["protocolVersion"],
            MCP_PROTOCOL_VERSION
        );
        let tools = by_id[&serde_json::json!(2)]["result"]["tools"]
            .as_array()
            .expect("tools/list result tools array");
        assert!(tools.iter().any(|tool| tool["name"] == "get_context"));

        let query_text = by_id[&serde_json::json!(3)]["result"]["content"][0]["text"]
            .as_str()
            .expect("query_graph text content");
        let query_value: serde_json::Value =
            serde_json::from_str(query_text).expect("query_graph payload json");
        assert_eq!(query_value[0]["qn"], "src/service.rs::fn::compute");

        let context_text = by_id[&serde_json::json!(4)]["result"]["content"][0]["text"]
            .as_str()
            .expect("get_context text content");
        assert!(context_text.contains("src/service.rs::fn::compute"));
    }

    #[test]
    #[cfg(unix)]
    fn socket_transport_rejects_handshake_mismatch() {
        let fixture = setup_fixture();
        let (server_stream, mut client_stream) = UnixStream::pair().expect("unix pair");
        let worker_pool = Arc::new(
            WorkerPool::new("atlas-mcp:socket-test", 1, Duration::from_secs(1))
                .expect("spawn workers"),
        );
        let active_streams = Arc::new(Mutex::new(Vec::new()));
        let db_path = fixture.db_path.clone();
        let server = std::thread::spawn(move || {
            serve_socket_connection(
                server_stream,
                "/repo",
                &db_path,
                worker_pool,
                ServerOptions::default(),
                active_streams,
            )
        });

        writeln!(
            client_stream,
            "{}",
            serde_json::to_string(&DaemonHandshakeRequest {
                protocol_version: "bad-version".to_owned(),
                repo_root: "/repo".to_owned(),
                db_path: fixture.db_path.clone(),
            })
            .expect("serialize handshake")
        )
        .expect("write handshake");
        client_stream.flush().expect("flush handshake");
        client_stream
            .shutdown(Shutdown::Write)
            .expect("shutdown write");

        let mut output = String::new();
        client_stream
            .read_to_string(&mut output)
            .expect("read responses");
        let server_result = server.join().expect("server join");
        assert!(server_result.is_err(), "bad handshake must fail");

        let response: DaemonHandshakeResponse =
            serde_json::from_str(output.trim()).expect("parse handshake response");
        assert!(!response.ok);
        assert_eq!(response.protocol_version, MCP_PROTOCOL_VERSION);
        assert!(
            response
                .error
                .unwrap_or_default()
                .contains("protocol mismatch")
        );
    }

    #[test]
    #[cfg(unix)]
    fn secure_socket_path_sets_owner_only_permissions() {
        let temp = tempfile::tempdir().expect("tempdir");
        let path = temp.path().join("atlas.sock");
        std::fs::write(&path, b"socket-placeholder").expect("write placeholder");

        secure_socket_path(&path).expect("secure socket path");

        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn worker_pool_runs_tasks_on_named_threads() {
        let worker_pool = WorkerPool::new("atlas-mcp:test-worker", 2, Duration::from_secs(1))
            .expect("spawn workers");
        let first = worker_pool
            .run(|| {
                Ok((
                    std::thread::current().id(),
                    std::thread::current()
                        .name()
                        .expect("worker thread name")
                        .to_owned(),
                ))
            })
            .expect("first run");
        let second = worker_pool
            .run(|| Ok(std::thread::current().id()))
            .expect("second run");

        assert!(first.1.starts_with("atlas-mcp:test-worker-"));
        assert!(first.0 == second || first.1.starts_with("atlas-mcp:test-worker-"));
    }

    #[test]
    fn worker_pool_surfaces_panic_message() {
        let worker_pool = WorkerPool::new("atlas-mcp:panic-worker", 1, Duration::from_secs(1))
            .expect("spawn workers");
        let error = worker_pool
            .run::<(), _>(|| panic!("boom"))
            .expect_err("panic must surface as error");

        assert!(error.to_string().contains("panic-worker"));
        assert!(error.to_string().contains("boom"));
    }

    #[test]
    fn worker_pool_enforces_timeout() {
        let worker_pool = WorkerPool::new("atlas-mcp:timeout-worker", 1, Duration::from_millis(10))
            .expect("spawn workers");
        let error = worker_pool
            .run(|| {
                std::thread::sleep(Duration::from_millis(50));
                Ok(())
            })
            .expect_err("slow task must time out");

        assert!(error.to_string().contains("timed out"));
        assert!(error.to_string().contains("10 ms"));
    }

    #[test]
    fn worker_pool_bounds_thread_count_under_concurrency() {
        use std::collections::HashSet;
        use std::sync::{Arc, Barrier, Mutex};

        let worker_pool = Arc::new(
            WorkerPool::new("atlas-mcp:bounded-worker", 2, Duration::from_secs(1))
                .expect("spawn workers"),
        );
        let barrier = Arc::new(Barrier::new(5));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let mut callers = Vec::new();

        for _ in 0..4 {
            let worker_pool = Arc::clone(&worker_pool);
            let barrier = Arc::clone(&barrier);
            let seen = Arc::clone(&seen);
            callers.push(std::thread::spawn(move || {
                barrier.wait();
                let thread_id = worker_pool
                    .run(|| {
                        std::thread::sleep(Duration::from_millis(20));
                        Ok(std::thread::current().id())
                    })
                    .expect("worker run");
                seen.lock().expect("seen lock").push(thread_id);
            }));
        }

        barrier.wait();
        for caller in callers {
            caller.join().expect("caller thread join");
        }

        let seen = seen.lock().expect("seen lock");
        let unique: HashSet<_> = seen.iter().copied().collect();
        assert_eq!(seen.len(), 4);
        assert!(
            unique.len() <= 2,
            "pool must not exceed configured worker count"
        );
    }

    #[test]
    fn worker_pool_survives_burst_load_without_dropping_tasks() {
        use std::sync::Arc;

        let worker_pool = Arc::new(
            WorkerPool::new("atlas-mcp:burst-worker", 2, Duration::from_secs(1))
                .expect("spawn workers"),
        );
        let completed = Arc::new(AtomicUsize::new(0));
        let mut callers = Vec::new();

        for _ in 0..24 {
            let worker_pool = Arc::clone(&worker_pool);
            let completed = Arc::clone(&completed);
            callers.push(std::thread::spawn(move || {
                worker_pool
                    .run(move || {
                        std::thread::sleep(Duration::from_millis(5));
                        completed.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    })
                    .expect("worker run under burst load");
            }));
        }

        for caller in callers {
            caller.join().expect("caller thread join");
        }

        assert_eq!(completed.load(Ordering::SeqCst), 24);
    }

    #[test]
    fn worker_pool_from_env_prefers_env_over_server_options() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        // SAFETY: test-scoped env mutation.
        unsafe {
            std::env::set_var(MCP_WORKER_THREADS_ENV, "3");
            std::env::set_var(MCP_TOOL_TIMEOUT_MS_ENV, "2000");
        }

        let worker_pool = WorkerPool::from_env(
            "atlas-mcp:env-worker",
            ServerOptions {
                worker_threads: 1,
                tool_timeout_ms: 1000,
                tool_timeout_ms_by_tool: HashMap::new(),
            },
        )
        .expect("pool from env");

        assert_eq!(worker_pool.handles.len(), 3);
        assert_eq!(worker_pool.timeout(), Duration::from_millis(2000));

        // SAFETY: test-scoped env cleanup.
        unsafe {
            std::env::remove_var(MCP_WORKER_THREADS_ENV);
            std::env::remove_var(MCP_TOOL_TIMEOUT_MS_ENV);
        }
    }

    #[test]
    fn worker_pool_from_env_rejects_invalid_values() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        // SAFETY: test-scoped env mutation.
        unsafe {
            std::env::set_var(MCP_WORKER_THREADS_ENV, "abc");
            std::env::remove_var(MCP_TOOL_TIMEOUT_MS_ENV);
        }

        let error = WorkerPool::from_env("atlas-mcp:bad-env", ServerOptions::default())
            .err()
            .expect("invalid worker thread env must fail");
        assert!(error.to_string().contains(MCP_WORKER_THREADS_ENV));

        // SAFETY: test-scoped env mutation.
        unsafe {
            std::env::remove_var(MCP_WORKER_THREADS_ENV);
            std::env::set_var(MCP_TOOL_TIMEOUT_MS_ENV, "bad-timeout");
        }

        let error = WorkerPool::from_env("atlas-mcp:bad-env", ServerOptions::default())
            .err()
            .expect("invalid timeout env must fail");
        assert!(error.to_string().contains(MCP_TOOL_TIMEOUT_MS_ENV));

        // SAFETY: test-scoped env cleanup.
        unsafe {
            std::env::remove_var(MCP_TOOL_TIMEOUT_MS_ENV);
        }
    }

    #[test]
    fn worker_pool_from_env_uses_server_options_when_env_missing() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        // SAFETY: test-scoped env cleanup.
        unsafe {
            std::env::remove_var(MCP_WORKER_THREADS_ENV);
            std::env::remove_var(MCP_TOOL_TIMEOUT_MS_ENV);
        }

        let worker_pool = WorkerPool::from_env(
            "atlas-mcp:option-worker",
            ServerOptions {
                worker_threads: 4,
                tool_timeout_ms: 4321,
                tool_timeout_ms_by_tool: HashMap::new(),
            },
        )
        .expect("pool from options");

        assert_eq!(worker_pool.handles.len(), 4);
        assert_eq!(worker_pool.timeout(), Duration::from_millis(4321));
    }

    #[test]
    fn transport_applies_per_tool_timeout_overrides() {
        let fixture = setup_fixture();
        let input = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{\"name\":\"__test_sleep\",\"arguments\":{\"sleep_ms\":1200}}}\n";
        let reader = BufReader::new(Cursor::new(input.as_bytes()));
        let mut writer = Vec::new();

        run_server_io(
            reader,
            &mut writer,
            "/ignored",
            &fixture.db_path,
            ServerOptions {
                worker_threads: 1,
                tool_timeout_ms: 2_000,
                tool_timeout_ms_by_tool: HashMap::from([("__test_sleep".to_owned(), 1_000)]),
            },
        )
        .expect("run server io");

        let responses = parse_output_lines(writer);
        let response = responses
            .iter()
            .find(|value| value["id"] == serde_json::json!(1))
            .expect("sleep response");
        assert_eq!(response["error"]["code"], serde_json::json!(-32003));
    }

    #[test]
    fn transport_rate_limits_when_queue_is_saturated() {
        let fixture = setup_fixture();
        let mut input = String::new();
        for id in 1..=12 {
            input.push_str(&format!(
                "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"method\":\"tools/call\",\"params\":{{\"name\":\"__test_sleep\",\"arguments\":{{\"sleep_ms\":200}}}}}}\n"
            ));
        }
        let reader = BufReader::new(Cursor::new(input.into_bytes()));
        let mut writer = Vec::new();

        run_server_io(
            reader,
            &mut writer,
            "/ignored",
            &fixture.db_path,
            ServerOptions {
                worker_threads: 1,
                tool_timeout_ms: 1_000,
                tool_timeout_ms_by_tool: HashMap::from([("__test_sleep".to_owned(), 1_000)]),
            },
        )
        .expect("run server io");

        let responses = parse_output_lines(writer);
        assert!(
            responses
                .iter()
                .any(|value| value["error"]["code"] == serde_json::json!(-32004)),
            "queue saturation should return rate-limited error"
        );
    }

    #[test]
    fn timed_out_request_emits_single_response_even_if_completion_arrives_late() {
        let mut pending = HashMap::new();
        let mut stats = TransportStats::default();
        let mut writer = Vec::new();
        let token = 7;
        let request_id = serde_json::json!(99);
        let connection_state = test_connection_state();

        pending.insert(
            token,
            PendingRequest {
                id: request_id.clone(),
                request: RequestLogContext {
                    request_id: "99".to_owned(),
                    method: "tools/call".to_owned(),
                    tool_name: Some("slow_tool".to_owned()),
                },
                queued_at: Instant::now() - Duration::from_millis(20),
                deadline: Instant::now() - Duration::from_millis(1),
                timeout_ms: 10,
                progress_token: None,
                cancel_flag: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
        );

        drain_expired_requests(&mut writer, &mut pending, &mut stats, &connection_state)
            .expect("expire request");
        assert!(
            pending.is_empty(),
            "timed out request must be removed from pending set"
        );
        assert_eq!(stats.timed_out, 1);

        handle_completion_event(
            &mut writer,
            &mut pending,
            token,
            jsonrpc_ok(request_id.clone(), serde_json::json!({"late": true})),
            RequestCompletion {
                request: RequestLogContext {
                    request_id: "99".to_owned(),
                    method: "tools/call".to_owned(),
                    tool_name: Some("slow_tool".to_owned()),
                },
                queue_wait_ms: 5,
                execution_ms: 20,
                success: true,
            },
            &mut stats,
            &connection_state,
        )
        .expect("late completion handling");

        let responses = parse_output_lines(writer);
        assert_eq!(
            responses.len(),
            1,
            "late completion must not emit duplicate response"
        );
        assert_eq!(responses[0]["id"], request_id);
        assert_eq!(responses[0]["error"]["code"], serde_json::json!(-32003));
        assert_eq!(
            responses[0]["error"]["data"]["atlas_error_code"],
            serde_json::json!("request_timed_out")
        );
        assert!(
            responses[0]["error"]["message"]
                .as_str()
                .expect("timeout error message")
                .contains("timed out")
        );
        assert_eq!(
            stats.completed, 0,
            "late completion must not count as completed response"
        );
        assert_eq!(
            stats.dropped_late, 1,
            "late completion must be tracked as dropped"
        );
    }
}
