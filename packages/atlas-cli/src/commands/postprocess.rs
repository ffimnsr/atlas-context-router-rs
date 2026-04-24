use anyhow::{Context, Result};
use atlas_engine::{PostprocessOptions, postprocess_graph};
use atlas_repo::find_repo_root;
use camino::Utf8Path;

use crate::cli::{Cli, Command};

use super::{db_path, print_json, resolve_repo};

pub fn run_postprocess(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let repo_root = find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let db_path = db_path(cli, &repo);
    let (changed_only, stage, dry_run) = match &cli.command {
        Command::Postprocess {
            changed_only,
            stage,
            dry_run,
        } => (*changed_only, stage.clone(), *dry_run),
        _ => unreachable!(),
    };

    let summary = postprocess_graph(
        repo_root.as_path(),
        &db_path,
        &PostprocessOptions {
            changed_only,
            stage,
            dry_run,
        },
    )?;

    if cli.json {
        print_json("postprocess", serde_json::to_value(&summary)?)?;
        return Ok(());
    }

    if summary.noop {
        println!("Postprocess skipped: {}", summary.message);
        return Ok(());
    }

    if !summary.ok {
        anyhow::bail!(summary.message);
    }

    println!(
        "Postprocess complete ({:?}, {} ms)",
        summary.requested_mode, summary.total_elapsed_ms
    );
    for stage in &summary.stages {
        println!(
            "  {:<26} {:<10} items={} files={} elapsed={}ms",
            stage.stage,
            stage.status.as_str(),
            stage.item_count,
            stage.affected_file_count,
            stage.elapsed_ms
        );
    }
    Ok(())
}
