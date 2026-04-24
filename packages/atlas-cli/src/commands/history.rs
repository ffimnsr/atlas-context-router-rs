use anyhow::{Context, Result};
use atlas_history::git;
use atlas_history::status::HistoryStatus;
use atlas_history::{
    CommitSelector, HistoryRetentionPolicy, build_historical_graph, compute_churn_report,
    diff_snapshots, prune_historical_graph, query_dependency_history,
    query_file_history_with_options, query_module_history, query_symbol_history,
    rebuild_historical_snapshot, recompute_lifecycle, update_historical_graph,
};
use atlas_parser::ParserRegistry;
use atlas_store_sqlite::Store;

use crate::cli::{Cli, Command, HistoryCommand};

use super::{db_path, print_json, resolve_repo};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HistoryOutputMode {
    Summary,
    Full,
}

pub fn run_history(cli: &Cli) -> Result<()> {
    let sub = match &cli.command {
        Command::History { subcommand } => subcommand,
        _ => unreachable!(),
    };

    match sub {
        HistoryCommand::Status => run_history_status(cli),
        HistoryCommand::Build { .. } => run_history_build(cli),
        HistoryCommand::Update { .. } => run_history_update(cli),
        HistoryCommand::Rebuild { .. } => run_history_rebuild(cli),
        HistoryCommand::Diff { .. } => run_history_diff(cli),
        HistoryCommand::Symbol { .. } => run_history_symbol(cli),
        HistoryCommand::File { .. } => run_history_file(cli),
        HistoryCommand::Dependency { .. } => run_history_dependency(cli),
        HistoryCommand::Module { .. } => run_history_module(cli),
        HistoryCommand::Churn { .. } => run_history_churn(cli),
        HistoryCommand::Prune { .. } => run_history_prune(cli),
    }
}

fn run_history_status(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);

    // Canonicalize the repo root before using it as a storage key.
    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();

    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;

    let summary = store
        .history_status(&canonical_root)
        .context("query history status")?;

    let is_shallow = git::is_shallow(std::path::Path::new(&repo)).unwrap_or(false);

    let status = HistoryStatus::from_summary(summary, is_shallow);

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
        for w in &status.warnings {
            eprintln!("warning: {w}");
        }
    }

    Ok(())
}

fn run_history_build(cli: &Cli) -> Result<()> {
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

    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);
    let repo_path = std::path::Path::new(&repo);

    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();

    // Build commit selector from flags.
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

    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;
    let registry = ParserRegistry::with_defaults();

    let selector_indexed_ref = selector.source_ref_label();
    let summary = build_historical_graph(
        repo_path,
        &canonical_root,
        &store,
        &selector,
        &registry,
        selector_indexed_ref.as_deref(),
    )
    .context("build historical graph")?;
    let lifecycle = recompute_lifecycle(&canonical_root, &store).context("recompute lifecycle")?;

    // Print errors as warnings.
    for e in &summary.errors {
        eprintln!("warning: {e}");
    }
    print_warnings(&summary.warnings);

    if cli.json {
        let val = serde_json::json!({
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
        print_json("history_build", val)?;
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

fn run_history_update(cli: &Cli) -> Result<()> {
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

    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);
    let repo_path = std::path::Path::new(&repo);
    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();

    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;
    let registry = ParserRegistry::with_defaults();
    let summary = update_historical_graph(
        repo_path,
        &canonical_root,
        &store,
        branch,
        repair,
        max_commits,
        &registry,
    )
    .context("update historical graph")?;

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

fn run_history_rebuild(cli: &Cli) -> Result<()> {
    let commit_sha = match &cli.command {
        Command::History {
            subcommand: HistoryCommand::Rebuild { commit_sha },
        } => commit_sha.as_str(),
        _ => unreachable!(),
    };

    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);
    let repo_path = std::path::Path::new(&repo);
    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();

    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;
    let registry = ParserRegistry::with_defaults();
    let summary = rebuild_historical_snapshot(
        repo_path,
        &canonical_root,
        &store,
        commit_sha,
        &registry,
        Some(commit_sha),
    )
    .context("rebuild historical snapshot")?;

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

fn run_history_diff(cli: &Cli) -> Result<()> {
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

    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);
    let repo_path = std::path::Path::new(&repo);
    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();
    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;

    let report = diff_snapshots(repo_path, &store, &canonical_root, commit_a, commit_b)
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

fn run_history_symbol(cli: &Cli) -> Result<()> {
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

    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);
    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();
    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;
    let report = query_symbol_history(&store, &canonical_root, qualified_name)
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

fn run_history_file(cli: &Cli) -> Result<()> {
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

    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);
    let repo_path = std::path::Path::new(&repo);
    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();
    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;
    let report =
        query_file_history_with_options(&store, &canonical_root, repo_path, path, follow_renames)
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

fn run_history_dependency(cli: &Cli) -> Result<()> {
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

    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);
    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();
    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;
    let report = query_dependency_history(&store, &canonical_root, source, target)
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

fn run_history_module(cli: &Cli) -> Result<()> {
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

    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);
    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();
    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;
    let report =
        query_module_history(&store, &canonical_root, module).context("query module history")?;

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

fn run_history_churn(cli: &Cli) -> Result<()> {
    let (stat_only, full) = match &cli.command {
        Command::History {
            subcommand: HistoryCommand::Churn { stat_only, full },
        } => (*stat_only, *full),
        _ => unreachable!(),
    };

    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);
    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();
    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;
    let report = compute_churn_report(&store, &canonical_root, &db)
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

fn run_history_prune(cli: &Cli) -> Result<()> {
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

    let repo = resolve_repo(cli)?;
    let db = db_path(cli, &repo);
    let repo_path = std::path::Path::new(&repo);
    let canonical_root = std::fs::canonicalize(&repo)
        .with_context(|| format!("cannot canonicalize repo root: {repo}"))?
        .to_string_lossy()
        .into_owned();
    let store = Store::open(&db).with_context(|| format!("cannot open database: {db}"))?;
    let summary = prune_historical_graph(
        repo_path,
        &canonical_root,
        &store,
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

fn output_mode(_stat_only: bool, full: bool) -> HistoryOutputMode {
    if full {
        HistoryOutputMode::Full
    } else {
        HistoryOutputMode::Summary
    }
}

fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
}

fn print_diff_details(report: &atlas_history::GraphDiffReport) {
    if !report.modified_files.is_empty() {
        println!("modified file details:");
        for file in &report.modified_files {
            println!(
                "  {} old_hash={} new_hash={}",
                file.file_path,
                file.old_hash.as_deref().unwrap_or("-"),
                file.new_hash.as_deref().unwrap_or("-")
            );
        }
    }
    if !report.changed_nodes.is_empty() {
        println!("changed nodes:");
        for node in &report.changed_nodes {
            println!(
                "  {} [{}] {}",
                node.qualified_name,
                node.kind,
                node.changed_fields.join(", ")
            );
        }
    }
    if !report.changed_edges.is_empty() {
        println!("changed edges:");
        for edge in &report.changed_edges {
            println!(
                "  {} -> {} [{}] {}",
                edge.source_qn,
                edge.target_qn,
                edge.kind,
                edge.changed_fields.join(", ")
            );
        }
    }
}

fn print_symbol_details(report: &atlas_history::NodeHistoryReport) {
    if !report.findings.commits_where_changed.is_empty() {
        println!("change commits:");
        for change in &report.findings.commits_where_changed {
            println!("  {} {}", change.commit_sha, change.change_kinds.join(", "));
        }
    }
    if !report.findings.file_path_changes.is_empty() {
        println!("file path timeline:");
        for snapshot in &report.findings.file_path_changes {
            println!(
                "  {} {}",
                snapshot.commit_sha,
                snapshot.file_paths.join(", ")
            );
        }
    }
}

fn print_file_details(report: &atlas_history::FileHistoryReport) {
    if !report.findings.timeline.is_empty() {
        println!("timeline:");
        for point in &report.findings.timeline {
            println!(
                "  {} exists={} nodes={} edges={} added={} removed={}",
                point.commit_sha,
                point.exists,
                point.node_count,
                point.edge_count,
                point.symbol_additions.len(),
                point.symbol_removals.len()
            );
        }
    }
}

fn print_dependency_details(report: &atlas_history::EdgeHistoryReport) {
    if !report.findings.timeline.is_empty() {
        println!("timeline:");
        for point in &report.findings.timeline {
            println!(
                "  {} present={} edges={} added={} removed={}",
                point.commit_sha,
                point.present,
                point.edge_count,
                point.added_edges.len(),
                point.removed_edges.len()
            );
        }
    }
}

fn print_module_details(report: &atlas_history::ModuleHistoryReport) {
    if !report.findings.timeline.is_empty() {
        println!("timeline:");
        for point in &report.findings.timeline {
            println!(
                "  {} nodes={} deps={} coupling={} tests={}",
                point.commit_sha,
                point.node_count,
                point.dependency_count,
                point.coupling_count,
                point.test_adjacency_count
            );
        }
    }
}

fn print_churn_details(report: &atlas_history::ChurnReport) {
    if !report.symbol_churn.is_empty() {
        println!("top symbol churn:");
        for record in report.symbol_churn.iter().take(5) {
            println!(
                "  {} changes={} first={} last={}",
                record.qualified_name,
                record.change_count,
                record.first_commit_sha,
                record.last_commit_sha
            );
        }
    }
    if !report.trends.timeline.is_empty() {
        println!("trend timeline:");
        for point in &report.trends.timeline {
            println!(
                "  {} files={} nodes={} edges={} cycles={}",
                point.commit_sha,
                point.file_count,
                point.node_count,
                point.edge_count,
                point.cycle_count
            );
        }
    }
    println!(
        "storage summary: unique_hashes={} memberships={} snapshot_density={:.2}",
        report.storage_diagnostics.unique_file_hashes,
        report.storage_diagnostics.snapshot_file_memberships,
        report.storage_diagnostics.snapshot_density
    );
}
