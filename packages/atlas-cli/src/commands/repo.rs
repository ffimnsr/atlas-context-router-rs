use anyhow::{Context, Result};
use atlas_repo::{RepoRegistry, add_manual_repo, bootstrap_registry};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

use crate::cli::{Cli, Command, RepoCommand};

use super::{db_path, print_json, resolve_repo};

pub fn run_repo(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let registry_root = Utf8Path::new(&repo);
    let Command::Repo { subcommand } = &cli.command else {
        unreachable!();
    };

    match subcommand {
        RepoCommand::List => run_list(cli, registry_root),
        RepoCommand::Add { path } => run_add(cli, registry_root, path),
        RepoCommand::Remove { repo_id } => run_remove(cli, registry_root, repo_id),
        RepoCommand::Sync => run_sync(cli, registry_root),
    }
}

pub(crate) fn bootstrap_and_save_registry(registry_root: &Utf8Path) -> Result<RepoRegistry> {
    let mut registry = if atlas_repo::registry_path(registry_root).exists() {
        RepoRegistry::load(registry_root)?
    } else {
        bootstrap_registry(registry_root)?
    };
    registry.sync(registry_root)?;
    registry.save(registry_root)?;
    Ok(registry)
}

fn refresh_synthetic_registry_graph(
    cli: &Cli,
    registry_root: &Utf8Path,
    registry: &RepoRegistry,
) -> Result<()> {
    let db_path = db_path(cli, registry_root.as_str());
    let mut store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
    atlas_engine::refresh_repo_registry_graph(&mut store, registry)
}

fn run_list(cli: &Cli, registry_root: &Utf8Path) -> Result<()> {
    let registry = RepoRegistry::load_or_bootstrap(registry_root)?;
    if cli.json {
        return print_json("repo.list", serde_json::to_value(registry)?);
    }

    println!(
        "Repo registry: {}",
        atlas_repo::registry_path(registry_root)
    );
    for entry in &registry.registrations {
        println!(
            "{}\t{}\t{:?}\t{:?}\tenabled={}",
            entry.repo_id,
            entry.display_alias,
            entry.relationship.kind,
            entry.trust_state,
            entry.enabled
        );
    }
    for warning in &registry.warnings {
        println!("warning\t{}\t{}", warning.code, warning.message);
    }
    Ok(())
}

fn run_add(cli: &Cli, registry_root: &Utf8Path, path: &str) -> Result<()> {
    let mut registry = RepoRegistry::load_or_bootstrap(registry_root)?;
    let registration = add_manual_repo(registry_root, Utf8Path::new(path))?;
    let repo_id = registration.repo_id.clone();
    let display_alias = registration.display_alias.clone();
    registry.upsert(registration);
    registry.save(registry_root)?;
    refresh_synthetic_registry_graph(cli, registry_root, &registry)?;

    if cli.json {
        print_json(
            "repo.add",
            serde_json::json!({
                "repo_id": repo_id,
                "display_alias": display_alias,
                "registry_path": atlas_repo::registry_path(registry_root),
            }),
        )
    } else {
        println!("Registered repo {repo_id} ({display_alias})");
        Ok(())
    }
}

fn run_remove(cli: &Cli, registry_root: &Utf8Path, repo_id: &str) -> Result<()> {
    let mut registry = RepoRegistry::load(registry_root)
        .with_context(|| "repo registry missing; run `atlas init` or `atlas repo sync` first")?;
    let removed = registry
        .remove(repo_id)
        .with_context(|| format!("repo id '{repo_id}' is not registered"))?;
    registry.save(registry_root)?;
    refresh_synthetic_registry_graph(cli, registry_root, &registry)?;

    if cli.json {
        print_json(
            "repo.remove",
            serde_json::json!({
                "repo_id": removed.repo_id,
                "display_alias": removed.display_alias,
                "graph_data_deleted": false,
            }),
        )
    } else {
        println!(
            "Removed repo {} ({}); graph data untouched",
            removed.repo_id, removed.display_alias
        );
        Ok(())
    }
}

fn run_sync(cli: &Cli, registry_root: &Utf8Path) -> Result<()> {
    let registry = bootstrap_and_save_registry(registry_root)?;
    refresh_synthetic_registry_graph(cli, registry_root, &registry)?;
    if cli.json {
        return print_json("repo.sync", serde_json::to_value(registry)?);
    }

    println!("Synced {} repo registrations", registry.registrations.len());
    for warning in &registry.warnings {
        println!("warning\t{}\t{}", warning.code, warning.message);
    }
    Ok(())
}
