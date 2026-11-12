use anyhow::{Context, Result};
use atlas_session::{
    OverviewFreshness, OverviewLevel, OverviewListFilter, OverviewSearchFilter, OverviewSourceKind,
    OverviewStaleFilter, OverviewSubjectKind, render_level,
};

use crate::cli::{Cli, Command, OverviewCommand};

use super::{open_graph_store, open_session_store, print_json};

pub(super) fn run_read(cli: &Cli, repo: &str, db_path: &str) -> Result<()> {
    match &cli.command {
        Command::Overview {
            subcommand: OverviewCommand::Show { subject, level },
        } => {
            let session = open_session_store(db_path)?;
            let record = session.resolve_overview_subject(repo, subject)?;
            if cli.json {
                let mut value = serde_json::to_value(&record)?;
                if let Some(level) = level {
                    let level: OverviewLevel = level.parse()?;
                    value["view"] = serde_json::to_value(render_level(&record, level))?;
                }
                print_json("overview.show", value)?;
            } else {
                print_record(&record);
            }
        }
        Command::Overview {
            subcommand:
                OverviewCommand::Search {
                    query,
                    subject_kind,
                    freshness,
                    package,
                    module,
                    file,
                    source_kind,
                    limit,
                    level,
                },
        } => {
            let session = open_session_store(db_path)?;
            let filter = OverviewSearchFilter {
                subject_kind: parse_kind(subject_kind.as_deref())?,
                freshness: parse_freshness(freshness.as_deref())?,
                package_name: package.clone(),
                module_path: module.clone(),
                canonical_file_path: file.clone(),
                source_kind: parse_source_kind(source_kind.as_deref())?,
                limit: *limit,
            };
            let hits = session.search_overview_records(repo, query, &filter)?;
            if cli.json {
                let level = level
                    .as_deref()
                    .map(str::parse::<OverviewLevel>)
                    .transpose()?;
                let payload = match level {
                    Some(level) => serde_json::json!({
                        "hits": hits.iter().map(|hit| serde_json::json!({
                            "score": hit.score,
                            "ranking": hit.ranking,
                            "record": hit.record,
                            "view": render_level(&hit.record, level),
                        })).collect::<Vec<_>>(),
                    }),
                    None => serde_json::to_value(&hits)?,
                };
                print_json("overview.search", payload)?;
            } else {
                println!("Overview search: {} hit(s)", hits.len());
                for hit in &hits {
                    println!(
                        "  [{:.2}] {} · {} ({} · confidence {:.2})",
                        hit.score,
                        subject_label(&hit.record),
                        hit.record.subject_kind,
                        hit.record.freshness,
                        hit.record.confidence
                    );
                    println!("      id: {}", hit.record.id);
                }
            }
        }
        Command::Overview {
            subcommand:
                OverviewCommand::List {
                    subject_kind,
                    freshness,
                    source_kind,
                    older_than,
                    newer_than,
                    limit,
                },
        } => {
            let session = open_session_store(db_path)?;
            let records = session.list_overview_records(
                repo,
                &OverviewListFilter {
                    subject_kind: parse_kind(subject_kind.as_deref())?,
                    freshness: parse_freshness(freshness.as_deref())?,
                    source_kind: parse_source_kind(source_kind.as_deref())?,
                    older_than: older_than.clone(),
                    newer_than: newer_than.clone(),
                    limit: *limit,
                },
            )?;
            if cli.json {
                print_json("overview.list", serde_json::to_value(&records)?)?;
            } else {
                println!("Overview records: {}", records.len());
                for record in &records {
                    println!(
                        "  {} · {} · {} · {}",
                        record.subject_kind,
                        subject_label(record),
                        record.freshness,
                        record.commit_sha
                    );
                }
            }
        }
        Command::Overview {
            subcommand:
                OverviewCommand::Stale {
                    subject_kind,
                    file,
                    package,
                    module,
                    since,
                    limit,
                },
        } => {
            let session = open_session_store(db_path)?;
            let records = session.list_stale_overview_records(
                repo,
                &OverviewStaleFilter {
                    subject_kind: parse_kind(subject_kind.as_deref())?,
                    canonical_file_path: file.clone(),
                    package_name: package.clone(),
                    module_path: module.clone(),
                    since: since.clone(),
                    limit: *limit,
                },
            )?;
            if cli.json {
                print_json("overview.stale", serde_json::to_value(&records)?)?;
            } else {
                println!("Stale overview records: {}", records.len());
                for record in &records {
                    println!(
                        "  {} · {} · {}",
                        record.subject_kind,
                        subject_label(record),
                        record.freshness
                    );
                }
            }
        }
        Command::Overview {
            subcommand:
                OverviewCommand::RefreshPlan {
                    changed: _,
                    since,
                    apply,
                },
        } => {
            let store = open_graph_store(db_path)?;
            let mut session = open_session_store(db_path)?;
            let plan = atlas_review::overview::refresh_overview_plan(
                repo,
                &store,
                &mut session,
                since.as_deref(),
                *apply,
            )?;
            if cli.json {
                print_json("overview.refresh_plan", serde_json::to_value(&plan)?)?;
            } else {
                println!(
                    "Overview refresh plan: {} changed file(s), direct={}, possibly={}, orphaned={}, unchanged={}",
                    plan.changed_files.len(),
                    plan.direct_stale.len(),
                    plan.possibly_stale.len(),
                    plan.orphaned.len(),
                    plan.unchanged_count
                );
                if plan.apply {
                    println!("Applied freshness updates: {}", plan.applied_count);
                }
                for item in plan
                    .direct_stale
                    .iter()
                    .chain(plan.possibly_stale.iter())
                    .chain(plan.orphaned.iter())
                {
                    println!(
                        "  [p{}] {} · {} · {}",
                        item.priority, item.subject_kind, item.subject_label, item.stale_reason
                    );
                    println!("      next: {}", item.suggested_export_command);
                }
            }
        }
        Command::Overview {
            subcommand:
                OverviewCommand::Delete {
                    overview_id,
                    dry_run,
                },
        } => {
            let mut session = open_session_store(db_path)?;
            let result = session.delete_overview_record(overview_id, *dry_run)?;
            if cli.json {
                print_json("overview.delete", serde_json::to_value(&result)?)?;
            } else if result.found {
                println!(
                    "Overview record {} {}",
                    overview_id,
                    if result.deleted {
                        "deleted"
                    } else {
                        "would be deleted (dry run)"
                    }
                );
            } else {
                println!("Overview record {overview_id} not found");
            }
        }
        Command::Overview {
            subcommand: OverviewCommand::ExportDocs { format, output },
        } => {
            if format != "markdown" {
                anyhow::bail!("unsupported overview export-docs format: {format} (only markdown)");
            }
            let session = open_session_store(db_path)?;
            let docs = atlas_review::overview::export_overview_docs(repo, &session)?;
            if cli.json {
                print_json("overview.export_docs", serde_json::to_value(&docs)?)?;
            } else if output == "-" {
                print!("{}", docs.markdown);
            } else {
                std::fs::write(output, &docs.markdown)
                    .with_context(|| format!("cannot write {output}"))?;
                println!(
                    "Wrote {} overview record(s) to {output} ({} stale)",
                    docs.record_count, docs.stale_count
                );
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

fn print_record(record: &atlas_session::OverviewRecord) {
    println!(
        "{} · {} ({})",
        record.subject_kind,
        subject_label(record),
        record.freshness
    );
    println!("Id       : {}", record.id);
    println!("Subject  : {}", record.subject_id);
    println!("Commit   : {}", record.commit_sha);
    println!("Confidence: {:.2}", record.confidence);
    if !record.title.is_empty() {
        println!("Title    : {}", record.title);
    }
    if !record.summary.is_empty() {
        println!("Summary  : {}", record.summary);
    }
    if !record.description_preview.is_empty() {
        println!("Preview  : {}", record.description_preview);
    }
    if let Some(source_id) = &record.source_id {
        println!("Evidence : source_id={source_id}");
    }
}

fn subject_label(record: &atlas_session::OverviewRecord) -> String {
    record
        .qualified_name
        .clone()
        .or_else(|| record.canonical_file_path.clone())
        .or_else(|| record.package_name.clone())
        .or_else(|| record.module_path.clone())
        .unwrap_or_else(|| record.subject_id.clone())
}

fn parse_kind(value: Option<&str>) -> Result<Option<OverviewSubjectKind>> {
    value
        .map(|value| value.parse().map_err(anyhow::Error::from))
        .transpose()
}

fn parse_freshness(value: Option<&str>) -> Result<Option<OverviewFreshness>> {
    value
        .map(|value| value.parse().map_err(anyhow::Error::from))
        .transpose()
}

fn parse_source_kind(value: Option<&str>) -> Result<Option<OverviewSourceKind>> {
    value
        .map(|value| value.parse().map_err(anyhow::Error::from))
        .transpose()
}
