//! M4 — `atlas index rebuild`: regenerate derived retrieval indexes.

use anyhow::{Context, Result};

use atlas_contentstore::ContentStore;

use crate::cli::{Cli, Command, IndexCommand};

use super::{db_path, print_json, resolve_repo};

pub fn run_index(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let sub = match &cli.command {
        Command::Index { subcommand } => subcommand,
        _ => return Ok(()),
    };
    let IndexCommand::Rebuild { full, embeddings } = sub;

    let graph_db = db_path(cli, &repo);
    let content_db = atlas_engine::paths::content_db_path(&graph_db);
    let mut store = ContentStore::open(&content_db)
        .with_context(|| format!("cannot open content store at {content_db}"))?;
    let indexed = store.rebuild_from_markdown(*full)?;

    if cli.json {
        print_json(
            "index.rebuild",
            serde_json::json!({
                "repo_root": repo,
                "full": full,
                "indexed_artifacts": indexed,
                "embeddings": if *embeddings { "deferred" } else { "skipped" },
            }),
        )?;
        return Ok(());
    }
    println!(
        "Rebuilt retrieval index for {repo}: {indexed} artifact(s) {}",
        if *full { "(full rebuild)" } else { "" }
    );
    if *embeddings {
        println!("Embeddings are regenerated lazily by the embedding-enabled search path.");
    }
    Ok(())
}
