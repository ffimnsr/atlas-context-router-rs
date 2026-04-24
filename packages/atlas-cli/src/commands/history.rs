use anyhow::{Context, Result};
use atlas_history::git;
use atlas_history::status::HistoryStatus;
use atlas_history::{CommitSelector, build_historical_graph};
use atlas_parser::ParserRegistry;
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
        HistoryCommand::Build { .. } => run_history_build(cli),
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

fn run_history_build(cli: &Cli) -> Result<()> {
    let sub = match &cli.command {
        Command::History { subcommand } => subcommand,
        _ => unreachable!(),
    };

    let (since, until, max_commits, branch, commits) = match sub {
        HistoryCommand::Build {
            since,
            until,
            max_commits,
            branch,
            commits,
        } => (since, until, max_commits, branch, commits),
        _ => unreachable!(),
    };

    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);
    let repo_path = std::path::Path::new(&repo);

    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();

    // Build commit selector from flags.
    let selector = if !commits.is_empty() {
        CommitSelector::Explicit {
            shas: commits.clone(),
        }
    } else {
        let start_ref = branch.clone().unwrap_or_else(|| "HEAD".to_owned());
        CommitSelector::Bounded {
            start_ref,
            max_commits: *max_commits,
            since: since.clone(),
            until: until.clone(),
        }
    };

    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;
    let registry = ParserRegistry::with_defaults();

    let summary = build_historical_graph(repo_path, &canonical_root, &store, &selector, &registry)
        .context("build historical graph")?;

    // Print errors as warnings.
    for e in &summary.errors {
        eprintln!("warning: {e}");
    }

    if cli.json {
        let val = serde_json::json!({
            "commits_processed": summary.commits_processed,
            "commits_already_indexed": summary.commits_already_indexed,
            "files_reused": summary.files_reused,
            "files_parsed": summary.files_parsed,
            "files_skipped": summary.files_skipped,
            "nodes_reused": summary.nodes_reused,
            "nodes_written": summary.nodes_written,
            "edges_written": summary.edges_written,
            "errors": summary.errors.len(),
            "elapsed_secs": summary.elapsed_secs,
        });
        print_json("history_build", val)?;
    } else {
        println!("commits processed : {}", summary.commits_processed);
        println!("commits skipped   : {}", summary.commits_already_indexed);
        println!("files parsed      : {}", summary.files_parsed);
        println!("files reused      : {}", summary.files_reused);
        println!("files skipped     : {}", summary.files_skipped);
        println!("nodes written     : {}", summary.nodes_written);
        println!("nodes reused      : {}", summary.nodes_reused);
        println!("edges written     : {}", summary.edges_written);
        println!("elapsed           : {:.2}s", summary.elapsed_secs);
        if !summary.errors.is_empty() {
            eprintln!("errors            : {}", summary.errors.len());
        }
    }

    Ok(())
}
