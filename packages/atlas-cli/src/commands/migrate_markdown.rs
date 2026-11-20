//! M1 — `atlas migrate markdown`: export continuity truth to markdown.

use anyhow::{Context, Result};

use atlas_session::{MemoryListFilter, SessionStore, SessionStoreConfig};

use crate::cli::{Cli, Command, MigrateCommand};

use super::{print_json, resolve_repo};

/// Run the markdown continuity migration.
pub fn run_migrate_markdown(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let sub = match &cli.command {
        Command::Migrate {
            subcommand:
                Some(MigrateCommand::Markdown {
                    dry_run,
                    finalize,
                    embeddings,
                }),
        } => (*dry_run, *finalize, *embeddings),
        _ => return Ok(()),
    };
    let (dry_run, finalize, embeddings) = sub;

    let session_db = atlas_engine::paths::session_db_path(&super::db_path(cli, &repo));
    let mut store = SessionStore::open_with_config(
        &session_db,
        SessionStoreConfig {
            markdown_reconcile_on_open: false,
            ..Default::default()
        },
    )
    .with_context(|| format!("cannot open session store at {session_db}"))?;
    let report = store.migrate_markdown(dry_run, finalize)?;

    if embeddings && !dry_run && !finalize {
        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
        super::memory_embed::run_embed(
            &repo,
            &mut store,
            &config,
            &MemoryListFilter::default(),
            500,
            false,
            cli.json,
        )?;
    }

    if cli.json {
        print_json(
            "migrate.markdown",
            serde_json::json!({
                "repo_root": repo,
                "dry_run": report.dry_run,
                "written": report.written,
                "written_count": report.written.len(),
                "pending": report.pending,
                "backup": report.backup,
                "records_written": report.records_written,
            }),
        )?;
        return Ok(());
    }

    if report.dry_run {
        println!(
            "Would export {} memory file(s) to markdown ({} still pending)",
            report.written.len(),
            report.pending
        );
        for path in &report.written {
            println!("  {path}");
        }
        return Ok(());
    }
    if let Some(backup) = &report.backup {
        if report.written.is_empty() {
            println!("Removed migration backup: {backup}");
        } else {
            println!("Migration backup: {backup}");
        }
    }
    if report.written.is_empty() {
        println!(
            "No pending memory exports; synced {} durable markdown record(s).",
            report.records_written
        );
    } else {
        println!(
            "Exported {} memory file(s) to markdown ({} still pending)",
            report.written.len(),
            report.pending
        );
        for path in &report.written {
            println!("  {path}");
        }
    }
    Ok(())
}
