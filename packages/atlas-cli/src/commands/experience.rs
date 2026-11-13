//! `atlas experience` command dispatch (VR4).

use anyhow::{Context, Result, bail};
use atlas_session::{ExperienceDirection, SessionStore};
use serde_json::json;

use crate::cli::{Cli, Command, ExperienceCommand};

use super::{db_path, print_json, resolve_repo};

pub fn run_experience(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let session_db = atlas_engine::paths::session_db_path(&db_path);
    if let Some(parent) = std::path::Path::new(&session_db).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let mut store = SessionStore::open(&session_db)
        .with_context(|| format!("cannot open session store at {session_db}"))?;

    // `atlas experience <uri>` is shorthand for `experience inspect <uri>`.
    let (uri, direction, limit) = match &cli.command {
        Command::Experience {
            uri: Some(uri),
            subcommand: None,
        } => (uri.clone(), "both".to_owned(), 20),
        Command::Experience {
            subcommand:
                Some(ExperienceCommand::Inspect {
                    uri,
                    direction,
                    limit,
                }),
            ..
        } => (uri.clone(), direction.clone(), *limit),
        Command::Experience {
            subcommand:
                Some(ExperienceCommand::Prune {
                    older_than,
                    dry_run,
                }),
            ..
        } => return run_prune(cli, &mut store, &repo, *older_than, *dry_run),
        _ => bail!("provide a URI or a subcommand: inspect|prune"),
    };

    run_inspect(cli, &mut store, &repo, &uri, &direction, limit)
}

fn run_inspect(
    cli: &Cli,
    store: &mut SessionStore,
    repo: &str,
    uri: &str,
    direction: &str,
    limit: usize,
) -> Result<()> {
    let direction: ExperienceDirection = direction.parse()?;
    let edges = store.inspect_experience_edges(repo, uri, direction, limit)?;
    let edge_count = edges.len();
    if cli.json {
        print_json(
            "experience.inspect",
            json!({
                "tool": "experience_inspect",
                "repo_root": repo,
                "uri": uri,
                "direction": direction,
                "edges": edges,
                "summary": {
                    "edge_count": edge_count,
                    "limit": limit,
                },
                "warnings": [],
            }),
        )?;
    } else {
        println!("Experience edges for {uri}: {}", edges.len());
        for edge in &edges {
            println!(
                "  {} -> {} · {} (confidence {:.2}, uses {}, {})",
                edge.source_uri,
                edge.target_uri,
                edge.query_text,
                edge.confidence,
                edge.use_count,
                edge.freshness.as_str()
            );
        }
    }
    Ok(())
}

fn run_prune(
    cli: &Cli,
    store: &mut SessionStore,
    repo: &str,
    older_than: u64,
    dry_run: bool,
) -> Result<()> {
    let matched = store.prune_experience_edges(repo, older_than, dry_run)?;
    if cli.json {
        print_json(
            "experience.prune",
            json!({
                "repo_root": repo,
                "older_than_days": older_than,
                "dry_run": dry_run,
                "matched": matched,
                "deleted": if dry_run { 0 } else { matched },
            }),
        )?;
    } else if dry_run {
        println!("Would prune {matched} experience edge(s).");
    } else {
        println!("Pruned {matched} experience edge(s).");
    }
    Ok(())
}
