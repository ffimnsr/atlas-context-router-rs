use anyhow::{Context, Result};
use atlas_contentstore::ContentStore;
use atlas_core::{graph_health_error_message, graph_health_error_suggestions};
use atlas_session::DEFAULT_SESSION_DB;
use atlas_store_sqlite::Store;
use std::path::{Path, PathBuf};
use toml::Value as TomlValue;

use crate::cli::{Cli, Command, ConfigCommand};

use super::{db_path, print_json, resolve_repo};

#[derive(Debug, Clone)]
struct MigrationReport {
    label: &'static str,
    path: String,
    schema_version: i32,
    latest_version: i32,
}

#[derive(Debug, Clone)]
struct ConfigSourceValue {
    value: serde_json::Value,
    source: &'static str,
}

fn json_value_from_toml(value: &TomlValue) -> serde_json::Value {
    serde_json::to_value(value).unwrap_or(serde_json::Value::Null)
}

fn flatten_runtime_json(
    prefix: Option<&str>,
    value: &serde_json::Value,
    out: &mut Vec<(String, serde_json::Value)>,
) {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            for key in keys {
                let child_prefix = match prefix {
                    Some(prefix) => format!("{prefix}.{key}"),
                    None => key.clone(),
                };
                flatten_runtime_json(Some(&child_prefix), &map[&key], out);
            }
        }
        other => {
            if let Some(prefix) = prefix {
                out.push((prefix.to_owned(), other.clone()));
            }
        }
    }
}

fn config_runtime_json(config: &atlas_engine::Config) -> serde_json::Value {
    serde_json::json!({
        "build": {
            "parse_batch_size": config.parse_batch_size(),
            "max_files_per_run": config.build.max_files_per_run,
            "max_total_bytes_per_run": config.build.max_total_bytes_per_run,
            "max_file_bytes": config.build.max_file_bytes,
            "max_parse_failures": config.build.max_parse_failures,
            "max_parse_failure_ratio": config.build.max_parse_failure_ratio,
            "max_wall_time_ms": config.build.max_wall_time_ms,
        },
        "search": {
            "hybrid_enabled": config.search.hybrid_enabled,
            "top_k_fts": config.search.top_k_fts,
            "top_k_vector": config.search.top_k_vector,
            "rrf_k": config.search.rrf_k,
            "max_query_candidates": config.search.max_query_candidates,
            "max_query_wall_time_ms": config.search.max_query_wall_time_ms,
            "embedding": {
                "url": config.search.embedding.url,
                "model": config.search.embedding.model,
                "timeout_secs": config.search.embedding.timeout_secs,
                "max_retries": config.search.embedding.max_retries,
                "retry_backoff_ms": config.search.embedding.retry_backoff_ms,
            },
        },
        "analysis": {
            "dead_code_certainty_threshold": config.analysis.dead_code_certainty_threshold,
            "refactor_safety_threshold": config.analysis.refactor_safety_threshold,
            "impact_max_depth": config.analysis.impact_max_depth,
            "impact_max_nodes": config.analysis.impact_max_nodes,
            "dynamic_usage_allowlist": config.analysis.dynamic_usage_allowlist,
            "entrypoint_allowlist": config.analysis.entrypoint_allowlist,
            "framework_conventions_file": config.analysis.framework_conventions_file,
            "feedback_adjustment": {
                "enabled": config.analysis.feedback_adjustment.enabled,
            },
        },
        "context": {
            "max_context_nodes": config.context.max_context_nodes,
            "max_context_depth": config.context.max_context_depth,
            "max_seed_nodes": config.context.max_seed_nodes,
            "max_seed_files": config.context.max_seed_files,
            "max_traversal_depth": config.context.max_traversal_depth,
            "max_traversal_nodes": config.context.max_traversal_nodes,
            "max_traversal_edges": config.context.max_traversal_edges,
            "max_review_source_bytes": config.context.max_review_source_bytes,
            "max_context_payload_bytes": config.context.max_context_payload_bytes,
            "max_context_tokens_estimate": config.context.max_context_tokens_estimate,
            "max_file_excerpt_bytes": config.context.max_file_excerpt_bytes,
            "max_saved_context_bytes": config.context.max_saved_context_bytes,
            "tokenizer": {
                "encoding": config.context.tokenizer.encoding,
                "model": config.context.tokenizer.model,
                "tokenizer_file": config.context.tokenizer.tokenizer_file,
                "fallback": config.context.tokenizer.fallback.as_str(),
                "bytes_per_token": config.context.tokenizer.bytes_per_token,
            },
        },
        "mcp": {
            "worker_threads": config.mcp_worker_threads(),
            "tool_timeout_ms": config.mcp_tool_timeout_ms(),
            "tool_timeout_ms_by_tool": config.mcp_tool_timeout_ms_by_tool(),
            "max_mcp_response_bytes": config.mcp.max_mcp_response_bytes,
            "http_auth": {
                "enabled": config.mcp.http_auth.enabled,
                "issuer": config.mcp.http_auth.issuer,
                "discovery_url": config.mcp.http_auth.discovery_url,
                "jwks_url": config.mcp.http_auth.jwks_url,
                "resource": config.mcp.http_auth.resource,
                "required_scopes": config.mcp.http_auth.required_scopes,
                "allowed_origins": config.mcp.http_auth.allowed_origins,
            },
        },
        "memory": {
            "allow_custom_frontends": config.memory.allow_custom_frontends,
            "decay": {
                "enabled": config.memory.decay.enabled,
                "low_days": config.memory.decay.low_days,
                "normal_days": config.memory.decay.normal_days,
                "high_days": config.memory.decay.high_days,
                "critical_never_prune": config.memory.decay.critical_never_prune,
            },
            "wake_up": {
                "max_items": config.memory.wake_up.max_items,
                "max_feedback_items": config.memory.wake_up.max_feedback_items,
                "max_pending_changes": config.memory.wake_up.max_pending_changes,
            },
            "recall": {
                "freshness_window_days": config.memory.recall.freshness_window_days,
                "weight_time": config.memory.recall.weight_time,
                "weight_business": config.memory.recall.weight_business,
                "primary_quota_ratio": config.memory.recall.primary_quota_ratio,
                "aux_max_hops": config.memory.recall.aux_max_hops,
            },
        },
    })
}

fn config_sources(
    raw_config: Option<&str>,
    config: &atlas_engine::Config,
) -> Vec<(String, ConfigSourceValue)> {
    let mut runtime = Vec::new();
    flatten_runtime_json(None, &config_runtime_json(config), &mut runtime);

    let mut explicit = std::collections::HashMap::new();
    if let Some(raw) = raw_config
        && !raw.trim().is_empty()
    {
        let parsed: TomlValue = toml::from_str(raw)
            .context("cannot parse raw config for source tracing")
            .unwrap_or(TomlValue::Table(Default::default()));
        let mut flattened = Vec::new();
        flatten_runtime_json(None, &json_value_from_toml(&parsed), &mut flattened);
        for (key, value) in flattened {
            explicit.insert(key, value);
        }
    }

    runtime
        .into_iter()
        .map(|(key, value)| {
            let source = if explicit.contains_key(&key) {
                "file"
            } else {
                "default"
            };
            (key, ConfigSourceValue { value, source })
        })
        .collect()
}

fn cli_runtime_overrides(cli: &Cli, repo: &str) -> Vec<(String, ConfigSourceValue)> {
    let repo_source = if cli.repo.is_some() { "cli" } else { "auto" };
    let db_source = if cli.db.is_some() { "cli" } else { "default" };
    vec![
        (
            "runtime.repo_root".to_owned(),
            ConfigSourceValue {
                value: serde_json::json!(repo),
                source: repo_source,
            },
        ),
        (
            "runtime.db_path".to_owned(),
            ConfigSourceValue {
                value: serde_json::json!(db_path(cli, repo)),
                source: db_source,
            },
        ),
        (
            "runtime.output.json".to_owned(),
            ConfigSourceValue {
                value: serde_json::json!(cli.json),
                source: "cli",
            },
        ),
        (
            "runtime.output.verbose".to_owned(),
            ConfigSourceValue {
                value: serde_json::json!(cli.verbose),
                source: "cli",
            },
        ),
    ]
}

fn env_runtime_overrides() -> Vec<(String, ConfigSourceValue)> {
    [("ATLAS_HTTP_BIND", false)]
        .into_iter()
        .map(|(name, redacted)| {
            let value = match std::env::var(name) {
                Ok(_value) if redacted => serde_json::json!("<redacted>"),
                Ok(value) => serde_json::json!(value),
                Err(_) => serde_json::Value::Null,
            };
            let source = if value.is_null() { "unset" } else { "env" };
            (format!("env.{name}"), ConfigSourceValue { value, source })
        })
        .collect()
}

fn render_debug_config_payload(cli: &Cli, repo: &str) -> Result<serde_json::Value> {
    let atlas_dir = atlas_engine::paths::atlas_dir(repo);
    let config_path = atlas_engine::paths::config_path(repo);
    let raw_config = std::fs::read_to_string(&config_path).ok();
    let config = atlas_engine::Config::load(&atlas_dir)?;
    config.build_run_budget()?;
    config.budget_policy()?;

    let mut entries = config_sources(raw_config.as_deref(), &config);
    entries.extend(cli_runtime_overrides(cli, repo));
    entries.extend(env_runtime_overrides());
    entries.sort_by(|left, right| left.0.cmp(&right.0));

    let values = entries
        .iter()
        .map(|(key, entry)| {
            (
                key.clone(),
                serde_json::json!({
                    "value": entry.value,
                    "source": entry.source,
                }),
            )
        })
        .collect::<serde_json::Map<String, serde_json::Value>>();

    Ok(serde_json::json!({
        "repo_root": repo,
        "atlas_dir": atlas_dir.display().to_string(),
        "config_path": config_path.display().to_string(),
        "config_exists": config_path.exists(),
        "resolved": values,
    }))
}

fn graph_migration_report(path: &str) -> Result<MigrationReport> {
    let mut store = Store::open(path)?;
    store.migrate()?;
    Ok(MigrationReport {
        label: "graph_db",
        path: path.to_owned(),
        schema_version: store.schema_version()?,
        latest_version: store.schema_version()?,
    })
}

fn content_migration_report(path: &str) -> Result<MigrationReport> {
    let mut store = ContentStore::open(path)?;
    store.migrate()?;
    Ok(MigrationReport {
        label: "content_db",
        path: path.to_owned(),
        schema_version: store.schema_version()?,
        latest_version: store.schema_version()?,
    })
}

fn session_migration_report(path: &str) -> Result<MigrationReport> {
    let mut store = atlas_session::SessionStore::open(path)?;
    store.migrate()?;
    Ok(MigrationReport {
        label: "session_db",
        path: path.to_owned(),
        schema_version: store.schema_version()?,
        latest_version: store.schema_version()?,
    })
}

#[derive(Debug, Clone)]
struct PurgeTargetResult {
    label: &'static str,
    path: String,
    removed: bool,
    existed: bool,
    removed_artifacts: Vec<String>,
}

fn sqlite_artifact_paths(path: &Path) -> Vec<PathBuf> {
    let base = path.to_path_buf();
    vec![
        base.clone(),
        PathBuf::from(format!("{}.wal", base.display())),
        PathBuf::from(format!("{}.shm", base.display())),
    ]
}

fn purge_sqlite_artifacts(label: &'static str, path: PathBuf) -> Result<PurgeTargetResult> {
    let mut existed = false;
    let mut removed_artifacts = Vec::new();

    for artifact_path in sqlite_artifact_paths(&path) {
        if !artifact_path.exists() {
            continue;
        }
        existed = true;
        std::fs::remove_file(&artifact_path)
            .with_context(|| format!("cannot remove {}", artifact_path.display()))?;
        removed_artifacts.push(artifact_path.display().to_string());
    }

    Ok(PurgeTargetResult {
        label,
        path: path.display().to_string(),
        removed: !removed_artifacts.is_empty(),
        existed,
        removed_artifacts,
    })
}

pub fn run_purge_noncanonical(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path_str = db_path(cli, &repo);
    let context_db = PathBuf::from(atlas_engine::paths::content_db_path(&db_path_str));
    let session_db = atlas_engine::paths::atlas_dir(&repo).join(DEFAULT_SESSION_DB);

    let context_result = purge_sqlite_artifacts("context_db", context_db)?;
    let session_result = purge_sqlite_artifacts("session_db", session_db)?;
    let graph_db = PathBuf::from(&db_path_str);

    if cli.json {
        print_json(
            "purge_noncanonical",
            serde_json::json!({
                "repo_root": repo,
                "error_code": "none",
                "message": "Repo-local context/session stores purged for canonical path recovery.",
                "context_db": {
                    "path": context_result.path,
                    "removed": context_result.removed,
                    "existed": context_result.existed,
                    "removed_artifacts": context_result.removed_artifacts,
                },
                "session_db": {
                    "path": session_result.path,
                    "removed": session_result.removed,
                    "existed": session_result.existed,
                    "removed_artifacts": session_result.removed_artifacts,
                },
                "graph_db": {
                    "path": graph_db.display().to_string(),
                    "preserved": graph_db.exists(),
                },
                "next_steps": [
                    "atlas build",
                    "atlas session start"
                ],
            }),
        )?;
        return Ok(());
    }

    println!("Purged repo-local derived stores for canonical path recovery.");
    for result in [&context_result, &session_result] {
        let status = if result.removed {
            "removed"
        } else if result.existed {
            "present"
        } else {
            "not found"
        };
        println!("  {}: {} ({status})", result.label, result.path);
        for artifact in &result.removed_artifacts {
            println!("    removed {artifact}");
        }
    }
    println!("  graph_db: {} (preserved)", graph_db.display());
    println!();
    println!("Next:");
    println!("  atlas build");
    println!("  atlas session start");
    Ok(())
}

pub fn run_migrate(cli: &Cli) -> Result<()> {
    if let Command::Migrate {
        subcommand: Some(crate::cli::MigrateCommand::Markdown { .. }),
    } = &cli.command
    {
        return super::migrate_markdown::run_migrate_markdown(cli);
    }
    let repo = resolve_repo(cli)?;
    let graph_db = db_path(cli, &repo);
    let content_db = atlas_engine::paths::content_db_path(&graph_db);
    let session_db = atlas_engine::paths::session_db_path(&graph_db);

    let reports = vec![
        graph_migration_report(&graph_db)?,
        content_migration_report(&content_db)?,
        session_migration_report(&session_db)?,
    ];

    if cli.json {
        let dbs = reports
            .iter()
            .map(|report| {
                serde_json::json!({
                    "label": report.label,
                    "path": report.path,
                    "schema_version": report.schema_version,
                    "latest_version": report.latest_version,
                })
            })
            .collect::<Vec<_>>();
        print_json(
            "migrate",
            serde_json::json!({
                "repo_root": repo,
                "ok": true,
                "error_code": "none",
                "message": "Repo-local Atlas databases migrated.",
                "databases": dbs,
            }),
        )?;
        return Ok(());
    }

    println!("Migrated Atlas databases for {repo}");
    for report in &reports {
        println!(
            "  {}: {} (schema {} / latest {})",
            report.label, report.path, report.schema_version, report.latest_version
        );
    }
    Ok(())
}

pub fn run_debug_config(cli: &Cli) -> Result<()> {
    match &cli.command {
        Command::DebugConfig => {}
        Command::Config {
            subcommand: ConfigCommand::Show,
        } => {}
        _ => unreachable!(),
    }

    let repo = resolve_repo(cli)?;
    let payload = render_debug_config_payload(cli, &repo)?;

    if cli.json {
        print_json("debug_config", payload)?;
        return Ok(());
    }

    println!("Resolved config for {repo}");
    println!(
        "  atlas_dir   : {}",
        payload["atlas_dir"].as_str().unwrap_or_default()
    );
    println!(
        "  config_path : {}",
        payload["config_path"].as_str().unwrap_or_default()
    );
    println!(
        "  config_file : {}",
        if payload["config_exists"] == serde_json::json!(true) {
            "present"
        } else {
            "missing (defaults only)"
        }
    );
    println!();
    let resolved = payload["resolved"]
        .as_object()
        .context("resolved config payload malformed")?;
    for (key, value) in resolved {
        println!(
            "  {key:<40} {:<8} {}",
            value["source"].as_str().unwrap_or("unknown"),
            match &value["value"] {
                serde_json::Value::String(text) => text.clone(),
                other => other.to_string(),
            }
        );
    }
    Ok(())
}

pub fn run_selfupdate(cli: &Cli) -> Result<()> {
    let message = "Self-update is not built into atlas. Reinstall with the documented installer or your package manager to pick up a newer binary.";
    let next_steps = vec![
        "./install.sh".to_owned(),
        "cargo install --path packages/atlas-cli --force".to_owned(),
    ];

    if cli.json {
        print_json(
            "selfupdate",
            serde_json::json!({
                "ok": false,
                "error_code": "selfupdate_not_supported",
                "message": message,
                "next_steps": next_steps,
            }),
        )?;
        return Ok(());
    }

    println!("{message}");
    println!("Suggested commands:");
    for step in &next_steps {
        println!("  {step}");
    }
    Ok(())
}

pub fn run_debug_graph(cli: &Cli) -> Result<()> {
    let limit = match &cli.command {
        Command::DebugGraph { limit } => *limit,
        _ => unreachable!(),
    };

    let repo = resolve_repo(cli)?;
    let db_path = db_path(cli, &repo);
    let store =
        Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

    let stats = store.graph_stats().context("cannot read graph stats")?;
    let edge_kinds = store.edge_kind_stats().context("edge kind stats failed")?;
    let top_files = store
        .top_files_by_node_count(limit)
        .context("top files query failed")?;
    let orphans = store
        .orphan_nodes(limit)
        .context("orphan node query failed")?;
    let dangling = store
        .structural_dangling_edges(limit)
        .context("dangling edge query failed")?;

    if cli.json {
        print_json(
            "debug_graph",
            serde_json::json!({
                "ok": true,
                "error_code": "none",
                "message": graph_health_error_message("none"),
                "suggestions": graph_health_error_suggestions("none"),
                "nodes": stats.node_count,
                "edges": stats.edge_count,
                "files": stats.file_count,
                "nodes_by_kind": stats.nodes_by_kind,
                "edges_by_kind": edge_kinds,
                "top_files_by_node_count": top_files,
                "orphan_nodes": orphans.iter().map(|n| serde_json::json!({
                    "kind": n.kind.as_str(),
                    "qualified_name": n.qualified_name,
                    "file_path": n.file_path,
                    "line_start": n.line_start,
                })).collect::<Vec<_>>(),
                "dangling_edges": dangling.iter().map(|(id, src, tgt, kind, side)| serde_json::json!({
                    "id": id,
                    "kind": kind,
                    "source_qn": src,
                    "target_qn": tgt,
                    "missing_side": side,
                })).collect::<Vec<_>>(),
            }),
        )?;
    } else {
        println!("Graph summary");
        println!("  Nodes : {}", stats.node_count);
        println!("  Edges : {}", stats.edge_count);
        println!("  Files : {}", stats.file_count);

        if !stats.nodes_by_kind.is_empty() {
            println!("\nNodes by kind:");
            for (kind, count) in &stats.nodes_by_kind {
                println!("  {kind:<16} {count}");
            }
        }

        if !edge_kinds.is_empty() {
            println!("\nEdges by kind:");
            for (kind, count) in &edge_kinds {
                println!("  {kind:<16} {count}");
            }
        }

        if !top_files.is_empty() {
            println!("\nTop files by node count:");
            for (path, count) in &top_files {
                println!("  {count:>6}  {path}");
            }
        }

        println!("\nData integrity:");
        if orphans.is_empty() {
            println!("  Orphan nodes    : 0 (OK)");
        } else {
            println!("  Orphan nodes    : {} (no edges)", orphans.len());
            for n in orphans.iter().take(5) {
                println!(
                    "    {} {} ({}:{})",
                    n.kind.as_str(),
                    n.qualified_name,
                    n.file_path,
                    n.line_start
                );
            }
            if orphans.len() > 5 {
                println!(
                    "    … and {} more (use --limit to show more)",
                    orphans.len() - 5
                );
            }
        }
        if dangling.is_empty() {
            println!("  Dangling edges  : 0 (OK)");
        } else {
            println!("  Dangling edges  : {} (missing endpoint)", dangling.len());
            for (id, src, tgt, kind, side) in dangling.iter().take(5) {
                println!("    edge {id} [{kind}] missing {side}: {src} -> {tgt}");
            }
            if dangling.len() > 5 {
                println!("    … and {} more", dangling.len() - 5);
            }
        }
    }

    Ok(())
}

pub fn run_embedding_config(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db_path_str = db_path(cli, &repo);
    let content_db = atlas_engine::paths::content_db_path(&db_path_str);

    // Active config from .atlas/config.toml.
    let atlas_dir = atlas_engine::paths::atlas_dir(&repo);
    let config = atlas_engine::Config::load(&atlas_dir).unwrap_or_default();
    let embedding_url = config.search.embedding.url.clone().unwrap_or_default();
    let embedding_model = config.search.embedding.model.clone();
    let hybrid_enabled = config.search.hybrid_enabled;

    // Derive provider name the same way the embed command does.
    let active_provider = if embedding_url.is_empty() {
        None
    } else {
        Some(atlas_contentstore::store::registry::provider_name_from_url(&embedding_url).to_owned())
    };

    // Read persisted registry from content store.
    let (registry_entries, registry_error) = match ContentStore::open(&content_db) {
        Ok(mut store) => {
            let _ = store.migrate();
            match store.list_embedding_providers() {
                Ok(entries) => (entries, None),
                Err(e) => (vec![], Some(e.to_string())),
            }
        }
        Err(e) => (vec![], Some(e.to_string())),
    };

    if cli.json {
        let entries: Vec<serde_json::Value> = registry_entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "provider_name": e.provider_name,
                    "model_name": e.model_name,
                    "dimension": e.dimension,
                    "discovered_at": e.discovered_at,
                    "index_schema_version": e.index_schema_version,
                })
            })
            .collect();
        print_json(
            "embedding_config",
            serde_json::json!({
                "repo_root": repo,
                "active": {
                    "url": if embedding_url.is_empty() { serde_json::Value::Null } else { serde_json::json!(embedding_url) },
                    "model": embedding_model,
                    "provider_name": active_provider,
                    "hybrid_enabled": hybrid_enabled,
                },
                "registry": entries,
                "registry_error": registry_error,
                "content_db_path": content_db,
            }),
        )?;
        return Ok(());
    }

    println!("Embedding configuration for {repo}");
    println!();
    println!("Active config:");
    if embedding_url.is_empty() {
        println!("  URL            : (not configured)");
        println!("  Model          : {embedding_model}");
        println!("  Hybrid enabled : {hybrid_enabled}");
    } else {
        println!("  URL            : {embedding_url}");
        println!("  Model          : {embedding_model}");
        println!(
            "  Provider       : {}",
            active_provider.as_deref().unwrap_or("unknown")
        );
        println!("  Hybrid enabled : {hybrid_enabled}");
    }
    println!();
    println!(
        "Registered dimension registry ({} entries):",
        registry_entries.len()
    );
    if let Some(err) = &registry_error {
        println!("  (error reading registry: {err})");
    } else if registry_entries.is_empty() {
        println!("  (none — run `atlas embed` to register dimensions)");
    } else {
        for e in &registry_entries {
            println!(
                "  {}/{}: dim={}, schema_v={}, discovered={}",
                e.provider_name, e.model_name, e.dimension, e.index_schema_version, e.discovered_at
            );
        }
    }
    Ok(())
}
