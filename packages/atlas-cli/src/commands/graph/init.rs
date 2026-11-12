use std::fs;
use std::path::Path;

use crate::cli::{Cli, Command, InitMode};
use crate::install::{apply_init_mode, plan_init_mode};
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
    let db_path = db_path(cli, &repo);
    let content_db_path = atlas_engine::paths::content_db_path(&db_path);
    let session_db_path = atlas_engine::paths::session_db_path(&db_path);
    let config_path = atlas_engine::paths::config_path(&repo);

    let (profile_label, mode, dry_run, force) = match &cli.command {
        Command::Init {
            profile,
            mode,
            dry_run,
            force,
        } => (profile.as_str(), *mode, *dry_run, *force),
        _ => unreachable!(),
    };

    // Dry run is strictly read-only: no directories, databases, or config.
    if dry_run {
        return report_init_plan(
            cli,
            &repo,
            &atlas_dir,
            &db_path,
            &content_db_path,
            &session_db_path,
            &config_path,
            profile_label,
            mode,
        );
    }

    fs::create_dir_all(&atlas_dir)
        .with_context(|| format!("cannot create {}", atlas_dir.display()))?;
    debug!(atlas_dir = %atlas_dir.display(), "init: ensured atlas directory");

    Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    debug!(db_path = %db_path, "init: opened graph database");

    let mut content_store = ContentStore::open(&content_db_path)
        .with_context(|| format!("cannot open content store at {content_db_path}"))?;
    content_store
        .migrate()
        .with_context(|| format!("cannot migrate content store at {content_db_path}"))?;
    debug!(content_db_path = %content_db_path, "init: opened content store");

    SessionStore::open(&session_db_path)
        .with_context(|| format!("cannot open session store at {session_db_path}"))?;
    debug!(session_db_path = %session_db_path, "init: opened session store");

    // Tuning inputs are collected only when a config will actually be written
    // so re-runs stay cheap and never fail on probe errors (e.g. non-git dirs
    // with an existing config).
    let config_missing = !config_path.exists();
    let (config_created, mut tuning) = match profile_label {
        "auto" if config_missing => {
            let system = atlas_engine::config::probe_system();
            let repo_estimate = atlas_engine::config::probe_repo(Path::new(&repo))
                .with_context(|| format!("cannot estimate repo size for {repo}"))?;
            let eff_cores = system.physical_cores.clamp(1, 8);
            let wall_est_s =
                atlas_engine::config::estimate_build_wall_seconds(repo_estimate.files, eff_cores);
            let created =
                atlas_engine::Config::write_auto_template(&atlas_dir, &system, &repo_estimate)
                    .with_context(|| format!("cannot write config to {}", config_path.display()))?;
            let tuning = serde_json::json!({
                "logical_cores": system.logical_cores,
                "physical_cores": system.physical_cores,
                "ram_total_mib": system.ram_total_bytes / (1024 * 1024),
                "tracked_files": repo_estimate.files,
                "tracked_bytes": repo_estimate.bytes,
                "est_build_seconds": (wall_est_s * 10.0).round() as u64 / 10,
            });
            (created, if created { Some(tuning) } else { None })
        }
        "auto" => (false, None),
        other => {
            let profile = match other {
                "minimal" => atlas_engine::config::ConfigTemplateProfile::Minimal,
                "standard" => atlas_engine::config::ConfigTemplateProfile::Standard,
                "full" => atlas_engine::config::ConfigTemplateProfile::Full,
                unsupported => anyhow::bail!("unsupported init profile: {unsupported}"),
            };
            let created = atlas_engine::Config::write_template(&atlas_dir, profile)
                .with_context(|| format!("cannot write config to {}", config_path.display()))?;
            (created, None)
        }
    };
    let profile = profile_label.to_owned();
    debug!(config_path = %config_path.display(), config_created, profile, "init: prepared config template");

    let repo_registry = super::super::repo::bootstrap_and_save_registry(Utf8Path::new(&repo))
        .context("cannot bootstrap repo registry")?;
    let repo_registry_path = atlas_repo::registry_path(Utf8Path::new(&repo));
    if let Ok(mut store) = Store::open(&db_path) {
        atlas_engine::refresh_repo_registry_graph(&mut store, &repo_registry)
            .context("cannot refresh synthetic repo registry graph")?;
    }
    debug!(registry_path = %repo_registry_path, registrations = repo_registry.registrations.len(), "init: prepared repo registry");

    // ICM-H1: shell-first assets (`mcp`, `hook`, `cli`, `all`) are idempotent.
    let install_summary = apply_init_mode(Path::new(&repo), mode, false, force)?;
    debug!(
        mode = install_summary.mode,
        files = install_summary.files.len(),
        "init: applied install mode"
    );

    if cli.json {
        let mut payload = serde_json::json!({
            "atlas_dir": atlas_dir.display().to_string(),
            "db_path": db_path,
            "content_db_path": content_db_path,
            "session_db_path": session_db_path,
            "config_path": config_path.display().to_string(),
            "config_created": config_created,
            "config_profile": profile,
            "repo_registry_path": repo_registry_path.to_string(),
            "repo_registrations": repo_registry.registrations.len(),
            "repo_registry_warnings": repo_registry.warnings,
            "mode": install_summary.mode,
            "install": install_summary.files,
        });
        if let Some(tuning) = tuning.take() {
            payload["auto_tuning"] = tuning;
        }
        print_json("init", payload)?;
    } else if super::super::init_wizard::should_run(cli.json) {
        super::super::init_wizard::run(Path::new(&repo))?;
    } else {
        println!("Initialized atlas in {}", atlas_dir.display());
        println!("Database: {db_path}");
        println!("Content : {content_db_path}");
        println!("Session : {session_db_path}");
        println!("Registry: {repo_registry_path}");
        if config_created {
            println!("Config  : {} ({})", config_path.display(), profile.as_str());
        }
        println!("Mode    : {}", install_summary.mode);
        for entry in install_summary.touched() {
            println!("  {} {} ({})", entry.asset, entry.path, entry.action);
        }
    }
    Ok(())
}

/// Read-only `atlas init --dry-run` report: every path the mode would write.
#[allow(clippy::too_many_arguments)]
fn report_init_plan(
    cli: &Cli,
    repo: &str,
    atlas_dir: &Path,
    db_path: &str,
    content_db_path: &str,
    session_db_path: &str,
    config_path: &Path,
    profile: &str,
    mode: InitMode,
) -> Result<()> {
    let files = plan_init_mode(Path::new(repo), mode);
    let config_created = !config_path.exists();

    if cli.json {
        print_json(
            "init",
            serde_json::json!({
                "dry_run": true,
                "atlas_dir": atlas_dir.display().to_string(),
                "db_path": db_path,
                "content_db_path": content_db_path,
                "session_db_path": session_db_path,
                "config_path": config_path.display().to_string(),
                "config_created": config_created,
                "config_profile": profile,
                "mode": mode.as_str(),
                "install": files,
            }),
        )?;
        return Ok(());
    }

    println!(
        "[dry-run] init would prepare {} in {}",
        atlas_dir.display(),
        repo
    );
    println!("[dry-run] database : {db_path}");
    println!("[dry-run] content  : {content_db_path}");
    println!("[dry-run] session  : {session_db_path}");
    if config_created {
        println!("[dry-run] config   : {} ({profile})", config_path.display());
    }
    println!("[dry-run] mode     : {}", mode.as_str());
    for entry in &files {
        println!(
            "[dry-run] {} {} ({})",
            entry.asset, entry.path, entry.action
        );
    }
    Ok(())
}
