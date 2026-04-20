mod cli;
mod commands;
mod install;
mod logging;

use clap::Parser;
use cli::{Cli, Command};

fn main() {
    let cli = Cli::parse();
    logging::init(cli.verbose);

    let result = match &cli.command {
        Command::Init => commands::run_init(&cli),
        Command::Build { .. } => commands::run_build(&cli),
        Command::Update { .. } => commands::run_update(&cli),
        Command::Status { .. } => commands::run_status(&cli),
        Command::DetectChanges { .. } => commands::run_detect_changes(&cli),
        Command::Query { .. } => commands::run_query(&cli),
        Command::Embed { .. } => commands::run_embed(&cli),
        Command::Impact { .. } => commands::run_impact(&cli),
        Command::ReviewContext { .. } => commands::run_review_context(&cli),
        Command::Serve => commands::run_serve(&cli),
        Command::DbCheck => commands::run_db_check(&cli),
        Command::Doctor => commands::run_doctor(&cli),
        Command::DebugGraph { .. } => commands::run_debug_graph(&cli),
        Command::ExplainQuery { .. } => commands::run_explain_query(&cli),
        Command::ExplainChange { .. } => commands::run_explain_change(&cli),
        Command::Install { .. } => commands::run_install(&cli),
        Command::Completions { .. } => commands::run_completions(&cli),
        Command::Shell { .. } => commands::run_shell(&cli),
        Command::Watch { .. } => commands::run_watch(&cli),
        Command::Context { .. } => commands::run_context(&cli),
        Command::Analyze { .. } => commands::run_analyze(&cli),
        Command::Refactor { .. } => commands::run_refactor(&cli),
        Command::Flows { .. } => commands::run_flows(&cli),
        Command::Communities { .. } => commands::run_communities(&cli),
        Command::Session { .. } => commands::run_session(&cli),
    };

    if let Err(err) = result {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
