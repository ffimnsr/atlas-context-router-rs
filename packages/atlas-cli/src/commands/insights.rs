use anyhow::{Context, Result};
use atlas_adapters::{AdapterHooks, CliAdapter};
use atlas_core::GraphToolRequirement;
use atlas_core::{InsightFinding, InsightSummary};
use atlas_reasoning::{
    InsightsEngine, LargeFunctionMode, LargeFunctionRequest, RiskAssessmentTarget,
};
use atlas_store_sqlite::Store;

use crate::cli::{Cli, Command, InsightsCommand};

use super::{
    check_graph_readiness, db_path, derive_graph_readiness, derive_graph_readiness_open_failed,
    print_json, readiness_overrides, resolve_repo,
};

fn insights_command_label(subcommand: &InsightsCommand) -> &'static str {
    match subcommand {
        InsightsCommand::Architecture { .. } => "insights:architecture",
        InsightsCommand::Metrics { .. } => "insights:metrics",
        InsightsCommand::Risk { .. } => "insights:risk",
        InsightsCommand::Patterns { .. } => "insights:patterns",
        InsightsCommand::LargeFunctions { .. } => "insights:large-functions",
        InsightsCommand::ComplexFunctions { .. } => "insights:complex-functions",
    }
}

fn apply_finding_limit(
    findings: &mut Vec<InsightFinding>,
    summary: &mut InsightSummary,
    limit: Option<usize>,
) {
    if let Some(limit) = limit {
        *findings = std::mem::take(findings).into_iter().take(limit).collect();
        summary.total_findings = findings.len();
        summary.highest_severity = findings.iter().map(|finding| finding.severity).max();
    }
}

fn print_compact_report(title: &str, findings: &[InsightFinding]) {
    if findings.is_empty() {
        println!("No {title} findings.");
        return;
    }

    println!("{title} ({})", findings.len());
    for finding in findings {
        println!("  - [{}] {}", finding.severity, finding.title);
        println!("    {}", finding.message);
    }
}

pub fn run_insights(cli: &Cli) -> Result<()> {
    let repo = resolve_repo(cli)?;
    let command_label = match &cli.command {
        Command::Insights { subcommand, .. } => insights_command_label(subcommand),
        _ => "insights",
    };
    let mut adapter = CliAdapter::open(&repo);
    if let Some(ref mut active) = adapter {
        active.before_command(command_label);
    }

    let result = (|| -> Result<()> {
        let db_path = db_path(cli, &repo);
        let (allow_stale, allow_partial) = match &cli.command {
            Command::Insights {
                allow_stale,
                allow_partial,
                ..
            } => (*allow_stale, *allow_partial),
            _ => (false, false),
        };

        let store = match Store::open(&db_path) {
            Ok(store) => store,
            Err(error) => {
                let readiness =
                    derive_graph_readiness_open_failed(&repo, &db_path, &error.to_string());
                check_graph_readiness(
                    &readiness,
                    GraphToolRequirement::Analysis,
                    readiness_overrides(allow_stale, allow_partial),
                    "insights",
                    cli,
                )?;
                return Err(error).with_context(|| format!("cannot open database at {db_path}"));
            }
        };

        let readiness = derive_graph_readiness(&store, &repo, &db_path);
        if let Some(warning) = check_graph_readiness(
            &readiness,
            GraphToolRequirement::Analysis,
            readiness_overrides(allow_stale, allow_partial),
            "insights",
            cli,
        )? {
            eprintln!("Warning: {warning}");
        }

        let config =
            atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(&repo)).unwrap_or_default();
        let engine = InsightsEngine::new(&store, config.insights.clone())
            .context("cannot initialize insights engine")?;

        let subcommand = match &cli.command {
            Command::Insights { subcommand, .. } => subcommand,
            _ => unreachable!(),
        };

        match subcommand {
            InsightsCommand::Architecture { limit } => {
                let mut analysis = engine
                    .analyze_architecture(&repo)
                    .context("architecture analysis failed")?;
                apply_finding_limit(
                    &mut analysis.report.findings,
                    &mut analysis.report.summary,
                    *limit,
                );

                if cli.json {
                    print_json(
                        "insights_architecture",
                        serde_json::to_value(&analysis.report)?,
                    )?;
                } else {
                    print_compact_report("architecture insight", &analysis.report.findings);
                }
            }
            InsightsCommand::Metrics { limit } => {
                let mut analysis = engine
                    .analyze_metrics(&repo)
                    .context("metrics analysis failed")?;
                apply_finding_limit(
                    &mut analysis.report.findings,
                    &mut analysis.report.summary,
                    *limit,
                );

                if cli.json {
                    print_json("insights_metrics", serde_json::to_value(&analysis.report)?)?;
                } else {
                    print_compact_report("metrics insight", &analysis.report.findings);
                }
            }
            InsightsCommand::Risk { symbol } => {
                let analysis = engine
                    .assess_risk(
                        &repo,
                        RiskAssessmentTarget::Symbol {
                            symbol: symbol.clone(),
                        },
                    )
                    .with_context(|| format!("risk assessment failed for `{symbol}`"))?;

                if cli.json {
                    print_json("insights_risk", serde_json::to_value(&analysis.report)?)?;
                } else {
                    println!("Risk for {}", analysis.target.qualified_name);
                    println!("  Score         : {:.2}", analysis.score);
                    println!("  Classification: {}", analysis.classification);
                    for factor in analysis.factor_contributions.iter().take(5) {
                        println!(
                            "  - {} {:.2} ({})",
                            factor.factor, factor.contribution, factor.reason,
                        );
                    }
                }
            }
            InsightsCommand::Patterns { limit } => {
                let mut report = engine
                    .analyze_patterns()
                    .context("pattern analysis failed")?;
                apply_finding_limit(&mut report.findings, &mut report.summary, *limit);

                if cli.json {
                    print_json("insights_patterns", serde_json::to_value(&report)?)?;
                } else {
                    print_compact_report("pattern insight", &report.findings);
                }
            }
            InsightsCommand::LargeFunctions {
                files,
                threshold,
                complexity_threshold,
                cognitive_threshold,
                nesting_threshold,
                mode,
                limit,
                include_tests,
            } => {
                let analysis = engine
                    .find_large_functions(
                        &repo,
                        LargeFunctionRequest {
                            files: (!files.is_empty()).then(|| files.clone()),
                            changed_files: None,
                            threshold: *threshold,
                            complexity_threshold: *complexity_threshold,
                            cognitive_threshold: *cognitive_threshold,
                            nesting_threshold: *nesting_threshold,
                            mode: (*mode).into(),
                            limit: *limit,
                            include_tests: *include_tests,
                        },
                    )
                    .context("large-function analysis failed")?;

                if cli.json {
                    print_json(
                        "insights_large_functions",
                        serde_json::to_value(&analysis.report)?,
                    )?;
                } else if analysis.candidates.is_empty() {
                    println!("No large or complex functions matched current thresholds.");
                } else {
                    println!(
                        "Large/complex functions ({}):",
                        analysis.report.summary.total_findings
                    );
                    for candidate in &analysis.candidates {
                        println!(
                            "  - {} {}:{}-{} loc={} fan-in={} fan-out={}",
                            candidate.qualified_name,
                            candidate.file_path,
                            candidate.line_start,
                            candidate.line_end,
                            candidate.loc,
                            candidate.fan_in,
                            candidate.fan_out,
                        );
                        println!("    {}", candidate.ranking_reason);
                    }
                }
            }
            InsightsCommand::ComplexFunctions {
                files,
                complexity_threshold,
                cognitive_threshold,
                nesting_threshold,
                limit,
                include_tests,
            } => {
                let analysis = engine
                    .find_large_functions(
                        &repo,
                        LargeFunctionRequest {
                            files: (!files.is_empty()).then(|| files.clone()),
                            changed_files: None,
                            threshold: None,
                            complexity_threshold: *complexity_threshold,
                            cognitive_threshold: *cognitive_threshold,
                            nesting_threshold: *nesting_threshold,
                            mode: LargeFunctionMode::Complex,
                            limit: *limit,
                            include_tests: *include_tests,
                        },
                    )
                    .context("complex-function analysis failed")?;

                if cli.json {
                    print_json(
                        "insights_complex_functions",
                        serde_json::to_value(&analysis.report)?,
                    )?;
                } else if analysis.candidates.is_empty() {
                    println!("No complex functions matched current thresholds.");
                } else {
                    println!(
                        "Complex functions ({}):",
                        analysis.report.summary.total_findings
                    );
                    for candidate in &analysis.candidates {
                        println!(
                            "  - {} {}:{}-{} cyclomatic={:?} cognitive={:?} nesting={:?}",
                            candidate.qualified_name,
                            candidate.file_path,
                            candidate.line_start,
                            candidate.line_end,
                            candidate.cyclomatic_complexity,
                            candidate.cognitive_complexity,
                            candidate.max_nesting_depth,
                        );
                        println!("    {}", candidate.ranking_reason);
                    }
                }
            }
        }

        Ok(())
    })();

    if let Some(ref mut active) = adapter {
        active.after_command(command_label, result.is_ok());
    }
    result
}
