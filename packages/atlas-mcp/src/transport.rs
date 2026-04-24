//! JSON-RPC 2.0 / MCP stdio transport loop.
//!
//! Reads newline-delimited JSON from stdin, dispatches each request, and
//! writes newline-delimited JSON responses to stdout.  Follows the MCP
//! 2024-11-05 protocol specification.

use std::collections::HashMap;
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
use std::os::unix::net::{UnixListener, UnixStream};

const MCP_WORKER_THREADS_ENV: &str = "ATLAS_MCP_WORKER_THREADS";
const MCP_TOOL_TIMEOUT_MS_ENV: &str = "ATLAS_MCP_TOOL_TIMEOUT_MS";
const DEFAULT_WORKER_THREADS: usize = 2;
const DEFAULT_TOOL_TIMEOUT_MS: u64 = 300_000;
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

#[derive(Clone, Copy, Debug)]
pub struct ServerOptions {
    pub worker_threads: usize,
    pub tool_timeout_ms: u64,
}

impl Default for ServerOptions {
    fn default() -> Self {
        Self {
            worker_threads: DEFAULT_WORKER_THREADS,
            tool_timeout_ms: DEFAULT_TOOL_TIMEOUT_MS,
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
    eprintln!("atlas-mcp: server ready (repo={repo_root}, db={db_path})");
    eprintln!("atlas-mcp: reading JSON-RPC requests from stdin");

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let reader = BufReader::new(stdin);
    let mut writer = std::io::BufWriter::new(stdout.lock());

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

    let listener = UnixListener::bind(socket_path)
        .with_context(|| format!("cannot bind {}", socket_path.display()))?;
    let worker_pool = Arc::new(WorkerPool::from_env("atlas-mcp:tool-worker", options)?);
    let next_connection = AtomicUsize::new(0);

    eprintln!(
        "atlas-mcp: daemon ready (repo={repo_root}, db={db_path}, socket={})",
        socket_path.display()
    );

    for stream in listener.incoming() {
        let stream = stream
            .with_context(|| format!("cannot accept connection on {}", socket_path.display()))?;
        let repo_root = repo_root.to_owned();
        let db_path = db_path.to_owned();
        let worker_pool = Arc::clone(&worker_pool);
        let connection_id = next_connection.fetch_add(1, Ordering::Relaxed) + 1;
        let thread_name = format!("atlas-mcp:daemon-client-{connection_id}");
        let log_thread_name = thread_name.clone();
        thread::Builder::new()
            .name(thread_name.clone())
            .spawn(move || {
                if let Err(error) = serve_socket_connection(stream, &repo_root, &db_path, worker_pool) {
                    tracing::warn!(error = %error, client = %log_thread_name, "daemon client failed");
                }
            })
            .with_context(|| format!("cannot spawn '{thread_name}'"))?;
    }

    Ok(())
}

#[cfg(not(unix))]
pub fn run_socket_server_with_options(
    _socket_path: &Path,
    _repo_root: &str,
    _db_path: &str,
    _options: ServerOptions,
) -> Result<()> {
    Err(anyhow::anyhow!(
        "Unix socket MCP daemon transport is unsupported on this platform"
    ))
}

fn run_server_io<R: BufRead + Send, W: Write>(
    reader: R,
    writer: &mut W,
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
) -> Result<()> {
    let worker_pool = Arc::new(WorkerPool::from_env("atlas-mcp:tool-worker", options)?);
    serve_connection(reader, writer, repo_root, db_path, worker_pool)
}

fn serve_connection<R: BufRead + Send, W: Write>(
    reader: R,
    writer: &mut W,
    repo_root: &str,
    db_path: &str,
    worker_pool: Arc<WorkerPool>,
) -> Result<()> {
    let (event_tx, event_rx) = mpsc::channel::<TransportEvent>();

    thread::scope(|scope| -> Result<()> {
        let reader_tx = event_tx.clone();
        scope.spawn(move || read_requests(reader, reader_tx));
        process_requests(
            writer,
            repo_root,
            db_path,
            worker_pool.as_ref(),
            event_tx,
            event_rx,
        )
    })
}

#[cfg(unix)]
fn serve_socket_connection(
    stream: UnixStream,
    repo_root: &str,
    db_path: &str,
    worker_pool: Arc<WorkerPool>,
) -> Result<()> {
    let reader_stream = stream.try_clone().context("cannot clone daemon stream")?;
    let mut reader = BufReader::new(reader_stream);
    let mut writer = std::io::BufWriter::new(stream);

    perform_socket_handshake(&mut reader, &mut writer, repo_root, db_path)?;
    serve_connection(reader, &mut writer, repo_root, db_path, worker_pool)
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

fn process_requests<W: Write>(
    writer: &mut W,
    repo_root: &str,
    db_path: &str,
    worker_pool: &WorkerPool,
    event_tx: mpsc::Sender<TransportEvent>,
    event_rx: mpsc::Receiver<TransportEvent>,
) -> Result<()> {
    let mut input_closed = false;
    let mut next_token = 0_u64;
    let mut pending = HashMap::<u64, PendingRequest>::new();
    let mut stats = TransportStats::default();
    let request_ctx = RequestDispatchContext {
        repo_root,
        db_path,
        worker_pool,
        event_tx: &event_tx,
    };

    loop {
        drain_expired_requests(writer, &mut pending, &mut stats)?;
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
            )?,
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
                -32700,
                format!("parse error: {error}"),
            );
            write_response(writer, &response)?;
            return Ok(());
        }
    };

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
        let timeout = ctx.worker_pool.timeout();
        let queued_at = Instant::now();
        let request_log_for_worker = request_log.clone();
        tracing::debug!(
            request_id = %request_log.request_id,
            method = %request_log.method,
            tool = request_log.tool_name.as_deref().unwrap_or("-"),
            timeout_ms = timeout.as_millis() as u64,
            "queued MCP request"
        );
        pending.insert(
            token,
            PendingRequest {
                id,
                request: request_log,
                queued_at,
                deadline: Instant::now() + timeout,
                timeout_ms: timeout.as_millis(),
            },
        );
        ctx.worker_pool.submit(move || {
            let started_at = Instant::now();
            let queue_wait_ms = started_at.duration_since(queued_at).as_millis();
            tracing::debug!(
                request_id = %request_log_for_worker.request_id,
                method = %request_log_for_worker.method,
                tool = request_log_for_worker.tool_name.as_deref().unwrap_or("-"),
                queue_wait_ms = queue_wait_ms as u64,
                "started MCP request"
            );
            let dispatch_started_at = Instant::now();
            let result = dispatch(&method_name, params.as_ref(), &repo_root, &db_path);
            let execution_ms = dispatch_started_at.elapsed().as_millis();
            let success = result.is_ok();
            let response = match result {
                Ok(result) => jsonrpc_ok(request_id, result),
                Err(error) => jsonrpc_error(request_id, -32000, user_visible_error_message(&error)),
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
        })?;
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
                error = %error,
                "MCP method failed"
            );
            jsonrpc_error(id, -32000, user_visible_error_message(&error))
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
                -32000,
                format!(
                    "worker pool timed out after {} ms while handling request",
                    request.timeout_ms
                ),
            );
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
}

struct RequestDispatchContext<'a> {
    repo_root: &'a str,
    db_path: &'a str,
    worker_pool: &'a WorkerPool,
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
    dropped_late: u64,
}

enum TransportEvent {
    InputLine(String),
    InputClosed,
    InputError(String),
    Response {
        token: u64,
        response: String,
        completion: RequestCompletion,
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

// ---------------------------------------------------------------------------
// Method dispatch
// ---------------------------------------------------------------------------

fn dispatch(
    method: &str,
    params: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
) -> Result<serde_json::Value> {
    match method {
        "initialize" => Ok(serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": {
                "tools": {},
                "prompts": { "listChanged": false }
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
                .ok_or_else(|| anyhow::anyhow!("missing prompt name"))?;
            let args = params.and_then(|p| p.get("arguments"));
            prompts::prompt_get(name, args)
        }

        "tools/call" => {
            let name = params
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
                .ok_or_else(|| anyhow::anyhow!("missing tool name"))?;
            let args = params.and_then(|p| p.get("arguments"));
            tools::call(name, args, repo_root, db_path)
        }

        other => Err(anyhow::anyhow!("method not found: {other}")),
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

struct WorkerPool {
    thread_name_prefix: String,
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

    fn submit<F>(&self, task: F) -> Result<()>
    where
        F: FnOnce() + Send + 'static,
    {
        self.sender
            .send(WorkerMessage::Run(Box::new(task)))
            .map_err(|_| {
                anyhow::anyhow!("worker pool '{}' is unavailable", self.thread_name_prefix)
            })
    }

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

fn user_visible_error_message(error: &anyhow::Error) -> String {
    user_facing_error_message(&error.to_string(), &format!("{error:#}"))
}

fn jsonrpc_error(id: serde_json::Value, code: i32, message: String) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": code, "message": message }
    })
    .to_string()
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
        assert_eq!(responses.len(), 2);
        assert_eq!(responses[0]["error"]["code"], -32700);
        assert_eq!(responses[0]["id"], serde_json::Value::Null);
        assert_eq!(responses[1]["id"], 7);
        assert_eq!(responses[1]["error"]["code"], -32000);
        assert!(
            responses[1]["error"]["message"]
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

        assert_eq!(response["error"]["code"], -32000);
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
    #[cfg(unix)]
    fn socket_transport_handles_handshake_and_tool_calls() {
        let fixture = setup_fixture();
        let (server_stream, mut client_stream) = UnixStream::pair().expect("unix pair");
        let worker_pool = Arc::new(
            WorkerPool::new("atlas-mcp:socket-test", 2, Duration::from_secs(1))
                .expect("spawn workers"),
        );
        let db_path = fixture.db_path.clone();
        let expected_db_path = fixture.db_path.clone();
        let server = std::thread::spawn(move || {
            serve_socket_connection(server_stream, "/repo", &db_path, worker_pool)
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
        let db_path = fixture.db_path.clone();
        let server = std::thread::spawn(move || {
            serve_socket_connection(server_stream, "/repo", &db_path, worker_pool)
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
            },
        )
        .expect("pool from options");

        assert_eq!(worker_pool.handles.len(), 4);
        assert_eq!(worker_pool.timeout(), Duration::from_millis(4321));
    }

    #[test]
    fn timed_out_request_emits_single_response_even_if_completion_arrives_late() {
        let mut pending = HashMap::new();
        let mut stats = TransportStats::default();
        let mut writer = Vec::new();
        let token = 7;
        let request_id = serde_json::json!(99);

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
            },
        );

        drain_expired_requests(&mut writer, &mut pending, &mut stats).expect("expire request");
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
        )
        .expect("late completion handling");

        let responses = parse_output_lines(writer);
        assert_eq!(
            responses.len(),
            1,
            "late completion must not emit duplicate response"
        );
        assert_eq!(responses[0]["id"], request_id);
        assert_eq!(responses[0]["error"]["code"], serde_json::json!(-32000));
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
