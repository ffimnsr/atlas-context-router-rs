//! Worker pool for executing MCP tool calls on background threads.

#[cfg(unix)]
use std::net::Shutdown;
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};

use super::jsonrpc::JsonRpcErrorKind;
use super::types::ServerOptions;

const MCP_WORKER_THREADS_ENV: &str = "ATLAS_MCP_WORKER_THREADS";
const MCP_TOOL_TIMEOUT_MS_ENV: &str = "ATLAS_MCP_TOOL_TIMEOUT_MS";

// ---------------------------------------------------------------------------
// Worker types
// ---------------------------------------------------------------------------

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
pub(crate) struct WorkerSubmitError {
    pub(crate) kind: JsonRpcErrorKind,
    pub(crate) message: String,
}

// ---------------------------------------------------------------------------
// WorkerPool
// ---------------------------------------------------------------------------

pub(crate) struct WorkerPool {
    thread_name_prefix: String,
    #[cfg(test)]
    #[allow(dead_code)]
    timeout: Duration,
    sender: std::sync::mpsc::SyncSender<WorkerMessage>,
    handles: Vec<thread::JoinHandle<()>>,
}

impl WorkerPool {
    pub(crate) fn from_env(thread_name_prefix: &str, options: ServerOptions) -> Result<Self> {
        let worker_count = parse_env_usize(MCP_WORKER_THREADS_ENV, options.worker_threads)?;
        let timeout_ms = parse_env_u64(MCP_TOOL_TIMEOUT_MS_ENV, options.tool_timeout_ms)?;
        Self::new(
            thread_name_prefix,
            worker_count,
            Duration::from_millis(timeout_ms),
        )
    }

    #[cfg_attr(not(test), allow(unused_variables))]
    pub(crate) fn new(
        thread_name_prefix: &str,
        worker_count: usize,
        timeout: Duration,
    ) -> Result<Self> {
        let worker_count = worker_count.max(1);
        let queue_bound = worker_count.saturating_mul(4).max(1);
        let (sender, receiver) = std::sync::mpsc::sync_channel::<WorkerMessage>(queue_bound);
        let receiver = Arc::new(std::sync::Mutex::new(receiver));
        let mut handles = Vec::with_capacity(worker_count);
        let thread_name_prefix = thread_name_prefix.to_owned();

        for index in 0..worker_count {
            let receiver = Arc::clone(&receiver);
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
                            Ok(WorkerMessage::Run(task)) => {
                                if let Err(payload) = std::panic::catch_unwind(
                                    std::panic::AssertUnwindSafe(move || task.run()),
                                ) {
                                    tracing::error!(
                                        message = %panic_payload_message(&payload),
                                        "MCP worker task panicked; worker thread survived"
                                    );
                                }
                            }
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
    #[allow(dead_code)]
    pub(crate) fn run<T, F>(&self, task: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce() -> Result<T> + Send + 'static,
    {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel::<Result<T>>();
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
                std::sync::mpsc::RecvTimeoutError::Timeout => anyhow::anyhow!(
                    "worker pool '{}' timed out after {} ms while handling task",
                    self.thread_name_prefix,
                    self.timeout.as_millis()
                ),
                std::sync::mpsc::RecvTimeoutError::Disconnected => {
                    anyhow::anyhow!("worker pool '{}' dropped response", self.thread_name_prefix)
                }
            })?
    }

    pub(crate) fn submit<F>(&self, task: F) -> std::result::Result<(), WorkerSubmitError>
    where
        F: FnOnce() + Send + 'static,
    {
        match self.sender.try_send(WorkerMessage::Run(Box::new(task))) {
            Ok(()) => Ok(()),
            Err(std::sync::mpsc::TrySendError::Full(_)) => Err(WorkerSubmitError {
                kind: JsonRpcErrorKind::RateLimited,
                message: format!(
                    "worker pool '{}' is saturated; retry later",
                    self.thread_name_prefix
                ),
            }),
            Err(std::sync::mpsc::TrySendError::Disconnected(_)) => Err(WorkerSubmitError {
                kind: JsonRpcErrorKind::WorkerUnavailable,
                message: format!("worker pool '{}' is unavailable", self.thread_name_prefix),
            }),
        }
    }

    #[cfg(test)]
    #[allow(dead_code)]
    pub(crate) fn timeout(&self) -> Duration {
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

// ---------------------------------------------------------------------------
// Signal handlers (Unix)
// ---------------------------------------------------------------------------

#[cfg(unix)]
pub(crate) struct ShutdownHandlerGuard(signal_hook::iterator::Handle);

#[cfg(unix)]
impl Drop for ShutdownHandlerGuard {
    fn drop(&mut self) {
        self.0.close();
    }
}

#[cfg(unix)]
pub(crate) fn install_stdio_shutdown_handler() -> Result<ShutdownHandlerGuard> {
    spawn_signal_handler(move || unsafe {
        let _ = libc::close(libc::STDIN_FILENO);
    })
}

#[cfg(unix)]
pub(crate) fn install_socket_shutdown_handler(
    shutdown: Arc<AtomicBool>,
    active_streams: Arc<std::sync::Mutex<Vec<UnixStream>>>,
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
pub(crate) struct ActiveStreamGuard {
    raw_fd: i32,
    active_streams: Arc<std::sync::Mutex<Vec<UnixStream>>>,
}

#[cfg(unix)]
impl ActiveStreamGuard {
    pub(crate) fn new(raw_fd: i32, active_streams: Arc<std::sync::Mutex<Vec<UnixStream>>>) -> Self {
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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

fn panic_payload_message(payload: &Box<dyn std::any::Any + Send + 'static>) -> String {
    if let Some(message) = payload.downcast_ref::<&'static str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".to_owned()
    }
}
