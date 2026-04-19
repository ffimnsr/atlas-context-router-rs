mod cli;
mod commands;
mod logging;
mod paths;

use clap::Parser;
use cli::{Cli, Command};

fn main() {
    let cli = Cli::parse();
    logging::init(cli.verbose);

    let result = match &cli.command {
        Command::Init => commands::run_init(&cli),
        Command::Build { .. } => commands::run_build(&cli),
        Command::Update { .. } => commands::run_update(&cli),
        Command::Status => commands::run_status(&cli),
        Command::DetectChanges { .. } => commands::run_detect_changes(&cli),
        Command::Query { .. } => commands::run_query(&cli),
        Command::Impact { .. } => commands::run_impact(&cli),
        Command::ReviewContext { .. } => commands::run_review_context(&cli),
        Command::Serve => commands::run_serve(&cli),
        Command::DbCheck => commands::run_db_check(&cli),
    };

    if let Err(err) = result {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
