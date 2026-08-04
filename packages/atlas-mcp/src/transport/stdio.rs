//! Stdio-based MCP transport entry points.

use std::sync::mpsc;

use anyhow::{Context, Result};
use rmcp::serve_server;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::mpsc as tokio_mpsc;

use crate::rmcp_server::AtlasRmcpServer;

use super::types::ServerOptions;
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

    #[cfg(unix)]
    let _shutdown_guard = install_stdio_shutdown_handler()?;

    run_rmcp_stdio_server(repo_root, db_path, options)
}

#[doc(hidden)]
#[derive(Debug)]
pub struct StdioTestScriptResult {
    pub output: Vec<serde_json::Value>,
    pub server_error: Option<String>,
}

#[doc(hidden)]
pub fn run_stdio_jsonrpc_session_for_tests(
    input: &str,
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
) -> Result<Vec<serde_json::Value>> {
    let result = run_stdio_jsonrpc_session_capture_for_tests(input, repo_root, db_path, options)?;
    if let Some(error) = result.server_error {
        return Err(anyhow::anyhow!(error));
    }
    Ok(result.output)
}

#[doc(hidden)]
pub fn run_stdio_jsonrpc_session_capture_for_tests(
    input: &str,
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
) -> Result<StdioTestScriptResult> {
    let (output_tx, output_rx) = mpsc::channel::<serde_json::Value>();
    let input = normalize_stdio_test_script(input);
    let repo_root = repo_root.to_owned();
    let db_path = db_path.to_owned();
    let join_result = std::thread::Builder::new()
        .name("atlas-mcp:stdio-test-script".to_owned())
        .spawn(move || run_rmcp_stdio_test_script(&input, &repo_root, &db_path, options, output_tx))
        .context("cannot spawn stdio test script")?
        .join()
        .map_err(|_| anyhow::anyhow!("stdio test script thread panicked"))?;
    let mut output = Vec::new();
    while let Ok(value) = output_rx.try_recv() {
        output.push(value);
    }
    Ok(StdioTestScriptResult {
        output,
        server_error: join_result.err().map(|error| error.to_string()),
    })
}

fn run_rmcp_stdio_server(repo_root: &str, db_path: &str, options: ServerOptions) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(options.worker_threads.max(1))
        .enable_all()
        .build()
        .context("cannot build tokio runtime for rmcp stdio server")?;
    runtime.block_on(async move {
        let server = AtlasRmcpServer::new(repo_root, db_path, options);
        let running = serve_server(server, rmcp::transport::stdio())
            .await
            .map_err(|error| anyhow::anyhow!("rmcp stdio initialize failed: {error}"))?;
        running
            .waiting()
            .await
            .map_err(|error| anyhow::anyhow!("rmcp stdio task join failed: {error}"))?;
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// InteractiveStdioTestSession
// ---------------------------------------------------------------------------

#[derive(Debug)]
enum SessionCommand {
    Json(serde_json::Value),
    Close,
}

#[doc(hidden)]
pub struct InteractiveStdioTestSession {
    command_tx: tokio_mpsc::UnboundedSender<SessionCommand>,
    output_rx: mpsc::Receiver<serde_json::Value>,
    join_handle: Option<std::thread::JoinHandle<Result<()>>>,
}

#[doc(hidden)]
impl InteractiveStdioTestSession {
    pub fn start(repo_root: &str, db_path: &str, options: ServerOptions) -> Result<Self> {
        Self::start_with_state(Some(repo_root), Some(db_path), false, options)
    }

    fn start_with_state(
        repo_root: Option<&str>,
        db_path: Option<&str>,
        _dynamic_roots: bool,
        options: ServerOptions,
    ) -> Result<Self> {
        let (command_tx, command_rx) = tokio_mpsc::unbounded_channel::<SessionCommand>();
        let (output_tx, output_rx) = mpsc::channel::<serde_json::Value>();
        let repo_root = repo_root.unwrap_or_default().to_owned();
        let db_path = db_path.unwrap_or_default().to_owned();
        let join_handle = std::thread::Builder::new()
            .name("atlas-mcp:stdio-test-session".to_owned())
            .spawn(move || {
                run_rmcp_stdio_test_session(&repo_root, &db_path, options, command_rx, output_tx)
            })
            .context("cannot spawn stdio test session")?;
        Ok(Self {
            command_tx,
            output_rx,
            join_handle: Some(join_handle),
        })
    }

    pub fn send_json(&self, value: &serde_json::Value) -> Result<()> {
        self.command_tx
            .send(SessionCommand::Json(value.clone()))
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
        let _ = self.command_tx.send(SessionCommand::Close);
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

fn run_rmcp_stdio_test_script(
    input: &str,
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
    output_tx: mpsc::Sender<serde_json::Value>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(options.worker_threads.max(1))
        .enable_all()
        .build()
        .context("cannot build tokio runtime for rmcp stdio test script")?;
    runtime.block_on(async move {
        let (server_io, client_io) = tokio::io::duplex(1024 * 1024);
        let server = AtlasRmcpServer::new(repo_root, db_path, options);
        let server_task = tokio::spawn(async move {
            let running = serve_server(server, server_io)
                .await
                .map_err(|error| anyhow::anyhow!("rmcp stdio initialize failed: {error}"))?;
            running
                .waiting()
                .await
                .map_err(|error| anyhow::anyhow!("rmcp stdio task join failed: {error}"))?;
            Ok::<(), anyhow::Error>(())
        });
        let (client_r, mut client_w) = tokio::io::split(client_io);
        let reader_task =
            tokio::spawn(async move { forward_rmcp_output(client_r, output_tx).await });
        client_w.write_all(input.as_bytes()).await?;
        client_w.shutdown().await?;
        server_task
            .await
            .map_err(|error| anyhow::anyhow!("rmcp stdio server task panicked: {error}"))??;
        reader_task
            .await
            .map_err(|error| anyhow::anyhow!("rmcp stdio reader task panicked: {error}"))??;
        Ok(())
    })
}

fn run_rmcp_stdio_test_session(
    repo_root: &str,
    db_path: &str,
    options: ServerOptions,
    mut command_rx: tokio_mpsc::UnboundedReceiver<SessionCommand>,
    output_tx: mpsc::Sender<serde_json::Value>,
) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(options.worker_threads.max(1))
        .enable_all()
        .build()
        .context("cannot build tokio runtime for rmcp stdio test session")?;
    runtime.block_on(async move {
        let (server_io, client_io) = tokio::io::duplex(1024 * 1024);
        let server = AtlasRmcpServer::new(repo_root, db_path, options);
        let server_task = tokio::spawn(async move {
            let running = serve_server(server, server_io)
                .await
                .map_err(|error| anyhow::anyhow!("rmcp stdio initialize failed: {error}"))?;
            running
                .waiting()
                .await
                .map_err(|error| anyhow::anyhow!("rmcp stdio task join failed: {error}"))?;
            Ok::<(), anyhow::Error>(())
        });
        let (client_r, mut client_w) = tokio::io::split(client_io);
        let reader_task =
            tokio::spawn(async move { forward_rmcp_output(client_r, output_tx).await });
        while let Some(command) = command_rx.recv().await {
            match command {
                SessionCommand::Json(value) => {
                    client_w
                        .write_all((serde_json::to_string(&value)? + "\n").as_bytes())
                        .await?;
                    client_w.flush().await?;
                }
                SessionCommand::Close => {
                    client_w.shutdown().await?;
                    break;
                }
            }
        }
        server_task
            .await
            .map_err(|error| anyhow::anyhow!("rmcp stdio server task panicked: {error}"))??;
        reader_task
            .await
            .map_err(|error| anyhow::anyhow!("rmcp stdio reader task panicked: {error}"))??;
        Ok(())
    })
}

fn normalize_stdio_test_script(input: &str) -> String {
    let Some(first_nonempty) = input.lines().find(|line| !line.trim().is_empty()) else {
        return String::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(first_nonempty) else {
        return input.to_owned();
    };
    let method = value.get("method").and_then(serde_json::Value::as_str);
    if matches!(method, Some("initialize") | Some("server/discover")) {
        return input.to_owned();
    }
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":-1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"zed\",\"version\":\"1.0.0\"}}}}}}\n{{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{{}}}}\n{}",
        crate::MCP_PROTOCOL_VERSION,
        input
    )
}

async fn forward_rmcp_output<R>(reader: R, output_tx: mpsc::Sender<serde_json::Value>) -> Result<()>
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut reader = tokio::io::BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        let bytes = reader.read_line(&mut line).await?;
        if bytes == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value =
            serde_json::from_str(trimmed).context("stdio test output must be valid JSON")?;
        output_tx
            .send(value)
            .map_err(|_| anyhow::anyhow!("stdio test output closed"))?;
    }
    Ok(())
}
