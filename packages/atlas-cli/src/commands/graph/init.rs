use std::fs;

use crate::cli::{Cli, Command};
use anyhow::{Context, Result};
use atlas_contentstore::ContentStore;
use atlas_session::SessionStore;
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use tracing::debug;

use super::super::{db_path, print_json, resolve_repo};

pub fn run_init(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    debug!(repo_root = %repo, "init: resolved repo root");
    let atlas_dir = atlas_engine::paths::atlas_dir(&repo);
    fs::create_dir_all(&atlas_dir)
        .with_context(|| format!("cannot create {}", atlas_dir.display()))?;
    debug!(atlas_dir = %atlas_dir.display(), "init: ensured atlas directory");

    let db_path = db_path(cli, &repo);
    Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    debug!(db_path = %db_path, "init: opened graph database");

    let content_db_path = atlas_engine::paths::content_db_path(&db_path);
    let mut content_store = ContentStore::open(&content_db_path)
        .with_context(|| format!("cannot open content store at {content_db_path}"))?;
    content_store
        .migrate()
        .with_context(|| format!("cannot migrate content store at {content_db_path}"))?;
    debug!(content_db_path = %content_db_path, "init: opened content store");

    let session_db_path = atlas_engine::paths::session_db_path(&db_path);
    SessionStore::open(&session_db_path)
        .with_context(|| format!("cannot open session store at {session_db_path}"))?;
    debug!(session_db_path = %session_db_path, "init: opened session store");

    let config_path = atlas_engine::paths::config_path(&repo);
    let profile = match &cli.command {
        Command::Init { profile } => match profile.as_str() {
            "minimal" => atlas_engine::config::ConfigTemplateProfile::Minimal,
            "standard" => atlas_engine::config::ConfigTemplateProfile::Standard,
            "full" => atlas_engine::config::ConfigTemplateProfile::Full,
            other => anyhow::bail!("unsupported init profile: {other}"),
        },
        _ => unreachable!(),
    };
    let config_created = atlas_engine::Config::write_template(&atlas_dir, profile)
        .with_context(|| format!("cannot write config to {}", config_path.display()))?;
    debug!(config_path = %config_path.display(), config_created, profile = profile.as_str(), "init: prepared config template");

    let repo_registry = super::super::repo::bootstrap_and_save_registry(Utf8Path::new(&repo))
        .context("cannot bootstrap repo registry")?;
    let repo_registry_path = atlas_repo::registry_path(Utf8Path::new(&repo));
    if let Ok(mut store) = Store::open(&db_path) {
        atlas_engine::refresh_repo_registry_graph(&mut store, &repo_registry)
            .context("cannot refresh synthetic repo registry graph")?;
    }
    debug!(registry_path = %repo_registry_path, registrations = repo_registry.registrations.len(), "init: prepared repo registry");

    if cli.json {
        print_json(
            "init",
            serde_json::json!({
                "atlas_dir": atlas_dir.display().to_string(),
                "db_path": db_path,
                "content_db_path": content_db_path,
                "session_db_path": session_db_path,
                "config_path": config_path.display().to_string(),
                "config_created": config_created,
                "config_profile": profile.as_str(),
                "repo_registry_path": repo_registry_path.to_string(),
                "repo_registrations": repo_registry.registrations.len(),
                "repo_registry_warnings": repo_registry.warnings,
            }),
        )?;
    } else if super::super::init_wizard::should_run(cli.json) {
        let repo_root = std::path::Path::new(&repo);
        super::super::init_wizard::run(repo_root)?;
    } else {
        println!("Initialized atlas in {}", atlas_dir.display());
        println!("Database: {db_path}");
        println!("Content : {content_db_path}");
        println!("Session : {session_db_path}");
        println!("Registry: {repo_registry_path}");
        if config_created {
            println!("Config  : {} ({})", config_path.display(), profile.as_str());
        }
    }
    Ok(())
}
