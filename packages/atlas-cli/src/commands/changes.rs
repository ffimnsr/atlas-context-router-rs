use crate::cli::{Cli, Command};
use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter};
use atlas_core::BudgetReport;
use atlas_core::model::{
    ChangeType, ContextIntent, ContextRequest, ContextResult, ContextTarget, ImpactResult,
    ReviewContext, ReviewImpactOverview, RiskSummary, SelectionReason,
};
use atlas_impact::analyze as advanced_impact;
use atlas_repo::{CanonicalRepoPath, DiffTarget, changed_files, find_repo_root};
use atlas_review::{ContextEngine, build_explain_change_summary, empty_explain_change_summary};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

use super::{
    augment_changes_with_node_counts, change_tag, db_path, detect_changes_target,
    load_budget_policy, print_json, resolve_repo,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn normalize_explicit_files(
    repo_root: &Utf8Path,
    explicit_files: &[String],
) -> Result<Vec<String>> {
    explicit_files
        .iter()
        .map(|path| {
            CanonicalRepoPath::from_cli_argument(repo_root, Utf8Path::new(path))
                .with_context(|| format!("invalid explicit file path '{path}'"))
                .map(|path| path.as_str().to_owned())
        })
        .collect()
}

fn print_review_context_text(ctx: &ContextResult, changed_files: &[String]) {
    println!("Changed files ({}):", changed_files.len());
    for path in changed_files {
        println!("  {path}");
    }

    println!("\nContext summary:");
    println!("  Selected nodes   : {}", ctx.nodes.len());
    println!("  Selected edges   : {}", ctx.edges.len());
    println!("  Selected files   : {}", ctx.files.len());
    println!(
        "  Max depth        : {}",
        ctx.request.depth.unwrap_or_default()
    );
    println!(
        "  Max nodes        : {}",
        ctx.request.max_nodes.unwrap_or(ctx.nodes.len())
    );

    let changed_symbols: Vec<_> = ctx
        .nodes
        .iter()
        .filter(|node| node.selection_reason == SelectionReason::DirectTarget)
        .collect();
    println!("\nChanged symbols: {}", changed_symbols.len());
    for selected in changed_symbols.iter().take(10) {
        println!(
            "  {} {} ({}:{})",
            selected.node.kind.as_str(),
            selected.node.qualified_name,
            selected.node.file_path,
            selected.node.line_start
        );
    }

    if let Some(workflow) = &ctx.workflow {
        let cross_package_impact = workflow
            .impacted_components
            .iter()
            .filter(|component| component.kind == "package")
            .count()
            > 1;
        println!("\nRisk summary:");
        println!("  Cross-package impact: {cross_package_impact}");
        if let Some(headline) = &workflow.headline {
            println!("\nFocus: {headline}");
        }
        if !workflow.high_impact_nodes.is_empty() {
            println!("\nHigh-impact nodes:");
            for node in workflow.high_impact_nodes.iter().take(5) {
                println!(
                    "  [{:.1}] {} {} ({})",
                    node.relevance_score, node.kind, node.qualified_name, node.file_path
                );
            }
        }
        if !workflow.impacted_components.is_empty() {
            println!("\nImpacted components:");
            for component in workflow.impacted_components.iter().take(5) {
                println!(
                    "  [{}] {} | changed {} | impacted {} | files {}",
                    component.kind,
                    component.label,
                    component.changed_node_count,
                    component.impacted_node_count,
                    component.file_count
                );
            }
        }
        if !workflow.call_chains.is_empty() {
            println!("\nCall chains:");
            for chain in workflow.call_chains.iter().take(5) {
                println!("  {}", chain.summary);
            }
        }
        if !workflow.ripple_effects.is_empty() {
            println!("\nRipple effects:");
            for ripple in &workflow.ripple_effects {
                println!("  {ripple}");
            }
        }
        println!("\nNoise reduction:");
        println!(
            "  Retained nodes   : {}",
            workflow.noise_reduction.retained_nodes
        );
        println!(
            "  Retained edges   : {}",
            workflow.noise_reduction.retained_edges
        );
        println!(
            "  Retained files   : {}",
            workflow.noise_reduction.retained_files
        );
        println!(
            "  Dropped nodes    : {}",
            workflow.noise_reduction.dropped_nodes
        );
        println!(
            "  Dropped edges    : {}",
            workflow.noise_reduction.dropped_edges
        );
        println!(
            "  Dropped files    : {}",
            workflow.noise_reduction.dropped_files
        );
    }

    if ctx.truncation.truncated {
        println!("\nTruncation:");
        println!("  Nodes dropped    : {}", ctx.truncation.nodes_dropped);
        println!("  Edges dropped    : {}", ctx.truncation.edges_dropped);
        println!("  Files dropped    : {}", ctx.truncation.files_dropped);
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

pub fn run_detect_changes(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let repo_root_path =
        find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
    let repo_root = repo_root_path.as_path();
    let db_path = db_path(cli, &repo);

    let (base, staged) = match &cli.command {
        Command::DetectChanges { base, staged } => (base.clone(), *staged),
        _ => unreachable!(),
    };
    let diff_target = detect_changes_target(&base, staged);

    let changes = changed_files(repo_root, &diff_target).context("cannot detect changed files")?;

    // Try to open the DB for graph summary — tolerate failure (DB may not exist yet).
    let store_result = Store::open(&db_path);

    if cli.json {
        print_json(
            "detect_changes",
            serde_json::json!({
                "diff_target": {
                    "base": base,
                    "staged": staged,
                    "kind": if staged { "staged" } else if base.is_some() { "base_ref" } else { "working_tree" },
                },
                "changes": augment_changes_with_node_counts(&changes, store_result.as_ref().ok()),
            }),
        )?;
    } else if changes.is_empty() {
        println!("No changed files detected.");
    } else {
        for cf in &changes {
            let node_info = store_result
                .as_ref()
                .ok()
                .and_then(|s| s.nodes_by_file(&cf.path).ok())
                .map(|ns| format!(" [{} nodes]", ns.len()))
                .unwrap_or_default();
            if let Some(old) = &cf.old_path {
                println!(
                    "{}  {old} -> {}{node_info}",
                    change_tag(cf.change_type),
                    cf.path
                );
            } else {
                println!("{}  {}{node_info}", change_tag(cf.change_type), cf.path);
            }
        }
        println!("\n{} file(s) changed.", changes.len());

        // Graph-level impact summary when DB is available.
        if let Ok(store) = &store_result {
            let policy = load_budget_policy(&repo)?;
            let non_deleted: Vec<&str> = changes
                .iter()
                .filter(|cf| cf.change_type != ChangeType::Deleted)
                .map(|cf| cf.path.as_str())
                .collect();
            if !non_deleted.is_empty()
                && let Ok(impact) = store.impact_radius(
                    &non_deleted,
                    5,
                    200,
                    policy.graph_traversal.edges.default_limit,
                )
            {
                println!("\nGraph impact summary:");
                println!("  Changed symbols : {}", impact.changed_nodes.len());
                println!("  Impacted nodes  : {}", impact.impacted_nodes.len());
                println!("  Impacted files  : {}", impact.impacted_files.len());
            }
        }
    }

    Ok(())
}

pub fn run_explain_change(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("explain-change");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let repo_root = repo_root_path.as_path();
        let db_path = db_path(cli, &repo);

        let (base, staged, explicit_files, max_depth, max_nodes) = match &cli.command {
            Command::ExplainChange {
                base,
                staged,
                files,
                max_depth,
                max_nodes,
            } => (
                base.clone(),
                *staged,
                files.clone(),
                *max_depth,
                *max_nodes as usize,
            ),
            _ => unreachable!(),
        };

        let changes = if !explicit_files.is_empty() {
            normalize_explicit_files(repo_root, &explicit_files)?
                .into_iter()
                .map(|path| atlas_core::model::ChangedFile {
                    path,
                    change_type: ChangeType::Modified,
                    old_path: None,
                })
                .collect()
        } else {
            changed_files(repo_root, &detect_changes_target(&base, staged))
                .context("cannot detect changed files")?
        };

        let target_files: Vec<String> = changes
            .iter()
            .filter(|change| change.change_type != ChangeType::Deleted)
            .map(|change| change.path.clone())
            .collect();

        if target_files.is_empty() {
            let empty = empty_explain_change_summary();
            if cli.json {
                print_json("explain_change", serde_json::to_value(&empty)?)?;
            } else {
                println!("No changed files detected.");
            }
            return Ok(());
        }

        let store =
            Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
        let policy = load_budget_policy(&repo)?;
        let summary = build_explain_change_summary(
            &store,
            &changes,
            &target_files,
            max_depth,
            max_nodes,
            &policy,
        )?;

        if cli.json {
            print_json("explain_change", serde_json::to_value(&summary)?)?;
        } else {
            println!("Risk level      : {}", summary.risk_level);
            println!("Changed files   : {}", summary.changed_file_count);
            println!("Changed symbols : {}", summary.changed_symbol_count);
            println!(
                "Diff summary    : +{} ~{} -{} r{}",
                summary.diff_summary.counts.added,
                summary.diff_summary.counts.modified,
                summary.diff_summary.counts.deleted,
                summary.diff_summary.counts.renamed
            );
            println!(
                "Change kinds    : api {} | signature {} | internal {}",
                summary.changed_by_kind.api_change,
                summary.changed_by_kind.signature_change,
                summary.changed_by_kind.internal_change
            );
            println!("Impacted files  : {}", summary.impacted_file_count);
            println!("Impacted nodes  : {}", summary.impacted_node_count);

            if !summary.changed_symbols.is_empty() {
                println!("\nChanged symbols:");
                for symbol in summary.changed_symbols.iter().take(20) {
                    println!(
                        "  [{}] {} {} ({}:{})",
                        symbol.change_kind, symbol.kind, symbol.qn, symbol.file, symbol.line
                    );
                }
            }

            if !summary.boundary_violations.is_empty() {
                println!("\nBoundary violations:");
                for violation in &summary.boundary_violations {
                    println!("  [{}] {}", violation.kind, violation.description);
                }
            }

            if !summary.impacted_components.is_empty() {
                println!("\nImpacted components:");
                for component in summary.impacted_components.iter().take(8) {
                    println!(
                        "  [{}] {} | changed {} | impacted {} | files {}",
                        component.kind,
                        component.label,
                        component.changed_node_count,
                        component.impacted_node_count,
                        component.file_count
                    );
                }
            }

            if !summary.call_chains.is_empty() {
                println!("\nCall chains:");
                for chain in summary.call_chains.iter().take(5) {
                    println!("  {}", chain.summary);
                }
            }

            if !summary.ripple_effects.is_empty() {
                println!("\nRipple effects:");
                for ripple in &summary.ripple_effects {
                    println!("  {ripple}");
                }
            }

            if summary.test_impact.affected_test_count > 0 {
                println!(
                    "\nAffected tests  : {}",
                    summary.test_impact.affected_test_count
                );
            }
            if summary.test_impact.uncovered_symbol_count > 0 {
                println!("Changed symbols without test coverage:");
                for symbol in &summary.test_impact.uncovered_symbols {
                    println!("  {symbol}");
                }
            }

            println!("\nSummary: {}", summary.summary);
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("explain-change", result.is_ok());
    }
    result
}

pub fn run_impact(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("impact");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let repo_root = repo_root_path.as_path();
        let db_path = db_path(cli, &repo);

        let store =
            Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
        let policy = load_budget_policy(&repo)?;

        let (base, explicit_files, max_depth, max_nodes) = match &cli.command {
            Command::Impact {
                base,
                files,
                max_depth,
                max_nodes,
            } => (base.clone(), files.clone(), *max_depth, *max_nodes as usize),
            _ => unreachable!(),
        };

        let target_files: Vec<String> = if !explicit_files.is_empty() {
            normalize_explicit_files(repo_root, &explicit_files)?
        } else {
            let diff_target = if let Some(base_ref) = &base {
                DiffTarget::BaseRef(base_ref.clone())
            } else {
                DiffTarget::WorkingTree
            };
            changed_files(repo_root, &diff_target)
                .context("cannot detect changed files")?
                .into_iter()
                .filter(|cf| cf.change_type != ChangeType::Deleted)
                .map(|cf| cf.path)
                .collect()
        };

        if target_files.is_empty() {
            if cli.json {
                print_json(
                    "impact",
                    serde_json::json!({
                        "files": target_files,
                        "analysis": ImpactResult {
                            changed_nodes: vec![],
                            impacted_nodes: vec![],
                            impacted_files: vec![],
                            relevant_edges: vec![],
                            seed_budgets: vec![],
                            traversal_budget: None,
                            budget: BudgetReport::not_applicable(),
                        }
                    }),
                )?;
            } else {
                println!("No changed files detected.");
            }
            return Ok(());
        }

        let path_refs: Vec<&str> = target_files.iter().map(String::as_str).collect();
        let t0 = std::time::Instant::now();
        let result = store
            .impact_radius(
                &path_refs,
                max_depth,
                max_nodes,
                policy.graph_traversal.edges.default_limit,
            )
            .context("impact radius query failed")?;
        let latency_ms = t0.elapsed().as_millis();

        let advanced = advanced_impact(result);

        if cli.json {
            print_json(
                "impact",
                serde_json::json!({
                    "files": target_files,
                    "analysis": advanced,
                }),
            )?;
        } else {
            println!("Changed files : {}", target_files.len());
            println!("Changed nodes : {}", advanced.base.changed_nodes.len());
            println!("Impacted nodes: {}", advanced.base.impacted_nodes.len());
            println!("Impacted files: {}", advanced.base.impacted_files.len());
            println!("Relevant edges: {}", advanced.base.relevant_edges.len());
            println!("Risk level    : {}", advanced.risk_level);
            println!("Latency       : {latency_ms}ms");
            if !advanced.base.impacted_files.is_empty() {
                println!("\nImpacted files:");
                for f in &advanced.base.impacted_files {
                    println!("  {f}");
                }
            }
            if !advanced.scored_nodes.is_empty() {
                println!("\nTop impacted nodes (by score):");
                for sn in advanced.scored_nodes.iter().take(20) {
                    let ck = sn
                        .change_kind
                        .map(|c| format!(" [{c}]"))
                        .unwrap_or_default();
                    println!(
                        "  {:>6.2}  {} {}{}",
                        sn.impact_score,
                        sn.node.kind.as_str(),
                        sn.node.qualified_name,
                        ck
                    );
                }
            }
            if !advanced.test_impact.affected_tests.is_empty() {
                println!(
                    "\nAffected tests: {}",
                    advanced.test_impact.affected_tests.len()
                );
            }
            if !advanced.test_impact.uncovered_changed_nodes.is_empty() {
                println!("\nChanged nodes with no test coverage:");
                for n in &advanced.test_impact.uncovered_changed_nodes {
                    println!("  {} {}", n.kind.as_str(), n.qualified_name);
                }
            }
            if !advanced.boundary_violations.is_empty() {
                println!("\nBoundary violations:");
                for v in &advanced.boundary_violations {
                    println!("  [{}] {}", v.kind, v.description);
                }
            }
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("impact", result.is_ok());
    }
    result
}

pub fn run_review_context(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("review-context");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let repo_root = repo_root_path.as_path();
        let db_path = db_path(cli, &repo);

        let store =
            Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;

        let (base, explicit_files, max_depth, max_nodes) = match &cli.command {
            Command::ReviewContext {
                base,
                files,
                max_depth,
                max_nodes,
            } => (base.clone(), files.clone(), *max_depth, *max_nodes as usize),
            _ => unreachable!(),
        };

        let target_files: Vec<String> = if !explicit_files.is_empty() {
            normalize_explicit_files(repo_root, &explicit_files)?
        } else {
            let diff_target = if let Some(base_ref) = &base {
                DiffTarget::BaseRef(base_ref.clone())
            } else {
                DiffTarget::WorkingTree
            };
            changed_files(repo_root, &diff_target)
                .context("cannot detect changed files")?
                .into_iter()
                .filter(|cf| cf.change_type != ChangeType::Deleted)
                .map(|cf| cf.path)
                .collect()
        };

        if target_files.is_empty() {
            if cli.json {
                let empty = ReviewContext {
                    changed_files: vec![],
                    changed_symbols: vec![],
                    changed_symbol_summaries: vec![],
                    impacted_neighbors: vec![],
                    critical_edges: vec![],
                    impact_overview: ReviewImpactOverview {
                        max_depth,
                        max_nodes,
                        impacted_node_count: 0,
                        impacted_file_count: 0,
                        relevant_edge_count: 0,
                        reached_node_limit: false,
                    },
                    risk_summary: RiskSummary {
                        changed_symbol_count: 0,
                        public_api_changes: 0,
                        test_adjacent: false,
                        affected_test_count: 0,
                        uncovered_changed_symbol_count: 0,
                        large_function_touched: false,
                        large_function_count: 0,
                        cross_module_impact: false,
                        cross_package_impact: false,
                    },
                };
                print_json(
                    "review_context",
                    serde_json::json!({
                        "files": target_files,
                        "review_context": empty,
                    }),
                )?;
            } else {
                println!("No changed files detected.");
            }
            return Ok(());
        }

        let workflow_request = ContextRequest {
            intent: ContextIntent::Review,
            target: ContextTarget::ChangedFiles {
                paths: target_files.clone(),
            },
            max_nodes: Some(max_nodes),
            depth: Some(max_depth),
            ..ContextRequest::default()
        };
        let workflow_result = ContextEngine::new(&store)
            .with_budget_policy(load_budget_policy(repo_root.as_str())?)
            .build(&workflow_request)
            .context("context engine failed")?;

        if cli.json {
            let mut value = serde_json::to_value(&workflow_result)?;
            if let Some(object) = value.as_object_mut() {
                object.insert(
                    "context_ranking_evidence_legend".to_owned(),
                    atlas_core::context_ranking_evidence_legend(),
                );
            }
            print_json("review_context", value)?;
            return Ok(());
        }

        print_review_context_text(&workflow_result, &target_files);

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("review-context", result.is_ok());
    }
    result
}
