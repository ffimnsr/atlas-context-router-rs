use anyhow::{Context, Result};
use atlas_history::status::HistoryStatus;
use atlas_history::{
    CommitSelector, HistoryRetentionPolicy, build_historical_graph_with_progress,
    compute_churn_report, diff_snapshots, prune_historical_graph, query_dependency_history,
    query_file_history_with_options, query_module_history, query_symbol_history,
    rebuild_historical_snapshot_with_progress, recompute_lifecycle,
    update_historical_graph_with_progress,
};

use crate::cli::{Cli, Command, HistoryCommand};
use crate::commands::print_json;

use super::context::HistoryContext;
use super::render::{
    HistoryBuildProgress, HistoryOutputMode, output_mode, print_churn_details,
    print_dependency_details, print_diff_details, print_file_details, print_module_details,
    print_symbol_details, print_warnings,
};

pub(crate) fn run_history_status(cli: &Cli) -> Result<()> {
    let ctx = HistoryContext::from_cli(cli)?;
    let summary = ctx
        .store
        .history_status(&ctx.canonical_root)
        .context("query history status")?;
    let status = HistoryStatus::from_summary(summary, ctx.is_shallow());

    if cli.json {
        print_json("history_status", serde_json::to_value(&status)?)?;
    } else {
        println!("indexed commits : {}", status.indexed_commit_count);
        println!("snapshots       : {}", status.snapshot_count);
        println!("partial         : {}", status.partial_snapshot_count);
        println!("parse errors    : {}", status.parse_error_snapshot_count);
        if let Some(sha) = &status.latest_commit_sha {
            let subject = status
                .latest_commit_subject
                .as_deref()
                .unwrap_or("(no subject)");
            let short = &sha[..sha.len().min(12)];
            println!("latest commit   : {short} {subject}");
        } else {
            println!("latest commit   : (none)");
        }
        println!(
            "latest ref      : {}",
            status.latest_indexed_ref.as_deref().unwrap_or("(none)")
        );
        for warning in &status.warnings {
            eprintln!("warning: {warning}");
        }
    }

    Ok(())
}

pub(crate) fn run_history_build(cli: &Cli) -> Result<()> {
    let sub = match &cli.command {
        Command::History { subcommand } => subcommand,
        _ => unreachable!(),
    };

    let (since, until, max_commits, branch, commits) = match sub {
        HistoryCommand::Build {
            since,
            until,
            max_commits,
            branch,
            commits,
        } => (since, until, max_commits, branch, commits),
        _ => unreachable!(),
    };

    let ctx = HistoryContext::from_cli(cli)?;
    let selector = if !commits.is_empty() {
        CommitSelector::Explicit {
            shas: commits.clone(),
        }
    } else {
        let start_ref = branch.clone().unwrap_or_else(|| "HEAD".to_owned());
        CommitSelector::Bounded {
            start_ref,
            max_commits: *max_commits,
            since: since.clone(),
            until: until.clone(),
        }
    };
    let registry = ctx.parser_registry();
    let selector_indexed_ref = selector.source_ref_label();
    let mut progress = HistoryBuildProgress::new(!cli.json);
    let summary = build_historical_graph_with_progress(
        ctx.repo_path(),
        &ctx.canonical_root,
        &ctx.store,
        &selector,
        &registry,
        selector_indexed_ref.as_deref(),
        |event| progress.observe(event),
    )
    .context("build historical graph")?;
    progress.finish();
    let lifecycle =
        recompute_lifecycle(&ctx.canonical_root, &ctx.store).context("recompute lifecycle")?;

    for error in &summary.errors {
        eprintln!("warning: {error}");
    }
    print_warnings(&summary.warnings);

    if cli.json {
        let value = serde_json::json!({
            "commits_processed": summary.commits_processed,
            "commits_already_indexed": summary.commits_already_indexed,
            "files_reused": summary.files_reused,
            "files_parsed": summary.files_parsed,
            "files_skipped": summary.files_skipped,
            "nodes_reused": summary.nodes_reused,
            "nodes_written": summary.nodes_written,
            "edges_written": summary.edges_written,
            "errors": summary.errors.len(),
            "warnings": summary.warnings,
            "node_history_rows": lifecycle.node_history_rows,
            "edge_history_rows": lifecycle.edge_history_rows,
            "elapsed_secs": summary.elapsed_secs,
        });
        print_json("history_build", value)?;
    } else {
        println!("commits processed : {}", summary.commits_processed);
        println!("commits skipped   : {}", summary.commits_already_indexed);
        println!("files parsed      : {}", summary.files_parsed);
        println!("files reused      : {}", summary.files_reused);
        println!("files skipped     : {}", summary.files_skipped);
        println!("nodes written     : {}", summary.nodes_written);
        println!("nodes reused      : {}", summary.nodes_reused);
        println!("edges written     : {}", summary.edges_written);
        println!("node history rows : {}", lifecycle.node_history_rows);
        println!("edge history rows : {}", lifecycle.edge_history_rows);
        println!("elapsed           : {:.2}s", summary.elapsed_secs);
        if !summary.errors.is_empty() {
            eprintln!("errors            : {}", summary.errors.len());
        }
    }

    Ok(())
}

pub(crate) fn run_history_update(cli: &Cli) -> Result<()> {
    let sub = match &cli.command {
        Command::History { subcommand } => subcommand,
        _ => unreachable!(),
    };

    let (branch, repair, max_commits) = match sub {
        HistoryCommand::Update {
            branch,
            repair,
            max_commits,
        } => (branch.as_deref(), *repair, *max_commits),
        _ => unreachable!(),
    };

    let ctx = HistoryContext::from_cli(cli)?;
    let registry = ctx.parser_registry();
    let mut progress = HistoryBuildProgress::new(!cli.json);
    let summary = update_historical_graph_with_progress(
        ctx.repo_path(),
        &ctx.canonical_root,
        &ctx.store,
        branch,
        repair,
        max_commits,
        &registry,
        |event| progress.observe(event),
    )
    .context("update historical graph")?;
    progress.finish();

    if cli.json {
        print_json("history_update", serde_json::to_value(&summary)?)?;
    } else {
        println!("branch            : {}", summary.branch);
        println!("head              : {}", summary.head_sha);
        println!(
            "indexed base      : {}",
            summary.indexed_base_sha.as_deref().unwrap_or("(none)")
        );
        println!(
            "latest indexed    : {}",
            summary.latest_indexed_sha.as_deref().unwrap_or("(none)")
        );
        println!("commits processed : {}", summary.commits_processed);
        println!(
            "divergence        : {}",
            if summary.divergence_detected {
                "yes"
            } else {
                "no"
            }
        );
        println!(
            "repair mode       : {}",
            if summary.repair_mode { "yes" } else { "no" }
        );
        println!(
            "node history rows : {}",
            summary.lifecycle.node_history_rows
        );
        println!(
            "edge history rows : {}",
            summary.lifecycle.edge_history_rows
        );
        println!("elapsed           : {:.2}s", summary.elapsed_secs);
        print_warnings(&summary.warnings);
    }

    Ok(())
}

pub(crate) fn run_history_rebuild(cli: &Cli) -> Result<()> {
    let commit_sha = match &cli.command {
        Command::History {
            subcommand: HistoryCommand::Rebuild { commit_sha },
        } => commit_sha.as_str(),
        _ => unreachable!(),
    };

    let ctx = HistoryContext::from_cli(cli)?;
    let registry = ctx.parser_registry();
    let mut progress = HistoryBuildProgress::new(!cli.json);
    let summary = rebuild_historical_snapshot_with_progress(
        ctx.repo_path(),
        &ctx.canonical_root,
        &ctx.store,
        commit_sha,
        &registry,
        Some(commit_sha),
        |event| progress.observe(event),
    )
    .context("rebuild historical snapshot")?;
    progress.finish();

    for error in &summary.build.errors {
        eprintln!("warning: {error}");
    }
    print_warnings(&summary.build.warnings);

    if cli.json {
        print_json("history_rebuild", serde_json::to_value(&summary)?)?;
    } else {
        println!("commit            : {}", summary.commit_sha);
        println!("replaced snapshot : {}", summary.replaced_snapshot_id);
        println!("rebuilt snapshot  : {}", summary.rebuilt_snapshot_id);
        println!("files parsed      : {}", summary.build.files_parsed);
        println!("files reused      : {}", summary.build.files_reused);
        println!("files skipped     : {}", summary.build.files_skipped);
        println!("nodes written     : {}", summary.build.nodes_written);
        println!("nodes reused      : {}", summary.build.nodes_reused);
        println!("edges written     : {}", summary.build.edges_written);
        println!("reclaimed hashes  : {}", summary.reclaimed_file_hashes);
        println!("reclaimed nodes   : {}", summary.reclaimed_historical_nodes);
        println!("reclaimed edges   : {}", summary.reclaimed_historical_edges);
        println!(
            "node history rows : {}",
            summary.lifecycle.node_history_rows
        );
        println!(
            "edge history rows : {}",
            summary.lifecycle.edge_history_rows
        );
        println!("elapsed           : {:.2}s", summary.build.elapsed_secs);
    }

    Ok(())
}

pub(crate) fn run_history_diff(cli: &Cli) -> Result<()> {
    let (commit_a, commit_b, stat_only, full) = match &cli.command {
        Command::History {
            subcommand:
                HistoryCommand::Diff {
                    commit_a,
                    commit_b,
                    stat_only,
                    full,
                },
        } => (commit_a.as_str(), commit_b.as_str(), *stat_only, *full),
        _ => unreachable!(),
    };

    let ctx = HistoryContext::from_cli(cli)?;
    let report = diff_snapshots(
        ctx.repo_path(),
        &ctx.store,
        &ctx.canonical_root,
        commit_a,
        commit_b,
    )
    .context("diff historical snapshots")?;

    if cli.json {
        if stat_only {
            print_json("history_diff", serde_json::to_value(&report.summary)?)?;
        } else {
            print_json("history_diff", serde_json::to_value(&report)?)?;
        }
    } else {
        println!("commit a          : {}", report.commit_a);
        println!("commit b          : {}", report.commit_b);
        println!("files added       : {}", report.summary.added_file_count);
        println!("files removed     : {}", report.summary.removed_file_count);
        println!("files modified    : {}", report.summary.modified_file_count);
        println!("files renamed     : {}", report.summary.renamed_file_count);
        println!("nodes added       : {}", report.summary.added_node_count);
        println!("nodes removed     : {}", report.summary.removed_node_count);
        println!("nodes changed     : {}", report.summary.changed_node_count);
        println!("edges added       : {}", report.summary.added_edge_count);
        println!("edges removed     : {}", report.summary.removed_edge_count);
        println!("edges changed     : {}", report.summary.changed_edge_count);
        println!("module changes    : {}", report.summary.module_change_count);
        println!("new cycles        : {}", report.summary.new_cycle_count);
        println!("broken cycles     : {}", report.summary.broken_cycle_count);
        if output_mode(stat_only, full) == HistoryOutputMode::Full {
            println!(
                "snapshot a        : nodes={} edges={} files={} completeness={:.3} parse_errors={}",
                report.snapshot_a.node_count,
                report.snapshot_a.edge_count,
                report.snapshot_a.file_count,
                report.snapshot_a.completeness,
                report.snapshot_a.parse_error_count
            );
            println!(
                "snapshot b        : nodes={} edges={} files={} completeness={:.3} parse_errors={}",
                report.snapshot_b.node_count,
                report.snapshot_b.edge_count,
                report.snapshot_b.file_count,
                report.snapshot_b.completeness,
                report.snapshot_b.parse_error_count
            );
            print_diff_details(&report);
        }
    }

    Ok(())
}

pub(crate) fn run_history_symbol(cli: &Cli) -> Result<()> {
    let (qualified_name, stat_only, full) = match &cli.command {
        Command::History {
            subcommand:
                HistoryCommand::Symbol {
                    qualified_name,
                    stat_only,
                    full,
                },
        } => (qualified_name.as_str(), *stat_only, *full),
        _ => unreachable!(),
    };

    let ctx = HistoryContext::from_cli(cli)?;
    let report = query_symbol_history(&ctx.store, &ctx.canonical_root, qualified_name)
        .context("query symbol history")?;

    if cli.json {
        if stat_only {
            print_json("history_symbol", serde_json::to_value(&report.summary)?)?;
        } else {
            print_json("history_symbol", serde_json::to_value(&report)?)?;
        }
    } else {
        println!("symbol            : {}", report.qualified_name);
        println!(
            "first appearance  : {}",
            report
                .summary
                .first_appearance_commit_sha
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "last appearance   : {}",
            report
                .summary
                .last_appearance_commit_sha
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "removal commit    : {}",
            report
                .summary
                .removal_commit_sha
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "change commits    : {}",
            report.findings.commits_where_changed.len()
        );
        println!(
            "signature versions: {}",
            report.findings.signature_evolution.len()
        );
        println!(
            "file path changes : {}",
            report.findings.file_path_changes.len()
        );
        if output_mode(stat_only, full) == HistoryOutputMode::Full {
            print_symbol_details(&report);
        }
    }

    Ok(())
}

pub(crate) fn run_history_file(cli: &Cli) -> Result<()> {
    let (path, follow_renames, stat_only, full) = match &cli.command {
        Command::History {
            subcommand:
                HistoryCommand::File {
                    path,
                    follow_renames,
                    stat_only,
                    full,
                },
        } => (path.as_str(), *follow_renames, *stat_only, *full),
        _ => unreachable!(),
    };

    let ctx = HistoryContext::from_cli(cli)?;
    let report = query_file_history_with_options(
        &ctx.store,
        &ctx.canonical_root,
        ctx.repo_path(),
        path,
        follow_renames,
    )
    .context("query file history")?;

    if cli.json {
        if stat_only {
            print_json("history_file", serde_json::to_value(&report.summary)?)?;
        } else {
            print_json("history_file", serde_json::to_value(&report)?)?;
        }
    } else {
        println!("file              : {}", report.file_path);
        println!(
            "first appearance  : {}",
            report
                .summary
                .first_appearance_commit_sha
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "last appearance   : {}",
            report
                .summary
                .last_appearance_commit_sha
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "removal commit    : {}",
            report
                .summary
                .removal_commit_sha
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "commits touched   : {}",
            report.findings.commits_touched.len()
        );
        println!("timeline points   : {}", report.findings.timeline.len());
        if output_mode(stat_only, full) == HistoryOutputMode::Full {
            print_file_details(&report);
        }
    }

    Ok(())
}

pub(crate) fn run_history_dependency(cli: &Cli) -> Result<()> {
    let (source, target, stat_only, full) = match &cli.command {
        Command::History {
            subcommand:
                HistoryCommand::Dependency {
                    source,
                    target,
                    stat_only,
                    full,
                },
        } => (source.as_str(), target.as_str(), *stat_only, *full),
        _ => unreachable!(),
    };

    let ctx = HistoryContext::from_cli(cli)?;
    let report = query_dependency_history(&ctx.store, &ctx.canonical_root, source, target)
        .context("query dependency history")?;

    if cli.json {
        if stat_only {
            print_json("history_dependency", serde_json::to_value(&report.summary)?)?;
        } else {
            print_json("history_dependency", serde_json::to_value(&report)?)?;
        }
    } else {
        println!("source            : {}", report.summary.source_qn);
        println!("target            : {}", report.summary.target_qn);
        println!(
            "first appearance  : {}",
            report
                .summary
                .first_appearance_commit_sha
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "last appearance   : {}",
            report
                .summary
                .last_appearance_commit_sha
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "disappearance     : {}",
            report
                .summary
                .disappearance_commit_sha
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "change commits    : {}",
            report.findings.commits_where_changed.len()
        );
        println!(
            "persistence secs  : {}",
            report.summary.persistence_duration_secs.unwrap_or_default()
        );
        if output_mode(stat_only, full) == HistoryOutputMode::Full {
            print_dependency_details(&report);
        }
    }

    Ok(())
}

pub(crate) fn run_history_module(cli: &Cli) -> Result<()> {
    let (module, stat_only, full) = match &cli.command {
        Command::History {
            subcommand:
                HistoryCommand::Module {
                    module,
                    stat_only,
                    full,
                },
        } => (module.as_str(), *stat_only, *full),
        _ => unreachable!(),
    };

    let ctx = HistoryContext::from_cli(cli)?;
    let report = query_module_history(&ctx.store, &ctx.canonical_root, module)
        .context("query module history")?;

    if cli.json {
        if stat_only {
            print_json("history_module", serde_json::to_value(&report.summary)?)?;
        } else {
            print_json("history_module", serde_json::to_value(&report)?)?;
        }
    } else {
        println!("module            : {}", report.module);
        println!(
            "first appearance  : {}",
            report
                .summary
                .first_appearance_commit_sha
                .as_deref()
                .unwrap_or("(none)")
        );
        println!(
            "last appearance   : {}",
            report
                .summary
                .last_appearance_commit_sha
                .as_deref()
                .unwrap_or("(none)")
        );
        println!("max nodes         : {}", report.summary.max_node_count);
        println!(
            "max dependencies  : {}",
            report.summary.max_dependency_count
        );
        println!("max coupling      : {}", report.summary.max_coupling_count);
        println!(
            "max test adjacent : {}",
            report.summary.max_test_adjacency_count
        );
        println!("timeline points   : {}", report.findings.timeline.len());
        if output_mode(stat_only, full) == HistoryOutputMode::Full {
            print_module_details(&report);
        }
    }

    Ok(())
}

pub(crate) fn run_history_churn(cli: &Cli) -> Result<()> {
    let (stat_only, full) = match &cli.command {
        Command::History {
            subcommand: HistoryCommand::Churn { stat_only, full },
        } => (*stat_only, *full),
        _ => unreachable!(),
    };

    let ctx = HistoryContext::from_cli(cli)?;
    let report = compute_churn_report(&ctx.store, &ctx.canonical_root, &ctx.db)
        .context("compute history churn report")?;

    if cli.json {
        if stat_only {
            print_json("history_churn", serde_json::to_value(&report.summary)?)?;
        } else {
            print_json("history_churn", serde_json::to_value(&report)?)?;
        }
    } else {
        println!("snapshots         : {}", report.summary.snapshot_count);
        println!("symbols           : {}", report.summary.symbol_count);
        println!("files             : {}", report.summary.file_count);
        println!("modules           : {}", report.summary.module_count);
        println!("stable symbols    : {}", report.summary.stable_symbol_count);
        println!(
            "unstable symbols  : {}",
            report.summary.unstable_symbol_count
        );
        println!(
            "dependency churn  : {}",
            report.summary.frequently_changing_dependency_count
        );
        println!("hotspots          : {}", report.summary.hotspot_count);
        println!("file growth       : {}", report.trends.file_count_growth);
        println!("node growth       : {}", report.trends.node_count_growth);
        println!("edge growth       : {}", report.trends.edge_count_growth);
        println!("cycle growth      : {}", report.trends.cycle_count_growth);
        println!(
            "dedup ratio       : {:.2}",
            report.storage_diagnostics.deduplication_ratio
        );
        println!(
            "db size bytes     : {}",
            report.storage_diagnostics.db_size_bytes
        );
        if output_mode(stat_only, full) == HistoryOutputMode::Full {
            print_churn_details(&report);
        }
    }

    Ok(())
}

pub(crate) fn run_history_prune(cli: &Cli) -> Result<()> {
    let (keep_all, keep_latest, older_than_days, keep_tagged_only, keep_weekly) = match &cli.command
    {
        Command::History {
            subcommand:
                HistoryCommand::Prune {
                    keep_all,
                    keep_latest,
                    older_than_days,
                    keep_tagged_only,
                    keep_weekly,
                },
        } => (
            *keep_all,
            *keep_latest,
            *older_than_days,
            *keep_tagged_only,
            *keep_weekly,
        ),
        _ => unreachable!(),
    };

    let ctx = HistoryContext::from_cli(cli)?;
    let summary = prune_historical_graph(
        ctx.repo_path(),
        &ctx.canonical_root,
        &ctx.store,
        &HistoryRetentionPolicy {
            keep_all,
            keep_latest,
            older_than_days,
            keep_tagged_only,
            keep_weekly,
        },
    )
    .context("prune historical graph")?;

    if cli.json {
        print_json("history_prune", serde_json::to_value(&summary)?)?;
    } else {
        println!("commits before    : {}", summary.commits_before);
        println!("commits after     : {}", summary.commits_after);
        println!("snapshots before  : {}", summary.snapshots_before);
        println!("snapshots after   : {}", summary.snapshots_after);
        println!("deleted commits   : {}", summary.deleted_commit_shas.len());
        println!("deleted snapshots : {}", summary.deleted_snapshot_ids.len());
        println!("reclaimed hashes  : {}", summary.reclaimed_file_hashes);
        println!(
            "node history rows : {}",
            summary.lifecycle.node_history_rows
        );
        println!(
            "edge history rows : {}",
            summary.lifecycle.edge_history_rows
        );
    }

    Ok(())
}
