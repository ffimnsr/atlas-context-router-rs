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
    };

    #[cfg(unix)]
    {
        run_stdio_broker(instance, options)
    }

    #[cfg(not(unix))]
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
    };

    #[cfg(unix)]
    {
        run_socket_daemon(instance, options)
    }

    #[cfg(not(unix))]
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

    let mut write_stream = stream
        .try_clone()
        .context("cannot clone broker socket stream")?;
    let stdin_thread = std::thread::spawn(move || -> Result<()> {
        let stdin = io::stdin();
        let mut input = stdin.lock();
        io::copy(&mut input, &mut write_stream).context("stdin relay failed")?;
        write_stream
            .shutdown(Shutdown::Write)
            .context("cannot close broker socket write half")?;
        Ok(())
    });

    let stdout = io::stdout();
    let mut output = stdout.lock();
    io::copy(&mut stream, &mut output).context("stdout relay failed")?;
    output.flush().context("cannot flush broker stdout")?;

    stdin_thread
        .join()
        .map_err(|_| anyhow::anyhow!("stdin relay thread panicked"))??;
    Ok(())
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

#[cfg(unix)]
#[derive(Debug, Serialize)]
struct DaemonHandshakeRequest {
    protocol_version: String,
    repo_root: String,
    db_path: String,
}

#[cfg(unix)]
#[derive(Debug, Deserialize)]
struct DaemonHandshakeResponse {
    ok: bool,
    error: Option<String>,
}

pub fn run_install(cli: &Cli) -> Result<()> {
    let (platform, scope, dry_run, validate_only, no_hooks, no_instructions) = match &cli.command {
        Command::Install {
            platform,
            scope,
            dry_run,
            validate_only,
            no_hooks,
            no_instructions,
        } => (
            platform.clone(),
            scope.clone(),
            *dry_run,
            *validate_only,
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
        dry_run,
        validate_only,
        no_hooks,
        no_instructions,
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
