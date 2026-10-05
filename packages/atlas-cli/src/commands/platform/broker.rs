//! Unix stdio broker: daemon attach/spawn, stdio relay, and reconnect loop.
//!
//! Extracted from `platform.rs` so the command module stays below the 1K LOC
//! threshold. Only compiled on Unix; the Windows named-pipe broker lives in
//! `platform.rs`.

use anyhow::{Context, Result};
use atlas_mcp::ServerOptions;

use super::broker_signals::BrokerShutdown;
use super::{
    DaemonHandshakeRequest, DaemonHandshakeResponse, broker_reconnect_delay, spawn_daemon_process,
};

enum RelayOutcome {
    /// stdin closed naturally or signal received — session is done.
    Clean,
    /// Daemon socket disconnected while stdin was still open — daemon crashed.
    DaemonDied,
}

const MAX_DAEMON_RECONNECTS: u32 = 3;

pub(super) fn run_stdio_broker(
    instance: crate::mcp_instance::McpInstance,
    options: ServerOptions,
) -> Result<()> {
    // Install shutdown handling before waiting on locks or daemon startup: a
    // signal in that window would otherwise kill the broker with the default
    // disposition and orphan a spawned daemon.
    let shutdown = BrokerShutdown::install()?;
    let coordination_lock = instance.acquire_lock_blocking()?;
    let stream = match instance.inspect_metadata()? {
        crate::mcp_instance::McpInstanceStatus::Ready(metadata) => {
            match connect_to_daemon(
                &metadata.socket_path,
                &instance.repo_root,
                &instance.db_path,
            ) {
                Ok(stream) => {
                    eprintln!(
                        "atlas-mcp: broker attach socket={} repo={} db={}",
                        metadata.socket_path, instance.repo_root, instance.db_path
                    );
                    stream
                }
                Err(error) => {
                    eprintln!("atlas-mcp: stale daemon state detected; respawn: {error:#}");
                    eprintln!(
                        "atlas-mcp: broker cleanup socket={} repo={} db={}",
                        metadata.socket_path, instance.repo_root, instance.db_path
                    );
                    instance.clear_runtime_state()?;
                    match spawn_and_wait_for_daemon(&instance, options.clone(), &shutdown)? {
                        Some(stream) => stream,
                        None => return Ok(()),
                    }
                }
            }
        }
        crate::mcp_instance::McpInstanceStatus::Missing => {
            match spawn_and_wait_for_daemon(&instance, options.clone(), &shutdown)? {
                Some(stream) => stream,
                None => return Ok(()),
            }
        }
        crate::mcp_instance::McpInstanceStatus::Stale(stale) => {
            eprintln!(
                "atlas-mcp: cleaning stale daemon state for {} socket={} ({:?})",
                instance.instance_id,
                instance.socket_path.display(),
                stale.reasons
            );
            instance.clear_runtime_state()?;
            match spawn_and_wait_for_daemon(&instance, options.clone(), &shutdown)? {
                Some(stream) => stream,
                None => return Ok(()),
            }
        }
    };

    drop(coordination_lock);

    let mut stream = stream;
    let mut reconnects = 0u32;
    loop {
        match relay_stdio(stream, &shutdown)? {
            RelayOutcome::Clean => break,
            RelayOutcome::DaemonDied => {
                reconnects += 1;
                if reconnects > MAX_DAEMON_RECONNECTS {
                    return Err(anyhow::anyhow!(
                        "atlas-mcp: daemon crashed {MAX_DAEMON_RECONNECTS} times; giving up"
                    ));
                }
                let delay = broker_reconnect_delay(reconnects);
                eprintln!(
                    "atlas-mcp: daemon died mid-session; waiting {}ms before reconnect attempt {reconnects}/{MAX_DAEMON_RECONNECTS}",
                    delay.as_millis()
                );
                if shutdown.is_requested() {
                    break;
                }
                std::thread::sleep(delay);
                if shutdown.is_requested() {
                    break;
                }
                instance.clear_runtime_state()?;
                stream = match spawn_and_wait_for_daemon(&instance, options.clone(), &shutdown)? {
                    Some(stream) => stream,
                    None => break,
                };
            }
        }
    }
    Ok(())
}

fn spawn_and_wait_for_daemon(
    instance: &crate::mcp_instance::McpInstance,
    options: ServerOptions,
    shutdown: &BrokerShutdown,
) -> Result<Option<std::os::unix::net::UnixStream>> {
    eprintln!(
        "atlas-mcp: broker spawn socket={} repo={} db={}",
        instance.socket_path.display(),
        instance.repo_root,
        instance.db_path
    );
    spawn_daemon_process(instance, options)?;
    wait_for_daemon_ready(instance, shutdown)
}

fn wait_for_daemon_ready(
    instance: &crate::mcp_instance::McpInstance,
    shutdown: &BrokerShutdown,
) -> Result<Option<std::os::unix::net::UnixStream>> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut last_error: Option<anyhow::Error> = None;

    while std::time::Instant::now() < deadline {
        if shutdown.is_requested() {
            return Ok(None);
        }
        match instance.read_metadata() {
            Ok(Some(metadata)) => match connect_to_daemon(
                &metadata.socket_path,
                &instance.repo_root,
                &instance.db_path,
            ) {
                Ok(stream) => return Ok(Some(stream)),
                Err(error) => last_error = Some(error),
            },
            Ok(None) => {}
            Err(error) => last_error = Some(error),
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    if shutdown.is_requested() {
        return Ok(None);
    }

    Err(anyhow::anyhow!(
        "daemon readiness handshake failed for repo={} db={}: {}",
        instance.repo_root,
        instance.db_path,
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "daemon never became ready".to_owned())
    ))
}

fn relay_stdio(
    mut stream: std::os::unix::net::UnixStream,
    shutdown: &BrokerShutdown,
) -> Result<RelayOutcome> {
    use std::io::{self, Write};
    use std::net::Shutdown;
    use std::os::fd::AsRawFd;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut write_stream = stream
        .try_clone()
        .context("cannot clone broker socket stream")?;
    // Register the relay socket before any blocking work so a shutdown signal
    // can interrupt an in-flight session, and bail out early when a signal
    // already arrived while the broker was starting up.
    shutdown.attach(
        stream
            .try_clone()
            .context("cannot clone broker socket stream for shutdown")?,
    );
    if shutdown.is_requested() {
        shutdown.detach();
        return Ok(RelayOutcome::Clean);
    }
    // Set to true when stdin closes naturally (EOF), false if we close it to
    // interrupt the relay because the daemon died first.
    let stdin_done = Arc::new(AtomicBool::new(false));
    let stdin_done_writer = Arc::clone(&stdin_done);
    let stdin_thread = std::thread::spawn(move || -> Result<()> {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        match io::copy(&mut input, &mut write_stream) {
            Ok(_) => {
                // stdin reached EOF naturally before the daemon disconnected.
                stdin_done_writer.store(true, Ordering::Relaxed);
            }
            Err(error) if is_benign_broker_stdin_disconnect(&error) => return Ok(()),
            Err(error) => return Err(error).context("stdin relay failed"),
        }
        finish_broker_socket_write_half(write_stream.shutdown(Shutdown::Write))?;
        Ok(())
    });

    let stdout = io::stdout();
    let mut output = stdout.lock();
    let stdout_result = io::copy(&mut stream, &mut output);
    output.flush().context("cannot flush broker stdout")?;

    // Determine the outcome before joining the stdin thread.
    let outcome = if shutdown.is_requested() {
        // Clean signal-triggered shutdown.
        RelayOutcome::Clean
    } else if stdin_done.load(Ordering::Relaxed) {
        // stdin reached EOF before the daemon closed — normal session end.
        RelayOutcome::Clean
    } else {
        // Daemon socket closed while stdin was still open — daemon died.
        // Interrupt the stdin relay so it exits.
        let _ = stream.shutdown(Shutdown::Both);
        RelayOutcome::DaemonDied
    };

    match stdout_result {
        Ok(_) => {}
        Err(_error) if shutdown.is_requested() => {}
        Err(_error) if matches!(outcome, RelayOutcome::DaemonDied) => {}
        Err(error) => return Err(error).context("stdout relay failed"),
    }

    match stdin_thread.join() {
        Ok(Ok(())) => {}
        Ok(Err(error)) if shutdown.is_requested() => {
            tracing::debug!(error = %error, fd = stream.as_raw_fd(), "broker stdin relay interrupted by shutdown signal");
        }
        Ok(Err(error)) if matches!(outcome, RelayOutcome::DaemonDied) => {
            tracing::debug!(error = %error, "broker stdin relay interrupted by daemon death");
        }
        Ok(Err(error)) => return Err(error),
        Err(_) => return Err(anyhow::anyhow!("stdin relay thread panicked")),
    }
    shutdown.detach();
    Ok(outcome)
}

fn is_benign_broker_stdin_disconnect(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::UnexpectedEof
    )
}

fn finish_broker_socket_write_half(result: std::io::Result<()>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(error) if is_benign_broker_stdin_disconnect(&error) => Ok(()),
        Err(error) => Err(error).context("cannot close broker socket write half"),
    }
}

fn connect_to_daemon(
    socket_path: &str,
    repo_root: &str,
    db_path: &str,
) -> Result<std::os::unix::net::UnixStream> {
    use std::io::{BufRead, BufReader, Write};

    let mut stream = std::os::unix::net::UnixStream::connect(socket_path)
        .with_context(|| format!("cannot connect {}", socket_path))?;
    let mut reader = BufReader::new(
        stream
            .try_clone()
            .context("cannot clone daemon socket for handshake")?,
    );
    let request = DaemonHandshakeRequest {
        protocol_version: atlas_mcp::MCP_PROTOCOL_VERSION.to_owned(),
        repo_root: repo_root.to_owned(),
        db_path: db_path.to_owned(),
    };
    writeln!(stream, "{}", serde_json::to_string(&request)?)
        .context("cannot write daemon handshake")?;
    stream.flush().context("cannot flush daemon handshake")?;

    let mut response_line = String::new();
    let bytes = reader
        .read_line(&mut response_line)
        .context("cannot read daemon handshake")?;
    if bytes == 0 {
        return Err(anyhow::anyhow!("daemon closed before handshake response"));
    }
    let response: DaemonHandshakeResponse =
        serde_json::from_str(response_line.trim()).context("invalid daemon handshake response")?;
    if response.ok {
        Ok(stream)
    } else {
        Err(anyhow::anyhow!(
            response
                .error
                .unwrap_or_else(|| "daemon handshake rejected".to_owned())
        ))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn broker_stdin_disconnect_classifier_keeps_expected_socket_teardowns_nonfatal() {
        let benign = [
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::NotConnected,
            std::io::ErrorKind::UnexpectedEof,
        ];

        for kind in benign {
            let error = std::io::Error::from(kind);
            assert!(
                super::is_benign_broker_stdin_disconnect(&error),
                "expected {kind:?} to be treated as a benign broker stdin disconnect"
            );
        }
    }

    #[test]
    fn broker_stdin_disconnect_classifier_keeps_real_io_failures_fatal() {
        let fatal = [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::InvalidInput,
            std::io::ErrorKind::Other,
        ];

        for kind in fatal {
            let error = std::io::Error::from(kind);
            assert!(
                !super::is_benign_broker_stdin_disconnect(&error),
                "expected {kind:?} to remain a fatal broker stdin error"
            );
        }
    }

    #[test]
    fn broker_write_half_close_keeps_expected_socket_teardowns_nonfatal() {
        let benign = [
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::NotConnected,
            std::io::ErrorKind::UnexpectedEof,
        ];

        for kind in benign {
            assert!(
                super::finish_broker_socket_write_half(Err(std::io::Error::from(kind))).is_ok(),
                "expected {kind:?} to be treated as a benign broker write-half close"
            );
        }
    }

    #[test]
    fn broker_write_half_close_keeps_real_io_failures_fatal() {
        let fatal = [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::InvalidInput,
            std::io::ErrorKind::Other,
        ];

        for kind in fatal {
            let error = super::finish_broker_socket_write_half(Err(std::io::Error::from(kind)))
                .expect_err("fatal shutdown error must propagate");
            assert!(
                error
                    .to_string()
                    .contains("cannot close broker socket write half"),
                "expected shutdown context for {kind:?}: {error:#}"
            );
        }
    }
}
