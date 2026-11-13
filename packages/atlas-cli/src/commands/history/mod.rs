use anyhow::{Context, Result};

use crate::cli::{Cli, Command, HistoryCommand};
use crate::commands::print_json;

mod context;
mod handlers;
mod render;

use context::HistoryContext;

pub fn run_history(cli: &Cli) -> Result<()> {
    let (sub, shorthand) = match &cli.command {
        Command::History {
            subcommand,
            path,
            symbol,
            follow_renames,
            stat_only,
            full,
        } => (
            subcommand.as_ref(),
            path.as_deref()
                .map(|path| (path, symbol.as_deref(), *follow_renames, *stat_only, *full)),
        ),
        _ => unreachable!(),
    };

    if let Some(sub) = sub {
        return match sub {
            HistoryCommand::Status => handlers::run_history_status(cli),
            HistoryCommand::Build { .. } => handlers::run_history_build(cli),
            HistoryCommand::Update { .. } => handlers::run_history_update(cli),
            HistoryCommand::Rebuild { .. } => handlers::run_history_rebuild(cli),
            HistoryCommand::Diff { .. } => handlers::run_history_diff(cli),
            HistoryCommand::Symbol { .. } => handlers::run_history_symbol(cli),
            HistoryCommand::File { .. } => handlers::run_history_file(cli),
            HistoryCommand::Dependency { .. } => handlers::run_history_dependency(cli),
            HistoryCommand::Module { .. } => handlers::run_history_module(cli),
            HistoryCommand::Churn { .. } => handlers::run_history_churn(cli),
            HistoryCommand::Prune { .. } => handlers::run_history_prune(cli),
        };
    }

    let Some((path, symbol, follow_renames, stat_only, _full)) = shorthand else {
        anyhow::bail!("history requires a subcommand or a file path; run `atlas history --help`");
    };
    run_history_shorthand(cli, path, symbol, follow_renames, stat_only)
}

/// Guide-compatible shorthand: `atlas history <file> [--symbol <name>]`.
fn run_history_shorthand(
    cli: &Cli,
    path: &str,
    symbol: Option<&str>,
    follow_renames: bool,
    stat_only: bool,
) -> Result<()> {
    let ctx = HistoryContext::from_cli(cli)?;
    let status = ctx
        .store
        .history_status(&ctx.canonical_root)
        .context("query history status")?;
    if status.indexed_commit_count == 0 {
        anyhow::bail!(
            "history index is empty for this repo; run `atlas history build` before querying history"
        );
    }

    if let Some(symbol) = symbol {
        let resolution = atlas_search::resolve_symbol_nodes(
            &ctx.store,
            &atlas_search::SymbolResolveRequest {
                name: symbol,
                kind: None,
                language: None,
                file: Some(path),
                limit: 5,
                fuzzy: false,
            },
        )
        .context("symbol resolution failed")?;
        let qualified_name = resolution
            .matches
            .first()
            .map(|node| node.qualified_name.clone())
            .ok_or_else(|| {
                anyhow::anyhow!("symbol '{symbol}' not found in history for file '{path}'")
            })?;
        let report =
            atlas_history::query_symbol_history(&ctx.store, &ctx.canonical_root, &qualified_name)
                .context("query symbol history")?;
        if cli.json {
            print_json(
                "history_symbol",
                serde_json::to_value(if stat_only {
                    serde_json::json!({
                        "file": path,
                        "qualified_name": qualified_name,
                        "summary": report.summary,
                    })
                } else {
                    serde_json::json!({
                        "file": path,
                        "report": report,
                    })
                })?,
            )?;
        } else {
            println!("symbol            : {qualified_name}");
            println!("file              : {path}");
            println!(
                "change commits    : {}",
                report.findings.commits_where_changed.len()
            );
        }
        return Ok(());
    }

    let report = atlas_history::query_file_history_with_options(
        &ctx.store,
        &ctx.canonical_root,
        ctx.repo_path(),
        path,
        follow_renames,
    )
    .context("query file history")?;
    if cli.json {
        print_json(
            "history_file",
            serde_json::to_value(if stat_only {
                serde_json::json!({ "summary": report.summary })
            } else {
                serde_json::to_value(&report)?
            })?,
        )?;
    } else {
        println!("file              : {}", report.file_path);
        println!(
            "commits touched   : {}",
            report.findings.commits_touched.len()
        );
    }
    Ok(())
}
