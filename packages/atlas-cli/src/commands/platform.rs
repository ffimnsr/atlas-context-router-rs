use std::process::Stdio;

use anyhow::{Context, Result};
use atlas_mcp::ServerOptions;
use serde::{Deserialize, Serialize};

use crate::cli::{Cli, Command};

use super::{db_path, print_json, resolve_repo};

pub fn run_serve(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let instance = crate::mcp_instance::McpInstance::for_repo_and_db(&repo, &db_path)?;
    let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
    let options = ServerOptions {
        worker_threads: config.mcp_worker_threads(),
        tool_timeout_ms: config.mcp_tool_timeout_ms(),
        tool_timeout_ms_by_tool: config.mcp_tool_timeout_ms_by_tool(),
    };

    #[cfg(any(unix, windows))]
    {
        run_stdio_broker(instance, options)
    }

    #[cfg(not(any(unix, windows)))]
    {
        atlas_mcp::run_server_with_options(&repo, &db_path, options)
    }
}

pub fn run_serve_daemon(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let instance = crate::mcp_instance::McpInstance::for_repo_and_db(&repo, &db_path)?;
    let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
    let options = ServerOptions {
        worker_threads: config.mcp_worker_threads(),
        tool_timeout_ms: config.mcp_tool_timeout_ms(),
        tool_timeout_ms_by_tool: config.mcp_tool_timeout_ms_by_tool(),
    };

    #[cfg(any(unix, windows))]
    {
        run_socket_daemon(instance, options)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = instance;
        let _ = options;
        Err(anyhow::anyhow!(
            "repo-scoped MCP daemon transport is unsupported on this platform"
        ))
    }
}

#[cfg(unix)]
fn run_stdio_broker(
    instance: crate::mcp_instance::McpInstance,
    options: ServerOptions,
) -> Result<()> {
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
                    spawn_and_wait_for_daemon(&instance, options)?
                }
            }
        }
        crate::mcp_instance::McpInstanceStatus::Missing => {
            spawn_and_wait_for_daemon(&instance, options)?
        }
        crate::mcp_instance::McpInstanceStatus::Stale(stale) => {
            eprintln!(
                "atlas-mcp: cleaning stale daemon state for {} socket={} ({:?})",
                instance.instance_id,
                instance.socket_path.display(),
                stale.reasons
            );
            instance.clear_runtime_state()?;
            spawn_and_wait_for_daemon(&instance, options)?
        }
    };

    drop(coordination_lock);
    relay_stdio(stream)
}

#[cfg(unix)]
fn run_socket_daemon(
    instance: crate::mcp_instance::McpInstance,
    options: ServerOptions,
) -> Result<()> {
    instance.clear_runtime_state()?;
    let metadata = instance.default_metadata(
        std::process::id(),
        atlas_mcp::MCP_PROTOCOL_VERSION,
        &daemon_started_at(),
    );
    instance.write_metadata(&metadata)?;
    let cleanup = DaemonCleanup {
        instance: instance.clone(),
    };
    let result = atlas_mcp::run_socket_server_with_options(
        &instance.socket_path,
        &instance.repo_root,
        &instance.db_path,
        options,
    );
    drop(cleanup);
    result
}

#[cfg(unix)]
fn spawn_and_wait_for_daemon(
    instance: &crate::mcp_instance::McpInstance,
    options: ServerOptions,
) -> Result<std::os::unix::net::UnixStream> {
    eprintln!(
        "atlas-mcp: broker spawn socket={} repo={} db={}",
        instance.socket_path.display(),
        instance.repo_root,
        instance.db_path
    );
    spawn_daemon_process(instance, options)?;
    wait_for_daemon_ready(instance)
}

#[cfg(unix)]
fn spawn_daemon_process(
    instance: &crate::mcp_instance::McpInstance,
    _options: ServerOptions,
) -> Result<()> {
    let current_exe = std::env::current_exe().context("cannot resolve atlas binary path")?;
    std::process::Command::new(&current_exe)
        .args([
            "--repo",
            &instance.repo_root,
            "--db",
            &instance.db_path,
            "serve-daemon",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("cannot spawn daemon from {}", current_exe.display()))?;
    Ok(())
}

#[cfg(unix)]
fn wait_for_daemon_ready(
    instance: &crate::mcp_instance::McpInstance,
) -> Result<std::os::unix::net::UnixStream> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut last_error: Option<anyhow::Error> = None;

    while std::time::Instant::now() < deadline {
        match instance.read_metadata() {
            Ok(Some(metadata)) => match connect_to_daemon(
                &metadata.socket_path,
                &instance.repo_root,
                &instance.db_path,
            ) {
                Ok(stream) => return Ok(stream),
                Err(error) => last_error = Some(error),
            },
            Ok(None) => {}
            Err(error) => last_error = Some(error),
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
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

#[cfg(unix)]
fn relay_stdio(mut stream: std::os::unix::net::UnixStream) -> Result<()> {
    use std::io::{self, Write};
    use std::net::Shutdown;
    use std::os::fd::AsRawFd;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut write_stream = stream
        .try_clone()
        .context("cannot clone broker socket stream")?;
    let signal_stream = stream
        .try_clone()
        .context("cannot clone broker socket stream for shutdown")?;
    let shutdown = Arc::new(AtomicBool::new(false));
    let signal_shutdown = Arc::clone(&shutdown);
    let mut signals = signal_hook::iterator::Signals::new([
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGTERM,
    ])
    .context("cannot install broker shutdown signals")?;
    let signal_handle = signals.handle();
    let signal_thread = std::thread::Builder::new()
        .name("atlas-cli:broker-signal-handler".to_owned())
        .spawn(move || {
            if signals.forever().next().is_some() {
                signal_shutdown.store(true, Ordering::Relaxed);
                let _ = signal_stream.shutdown(Shutdown::Both);
                unsafe {
                    let _ = libc::close(libc::STDIN_FILENO);
                }
            }
        })
        .context("cannot spawn broker shutdown signal handler")?;
    let stdin_thread = std::thread::spawn(move || -> Result<()> {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        match io::copy(&mut input, &mut write_stream) {
            Ok(_) => {}
            Err(error) if is_benign_broker_stdin_disconnect(&error) => return Ok(()),
            Err(error) => return Err(error).context("stdin relay failed"),
        }
        write_stream
            .shutdown(Shutdown::Write)
            .context("cannot close broker socket write half")?;
        Ok(())
    });

    let stdout = io::stdout();
    let mut output = stdout.lock();
    match io::copy(&mut stream, &mut output) {
        Ok(_) => {}
        Err(_error) if shutdown.load(Ordering::Relaxed) => {}
        Err(error) => return Err(error).context("stdout relay failed"),
    }
    output.flush().context("cannot flush broker stdout")?;

    match stdin_thread.join() {
        Ok(Ok(())) => {}
        Ok(Err(error)) if shutdown.load(Ordering::Relaxed) => {
            let _ = stream.shutdown(Shutdown::Both);
            tracing::debug!(error = %error, fd = stream.as_raw_fd(), "broker stdin relay interrupted by shutdown signal");
        }
        Ok(Err(error)) => return Err(error),
        Err(_) => return Err(anyhow::anyhow!("stdin relay thread panicked")),
    }
    signal_handle.close();
    let _ = signal_thread.join();
    Ok(())
}

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
fn daemon_started_at() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{}Z", now.as_secs(), now.subsec_millis())
}

#[cfg(unix)]
struct DaemonCleanup {
    instance: crate::mcp_instance::McpInstance,
}

#[cfg(unix)]
impl Drop for DaemonCleanup {
    fn drop(&mut self) {
        let _ = self.instance.clear_runtime_state();
    }
}

// ──────────────────────────────────────────────────────────────────────────────
// Windows named-pipe broker
// ──────────────────────────────────────────────────────────────────────────────

#[cfg(windows)]
fn run_stdio_broker(
    instance: crate::mcp_instance::McpInstance,
    options: ServerOptions,
) -> Result<()> {
    let coordination_lock = instance.acquire_lock_blocking()?;
    let (reader, writer) = match instance.inspect_metadata()? {
        crate::mcp_instance::McpInstanceStatus::Ready(metadata) => {
            match win_connect_to_daemon(
                &metadata.socket_path,
                &instance.repo_root,
                &instance.db_path,
            ) {
                Ok(pair) => {
                    eprintln!(
                        "atlas-mcp: broker attach pipe={} repo={} db={}",
                        metadata.socket_path, instance.repo_root, instance.db_path
                    );
                    pair
                }
                Err(error) => {
                    eprintln!("atlas-mcp: stale daemon state detected; respawn: {error:#}");
                    instance.clear_runtime_state()?;
                    win_spawn_and_wait_for_daemon(&instance, options)?
                }
            }
        }
        crate::mcp_instance::McpInstanceStatus::Missing => {
            win_spawn_and_wait_for_daemon(&instance, options)?
        }
        crate::mcp_instance::McpInstanceStatus::Stale(stale) => {
            eprintln!(
                "atlas-mcp: cleaning stale daemon state for {} pipe={} ({:?})",
                instance.instance_id,
                instance.socket_path.display(),
                stale.reasons
            );
            instance.clear_runtime_state()?;
            win_spawn_and_wait_for_daemon(&instance, options)?
        }
    };
    drop(coordination_lock);
    win_relay_stdio(reader, writer)
}

#[cfg(windows)]
fn run_socket_daemon(
    instance: crate::mcp_instance::McpInstance,
    options: ServerOptions,
) -> Result<()> {
    instance.clear_runtime_state()?;
    let metadata = instance.default_metadata(
        std::process::id(),
        atlas_mcp::MCP_PROTOCOL_VERSION,
        &daemon_started_at(),
    );
    instance.write_metadata(&metadata)?;
    let cleanup = WinDaemonCleanup {
        instance: instance.clone(),
    };
    let result = atlas_mcp::run_socket_server_with_options(
        &instance.socket_path,
        &instance.repo_root,
        &instance.db_path,
        options,
    );
    drop(cleanup);
    result
}

#[cfg(windows)]
fn daemon_started_at() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{}Z", now.as_secs(), now.subsec_millis())
}

#[cfg(windows)]
struct WinDaemonCleanup {
    instance: crate::mcp_instance::McpInstance,
}

#[cfg(windows)]
impl Drop for WinDaemonCleanup {
    fn drop(&mut self) {
        let _ = self.instance.clear_runtime_state();
    }
}

#[cfg(windows)]
fn win_spawn_and_wait_for_daemon(
    instance: &crate::mcp_instance::McpInstance,
    options: ServerOptions,
) -> Result<(std::io::BufReader<std::fs::File>, std::fs::File)> {
    eprintln!(
        "atlas-mcp: broker spawn pipe={} repo={} db={}",
        instance.socket_path.display(),
        instance.repo_root,
        instance.db_path
    );
    spawn_daemon_process(instance, options)?;
    win_wait_for_daemon_ready(instance)
}

#[cfg(windows)]
fn win_wait_for_daemon_ready(
    instance: &crate::mcp_instance::McpInstance,
) -> Result<(std::io::BufReader<std::fs::File>, std::fs::File)> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut last_error: Option<anyhow::Error> = None;

    while std::time::Instant::now() < deadline {
        match instance.read_metadata() {
            Ok(Some(metadata)) => {
                match win_connect_to_daemon(
                    &metadata.socket_path,
                    &instance.repo_root,
                    &instance.db_path,
                ) {
                    Ok(pair) => return Ok(pair),
                    Err(error) => last_error = Some(error),
                }
            }
            Ok(None) => {}
            Err(error) => last_error = Some(error),
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }

    Err(anyhow::anyhow!(
        "daemon readiness handshake failed for repo={} db={}: {}",
        instance.repo_root,
        instance.db_path,
        last_error
            .map(|e| e.to_string())
            .unwrap_or_else(|| "daemon never became ready".to_owned())
    ))
}

/// Open the named pipe and perform the daemon handshake.  Returns a
/// `(BufReader, File)` pair so the caller can relay stdio without dropping
/// any data that the BufReader already consumed during the handshake.
#[cfg(windows)]
fn win_connect_to_daemon(
    pipe_path: &str,
    repo_root: &str,
    db_path: &str,
) -> Result<(std::io::BufReader<std::fs::File>, std::fs::File)> {
    use std::fs::OpenOptions;
    use std::io::{BufRead, BufReader, Write};

    let writer_file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(pipe_path)
        .with_context(|| format!("cannot connect to daemon pipe {pipe_path}"))?;
    let reader_file = writer_file
        .try_clone()
        .context("cannot clone pipe handle for reader")?;
    let mut reader = BufReader::new(reader_file);
    let mut writer = writer_file
        .try_clone()
        .context("cannot clone pipe handle for handshake writer")?;

    let request = DaemonHandshakeRequest {
        protocol_version: atlas_mcp::MCP_PROTOCOL_VERSION.to_owned(),
        repo_root: repo_root.to_owned(),
        db_path: db_path.to_owned(),
    };
    writeln!(writer, "{}", serde_json::to_string(&request)?)
        .context("cannot write daemon handshake")?;
    writer.flush().context("cannot flush daemon handshake")?;

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
        // Return the already-positioned reader so no buffered bytes are lost,
        // and the original writer_file for writing stdin data to the daemon.
        Ok((reader, writer_file))
    } else {
        Err(anyhow::anyhow!(
            response
                .error
                .unwrap_or_else(|| "daemon handshake rejected".to_owned())
        ))
    }
}

#[cfg(windows)]
fn win_relay_stdio(
    mut reader: std::io::BufReader<std::fs::File>,
    mut writer: std::fs::File,
) -> Result<()> {
    use std::io::{self, Write};

    let stdin_thread = std::thread::spawn(move || -> Result<()> {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        match io::copy(&mut input, &mut writer) {
            Ok(_) => Ok(()),
            Err(error) if win_is_benign_disconnect(&error) => Ok(()),
            Err(error) => Err(error).context("stdin relay failed"),
        }
    });

    let stdout = io::stdout();
    let mut output = stdout.lock();
    match io::copy(&mut reader, &mut output) {
        Ok(_) => {}
        Err(error) if win_is_benign_disconnect(&error) => {}
        Err(error) => return Err(error).context("stdout relay failed"),
    }
    output.flush().context("cannot flush broker stdout")?;

    match stdin_thread.join() {
        Ok(Ok(())) | Ok(Err(_)) => {}
        Err(_) => return Err(anyhow::anyhow!("stdin relay thread panicked")),
    }
    Ok(())
}

#[cfg(windows)]
fn win_is_benign_disconnect(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::UnexpectedEof
    )
}

#[cfg(any(unix, windows))]
#[derive(Debug, Serialize)]
struct DaemonHandshakeRequest {
    protocol_version: String,
    repo_root: String,
    db_path: String,
}

#[cfg(any(unix, windows))]
#[derive(Debug, Deserialize)]
struct DaemonHandshakeResponse {
    ok: bool,
    error: Option<String>,
}

pub fn run_install(cli: &Cli) -> Result<()> {
    let (platform, scope, dry_run, validate_only, force, no_hooks, no_instructions) =
        match &cli.command {
            Command::Install {
                platform,
                scope,
                dry_run,
                validate_only,
                force,
                no_hooks,
                no_instructions,
            } => (
                platform.clone(),
                scope.clone(),
                *dry_run,
                *validate_only,
                *force,
                *no_hooks,
                *no_instructions,
            ),
            _ => unreachable!(),
        };

    let repo = resolve_repo(cli)?;
    let repo_root = std::path::Path::new(&repo);

    if validate_only {
        println!("Validate only — no files will be written.\n");
    } else if dry_run {
        println!("Dry run — no files will be written.\n");
    }

    let summary = crate::install::run_install(
        repo_root,
        &platform,
        &scope,
        crate::install::InstallOptions {
            dry_run,
            validate_only,
            force,
            no_hooks,
            no_instructions,
        },
    )?;

    if cli.json {
        print_json(
            "install",
            serde_json::json!({
                "scope": summary.scope,
                "dry_run": dry_run,
                "validate_only": summary.validate_only,
                "configured": summary.configured,
                "already_configured": summary.already_configured,
                "hook_paths": summary.hook_paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                "instruction_files": summary.instruction_files,
                "platform_hook_files": summary.platform_hook_files,
                "validation_checks": summary.validation_checks,
            }),
        )?;
    } else {
        for name in &summary.configured {
            println!("  Configured : {name}");
        }
        for name in &summary.already_configured {
            println!("  Skipped    : {name} (already configured)");
        }
        for hook in &summary.hook_paths {
            println!("  Git hook   : {}", hook.display());
        }
        for f in &summary.platform_hook_files {
            println!("  Hook config: {f}");
        }
        for f in &summary.instruction_files {
            println!("  Instructions updated: {f}");
        }
        for check in &summary.validation_checks {
            let status = if check.ok { "ok" } else { "fail" };
            println!("  Validate   : {status} {}", check.detail);
        }

        let total = summary.configured.len() + summary.already_configured.len();
        if total == 0 {
            println!("No platforms detected. Use --platform to target one explicitly.");
        } else if !dry_run && !validate_only {
            println!("\nDone. Restart your AI coding tool to pick up the new config.");
            println!("Run `atlas build` to build the knowledge graph.");
        }
    }

    Ok(())
}

pub fn run_completions(cli: &Cli) -> Result<()> {
    use clap::CommandFactory;
    use clap_complete::generate;

    let shell = match &cli.command {
        Command::Completions { shell } => *shell,
        _ => unreachable!(),
    };

    let mut cmd = crate::cli::Cli::command();
    generate(shell, &mut cmd, "atlas", &mut std::io::stdout());
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
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

    #[cfg(unix)]
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
}

/// `atlas serve-http` — HTTP + SSE MCP transport.
///
/// Requires the `http-transport` crate feature.
#[cfg(feature = "http-transport")]
pub fn run_serve_http(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
    let options = atlas_mcp::ServerOptions {
        worker_threads: config.mcp_worker_threads(),
        tool_timeout_ms: config.mcp_tool_timeout_ms(),
        tool_timeout_ms_by_tool: config.mcp_tool_timeout_ms_by_tool(),
    };
    atlas_mcp::run_http_server_with_options(&repo, &db_path, options)
}

/// Stub shown when the binary was built without `--features http-transport`.
#[cfg(not(feature = "http-transport"))]
pub fn run_serve_http(_cli: &Cli) -> Result<()> {
    Err(anyhow::anyhow!(
        "HTTP transport is not compiled in. Rebuild with `--features http-transport`."
    ))
}
