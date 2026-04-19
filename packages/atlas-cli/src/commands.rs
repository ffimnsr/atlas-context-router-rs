use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use atlas_store_sqlite::Store;

use crate::cli::Cli;

pub fn run_init(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let atlas_dir = Path::new(&repo).join(".atlas");
    fs::create_dir_all(&atlas_dir)
        .with_context(|| format!("cannot create {}", atlas_dir.display()))?;

    let db_path = db_path(cli, &repo);
    Store::open(&db_path)
        .with_context(|| format!("cannot open database at {db_path}"))?;

    println!("Initialized atlas in {}", atlas_dir.display());
    println!("Database: {db_path}");
    Ok(())
}

pub fn run_status(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);

    let store = Store::open(&db_path)
        .with_context(|| format!("cannot open database at {db_path}"))?;
    let stats = store.stats().context("cannot read stats")?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&stats)?);
    } else {
        println!("Repo root : {repo}");
        println!("Database  : {db_path}");
        println!("Files     : {}", stats.file_count);
        println!("Nodes     : {}", stats.node_count);
        println!("Edges     : {}", stats.edge_count);
        if !stats.languages.is_empty() {
            println!("Languages : {}", stats.languages.join(", "));
        }
        if !stats.nodes_by_kind.is_empty() {
            println!("Nodes by kind:");
            for (kind, count) in &stats.nodes_by_kind {
                println!("  {kind:<14} {count}");
            }
        }
        if let Some(ts) = &stats.last_indexed_at {
            println!("Last indexed: {ts}");
        }
    }
    Ok(())
}

pub fn run_build(_cli: &Cli) -> Result<()> {
    println!("atlas build: not yet implemented (Slice 5)");
    Ok(())
}

pub fn run_update(_cli: &Cli) -> Result<()> {
    println!("atlas update: not yet implemented (Slice 5)");
    Ok(())
}

pub fn run_detect_changes(_cli: &Cli) -> Result<()> {
    println!("atlas detect-changes: not yet implemented (Slice 3)");
    Ok(())
}

pub fn run_query(_cli: &Cli) -> Result<()> {
    println!("atlas query: not yet implemented (Slice 6)");
    Ok(())
}

pub fn run_impact(_cli: &Cli) -> Result<()> {
    println!("atlas impact: not yet implemented (Slice 6)");
    Ok(())
}

pub fn run_review_context(_cli: &Cli) -> Result<()> {
    println!("atlas review-context: not yet implemented (Slice 6)");
    Ok(())
}

// --- helpers -----------------------------------------------------------------

fn resolve_repo(cli: &Cli) -> Result<String> {
    if let Some(r) = &cli.repo {
        return Ok(r.clone());
    }
    // Fallback: use cwd for now; Slice 3 will replace with git-based detection.
    Ok(std::env::current_dir()
        .context("cannot determine cwd")?
        .to_string_lossy()
        .into_owned())
}

fn db_path(cli: &Cli, repo: &str) -> String {
    if let Some(p) = &cli.db {
        return p.clone();
    }
    Path::new(repo)
        .join(".atlas")
        .join("worldview.sqlite")
        .to_string_lossy()
        .into_owned()
}
