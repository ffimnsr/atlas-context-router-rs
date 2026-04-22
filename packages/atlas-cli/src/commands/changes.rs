use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter};
use atlas_core::model::{
    ChangeType, ContextIntent, ContextRequest, ContextTarget, ImpactResult, NoiseReductionSummary,
    ReviewContext, ReviewImpactOverview, RiskSummary, WorkflowCallChain, WorkflowComponent,
    WorkflowFocusNode,
};
use atlas_impact::analyze as advanced_impact;
use atlas_repo::{DiffTarget, changed_files, find_repo_root, repo_relative};
use atlas_review::{ContextEngine, assemble_review_context};
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use serde::Serialize;

use crate::cli::{Cli, Command};

use super::{
    augment_changes_with_node_counts, change_tag, db_path, detect_changes_target, print_json,
    resolve_repo,
};

// ---------------------------------------------------------------------------
// Explain-change structs
// ---------------------------------------------------------------------------

#[derive(Serialize, Default)]
struct ExplainChangedByKind {
    api_change: usize,
    signature_change: usize,
    internal_change: usize,
}

#[derive(Serialize)]
struct ExplainChangedSymbol {
    qn: String,
    kind: String,
    file: String,
    line: u32,
    change_kind: String,
    lang: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    sig: Option<String>,
}

#[derive(Serialize)]
struct ExplainBoundaryViolation {
    kind: String,
    description: String,
    nodes: Vec<String>,
}

#[derive(Serialize)]
struct ExplainTestImpact {
    affected_test_count: usize,
    uncovered_symbol_count: usize,
    uncovered_symbols: Vec<String>,
}

#[derive(Serialize)]
pub(crate) struct ExplainChangeSummary {
    risk_level: String,
    changed_file_count: usize,
    changed_symbol_count: usize,
    changed_by_kind: ExplainChangedByKind,
    diff_summary: ExplainDiffSummary,
    changed_symbols: Vec<ExplainChangedSymbol>,
    impacted_file_count: usize,
    impacted_node_count: usize,
    high_impact_nodes: Vec<WorkflowFocusNode>,
    impacted_components: Vec<WorkflowComponent>,
    call_chains: Vec<WorkflowCallChain>,
    ripple_effects: Vec<String>,
    boundary_violations: Vec<ExplainBoundaryViolation>,
    test_impact: ExplainTestImpact,
    noise_reduction: NoiseReductionSummary,
    summary: String,
}

#[derive(Serialize, Default)]
struct ExplainDiffCounts {
    added: usize,
    modified: usize,
    deleted: usize,
    renamed: usize,
    copied: usize,
}

#[derive(Serialize)]
struct ExplainDiffFile {
    path: String,
    change_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    old_path: Option<String>,
    changed_symbol_count: usize,
    impacted_symbol_count: usize,
}

#[derive(Serialize, Default)]
struct ExplainDiffSummary {
    counts: ExplainDiffCounts,
    files: Vec<ExplainDiffFile>,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn normalize_explicit_files(repo_root: &Utf8Path, explicit_files: &[String]) -> Vec<String> {
    explicit_files
        .iter()
        .map(|p| {
            let abs = Utf8Path::new(p);
            if abs.is_absolute() {
                repo_relative(repo_root, abs)
                    .unwrap_or_else(|_| abs.to_owned())
                    .to_string()
            } else {
                p.clone()
            }
        })
        .collect()
}

fn empty_explain_change_summary() -> ExplainChangeSummary {
    ExplainChangeSummary {
        risk_level: "low".to_string(),
        changed_file_count: 0,
        changed_symbol_count: 0,
        changed_by_kind: ExplainChangedByKind::default(),
        diff_summary: ExplainDiffSummary::default(),
        changed_symbols: vec![],
        impacted_file_count: 0,
        impacted_node_count: 0,
        high_impact_nodes: vec![],
        impacted_components: vec![],
        call_chains: vec![],
        ripple_effects: vec![],
        boundary_violations: vec![],
        test_impact: ExplainTestImpact {
            affected_test_count: 0,
            uncovered_symbol_count: 0,
            uncovered_symbols: vec![],
        },
        noise_reduction: NoiseReductionSummary {
            retained_nodes: 0,
            retained_edges: 0,
            retained_files: 0,
            dropped_nodes: 0,
            dropped_edges: 0,
            dropped_files: 0,
            rules_applied: vec![],
        },
        summary: "No changed files detected.".to_string(),
    }
}

pub(crate) fn build_explain_change_summary(
    store: &Store,
    changes: &[atlas_core::model::ChangedFile],
    files: &[String],
    max_depth: u32,
    max_nodes: usize,
) -> Result<ExplainChangeSummary> {
    let file_refs: Vec<&str> = files.iter().map(String::as_str).collect();
    let base_impact = store
        .impact_radius(&file_refs, max_depth, max_nodes)
        .context("impact radius query failed")?;
    let advanced = advanced_impact(base_impact);
    let workflow_request = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles {
            paths: files.to_vec(),
        },
        max_nodes: Some(max_nodes),
        depth: Some(max_depth),
        ..ContextRequest::default()
    };
    let workflow_result = ContextEngine::new(store)
        .build(&workflow_request)
        .context("workflow summary generation failed")?;
    let workflow = workflow_result.workflow.clone();

    let mut changed_by_kind = ExplainChangedByKind::default();

    let changed_symbols: Vec<ExplainChangedSymbol> = advanced
        .scored_nodes
        .iter()
        .filter_map(|sn| sn.change_kind.map(|ck| (&sn.node, ck)))
        .map(|(node, change_kind)| {
            let change_kind = match change_kind {
                atlas_core::ChangeKind::ApiChange => {
                    changed_by_kind.api_change += 1;
                    "api_change"
                }
                atlas_core::ChangeKind::SignatureChange => {
                    changed_by_kind.signature_change += 1;
                    "signature_change"
                }
                atlas_core::ChangeKind::InternalChange => {
                    changed_by_kind.internal_change += 1;
                    "internal_change"
                }
            };
            ExplainChangedSymbol {
                qn: node.qualified_name.clone(),
                kind: node.kind.as_str().to_string(),
                file: node.file_path.clone(),
                line: node.line_start,
                change_kind: change_kind.to_string(),
                lang: node.language.clone(),
                sig: node.params.clone(),
            }
        })
        .collect();

    let boundary_violations: Vec<ExplainBoundaryViolation> = advanced
        .boundary_violations
        .iter()
        .map(|violation| ExplainBoundaryViolation {
            kind: match violation.kind {
                atlas_core::BoundaryKind::CrossModule => "cross_module",
                atlas_core::BoundaryKind::CrossPackage => "cross_package",
            }
            .to_string(),
            description: violation.description.clone(),
            nodes: violation.nodes.clone(),
        })
        .collect();

    let uncovered_symbols: Vec<String> = advanced
        .test_impact
        .uncovered_changed_nodes
        .iter()
        .map(|node| node.qualified_name.clone())
        .collect();

    let risk_level = advanced.risk_level.to_string();
    let impacted_file_count = advanced.base.impacted_files.len();
    let impacted_node_count = advanced.base.impacted_nodes.len();
    let diff_summary = build_diff_summary(changes, &advanced.base);

    let mut summary_parts: Vec<String> = vec![format!("Risk: {}.", risk_level)];
    summary_parts.push(format!(
        "{} file change(s): {} modified, {} added, {} deleted, {} renamed.",
        changes.len(),
        diff_summary.counts.modified,
        diff_summary.counts.added,
        diff_summary.counts.deleted,
        diff_summary.counts.renamed,
    ));
    if changed_by_kind.api_change > 0 {
        summary_parts.push(format!("{} api change(s).", changed_by_kind.api_change));
    }
    if changed_by_kind.signature_change > 0 {
        summary_parts.push(format!(
            "{} signature change(s).",
            changed_by_kind.signature_change
        ));
    }
    if changed_by_kind.internal_change > 0 {
        summary_parts.push(format!(
            "{} internal change(s).",
            changed_by_kind.internal_change
        ));
    }
    summary_parts.push(format!(
        "Affects {} file(s), {} node(s).",
        impacted_file_count, impacted_node_count
    ));
    if !boundary_violations.is_empty() {
        summary_parts.push(format!(
            "{} boundary violation(s).",
            boundary_violations.len()
        ));
    }
    if !uncovered_symbols.is_empty() {
        summary_parts.push(format!(
            "{} changed symbol(s) lack test coverage.",
            uncovered_symbols.len()
        ));
    }
    if let Some(workflow) = &workflow {
        if let Some(headline) = &workflow.headline {
            summary_parts.push(headline.clone());
        }
        if let Some(ripple) = workflow.ripple_effects.first() {
            summary_parts.push(ripple.clone());
        }
    }

    Ok(ExplainChangeSummary {
        risk_level,
        changed_file_count: changes.len(),
        changed_symbol_count: changed_symbols.len(),
        changed_by_kind,
        diff_summary,
        changed_symbols,
        impacted_file_count,
        impacted_node_count,
        high_impact_nodes: workflow
            .as_ref()
            .map(|workflow| workflow.high_impact_nodes.clone())
            .unwrap_or_default(),
        impacted_components: workflow
            .as_ref()
            .map(|workflow| workflow.impacted_components.clone())
            .unwrap_or_default(),
        call_chains: workflow
            .as_ref()
            .map(|workflow| workflow.call_chains.clone())
            .unwrap_or_default(),
        ripple_effects: workflow
            .as_ref()
            .map(|workflow| workflow.ripple_effects.clone())
            .unwrap_or_default(),
        boundary_violations,
        test_impact: ExplainTestImpact {
            affected_test_count: advanced.test_impact.affected_tests.len(),
            uncovered_symbol_count: uncovered_symbols.len(),
            uncovered_symbols,
        },
        noise_reduction: workflow.map(|workflow| workflow.noise_reduction).unwrap_or(
            NoiseReductionSummary {
                retained_nodes: 0,
                retained_edges: 0,
                retained_files: 0,
                dropped_nodes: 0,
                dropped_edges: 0,
                dropped_files: 0,
                rules_applied: vec![],
            },
        ),
        summary: summary_parts.join(" "),
    })
}

fn build_diff_summary(
    changes: &[atlas_core::model::ChangedFile],
    impact: &atlas_core::ImpactResult,
) -> ExplainDiffSummary {
    let changed_by_file: std::collections::HashMap<&str, usize> =
        impact
            .changed_nodes
            .iter()
            .fold(std::collections::HashMap::new(), |mut acc, node| {
                *acc.entry(node.file_path.as_str()).or_insert(0) += 1;
                acc
            });
    let impacted_by_file: std::collections::HashMap<&str, usize> = impact
        .impacted_nodes
        .iter()
        .fold(std::collections::HashMap::new(), |mut acc, node| {
            *acc.entry(node.file_path.as_str()).or_insert(0) += 1;
            acc
        });

    let mut counts = ExplainDiffCounts::default();
    let files = changes
        .iter()
        .map(|change| {
            match change.change_type {
                ChangeType::Added => counts.added += 1,
                ChangeType::Modified => counts.modified += 1,
                ChangeType::Deleted => counts.deleted += 1,
                ChangeType::Renamed => counts.renamed += 1,
                ChangeType::Copied => counts.copied += 1,
            }

            ExplainDiffFile {
                path: change.path.clone(),
                change_type: match change.change_type {
                    ChangeType::Added => "added",
                    ChangeType::Modified => "modified",
                    ChangeType::Deleted => "deleted",
                    ChangeType::Renamed => "renamed",
                    ChangeType::Copied => "copied",
                }
                .to_string(),
                old_path: change.old_path.clone(),
                changed_symbol_count: changed_by_file
                    .get(change.path.as_str())
                    .copied()
                    .unwrap_or(0),
                impacted_symbol_count: impacted_by_file
                    .get(change.path.as_str())
                    .copied()
                    .unwrap_or(0),
            }
        })
        .collect();

    ExplainDiffSummary { counts, files }
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
            let non_deleted: Vec<&str> = changes
                .iter()
                .filter(|cf| cf.change_type != ChangeType::Deleted)
                .map(|cf| cf.path.as_str())
                .collect();
            if !non_deleted.is_empty()
                && let Ok(impact) = store.impact_radius(&non_deleted, 5, 200)
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
            normalize_explicit_files(repo_root, &explicit_files)
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
        let summary =
            build_explain_change_summary(&store, &changes, &target_files, max_depth, max_nodes)?;

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
            explicit_files
                .iter()
                .map(|p| {
                    let abs = Utf8Path::new(p);
                    if abs.is_absolute() {
                        repo_relative(repo_root, abs)
                            .unwrap_or_else(|_| abs.to_owned())
                            .to_string()
                    } else {
                        p.clone()
                    }
                })
                .collect()
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
            .impact_radius(&path_refs, max_depth, max_nodes)
            .context("impact radius query failed")?;
        let latency_ms = t0.elapsed().as_millis();

        let advanced = advanced_impact(result);

        if cli.json {
            print_json(
                "impact",
                serde_json::json!({
                    "files": target_files,
                    "latency_ms": latency_ms,
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
            explicit_files
                .iter()
                .map(|p| {
                    let abs = Utf8Path::new(p);
                    if abs.is_absolute() {
                        repo_relative(repo_root, abs)
                            .unwrap_or_else(|_| abs.to_owned())
                            .to_string()
                    } else {
                        p.clone()
                    }
                })
                .collect()
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

        let path_refs: Vec<&str> = target_files.iter().map(String::as_str).collect();
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
            .build(&workflow_request)
            .context("context engine failed")?;

        if cli.json {
            print_json("review_context", serde_json::to_value(&workflow_result)?)?;
            return Ok(());
        }

        let impact = store
            .impact_radius(&path_refs, max_depth, max_nodes)
            .context("impact radius query failed")?;

        let ctx = assemble_review_context(&impact, &target_files, max_depth, max_nodes);

        println!("Changed files ({}):", ctx.changed_files.len());
        for f in &ctx.changed_files {
            println!("  {f}");
        }
        println!("\nImpact radius:");
        println!("  Max depth         : {}", ctx.impact_overview.max_depth);
        println!("  Max nodes         : {}", ctx.impact_overview.max_nodes);
        println!(
            "  Impacted nodes    : {}",
            ctx.impact_overview.impacted_node_count
        );
        println!(
            "  Impacted files    : {}",
            ctx.impact_overview.impacted_file_count
        );
        println!(
            "  Relevant edges    : {}",
            ctx.impact_overview.relevant_edge_count
        );
        println!(
            "  Node limit reached: {}",
            ctx.impact_overview.reached_node_limit
        );
        println!(
            "\nChanged symbols: {}",
            ctx.risk_summary.changed_symbol_count
        );
        for summary in ctx.changed_symbol_summaries.iter().take(10) {
            println!(
                "  {} {} ({}:{}) | callers {} | callees {} | importers {} | tests {}",
                summary.node.kind.as_str(),
                summary.node.qualified_name,
                summary.node.file_path,
                summary.node.line_start,
                summary.callers.len(),
                summary.callees.len(),
                summary.importers.len(),
                summary.tests.len()
            );
        }
        println!(
            "\nImpacted neighbors (top {}):",
            ctx.impacted_neighbors.len().min(20)
        );
        for n in ctx.impacted_neighbors.iter().take(20) {
            println!(
                "  {} {} ({}:{})",
                n.kind.as_str(),
                n.qualified_name,
                n.file_path,
                n.line_start
            );
        }
        println!("\nRisk summary:");
        println!(
            "  Public API changes : {}",
            ctx.risk_summary.public_api_changes
        );
        println!(
            "  Affected tests     : {}",
            ctx.risk_summary.affected_test_count
        );
        println!(
            "  Uncovered changes  : {}",
            ctx.risk_summary.uncovered_changed_symbol_count
        );
        println!(
            "  Large functions    : {}",
            ctx.risk_summary.large_function_count
        );
        println!("  Test adjacent      : {}", ctx.risk_summary.test_adjacent);
        println!(
            "  Cross-module impact: {}",
            ctx.risk_summary.cross_module_impact
        );
        println!(
            "  Cross-package impact: {}",
            ctx.risk_summary.cross_package_impact
        );

        if let Some(workflow) = &workflow_result.workflow {
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
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("review-context", result.is_ok());
    }
    result
}
