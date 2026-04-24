use std::path::Path;

use anyhow::{Context, Result};
use atlas_history::git;
use atlas_parser::ParserRegistry;
use atlas_store_sqlite::Store;

use crate::cli::Cli;
use crate::commands::{db_path, resolve_repo};

pub(crate) struct HistoryContext {
    pub(crate) repo: String,
    pub(crate) db: String,
    pub(crate) canonical_root: String,
    pub(crate) store: Store,
}

impl HistoryContext {
    pub(crate) fn from_cli(cli: &Cli) -> Result<Self> {
        let repo = resolve_repo(cli)?;
        let db = db_path(cli, &repo);
        let canonical_root = std::fs::canonicalize(&repo)
            .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
            .to_string_lossy()
            .into_owned();
        let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;

        Ok(Self {
            repo,
            db,
            canonical_root,
            store,
        })
    }

    pub(crate) fn repo_path(&self) -> &Path {
        Path::new(&self.repo)
    }

    pub(crate) fn parser_registry(&self) -> ParserRegistry {
        ParserRegistry::with_defaults()
    }

    pub(crate) fn is_shallow(&self) -> bool {
        git::is_shallow(self.repo_path()).unwrap_or(false)
    }
}
