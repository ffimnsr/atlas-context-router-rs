//! Stdio-based MCP transport entry points.

use std::io::{BufReader, Write};
use std::sync::mpsc;

use anyhow::{Context, Result};

use super::io::{ConnectionStartup, process_requests, run_server_io, run_server_io_with_state};
use super::repo_selection::launch_cwd_repo_root;
use super::types::{ServerOptions, TransportEvent};
use super::worker::WorkerPool;
#[cfg(unix)]
use super::worker::install_stdio_shutdown_handler;

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Run the MCP server until stdin closes.
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

pub fn run_server_with_dynamic_roots(options: ServerOptions) -> Result<()> {
    crate::tools::health::mark_server_started();
    eprintln!("atlas-mcp: server ready (repo=<deferred>, db=<deferred>)");
    eprintln!("atlas-mcp: reading JSON-RPC requests from stdin");

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let reader = BufReader::new(stdin);
    let mut writer = std::io::BufWriter::new(stdout.lock());
    #[cfg(unix)]
    let _shutdown_guard = install_stdio_shutdown_handler()?;

    let launch_cwd_repo_root = launch_cwd_repo_root();
    run_server_io_with_state(
        reader,
        &mut writer,
        ConnectionStartup {
            repo_root: None,
            db_path: None,
            dynamic_roots: true,
            launch_cwd_repo_root: launch_cwd_repo_root.as_deref(),
        },
        options,
    )
}

#[doc(hidden)]
pub fn run_stdio_jsonrpc_session_for_tests(
    input: &str,
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
) -> Result<Vec<serde_json::Value>> {
    let reader = BufReader::new(std::io::Cursor::new(input.as_bytes()));
    let mut writer = Vec::new();
    run_server_io(reader, &mut writer, repo_root, db_path, options)?;
    String::from_utf8(writer)
        .context("stdio test output must be utf-8")?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).context("stdio test output must be valid JSON"))
        .collect()
}

// ---------------------------------------------------------------------------
// InteractiveStdioTestSession
// ---------------------------------------------------------------------------

#[doc(hidden)]
pub struct InteractiveStdioTestSession {
    event_tx: mpsc::Sender<TransportEvent>,
    output_rx: mpsc::Receiver<serde_json::Value>,
    join_handle: Option<std::thread::JoinHandle<Result<()>>>,
}

#[doc(hidden)]
impl InteractiveStdioTestSession {
    pub fn start(repo_root: &str, db_path: &str, options: ServerOptions) -> Result<Self> {
        Self::start_with_state(Some(repo_root), Some(db_path), false, None, options)
    }

    pub fn start_dynamic(options: ServerOptions) -> Result<Self> {
        Self::start_with_state(None, None, true, None, options)
    }

    pub fn start_dynamic_with_launch_cwd(
        launch_cwd_repo_root: &str,
        options: ServerOptions,
    ) -> Result<Self> {
        Self::start_with_state(None, None, true, Some(launch_cwd_repo_root), options)
    }

    fn start_with_state(
        repo_root: Option<&str>,
        db_path: Option<&str>,
        dynamic_roots: bool,
        launch_cwd_repo_root: Option<&str>,
        options: ServerOptions,
    ) -> Result<Self> {
        let (event_tx, event_rx) = mpsc::channel::<TransportEvent>();
        let (output_tx, output_rx) = mpsc::channel::<serde_json::Value>();
        let repo_root = repo_root.map(str::to_owned);
        let db_path = db_path.map(str::to_owned);
        let launch_cwd_repo_root = launch_cwd_repo_root.map(str::to_owned);
        let thread_event_tx = event_tx.clone();
        let join_handle = std::thread::Builder::new()
            .name("atlas-mcp:stdio-test-session".to_owned())
            .spawn(move || {
                let worker_pool = std::sync::Arc::new(WorkerPool::from_env(
                    "atlas-mcp:tool-worker",
                    options.clone(),
                )?);
                let connection_state = connection_state(
                    repo_root.as_deref(),
                    db_path.as_deref(),
                    dynamic_roots,
                    launch_cwd_repo_root.as_deref(),
                );
                let mut writer = JsonValueChannelWriter::new(output_tx);
                process_requests(
                    &mut writer,
                    worker_pool.as_ref(),
                    &options,
                    thread_event_tx,
                    event_rx,
                    connection_state,
                )
            })
            .context("cannot spawn stdio test session")?;
        Ok(Self {
            event_tx,
            output_rx,
            join_handle: Some(join_handle),
        })
    }

    pub fn send_json(&self, value: &serde_json::Value) -> Result<()> {
        self.event_tx
            .send(TransportEvent::InputLine(serde_json::to_string(value)?))
            .map_err(|_| anyhow::anyhow!("stdio test session disconnected"))
    }

    pub fn recv_json(&self, timeout: std::time::Duration) -> Result<Option<serde_json::Value>> {
        match self.output_rx.recv_timeout(timeout) {
            Ok(value) => Ok(Some(value)),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(mpsc::RecvTimeoutError::Disconnected) => Ok(None),
        }
    }

    pub fn finish(mut self) -> Result<Vec<serde_json::Value>> {
        let _ = self.event_tx.send(TransportEvent::InputClosed);
        let join_result = self
            .join_handle
            .take()
            .expect("stdio test session join handle")
            .join()
            .map_err(|_| anyhow::anyhow!("stdio test session thread panicked"))?;
        join_result?;
        let mut remaining = Vec::new();
        while let Ok(value) = self.output_rx.try_recv() {
            remaining.push(value);
        }
        Ok(remaining)
    }
}

// ---------------------------------------------------------------------------
// JsonValueChannelWriter
// ---------------------------------------------------------------------------

struct JsonValueChannelWriter {
    buffer: Vec<u8>,
    output_tx: mpsc::Sender<serde_json::Value>,
}

impl JsonValueChannelWriter {
    fn new(output_tx: mpsc::Sender<serde_json::Value>) -> Self {
        Self {
            buffer: Vec::new(),
            output_tx,
        }
    }
}

impl Write for JsonValueChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        while let Some(pos) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line = self.buffer.drain(..=pos).collect::<Vec<_>>();
            let line = String::from_utf8(line)
                .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "utf-8"))?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let value = serde_json::from_str(line).map_err(|error| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid json from stdio test session: {error}"),
                )
            })?;
            self.output_tx.send(value).map_err(|_| {
                std::io::Error::new(std::io::ErrorKind::BrokenPipe, "stdio test output closed")
            })?;
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// Helper: connection_state for test sessions
use super::types::connection_state;
