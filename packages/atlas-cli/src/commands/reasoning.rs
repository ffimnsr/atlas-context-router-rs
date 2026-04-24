use anyhow::{Context, Result};
use atlas_adapters::{
    AdapterHooks, CliAdapter, PendingEvent, extract_decision_event_with_details,
    extract_reasoning_event,
};
use atlas_reasoning::{
    AnalysisRankingPrimitives, AnalysisTrimmingPrimitives, ReasoningEngine,
    sort_dead_code_candidates, sort_dependency_result, sort_refactor_safety_result,
    sort_removal_result,
};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

use crate::cli::{AnalyzeCommand, Cli, Command, RefactorCommand};

use super::{db_path, print_json, resolve_repo};

fn print_refactor_result(result: &atlas_core::RefactorDryRunResult, dry_run: bool) {
    let mode = if dry_run { "dry-run" } else { "applied" };
    println!("Refactor ({mode}):");
    println!("  Files changed : {}", result.files_changed);
    println!("  Edits         : {}", result.edit_count);
    println!("  Safety        : {:?}", result.plan.estimated_safety);
    if !result.plan.manual_review.is_empty() {
        println!("\nManual review required:");
        for item in &result.plan.manual_review {
            println!("  ! {item}");
        }
    }
    if !result.validation.warnings.is_empty() {
        println!("\nValidation warnings:");
        for w in &result.validation.warnings {
            println!("  ~ {w}");
        }
    }
    if !result.patches.is_empty() {
        println!("\nPatches:");
        for p in &result.patches {
            println!("--- {}", p.file_path);
            println!("{}", p.unified_diff);
        }
    }
}

pub fn run_analyze(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    // Identify the subcommand for event labelling before the closure borrows cli.command.
    let analyze_label = match &cli.command {
        Command::Analyze { subcommand } => match subcommand {
            AnalyzeCommand::Remove { .. } => "analyze:remove",
            AnalyzeCommand::DeadCode { .. } => "analyze:dead-code",
            AnalyzeCommand::Safety { .. } => "analyze:safety",
            AnalyzeCommand::Dependency { .. } => "analyze:dependency",
        },
        _ => "analyze",
    };
    let mut adapter = CliAdapter::open(&repo);
    let mut decision_event: Option<PendingEvent> = None;
    if let Some(ref mut a) = adapter {
        a.before_command(analyze_label);
    }

    let result = (|| -> Result<()> {
        let db_path = db_path(cli, &repo);
        let store =
            Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
        let engine = ReasoningEngine::new(&store);
        let ranking = AnalysisRankingPrimitives::default();
        let trimming = AnalysisTrimmingPrimitives::default();

        let sub = match &cli.command {
            Command::Analyze { subcommand } => subcommand,
            _ => unreachable!(),
        };

        match sub {
            AnalyzeCommand::Remove {
                symbol,
                max_depth,
                max_nodes,
            } => {
                let mut result = engine
                    .analyze_removal(&[symbol.as_str()], Some(*max_depth), Some(*max_nodes))
                    .with_context(|| format!("removal analysis for `{symbol}` failed"))?;
                sort_removal_result(&mut result, &ranking);
                decision_event = Some(extract_decision_event_with_details(
                    &format!("removal impact for {symbol}"),
                    Some("reasoning analysis completed"),
                    serde_json::json!({
                        "query": symbol,
                        "conclusion": format!("{} impacted symbol(s)", result.impacted_symbols.len()),
                    }),
                ));

                if cli.json {
                    print_json("analyze_remove", serde_json::to_value(&result)?)?;
                } else {
                    println!("Removal impact for: {symbol}");
                    println!("  Seed nodes      : {}", result.seed.len());
                    println!("  Impacted symbols: {}", result.impacted_symbols.len());
                    println!("  Impacted files  : {}", result.impacted_files.len());
                    println!("  Impacted tests  : {}", result.impacted_tests.len());

                    // Primary impacts (Definite/Probable) shown first; containment-noise
                    // (Weak) demoted to a separate section to reduce output noise.
                    let (primary, containment): (Vec<_>, Vec<_>) = result
                        .impacted_symbols
                        .iter()
                        .partition(|im| im.impact_class != atlas_core::ImpactClass::Weak);

                    for im in primary.iter().take(trimming.removal_primary_preview_limit) {
                        println!(
                            "  [{:?}] {} {} (depth {})",
                            im.impact_class,
                            im.node.kind.as_str(),
                            im.node.qualified_name,
                            im.depth,
                        );
                    }
                    if !containment.is_empty() {
                        println!(
                            "\n  Containment/structural ({}): — child nodes of removed symbol",
                            containment.len()
                        );
                        for im in containment
                            .iter()
                            .take(trimming.removal_containment_preview_limit)
                        {
                            println!(
                                "  [Weak] {} {}",
                                im.node.kind.as_str(),
                                im.node.qualified_name,
                            );
                        }
                        if containment.len() > trimming.removal_containment_preview_limit {
                            println!(
                                "  ... and {} more",
                                containment.len() - trimming.removal_containment_preview_limit
                            );
                        }
                    }

                    if !result.uncertainty_flags.is_empty() {
                        println!("\nUncertainty:");
                        for flag in &result.uncertainty_flags {
                            println!("  ! {flag}");
                        }
                    }
                    if !result.warnings.is_empty() {
                        println!("\nWarnings:");
                        for w in &result.warnings {
                            println!("  [{:?}] {}", w.confidence, w.message);
                            for s in &w.suggestions {
                                println!("    -> {s}");
                            }
                        }
                    }
                }
            }

            AnalyzeCommand::DeadCode {
                allowlist,
                subpath,
                limit,
                summary,
                exclude_kind,
                max_files: _,
                max_edges: _,
                code_only: _,
            } => {
                let allowlist_refs: Vec<&str> = allowlist.iter().map(String::as_str).collect();
                let exclude_kinds: Vec<atlas_core::NodeKind> =
                    exclude_kind.iter().filter_map(|k| k.parse().ok()).collect();
                let mut candidates = engine
                    .detect_dead_code(
                        &allowlist_refs,
                        subpath.as_deref(),
                        Some(*limit),
                        &exclude_kinds,
                    )
                    .context("dead-code detection failed")?;
                sort_dead_code_candidates(&mut candidates, &ranking);
                decision_event = Some(extract_decision_event_with_details(
                    "dead-code scan",
                    Some("reasoning analysis completed"),
                    serde_json::json!({
                        "query": subpath.clone().unwrap_or_else(|| "repo".to_owned()),
                        "conclusion": format!("{} dead-code candidate(s)", candidates.len()),
                    }),
                ));

                if *summary {
                    println!("Dead-code candidates: {}", candidates.len());
                } else if cli.json {
                    print_json("analyze_dead_code", serde_json::to_value(&candidates)?)?;
                } else if candidates.is_empty() {
                    println!("No dead-code candidates found.");
                } else {
                    println!("Dead-code candidates ({}):", candidates.len());
                    for c in &candidates {
                        println!(
                            "  [{:?}] {} {} ({}:{})",
                            c.certainty,
                            c.node.kind.as_str(),
                            c.node.qualified_name,
                            c.node.file_path,
                            c.node.line_start,
                        );
                        for r in &c.reasons {
                            println!("    - {r}");
                        }
                        for b in &c.blockers {
                            println!("    ! blocker: {b}");
                        }
                    }
                }
            }

            AnalyzeCommand::Safety { symbol } => {
                let mut result = engine
                    .score_refactor_safety(symbol)
                    .with_context(|| format!("safety scoring for `{symbol}` failed"))?;
                sort_refactor_safety_result(&mut result);
                decision_event = Some(extract_decision_event_with_details(
                    &format!("refactor safety for {symbol}"),
                    Some("reasoning analysis completed"),
                    serde_json::json!({
                        "query": symbol,
                        "conclusion": format!("{:?}", result.safety.band),
                    }),
                ));

                if cli.json {
                    print_json("analyze_safety", serde_json::to_value(&result)?)?;
                } else {
                    println!("Refactor safety for: {symbol}");
                    println!("  Score    : {:.3}", result.safety.score);
                    println!("  Band     : {:?}", result.safety.band);
                    println!("  Fan-in   : {}", result.fan_in);
                    println!("  Fan-out  : {}", result.fan_out);
                    println!("  Tests    : {}", result.linked_test_count);
                    println!("  Coverage : {:?}", result.coverage_strength);
                    if !result.safety.reasons.is_empty() {
                        println!("\nReasons:");
                        for r in &result.safety.reasons {
                            println!("  - {r}");
                        }
                    }
                    if !result.safety.suggested_validations.is_empty() {
                        println!("\nSuggested validations:");
                        for v in &result.safety.suggested_validations {
                            println!("  - {v}");
                        }
                    }
                }
            }

            AnalyzeCommand::Dependency { symbol } => {
                let mut result = engine
                    .check_dependency_removal(symbol)
                    .with_context(|| format!("dependency check for `{symbol}` failed"))?;
                sort_dependency_result(&mut result, &ranking);
                decision_event = Some(extract_decision_event_with_details(
                    &format!("dependency removal for {symbol}"),
                    Some("reasoning analysis completed"),
                    serde_json::json!({
                        "query": symbol,
                        "conclusion": if result.removable { "removable" } else { "blocked" },
                    }),
                ));

                if cli.json {
                    print_json("analyze_dependency", serde_json::to_value(&result)?)?;
                } else {
                    let verdict = if result.removable {
                        "REMOVABLE"
                    } else {
                        "BLOCKED"
                    };
                    println!("Dependency check for: {symbol}");
                    println!("  Verdict   : {verdict}");
                    println!("  Confidence: {:?}", result.confidence);
                    println!("  Blocking  : {}", result.blocking_references.len());
                    for n in &result.blocking_references {
                        println!(
                            "  - {} {} ({})",
                            n.kind.as_str(),
                            n.qualified_name,
                            n.file_path
                        );
                    }
                    if !result.suggested_cleanups.is_empty() {
                        println!("\nSuggested cleanups:");
                        for s in &result.suggested_cleanups {
                            println!("  - {s}");
                        }
                    }
                    if !result.uncertainty_flags.is_empty() {
                        println!("\nUncertainty:");
                        for flag in &result.uncertainty_flags {
                            println!("  ! {flag}");
                        }
                    }
                }
            }
        }

        Ok(())
    })();

    if result.is_ok()
        && let Some(ref mut a) = adapter
    {
        if let Some(event) = decision_event.take() {
            a.record(event);
        }
        a.record(extract_reasoning_event(None, analyze_label));
    }
    if let Some(ref mut a) = adapter {
        a.after_command(analyze_label, result.is_ok());
    }
    result
}

pub fn run_refactor(cli: &Cli) -> Result<()> {
    use atlas_refactor::RefactorEngine;
    use atlas_repo::find_repo_root;

    let repo = resolve_repo(cli)?;
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut a) = adapter {
        a.before_command("refactor");
    }

    let result = (|| -> Result<()> {
        let repo_root_path =
            find_repo_root(Utf8Path::new(&repo)).context("cannot find git repo root")?;
        let repo_root = repo_root_path.as_std_path();
        let db_path = db_path(cli, &repo);
        let mut store =
            Store::open(&db_path).with_context(|| format!("cannot open database at {db_path}"))?;
        let mut engine = RefactorEngine::new(&mut store, repo_root);

        let sub = match &cli.command {
            Command::Refactor { subcommand } => subcommand,
            _ => unreachable!(),
        };

        match sub {
            RefactorCommand::Rename {
                symbol,
                to,
                legacy_symbol,
                legacy_to,
                dry_run,
            } => {
                let symbol = symbol
                    .as_deref()
                    .or(legacy_symbol.as_deref())
                    .context("rename requires --symbol <qualified-name>")?;
                let new_name = to
                    .as_deref()
                    .or(legacy_to.as_deref())
                    .context("rename requires --to <new-name>")?;
                let plan = engine
                    .plan_rename(symbol, new_name)
                    .with_context(|| format!("rename plan for `{symbol}` → `{new_name}` failed"))?;
                let result = engine
                    .apply_rename(&plan, *dry_run)
                    .context("apply rename failed")?;

                if cli.json {
                    print_json("refactor_rename", serde_json::to_value(&result)?)?;
                } else {
                    print_refactor_result(&result, *dry_run);
                }
            }

            RefactorCommand::RemoveDead { symbol, dry_run } => {
                let plan = engine
                    .plan_dead_code_removal(symbol)
                    .with_context(|| format!("remove-dead plan for `{symbol}` failed"))?;
                let result = engine
                    .apply_dead_code_removal(&plan, *dry_run)
                    .context("apply dead-code removal failed")?;

                if cli.json {
                    print_json("refactor_remove_dead", serde_json::to_value(&result)?)?;
                } else {
                    print_refactor_result(&result, *dry_run);
                }
            }

            RefactorCommand::CleanImports { file, dry_run } => {
                let plan = engine
                    .plan_import_cleanup(file)
                    .with_context(|| format!("import-cleanup plan for `{file}` failed"))?;
                let result = engine
                    .apply_import_cleanup(&plan, *dry_run)
                    .context("apply import cleanup failed")?;

                if cli.json {
                    print_json("refactor_clean_imports", serde_json::to_value(&result)?)?;
                } else {
                    print_refactor_result(&result, *dry_run);
                }
            }
        }

        Ok(())
    })();

    if let Some(ref mut a) = adapter {
        a.after_command("refactor", result.is_ok());
    }
    result
}
