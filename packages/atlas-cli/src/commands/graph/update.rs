use crate::cli::{Cli, Command};
use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter};
use atlas_engine::{UpdateOptions, UpdateTarget, update_graph};
use atlas_repo::{RepoRelationshipKind, changed_files, find_repo_root, stable_repo_id};
use atlas_store_sqlite::{BuildFinishStats, Store};
use camino::Utf8Path;

use super::super::{db_path, print_json, resolve_repo};
use super::build::enabled_registration_targets;
use super::{MultiRepoBudgetAggregate, print_summary_value};

fn update_state_for_summary(
    summary: &atlas_engine::UpdateSummary,
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
fn run_registered_updates(
    cli: &Cli,
    registry_root: &Utf8Path,
    db_path: &str,
    config: &atlas_engine::Config,
    fail_fast: bool,
    dry_run: bool,
    selected_repo_id: Option<&str>,
    all_repos: bool,
    affected_repos: bool,
) -> Result<()> {
    let (registrations, excluded_manual) =
        enabled_registration_targets(registry_root, selected_repo_id, all_repos || affected_repos)?;
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
        let target = match &cli.command {
            Command::Update {
                base,
                staged,
                files,
                ..
            } => {
                if !files.is_empty() && selected_repo_id.is_some() {
                    UpdateTarget::Files(files.clone())
                } else if *staged {
                    UpdateTarget::Staged
                } else if let Some(base_ref) = base {
                    UpdateTarget::BaseRef(base_ref.clone())
                } else {
                    UpdateTarget::WorkingTree
                }
            }
            _ => UpdateTarget::WorkingTree,
        };

        if affected_repos {
            let diff_target = match &target {
                UpdateTarget::Staged => atlas_repo::DiffTarget::Staged,
                UpdateTarget::BaseRef(base) => atlas_repo::DiffTarget::BaseRef(base.clone()),
                _ => atlas_repo::DiffTarget::WorkingTree,
            };
            let changes = match changed_files(registration.root.as_path(), &diff_target) {
                Ok(changes) => changes,
                Err(error) => {
                    aggregate.failed_repo_count += 1;
                    aggregate.skipped_repo_count += 1;
                    results.push(serde_json::json!({
                        "repo_id": registration.repo_id,
                        "display_alias": registration.display_alias,
                        "status": "skipped",
                        "ok": false,
                        "error": error.to_string(),
                    }));
                    continue;
                }
            };
            if changes.is_empty() {
                aggregate.skipped_repo_count += 1;
                results.push(serde_json::json!({
                    "repo_id": registration.repo_id,
                    "display_alias": registration.display_alias,
                    "status": "skipped",
                    "ok": true,
                    "skipped": "no_changes",
                }));
                continue;
            }
        }

        if !dry_run && let Ok(store) = Store::open(db_path) {
            let _ = store.begin_build_for_repo(&registration.repo_id, registration.root.as_str());
        }
        let result = update_graph(
            registration.root.as_path(),
            db_path,
            &UpdateOptions {
                fail_fast,
                dry_run,
                batch_size: config.parse_batch_size(),
                target,
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
                            state: update_state_for_summary(&summary),
                            files_discovered: (summary.parsed + summary.deleted + summary.renamed)
                                as i64,
                            files_processed: summary.parsed as i64,
                            files_accepted: summary.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: summary
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: summary.parse_errors as i64,
                            bytes_accepted: summary.budget_counters.bytes_accepted as i64,
                            bytes_skipped: summary.budget_counters.bytes_skipped as i64,
                            nodes_written: summary.nodes_updated as i64,
                            edges_written: summary.edges_updated as i64,
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
                    "deleted": summary.deleted,
                    "renamed": summary.renamed,
                    "nodes_updated": summary.nodes_updated,
                    "edges_updated": summary.edges_updated,
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
            "update",
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
            "Update complete for {} repo(s); processed={} failures={} skipped={}",
            aggregate.selected_repo_count,
            aggregate.processed_repo_count,
            aggregate.failed_repo_count,
            aggregate.skipped_repo_count,
        );
        if aggregate.excluded_manual_repo_count > 0 {
            println!(
                "Phase-1 rollout: excluded {} manual repo(s) from broad multi-repo update. Use --repo-id to target them explicitly.",
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

pub fn run_update(cli: &Cli) -> Result<()> {
    let (fail_fast, dry_run, selected_repo_id, all_repos, affected_repos) = match &cli.command {
        Command::Update {
            fail_fast,
            dry_run,
            repo_id,
            all_repos,
            affected_repos,
            ..
        } => (
            *fail_fast,
            *dry_run,
            repo_id.clone(),
            *all_repos,
            *affected_repos,
        ),
        _ => unreachable!(),
    };
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("update");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let db_path = db_path(cli, &repo);

        let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo))?;
        let build_budget = config.build_run_budget()?;

        if all_repos || affected_repos || selected_repo_id.is_some() {
            return run_registered_updates(
                cli,
                Utf8Path::new(&repo),
                &db_path,
                &config,
                fail_fast,
                dry_run,
                selected_repo_id.as_deref(),
                all_repos,
                affected_repos,
            );
        }

        let explicit_files: Vec<String> = match &cli.command {
            Command::Update { files, .. } => files.clone(),
            _ => vec![],
        };

        let target = if !explicit_files.is_empty() {
            UpdateTarget::Files(explicit_files)
        } else {
            match &cli.command {
                Command::Update { base, staged, .. } => {
                    if *staged {
                        UpdateTarget::Staged
                    } else if let Some(base_ref) = base {
                        UpdateTarget::BaseRef(base_ref.clone())
                    } else {
                        UpdateTarget::WorkingTree
                    }
                }
                _ => UpdateTarget::WorkingTree,
            }
        };

        let source_repo_id = stable_repo_id(repo_root_path.as_path());

        // Record lifecycle: building.
        if !dry_run && let Ok(store) = Store::open(&db_path) {
            let _ = store.begin_build_for_repo(&source_repo_id, repo_root_path.as_str());
        }

        let update_result = update_graph(
            repo_root_path.as_path(),
            &db_path,
            &UpdateOptions {
                fail_fast,
                dry_run,
                batch_size: config.parse_batch_size(),
                target,
                budget: build_budget,
                source_repo_id: Some(source_repo_id.clone()),
                namespace_qualified_names: false,
            },
        );

        // Record lifecycle: built or build_failed.
        if !dry_run && let Ok(store) = Store::open(&db_path) {
            match &update_result {
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
                            files_discovered: (s.parsed + s.deleted + s.renamed) as i64,
                            files_processed: s.parsed as i64,
                            files_accepted: s.budget_counters.files_accepted as i64,
                            files_skipped_by_byte_budget: s
                                .budget_counters
                                .files_skipped_by_byte_budget
                                as i64,
                            files_failed: s.parse_errors as i64,
                            bytes_accepted: s.budget_counters.bytes_accepted as i64,
                            bytes_skipped: s.budget_counters.bytes_skipped as i64,
                            nodes_written: s.nodes_updated as i64,
                            edges_written: s.edges_updated as i64,
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

        let summary = update_result?;

        if cli.json {
            print_json(
                "update",
                serde_json::json!({
                    "dry_run": dry_run,
                    "deleted": summary.deleted,
                    "renamed": summary.renamed,
                    "parsed": summary.parsed,
                    "skipped_unsupported": summary.skipped_unsupported,
                    "parse_errors": summary.parse_errors,
                    "chunk_upsert_failures": summary.chunk_upsert_failures,
                    "call_target_reconcile_failures": summary.call_target_reconcile_failures,
                    "nodes_updated": summary.nodes_updated,
                    "edges_updated": summary.edges_updated,
                    "warnings": summary.warnings,
                    "budget": summary.budget,
                    "budget_counters": summary.budget_counters,
                    "elapsed_ms": summary.elapsed_ms,
                    "nodes_per_sec": if summary.elapsed_ms > 0 {
                        (summary.nodes_updated as f64 / summary.elapsed_ms as f64 * 1000.0).round() as u64
                    } else { summary.nodes_updated as u64 },
                }),
            )?;
        } else {
            let nodes_per_sec = if summary.elapsed_ms > 0 {
                format!(
                    "{:.0} nodes/s",
                    summary.nodes_updated as f64 / summary.elapsed_ms as f64 * 1000.0
                )
            } else {
                String::from("—")
            };
            println!(
                "{} ({:.2}s, {nodes_per_sec})",
                if dry_run {
                    "Update dry run complete"
                } else {
                    "Update complete"
                },
                summary.elapsed_ms as f64 / 1000.0
            );
            print_summary_value("Deleted", summary.deleted);
            if summary.renamed > 0 {
                print_summary_value("Renamed", summary.renamed);
            }
            print_summary_value("Parsed", summary.parsed);
            if summary.skipped_unsupported > 0 {
                print_summary_value("Unsupported skipped", summary.skipped_unsupported);
            }
            print_summary_value("Files accepted", summary.budget_counters.files_accepted);
            if summary.budget_counters.files_skipped_by_byte_budget > 0 {
                print_summary_value(
                    "Byte-budget skipped",
                    summary.budget_counters.files_skipped_by_byte_budget,
                );
            }
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
            print_summary_value("Nodes", summary.nodes_updated);
            print_summary_value("Edges", summary.edges_updated);
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
        a.after_command("update", result.is_ok());
    }
    result
}
