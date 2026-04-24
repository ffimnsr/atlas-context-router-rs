use anyhow::{Context, Result};
use atlas_history::git;
use atlas_history::status::HistoryStatus;
use atlas_store_sqlite::Store;

use crate::cli::{Cli, Command, HistoryCommand};

use super::{db_path, print_json, resolve_repo};

pub fn run_history(cli: &Cli) -> Result<()> {
    let sub = match &cli.command {
        Command::History { subcommand } => subcommand,
        _ => unreachable!(),
    };

    match sub {
        HistoryCommand::Status => run_history_status(cli),
    }
}

fn run_history_status(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);

    // Canonicalize the repo root before using it as a storage key.
    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();

    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;

    let summary = store
        .history_status(&canonical_root)
        .context("query history status")?;

    let is_shallow = git::is_shallow(std::path::Path::new(&repo)).unwrap_or(false);

    let status = HistoryStatus::from_summary(summary, is_shallow);

    if cli.json {
        print_json("history_status", serde_json::to_value(&status)?)?;
    } else {
        println!("indexed commits : {}", status.indexed_commit_count);
        println!("snapshots       : {}", status.snapshot_count);
        if let Some(sha) = &status.latest_commit_sha {
            let subject = status
                .latest_commit_subject
                .as_deref()
                .unwrap_or("(no subject)");
            let short = &sha[..sha.len().min(12)];
            println!("latest commit   : {short} {subject}");
        } else {
            println!("latest commit   : (none)");
        }
        for w in &status.warnings {
            eprintln!("warning: {w}");
        }
    }

    Ok(())
}
