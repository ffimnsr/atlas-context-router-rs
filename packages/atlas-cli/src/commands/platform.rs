use anyhow::Result;
use atlas_mcp::ServerOptions;

use crate::cli::{Cli, Command};

use super::{db_path, print_json, resolve_repo};

/// Delegate to the `atlas-mcp` crate's stdin/stdout server.
pub fn run_serve(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
    atlas_mcp::run_server_with_options(
        &repo,
        &db_path,
        ServerOptions {
            worker_threads: config.mcp_worker_threads(),
            tool_timeout_ms: config.mcp_tool_timeout_ms(),
        },
    )
}

pub fn run_install(cli: &Cli) -> Result<()> {
    let (platform, dry_run, no_hooks, no_instructions) = match &cli.command {
        Command::Install {
            platform,
            dry_run,
            no_hooks,
            no_instructions,
        } => (platform.clone(), *dry_run, *no_hooks, *no_instructions),
        _ => unreachable!(),
    };

    let repo = resolve_repo(cli)?;
    let repo_root = std::path::Path::new(&repo);

    if dry_run {
        println!("Dry run — no files will be written.\n");
    }

    let summary =
        crate::install::run_install(repo_root, &platform, dry_run, no_hooks, no_instructions)?;

    if cli.json {
        print_json(
            "install",
            serde_json::json!({
                "dry_run": dry_run,
                "configured": summary.configured,
                "already_configured": summary.already_configured,
                "hook_paths": summary.hook_paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                "instruction_files": summary.instruction_files,
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
        for f in &summary.instruction_files {
            println!("  Instructions updated: {f}");
        }

        let total = summary.configured.len() + summary.already_configured.len();
        if total == 0 {
            println!("No platforms detected. Use --platform to target one explicitly.");
        } else if !dry_run {
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
