//! ICM-G6 — `atlas overview` command dispatch.

mod export;
mod external;
mod ingest;
mod read;

use std::path::Path;

use anyhow::{Context, Result};
use atlas_session::SessionStore;
use atlas_store_sqlite::Store;

use crate::cli::{Cli, Command, OverviewCommand};

use super::{db_path, resolve_repo};

pub fn run_overview(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let sub = match &cli.command {
        Command::Overview { subcommand } => subcommand,
        _ => unreachable!(),
    };

    match sub {
        OverviewCommand::Export { .. } => export::run_export(cli, &repo, &db_path),
        OverviewCommand::Ingest { .. } => ingest::run_ingest(cli, &repo, &db_path),
        OverviewCommand::Show { .. }
        | OverviewCommand::Search { .. }
        | OverviewCommand::List { .. }
        | OverviewCommand::Stale { .. }
        | OverviewCommand::RefreshPlan { .. }
        | OverviewCommand::Delete { .. }
        | OverviewCommand::ExportDocs { .. } => read::run_read(cli, &repo, &db_path),
        OverviewCommand::RunExternal { .. } => external::run_external(cli, &repo, &db_path),
    }
}

pub(crate) fn open_graph_store(db_path: &str) -> Result<Store> {
    Store::open(db_path).with_context(|| format!("cannot open database at {db_path}"))
}

pub(crate) fn open_session_store(db_path: &str) -> Result<SessionStore> {
    let session_db = atlas_engine::paths::session_db_path(db_path);
    if let Some(parent) = Path::new(&session_db).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    SessionStore::open(&session_db)
        .with_context(|| format!("cannot open session store at {session_db}"))
}

pub(crate) use super::print_json;
