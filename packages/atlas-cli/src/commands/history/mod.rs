use anyhow::Result;

use crate::cli::{Cli, Command, HistoryCommand};

mod context;
mod handlers;
mod render;

pub fn run_history(cli: &Cli) -> Result<()> {
    let sub = match &cli.command {
        Command::History { subcommand } => subcommand,
        _ => unreachable!(),
    };

    match sub {
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
    }
}
