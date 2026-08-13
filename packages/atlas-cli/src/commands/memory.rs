//! ICM-A2 — `atlas memory` CRUD commands.
//!
//! All behavior lives in the shared memory service layer in `atlas-session`;
//! this module only parses CLI flags, builds the shared input/filter types,
//! and renders output.

use std::path::Path;

use anyhow::{Context, Result, bail};

use atlas_contentstore::ContentStore;
use atlas_session::{
    MemoryDecayPolicy, MemoryImportance, MemoryListFilter, MemoryRecord, MemoryScope, MemoryViewer,
    NewMemory, SessionId, SessionStore, normalize_frontend,
};
use time::OffsetDateTime;

use crate::cli::{Cli, Command, MemoryCommand};

use super::{db_path, print_json, resolve_repo};

pub fn run_memory(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;

    let sub = match &cli.command {
        Command::Memory { subcommand } => subcommand,
        _ => unreachable!(),
    };

    // Memory surface policy; missing config file yields defaults.
    let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
    let allow_custom_frontends = config.allow_custom_frontends();

    let mut store = SessionStore::open_in_repo(Path::new(&repo))
        .with_context(|| format!("cannot open session store in {repo}"))?;

    match sub {
        MemoryCommand::Store {
            text,
            topic,
            title,
            importance,
            scope,
            frontend,
            source_id,
        } => {
            let importance = parse_optional_importance(importance.as_deref())?.unwrap_or_default();
            let scope = parse_optional_scope(scope.as_deref())?.unwrap_or_default();
            let frontend = frontend
                .as_deref()
                .map(|raw| normalize_frontend(raw, allow_custom_frontends))
                .transpose()?;
            let session_id = (scope == MemoryScope::Session)
                .then(|| SessionId::derive(&repo, "", "cli").as_str().to_owned());
            let input = NewMemory {
                repo_root: repo.clone(),
                session_id,
                frontend,
                scope,
                topic: topic.clone().unwrap_or_default(),
                title: title.clone().unwrap_or_default(),
                body: text.clone(),
                importance,
                source_id: source_id.clone(),
                metadata: serde_json::json!({}),
            };
            input.validate()?;
            let record = store.store_memory(&input)?;

            if cli.json {
                print_json("memory.store", serde_json::json!({ "memory": record }))?;
            } else {
                println!("Memory stored: {}", record.id);
                println!("Topic      : {}", display_or_dash(&record.topic));
                println!("Title      : {}", display_or_dash(&record.title));
                println!("Scope      : {}", record.scope);
                println!("Importance : {}", record.importance);
            }
        }

        MemoryCommand::Recall {
            query,
            topic,
            importance,
            scope,
            shared,
            limit,
        } => {
            let filter = MemoryListFilter {
                topic: topic.clone(),
                importance: parse_optional_importance(importance.as_deref())?,
                scope: parse_optional_scope(scope.as_deref())?,
                ..Default::default()
            };
            let viewer = MemoryViewer {
                frontend: "cli".to_owned(),
                session_id: SessionId::derive(&repo, "", "cli").as_str().to_owned(),
            };
            let hits = store.recall_memories(&repo, query, &filter, *shared, &viewer, *limit)?;

            if cli.json {
                let results = hits
                    .iter()
                    .map(|hit| {
                        serde_json::json!({
                            "memory": hit.memory,
                            "relevance_score": hit.relevance_score,
                        })
                    })
                    .collect::<Vec<_>>();
                print_json(
                    "memory.recall",
                    serde_json::json!({
                        "query": query,
                        "count": hits.len(),
                        "results": results,
                    }),
                )?;
            } else if hits.is_empty() {
                println!("No memories found for {query:?}");
            } else {
                println!("Found {} memories for {query:?}:", hits.len());
                for (index, hit) in hits.iter().enumerate() {
                    let memory = &hit.memory;
                    println!(
                        "{}. {} [{} · {}]",
                        index + 1,
                        memory_label(memory),
                        memory.importance,
                        memory.scope
                    );
                    println!("   {}", body_preview(&memory.body));
                    println!("   id: {}", memory.id);
                }
            }
        }

        MemoryCommand::List {
            topic,
            importance,
            scope,
            older_than,
            newer_than,
        } => {
            let filter = MemoryListFilter {
                topic: topic.clone(),
                importance: parse_optional_importance(importance.as_deref())?,
                scope: parse_optional_scope(scope.as_deref())?,
                older_than: older_than
                    .as_deref()
                    .map(parse_memory_timestamp)
                    .transpose()?,
                newer_than: newer_than
                    .as_deref()
                    .map(parse_memory_timestamp)
                    .transpose()?,
                ..Default::default()
            };
            let memories = store.list_memories(&repo, &filter)?;

            if cli.json {
                print_json(
                    "memory.list",
                    serde_json::json!({
                        "count": memories.len(),
                        "memories": memories,
                    }),
                )?;
            } else if memories.is_empty() {
                println!("No memories found");
            } else {
                for memory in &memories {
                    println!(
                        "{}  {:<8} {:<9} {} (id: {})",
                        memory.updated_at,
                        memory.importance.to_string(),
                        memory.scope.to_string(),
                        memory_label(memory),
                        memory.id
                    );
                }
            }
        }

        MemoryCommand::Delete { memory_id, dry_run } => {
            let result = store.delete_memory(&repo, memory_id, *dry_run)?;
            if !result.found {
                bail!("no memory with id {memory_id} in {repo}");
            }

            if cli.json {
                print_json(
                    "memory.delete",
                    serde_json::json!({
                        "memory_id": result.memory_id,
                        "deleted": result.deleted,
                        "dry_run": result.dry_run,
                    }),
                )?;
            } else if *dry_run {
                println!("Would delete memory: {memory_id}");
            } else {
                println!("Memory deleted: {memory_id}");
            }
        }

        MemoryCommand::Decay { topic, dry_run } => {
            let policy = decay_policy(&config);
            let filter = MemoryListFilter {
                topic: topic.clone(),
                ..Default::default()
            };
            let reports = store.decay_reports(&repo, &filter, &policy, *dry_run)?;

            if cli.json {
                print_json(
                    "memory.decay",
                    serde_json::json!({
                        "enabled": policy.enabled,
                        "dry_run": *dry_run,
                        "count": reports.len(),
                        "reports": reports,
                    }),
                )?;
            } else if !policy.enabled {
                println!("Memory decay disabled by config; no scores updated.");
            } else {
                let stale = reports.iter().filter(|r| r.stale).count();
                let protected = reports.iter().filter(|r| r.protected).count();
                println!(
                    "Memory decay {} for {} memories ({} stale, {} critical protected)",
                    if *dry_run { "plan" } else { "applied" },
                    reports.len(),
                    stale,
                    protected
                );
                for report in &reports {
                    let retention = report
                        .retention_days
                        .map(|days| days.to_string())
                        .unwrap_or_else(|| "never".to_owned());
                    let flags = if report.protected {
                        "[protected]"
                    } else if report.stale {
                        "[stale]"
                    } else {
                        ""
                    };
                    println!(
                        "  {}  topic={:<16} age={:.0}d retention={:>5} score={:.2} {}",
                        report.memory.id,
                        display_or_dash(&report.memory.topic),
                        report.age_days,
                        retention,
                        report.updated_decay_score,
                        flags
                    );
                }
            }
        }

        MemoryCommand::Stale { topic, scope } => {
            let policy = decay_policy(&config);
            let filter = MemoryListFilter {
                topic: topic.clone(),
                scope: parse_optional_scope(scope.as_deref())?,
                ..Default::default()
            };
            let reports = store.stale_memories(&repo, &filter, &policy)?;

            if cli.json {
                print_json(
                    "memory.stale",
                    serde_json::json!({
                        "enabled": policy.enabled,
                        "count": reports.len(),
                        "reports": reports,
                    }),
                )?;
            } else if reports.is_empty() {
                println!("No stale memories found");
            } else {
                println!("{} stale memories:", reports.len());
                for report in &reports {
                    println!(
                        "  {}  topic={:<16} age={:.0}d score={:.2}",
                        report.memory.id,
                        display_or_dash(&report.memory.topic),
                        report.age_days,
                        report.updated_decay_score
                    );
                }
            }
        }

        MemoryCommand::Prune {
            dry_run,
            topic,
            importance,
            older_than,
            allow_critical,
        } => {
            let policy = decay_policy(&config);
            let filter = MemoryListFilter {
                topic: topic.clone(),
                importance: parse_optional_importance(importance.as_deref())?,
                older_than: older_than
                    .as_deref()
                    .map(parse_memory_timestamp)
                    .transpose()?,
                ..Default::default()
            };
            let result =
                store.prune_memories(&repo, &filter, &policy, *dry_run, *allow_critical)?;

            if cli.json {
                print_json(
                    "memory.prune",
                    serde_json::json!({
                        "enabled": policy.enabled,
                        "dry_run": result.dry_run,
                        "candidate_count": result.candidate_count,
                        "deleted_count": result.deleted_count,
                        "protected_count": result.protected_count,
                        "candidates": result.candidates,
                    }),
                )?;
            } else if !policy.enabled {
                println!("Memory decay disabled by config; nothing to prune.");
            } else if result.candidate_count == 0 {
                println!("No pruneable memories found");
            } else {
                println!(
                    "{} prune {} memory records{}:",
                    if result.dry_run { "Would" } else { "Pruned" },
                    result.deleted_count.max(result.candidate_count),
                    if result.protected_count > 0 {
                        format!(" ({} critical protected)", result.protected_count)
                    } else {
                        String::new()
                    }
                );
                for memory in &result.candidates {
                    println!(
                        "  {}  topic={:<16} {}",
                        memory.id,
                        display_or_dash(&memory.topic),
                        memory_label(memory)
                    );
                }
            }
        }

        MemoryCommand::Health { topic, scope } => {
            let policy = decay_policy(&config);
            let filter = MemoryListFilter {
                topic: topic.clone(),
                scope: parse_optional_scope(scope.as_deref())?,
                ..Default::default()
            };
            let content_db = atlas_engine::paths::content_db_path(&db_path(cli, &repo));
            let content_store = ContentStore::open(&content_db).ok();
            let source_exists = |source_id: &str| -> bool {
                content_store
                    .as_ref()
                    .and_then(|store| store.get_source(source_id).ok().flatten())
                    .is_some()
            };
            let report = store.memory_health(&repo, &filter, &policy, &source_exists)?;

            if cli.json {
                print_json(
                    "memory.health",
                    serde_json::json!({
                        "total_memories": report.total_memories,
                        "finding_count": report.findings.len(),
                        "by_category": report.by_category,
                        "findings": report.findings,
                    }),
                )?;
            } else {
                println!(
                    "Memory health: {} memories, {} findings",
                    report.total_memories,
                    report.findings.len()
                );
                if report.by_category.is_empty() {
                    println!("All memories healthy.");
                }
                for finding in &report.findings {
                    let target = finding.memory_id.clone().unwrap_or_else(|| {
                        format!("topic '{}'", finding.topic.as_deref().unwrap_or(""))
                    });
                    println!("  [{}] {}", finding.category, target);
                    println!("      {}", finding.detail);
                    println!("      {}", finding.suggestion);
                    println!("      run: {}", finding.command);
                }
            }
        }

        MemoryCommand::Consolidate {
            topic,
            scope,
            dry_run,
        } => {
            let filter = MemoryListFilter {
                topic: topic.clone(),
                scope: parse_optional_scope(scope.as_deref())?,
                ..Default::default()
            };
            let plan = store.consolidate_memories(&repo, &filter, *dry_run)?;

            if cli.json {
                print_json(
                    "memory.consolidate",
                    serde_json::json!({
                        "dry_run": plan.dry_run,
                        "group_count": plan.groups.len(),
                        "kept_count": plan.kept_ids.len(),
                        "merged_count": plan.merged_ids.len(),
                        "kept_ids": plan.kept_ids,
                        "merged_ids": plan.merged_ids,
                        "groups": plan.groups,
                    }),
                )?;
            } else if plan.groups.is_empty() {
                println!("No consolidation candidates found");
            } else {
                println!(
                    "{} consolidate {} groups ({} kept, {} merged):",
                    if plan.dry_run {
                        "Would"
                    } else {
                        "Consolidated"
                    },
                    plan.groups.len(),
                    plan.kept_ids.len(),
                    plan.merged_ids.len()
                );
                for group in &plan.groups {
                    let target = group.consolidated_id.as_deref().unwrap_or("<new>");
                    println!(
                        "  topic={:<16} keep={} merged={} -> {}",
                        display_or_dash(&group.topic),
                        group.kept_memory_id,
                        group.merged_memory_ids.join(","),
                        target
                    );
                    if !group.source_ids.is_empty() {
                        println!("      preserved sources: {}", group.source_ids.join(", "));
                    }
                }
            }
        }
    }

    Ok(())
}

/// Build the effective decay policy from `[memory.decay]` config; a missing
/// config file yields the safe defaults (ICM-B1).
fn decay_policy(config: &atlas_engine::Config) -> MemoryDecayPolicy {
    MemoryDecayPolicy {
        enabled: config.memory.decay.enabled,
        low_days: config.memory.decay.low_days,
        normal_days: config.memory.decay.normal_days,
        high_days: config.memory.decay.high_days,
        critical_never_prune: config.memory.decay.critical_never_prune,
    }
}

// ── Flag parsing ──────────────────────────────────────────────────────────────

fn parse_optional_importance(value: Option<&str>) -> Result<Option<MemoryImportance>> {
    value
        .map(|raw| raw.parse().map_err(anyhow::Error::from))
        .transpose()
}

fn parse_optional_scope(value: Option<&str>) -> Result<Option<MemoryScope>> {
    value
        .map(|raw| raw.parse().map_err(anyhow::Error::from))
        .transpose()
}

/// Normalize a user-supplied date to a second-precision RFC 3339 string so
/// string comparison against stored timestamps equals chronological comparison.
fn parse_memory_timestamp(value: &str) -> Result<String> {
    if let Ok(ts) = OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339) {
        return Ok(format_timestamp(ts));
    }
    if let Ok(date) = time::Date::parse(
        value,
        &time::macros::format_description!("[year]-[month]-[day]"),
    ) {
        let midnight = date
            .with_hms(0, 0, 0)
            .expect("midnight is always a valid time")
            .assume_utc();
        return Ok(format_timestamp(midnight));
    }
    bail!("invalid date {value:?}: expected YYYY-MM-DD or an RFC 3339 timestamp")
}

fn format_timestamp(ts: OffsetDateTime) -> String {
    atlas_core::format_rfc3339(
        ts.replace_nanosecond(0)
            .expect("0 nanoseconds is always valid"),
    )
}

// ── Rendering helpers ─────────────────────────────────────────────────────────

fn display_or_dash(value: &str) -> &str {
    if value.is_empty() { "-" } else { value }
}

fn memory_label(memory: &MemoryRecord) -> &str {
    if !memory.title.is_empty() {
        &memory.title
    } else if !memory.topic.is_empty() {
        &memory.topic
    } else {
        "(untitled)"
    }
}

fn body_preview(body: &str) -> String {
    const PREVIEW_CHARS: usize = 160;
    let trimmed = body.trim();
    if trimmed.chars().count() <= PREVIEW_CHARS {
        trimmed.to_owned()
    } else {
        let cut = trimmed.chars().take(PREVIEW_CHARS).collect::<String>();
        format!("{cut}…")
    }
}
