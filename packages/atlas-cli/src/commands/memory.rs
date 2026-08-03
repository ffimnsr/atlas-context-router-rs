//! ICM-A2 — `atlas memory` CRUD commands.
//!
//! All behavior lives in the shared memory service layer in `atlas-session`;
//! this module only parses CLI flags, builds the shared input/filter types,
//! and renders output.

use std::path::Path;

use anyhow::{Context, Result, bail};

use atlas_session::{
    MemoryImportance, MemoryListFilter, MemoryRecord, MemoryScope, MemoryViewer, NewMemory,
    SessionId, SessionStore, normalize_frontend,
};
use time::OffsetDateTime;

use crate::cli::{Cli, Command, MemoryCommand};

use super::{print_json, resolve_repo};

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
    }

    Ok(())
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
