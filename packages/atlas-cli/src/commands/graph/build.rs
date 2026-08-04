use crate::cli::{Cli, Command};
use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter};
use atlas_engine::{BuildOptions, build_graph};
use atlas_repo::{
    RepoRegistration, RepoRegistry, RepoRelationshipKind, find_repo_root,
    phase1_multi_repo_supported, stable_repo_id,
};
use atlas_store_sqlite::{BuildFinishStats, Store};
use camino::Utf8Path;

use super::super::{db_path, print_json, resolve_repo};
use super::{MultiRepoBudgetAggregate, print_summary_value};

pub(super) const MAX_MULTI_REPO_SELECTION: usize = 32;
fn excluded_manual_repo_count(registry: &RepoRegistry) -> usize {
    registry
        .registrations
        .iter()
        .filter(|entry| entry.enabled)
        .filter(|entry| entry.relationship.kind == RepoRelationshipKind::Manual)
        .count()
}
pub fn run_build(cli: &Cli) -> Result<()> {
    let (fail_fast, dry_run, selected_repo_id, all_repos) = match &cli.command {
        Command::Build {
            fail_fast,
            dry_run,
            repo_id,
            all_repos,
        } => (*fail_fast, *dry_run, repo_id.clone(), *all_repos),
        _ => unreachable!(),
    };
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("build");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let db_path = db_path(cli, &repo);

        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
        let build_budget = config.build_run_budget()?;

        if all_repos || selected_repo_id.is_some() {
            return run_registered_builds(
                cli,
                Utf8Path::new(&repo),
                &db_path,
                &config,
                fail_fast,
                dry_run,
                selected_repo_id.as_deref(),
                all_repos,
            );
        }

        let source_repo_id = stable_repo_id(repo_root_path.as_path());

        // Record lifecycle: building.
        if !dry_run && let Ok(store) = Store::open(&db_path) {
            let _ = store.begin_build_for_repo(&source_repo_id, repo_root_path.as_str());
        }

        let build_result = build_graph(
            repo_root_path.as_path(),
            &db_path,
            &BuildOptions {
                fail_fast,
                dry_run,
                batch_size: config.parse_batch_size(),
                budget: build_budget,
                source_repo_id: Some(source_repo_id.clone()),
                namespace_qualified_names: false,
            },
        );

        // Record lifecycle: built or build_failed.
        if !dry_run && let Ok(store) = Store::open(&db_path) {
            match &build_result {
                Ok(s) => {
                    let state =
                        if matches!(s.budget.budget_status, atlas_core::BudgetStatus::Blocked) {
                            atlas_store_sqlite::GraphBuildState::BuildFailed
                        } else if s.is_degraded() {
                            atlas_store_sqlite::GraphBuildState::Degraded
                        } else {
                            atlas_store_sqlite::GraphBuildState::Built
                        };
                    let _ = store.finish_build_for_repo(
                        &source_repo_id,
                        repo_root_path.as_str(),
                        BuildFinishStats {
                            state,
                            files_discovered: s.scanned as i64,
                            files_processed: s.parsed as i64,
                            files_accepted: s.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: s
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: s.parse_errors as i64,
                            bytes_accepted: s.budget_counters.bytes_accepted as i64,
                            bytes_skipped: s.budget_counters.bytes_skipped as i64,
                            nodes_written: s.nodes_inserted as i64,
                            edges_written: s.edges_inserted as i64,
                            budget_stop_reason: s.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                }
                Err(e) => {
                    let _ = store.fail_build_for_repo(
                        &source_repo_id,
                        repo_root_path.as_str(),
                        &e.to_string(),
                    );
                }
            }
        }

        let summary = build_result?;

        if cli.json {
            print_json(
                "build",
                serde_json::json!({
                    "dry_run": dry_run,
                    "scanned": summary.scanned,
                    "skipped_unsupported": summary.skipped_unsupported,
                    "skipped_unchanged": summary.skipped_unchanged,
                    "parsed": summary.parsed,
                    "parse_errors": summary.parse_errors,
                    "chunk_upsert_failures": summary.chunk_upsert_failures,
                    "call_target_reconcile_failures": summary.call_target_reconcile_failures,
                    "nodes_inserted": summary.nodes_inserted,
                    "edges_inserted": summary.edges_inserted,
                    "warnings": summary.warnings,
                    "budget": summary.budget,
                    "budget_counters": summary.budget_counters,
                    "elapsed_ms": summary.elapsed_ms,
                    "nodes_per_sec": if summary.elapsed_ms > 0 {
                        (summary.nodes_inserted as f64 / summary.elapsed_ms as f64 * 1000.0).round() as u64
                    } else { summary.nodes_inserted as u64 },
                }),
            )?;
        } else {
            let nodes_per_sec = if summary.elapsed_ms > 0 {
                format!(
                    "{:.0} nodes/s",
                    summary.nodes_inserted as f64 / summary.elapsed_ms as f64 * 1000.0
                )
            } else {
                String::from("—")
            };
            println!(
                "{} ({:.2}s, {nodes_per_sec})",
                if dry_run {
                    "Build dry run complete"
                } else {
                    "Build complete"
                },
                summary.elapsed_ms as f64 / 1000.0
            );
            print_summary_value("Scanned", summary.scanned);
            print_summary_value("Unsupported skipped", summary.skipped_unsupported);
            print_summary_value("Unchanged skipped", summary.skipped_unchanged);
            print_summary_value("Parsed", summary.parsed);
            if summary.parse_errors > 0 {
                print_summary_value("Errors", summary.parse_errors);
            }
            if summary.chunk_upsert_failures > 0 {
                print_summary_value("Chunk indexing failures", summary.chunk_upsert_failures);
            }
            if summary.call_target_reconcile_failures > 0 {
                print_summary_value(
                    "Call-target reconcile failures",
                    summary.call_target_reconcile_failures,
                );
            }
            print_summary_value("Files accepted", summary.budget_counters.files_accepted);
            if summary.budget_counters.files_skipped_by_byte_budget > 0 {
                print_summary_value(
                    "Byte-budget skipped",
                    summary.budget_counters.files_skipped_by_byte_budget,
                );
            }
            print_summary_value("Nodes inserted", summary.nodes_inserted);
            print_summary_value("Edges inserted", summary.edges_inserted);
            if let Some(reason) = &summary.budget_counters.budget_stop_reason {
                print_summary_value("Budget stop reason", reason);
            }
            for warning in &summary.warnings {
                print_summary_value("Warning", warning);
            }
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("build", result.is_ok());
    }
    result
}

pub(super) fn enabled_registration_targets(
    registry_root: &Utf8Path,
    selected_repo_id: Option<&str>,
    all_repos: bool,
) -> Result<(Vec<RepoRegistration>, usize)> {
    let registry = RepoRegistry::load(registry_root)
        .with_context(|| "repo registry missing; run `atlas init` or `atlas repo sync` first")?;
    let excluded_manual = if all_repos {
        excluded_manual_repo_count(&registry)
    } else {
        0
    };
    let mut registrations: Vec<RepoRegistration> = registry
        .registrations
        .into_iter()
        .filter(|entry| entry.enabled)
        .collect();
    if let Some(repo_id) = selected_repo_id {
        registrations.retain(|entry| entry.repo_id == repo_id);
        anyhow::ensure!(
            !registrations.is_empty(),
            "enabled repo id '{repo_id}' is not registered"
        );
    } else if all_repos {
        registrations.retain(|entry| phase1_multi_repo_supported(entry.relationship.kind));
        anyhow::ensure!(
            registrations.len() <= MAX_MULTI_REPO_SELECTION,
            "all_repos scope exceeds max supported repo fan-out ({MAX_MULTI_REPO_SELECTION})"
        );
    } else {
        registrations.retain(|entry| entry.relationship.kind == RepoRelationshipKind::Root);
    }
    Ok((registrations, excluded_manual))
}

fn build_state_for_summary(
    summary: &atlas_engine::BuildSummary,
) -> atlas_store_sqlite::GraphBuildState {
    if matches!(
        summary.budget.budget_status,
        atlas_core::BudgetStatus::Blocked
    ) {
        atlas_store_sqlite::GraphBuildState::BuildFailed
    } else if summary.is_degraded() {
        atlas_store_sqlite::GraphBuildState::Degraded
    } else {
        atlas_store_sqlite::GraphBuildState::Built
    }
}

#[allow(clippy::too_many_arguments)]
fn run_registered_builds(
    cli: &Cli,
    registry_root: &Utf8Path,
    db_path: &str,
    config: &atlas_engine::Config,
    fail_fast: bool,
    dry_run: bool,
    selected_repo_id: Option<&str>,
    all_repos: bool,
) -> Result<()> {
    let (registrations, excluded_manual) =
        enabled_registration_targets(registry_root, selected_repo_id, all_repos)?;
    let build_budget = config.build_run_budget()?;
    let mut results = Vec::new();
    let mut aggregate = MultiRepoBudgetAggregate {
        selected_repo_count: registrations.len(),
        excluded_manual_repo_count: excluded_manual,
        ..MultiRepoBudgetAggregate::default()
    };

    for registration in registrations {
        if !registration.root.exists() {
            aggregate.failed_repo_count += 1;
            aggregate.skipped_repo_count += 1;
            results.push(serde_json::json!({
                "repo_id": registration.repo_id,
                "display_alias": registration.display_alias,
                "status": "skipped",
                "ok": false,
                "error": "repo root missing",
            }));
            continue;
        }
        if !dry_run && let Ok(store) = Store::open(db_path) {
            let _ = store.begin_build_for_repo(&registration.repo_id, registration.root.as_str());
        }
        let result = build_graph(
            registration.root.as_path(),
            db_path,
            &BuildOptions {
                fail_fast,
                dry_run,
                batch_size: config.parse_batch_size(),
                budget: build_budget,
                source_repo_id: Some(registration.repo_id.clone()),
                namespace_qualified_names: registration.relationship.kind
                    != RepoRelationshipKind::Root,
            },
        );
        match result {
            Ok(summary) => {
                if !dry_run && let Ok(store) = Store::open(db_path) {
                    let _ = store.finish_build_for_repo(
                        &registration.repo_id,
                        registration.root.as_str(),
                        BuildFinishStats {
                            state: build_state_for_summary(&summary),
                            files_discovered: summary.scanned as i64,
                            files_processed: summary.parsed as i64,
                            files_accepted: summary.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: summary
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: summary.parse_errors as i64,
                            bytes_accepted: summary.budget_counters.bytes_accepted as i64,
                            bytes_skipped: summary.budget_counters.bytes_skipped as i64,
                            nodes_written: summary.nodes_inserted as i64,
                            edges_written: summary.edges_inserted as i64,
                            budget_stop_reason: summary.budget_counters.budget_stop_reason.clone(),
                        },
                    );
                }
                aggregate.processed_repo_count += 1;
                if summary.budget.budget_hit {
                    aggregate.budget_hit_repo_count += 1;
                }
                aggregate.files_accepted += summary.budget_counters.files_accepted;
                aggregate.files_skipped_by_byte_budget +=
                    summary.budget_counters.files_skipped_by_byte_budget;
                aggregate.bytes_accepted += summary.budget_counters.bytes_accepted;
                aggregate.bytes_skipped += summary.budget_counters.bytes_skipped;
                results.push(serde_json::json!({
                    "repo_id": registration.repo_id,
                    "display_alias": registration.display_alias,
                    "status": if summary.is_degraded() { "degraded" } else { "ok" },
                    "ok": true,
                    "parsed": summary.parsed,
                    "nodes_inserted": summary.nodes_inserted,
                    "edges_inserted": summary.edges_inserted,
                    "budget_status": summary.budget.budget_status,
                    "budget_hit": summary.budget.budget_hit,
                    "files_accepted": summary.budget_counters.files_accepted,
                    "files_skipped_by_byte_budget": summary.budget_counters.files_skipped_by_byte_budget,
                    "bytes_accepted": summary.budget_counters.bytes_accepted,
                    "bytes_skipped": summary.budget_counters.bytes_skipped,
                    "budget_stop_reason": summary.budget_counters.budget_stop_reason,
                    "warnings": summary.warnings,
                }));
            }
            Err(error) => {
                aggregate.failed_repo_count += 1;
                if !dry_run && let Ok(store) = Store::open(db_path) {
                    let _ = store.fail_build_for_repo(
                        &registration.repo_id,
                        registration.root.as_str(),
                        &error.to_string(),
                    );
                }
                results.push(serde_json::json!({
                    "repo_id": registration.repo_id,
                    "display_alias": registration.display_alias,
                    "status": "error",
                    "ok": false,
                    "error": error.to_string(),
                }));
            }
        }
    }

    if cli.json {
        print_json(
            "build",
            serde_json::json!({
                "dry_run": dry_run,
                "partial_success": aggregate.failed_repo_count > 0,
                "repo_scope": {
                    "selected_repo_count": aggregate.selected_repo_count,
                    "processed_repo_count": aggregate.processed_repo_count,
                    "failed_repo_count": aggregate.failed_repo_count,
                    "skipped_repo_count": aggregate.skipped_repo_count,
                    "excluded_manual_repo_count": aggregate.excluded_manual_repo_count,
                },
                "budget_summary": {
                    "budget_hit_repo_count": aggregate.budget_hit_repo_count,
                    "files_accepted": aggregate.files_accepted,
                    "files_skipped_by_byte_budget": aggregate.files_skipped_by_byte_budget,
                    "bytes_accepted": aggregate.bytes_accepted,
                    "bytes_skipped": aggregate.bytes_skipped,
                },
                "repos": results,
            }),
        )
    } else {
        println!(
            "Build complete for {} repo(s); processed={} failures={} skipped={}",
            aggregate.selected_repo_count,
            aggregate.processed_repo_count,
            aggregate.failed_repo_count,
            aggregate.skipped_repo_count,
        );
        if aggregate.excluded_manual_repo_count > 0 {
            println!(
                "Phase-1 rollout: excluded {} manual repo(s) from --all-repos. Use --repo-id to target them explicitly.",
                aggregate.excluded_manual_repo_count
            );
        }
        println!(
            "Budget summary: accepted_files={} skipped_files={} accepted_bytes={} skipped_bytes={} budget_hit_repos={}",
            aggregate.files_accepted,
            aggregate.files_skipped_by_byte_budget,
            aggregate.bytes_accepted,
            aggregate.bytes_skipped,
            aggregate.budget_hit_repo_count,
        );
        for result in results {
            println!("  {}", result);
        }
        Ok(())
    }
}
