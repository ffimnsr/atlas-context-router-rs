use anyhow::{Context, Result};
use atlas_core::{
    BudgetManager, BudgetPolicy, BudgetStatus, InsightFinding, InsightSummary, NodeKind,
};
use atlas_reasoning::{
    AnalysisRankingPrimitives, AnalysisTrimmingPrimitives, InsightsEngine, LargeFunctionMode,
    LargeFunctionRequest, ReasoningEngine, RiskAssessmentTarget, sort_dead_code_candidates,
    sort_dependency_result, sort_refactor_safety_result, sort_removal_result,
};

use super::shared::{
    bool_arg, inject_budget_metadata, open_store, str_arg, string_array_arg, tool_result_value,
    u64_arg,
};
use crate::output::{OutputFormat, render_value};
use serde_json::json;

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

fn report_files(findings: &[InsightFinding]) -> Vec<String> {
    findings
        .iter()
        .flat_map(|finding| finding.evidence.iter())
        .filter_map(|evidence| evidence.file_path.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn insight_report_response<T: serde::Serialize>(
    full_payload: &T,
    compact_payload: serde_json::Value,
    output_format: OutputFormat,
    verbose: bool,
) -> Result<serde_json::Value> {
    let full_payload = serde_json::to_value(full_payload)?;
    let mut response = tool_result_value(&full_payload, output_format)?;

    if output_format != OutputFormat::Json && !verbose {
        let rendered = render_value(&compact_payload, output_format)?;
        response["content"][0]["text"] = json!(rendered.text);
        response["content"][0]["mimeType"] = json!(rendered.actual_format.mime_type());
        response["_meta"]["atlas:outputFormat"] = json!(rendered.actual_format.as_str());
        response["_meta"]["atlas:requestedOutputFormat"] =
            json!(rendered.requested_format.as_str());
        if let Some(reason) = rendered.fallback_reason {
            response["_meta"]["atlas:fallbackReason"] = json!(reason);
        } else if let Some(meta) = response
            .get_mut("_meta")
            .and_then(|value| value.as_object_mut())
        {
            meta.remove("atlas:fallbackReason");
        }
    }

    Ok(response)
}

pub(super) fn tool_analyze_architecture(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let limit = u64_arg(args, "limit").map(|value| value as usize);
    let verbose = bool_arg(args, "verbose").unwrap_or(false);
    let store = open_store(db_path)?;
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    let engine = InsightsEngine::new(&store, config.insights.clone())
        .context("cannot initialize insights engine")?;
    let mut analysis = engine
        .analyze_architecture(repo_root)
        .context("architecture analysis failed")?;
    apply_finding_limit(
        &mut analysis.report.findings,
        &mut analysis.report.summary,
        limit,
    );

    let compact = serde_json::json!({
        "summary": analysis.report.summary.clone(),
        "top_findings": analysis.report.findings.clone(),
        "module_count": analysis.modules.len(),
        "module_edge_count": analysis.edges.len(),
    });
    let mut response = insight_report_response(&analysis.report, compact, output_format, verbose)?;
    response["atlas_result_kind"] = serde_json::json!("architecture_report");
    response["atlas_result_count"] = serde_json::json!(analysis.report.summary.total_findings);
    response["atlas_result_files"] = serde_json::json!(report_files(&analysis.report.findings));
    Ok(response)
}

pub(super) fn tool_analyze_metrics(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let limit = u64_arg(args, "limit").map(|value| value as usize);
    let verbose = bool_arg(args, "verbose").unwrap_or(false);
    let store = open_store(db_path)?;
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    let engine = InsightsEngine::new(&store, config.insights.clone())
        .context("cannot initialize insights engine")?;
    let mut analysis = engine
        .analyze_metrics(repo_root)
        .context("metrics analysis failed")?;
    apply_finding_limit(
        &mut analysis.report.findings,
        &mut analysis.report.summary,
        limit,
    );

    let compact = serde_json::json!({
        "summary": analysis.report.summary.clone(),
        "top_findings": analysis.report.findings.clone(),
        "node_metric_count": analysis.metrics.node_metrics.len(),
        "file_metric_count": analysis.metrics.file_metrics.len(),
        "module_metric_count": analysis.metrics.module_metrics.len(),
    });
    let mut response = insight_report_response(&analysis.report, compact, output_format, verbose)?;
    response["atlas_result_kind"] = serde_json::json!("metrics_report");
    response["atlas_result_count"] = serde_json::json!(analysis.report.summary.total_findings);
    response["atlas_result_files"] = serde_json::json!(report_files(&analysis.report.findings));
    Ok(response)
}

pub(super) fn tool_assess_risk(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let symbol = str_arg(args, "symbol")?
        .ok_or_else(|| anyhow::anyhow!("assess_risk requires 'symbol'"))?
        .to_owned();
    let verbose = bool_arg(args, "verbose").unwrap_or(false);
    let store = open_store(db_path)?;
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    let engine = InsightsEngine::new(&store, config.insights.clone())
        .context("cannot initialize insights engine")?;
    let analysis = engine
        .assess_risk(
            repo_root,
            RiskAssessmentTarget::Symbol {
                symbol: symbol.clone(),
            },
        )
        .with_context(|| format!("risk assessment failed for `{symbol}`"))?;

    let compact = serde_json::json!({
        "target": analysis.target.qualified_name,
        "score": analysis.score,
        "classification": analysis.classification,
        "summary": analysis.report.summary.clone(),
        "top_findings": analysis.report.findings.clone(),
        "top_factors": analysis.factor_contributions.iter().take(5).collect::<Vec<_>>(),
    });
    let mut response = insight_report_response(&analysis.report, compact, output_format, verbose)?;
    response["atlas_result_kind"] = serde_json::json!("risk_report");
    response["atlas_result_count"] = serde_json::json!(analysis.report.summary.total_findings);
    response["atlas_result_files"] = serde_json::json!(report_files(&analysis.report.findings));
    Ok(response)
}

pub(super) fn tool_analyze_patterns(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let limit = u64_arg(args, "limit").map(|value| value as usize);
    let verbose = bool_arg(args, "verbose").unwrap_or(false);
    let store = open_store(db_path)?;
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    let engine = InsightsEngine::new(&store, config.insights.clone())
        .context("cannot initialize insights engine")?;
    let mut report = engine
        .analyze_patterns()
        .context("pattern analysis failed")?;
    apply_finding_limit(&mut report.findings, &mut report.summary, limit);

    let compact = serde_json::json!({
        "summary": report.summary.clone(),
        "top_findings": report.findings.clone(),
    });
    let mut response = insight_report_response(&report, compact, output_format, verbose)?;
    response["atlas_result_kind"] = serde_json::json!("pattern_report");
    response["atlas_result_count"] = serde_json::json!(report.summary.total_findings);
    response["atlas_result_files"] = serde_json::json!(report_files(&report.findings));
    Ok(response)
}

fn tool_find_large_functions_impl(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
    forced_mode: Option<LargeFunctionMode>,
    result_kind: &str,
) -> Result<serde_json::Value> {
    let files = string_array_arg(args, "files")?;
    let threshold = u64_arg(args, "threshold").map(|value| value as usize);
    let complexity_threshold = u64_arg(args, "complexity_threshold").map(|value| value as usize);
    let cognitive_threshold = u64_arg(args, "cognitive_threshold").map(|value| value as usize);
    let nesting_threshold = u64_arg(args, "nesting_threshold").map(|value| value as usize);
    let limit = u64_arg(args, "limit").map(|value| value as usize);
    let include_tests = bool_arg(args, "include_tests").unwrap_or(false);
    let verbose = bool_arg(args, "verbose").unwrap_or(false);
    let mode = match forced_mode {
        Some(mode) => mode,
        None => match str_arg(args, "mode")?.unwrap_or("large-or-complex") {
            "large" => LargeFunctionMode::Large,
            "complex" => LargeFunctionMode::Complex,
            "large-or-complex" => LargeFunctionMode::LargeOrComplex,
            other => {
                anyhow::bail!(
                    "find_large_functions unsupported mode '{other}'; expected 'large', 'complex', or 'large-or-complex'"
                )
            }
        },
    };

    let store = open_store(db_path)?;
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    let engine = InsightsEngine::new(&store, config.insights.clone())
        .context("cannot initialize insights engine")?;
    let analysis = engine
        .find_large_functions(
            repo_root,
            LargeFunctionRequest {
                files: (!files.is_empty()).then_some(files),
                changed_files: None,
                threshold,
                complexity_threshold,
                cognitive_threshold,
                nesting_threshold,
                mode,
                limit,
                include_tests,
            },
        )
        .context("large-function analysis failed")?;

    let compact = serde_json::json!({
        "mode": analysis.request.mode,
        "summary": analysis.report.summary.clone(),
        "top_findings": analysis.candidates.clone(),
    });
    let mut response =
        insight_report_response(&analysis.report_result(), compact, output_format, verbose)?;
    response["atlas_result_kind"] = serde_json::json!(result_kind);
    response["atlas_result_count"] = serde_json::json!(analysis.report.summary.total_findings);
    response["atlas_result_files"] = serde_json::json!(
        analysis
            .candidates
            .iter()
            .map(|candidate| candidate.file_path.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    );
    Ok(response)
}

pub(super) fn tool_find_large_functions(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    tool_find_large_functions_impl(
        args,
        repo_root,
        db_path,
        output_format,
        None,
        "large_function_report",
    )
}

pub(super) fn tool_find_complex_functions(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    tool_find_large_functions_impl(
        args,
        repo_root,
        db_path,
        output_format,
        Some(LargeFunctionMode::Complex),
        "complex_function_report",
    )
}

pub(super) fn tool_analyze_safety(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let symbol = str_arg(args, "symbol")?
        .ok_or_else(|| anyhow::anyhow!("analyze_safety requires 'symbol'"))?
        .to_owned();

    let store = open_store(db_path)?;
    let engine = ReasoningEngine::new(&store);
    let mut result = engine
        .score_refactor_safety(&symbol)
        .with_context(|| format!("safety scoring for `{symbol}` failed"))?;
    sort_refactor_safety_result(&mut result);

    let payload = serde_json::json!({
        "symbol": result.node.qualified_name,
        "kind": result.node.kind.as_str(),
        "file": result.node.file_path,
        "safety_score": result.safety.score,
        "safety_band": format!("{:?}", result.safety.band),
        "fan_in": result.fan_in,
        "fan_out": result.fan_out,
        "linked_tests": result.linked_test_count,
        "coverage_strength": format!("{:?}", result.coverage_strength),
        "unresolved_edges": result.unresolved_edge_count,
        "reasons": result.safety.reasons,
        "suggested_validations": result.safety.suggested_validations,
        "evidence": result.evidence.iter().map(|e| serde_json::json!({ "key": e.key, "value": e.value })).collect::<Vec<_>>(),
    });
    let mut response = tool_result_value(&payload, output_format)?;
    inject_budget_metadata(&mut response, &result.budget);
    Ok(response)
}

pub(super) fn tool_analyze_remove(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let symbols = string_array_arg(args, "symbols")?;
    if symbols.is_empty() {
        return Err(anyhow::anyhow!(
            "analyze_remove requires at least one symbol in 'symbols'"
        ));
    }
    let max_depth = u64_arg(args, "max_depth").unwrap_or(3) as u32;
    let max_nodes = u64_arg(args, "max_nodes").unwrap_or(200) as usize;
    // Compact output defaults — caller may raise these.
    let requested_max_files = u64_arg(args, "max_files").unwrap_or(20) as usize;
    let requested_max_edges = u64_arg(args, "max_edges").unwrap_or(50) as usize;

    let store = open_store(db_path)?;
    let engine = ReasoningEngine::new(&store);
    let policy = BudgetPolicy::default();
    let mut budgets = BudgetManager::new();
    let max_files = budgets.resolve_limit(
        policy.review_context_extraction.files,
        "review_context_extraction.max_files",
        Some(requested_max_files),
    );
    let max_edges = budgets.resolve_limit(
        policy.mcp_cli_payload_serialization.edges,
        "mcp_cli_payload_serialization.max_edges",
        Some(requested_max_edges),
    );
    let ranking = AnalysisRankingPrimitives::default();
    let trimming = AnalysisTrimmingPrimitives::default();
    let symbol_refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
    let mut result = engine
        .analyze_removal(&symbol_refs, Some(max_depth), Some(max_nodes))
        .context("analyze_removal failed")?;
    sort_removal_result(&mut result, &ranking);

    let omitted = result
        .impacted_symbols
        .len()
        .saturating_sub(trimming.removal_symbol_preview_limit);

    let impacted_preview: Vec<_> = result
        .impacted_symbols
        .iter()
        .take(trimming.removal_symbol_preview_limit)
        .map(|im| {
            serde_json::json!({
                "qn": im.node.qualified_name,
                "kind": im.node.kind.as_str(),
                "file": im.node.file_path,
                "depth": im.depth,
                "impact_class": format!("{:?}", im.impact_class),
            })
        })
        .collect();

    let omitted_files = result.impacted_files.len().saturating_sub(max_files);
    let omitted_edges = result.relevant_edges.len().saturating_sub(max_edges);

    let payload = serde_json::json!({
        "seed_count": result.seed.len(),
        "impacted_symbol_count": result.impacted_symbols.len(),
        "impacted_file_count": result.impacted_files.len(),
        "impacted_test_count": result.impacted_tests.len(),
        "impacted_symbols": impacted_preview,
        "impacted_files": &result.impacted_files[..result.impacted_files.len().min(max_files)],
        "omitted_file_count": omitted_files,
        "omitted_symbol_count": omitted,
        "omitted_edge_count": omitted_edges,
        "warnings": result.warnings.iter().map(|w| serde_json::json!({
            "message": w.message,
            "confidence": format!("{:?}", w.confidence),
            "error_code": w.error_code,
            "suggestions": w.suggestions,
        })).collect::<Vec<_>>(),
        "uncertainty_flags": result.uncertainty_flags,
        "evidence": result.evidence.iter().map(|e| serde_json::json!({ "key": e.key, "value": e.value })).collect::<Vec<_>>(),
    });
    let mut response = tool_result_value(&payload, output_format)?;
    let budget = if result.budget.budget_hit {
        result.budget.clone()
    } else {
        let local_budget =
            budgets.summary("review_context_extraction.max_files", max_files, max_files);
        if matches!(local_budget.budget_status, BudgetStatus::WithinBudget) {
            budgets.summary(
                "mcp_cli_payload_serialization.max_edges",
                max_edges,
                max_edges,
            )
        } else {
            local_budget
        }
    };
    inject_budget_metadata(&mut response, &budget);
    Ok(response)
}

pub(super) fn tool_analyze_dead_code(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let allowlist = string_array_arg(args, "allowlist").unwrap_or_default();
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    // Compact default: 50 candidates. Caller may raise with `limit`.
    let requested_limit = u64_arg(args, "limit").unwrap_or(50) as usize;
    let summary = bool_arg(args, "summary").unwrap_or(false);
    let exclude_kind_strs = string_array_arg(args, "exclude_kind").unwrap_or_default();
    let exclude_kinds: Vec<NodeKind> = exclude_kind_strs
        .iter()
        .filter_map(|k| k.parse().ok())
        .collect();
    // `code_only` is always true at the store level; the flag is accepted for
    // forward compatibility but has no effect on the current implementation.

    let store = open_store(db_path)?;
    let engine = ReasoningEngine::new(&store);
    let policy = BudgetPolicy::default();
    let mut budgets = BudgetManager::new();
    let limit = budgets.resolve_limit(
        policy.query_candidates_and_seeds.candidates,
        "query_candidates_and_seeds.max_candidates",
        Some(requested_limit),
    );
    let ranking = AnalysisRankingPrimitives::default();
    let trimming = AnalysisTrimmingPrimitives::default();
    let allowlist_refs: Vec<&str> = allowlist.iter().map(String::as_str).collect();
    let mut candidates = engine
        .detect_dead_code(
            &allowlist_refs,
            subpath.as_deref(),
            Some(limit),
            &exclude_kinds,
        )
        .context("detect_dead_code failed")?;
    sort_dead_code_candidates(&mut candidates, &ranking);

    if summary {
        let payload = serde_json::json!({
            "candidate_count": candidates.len(),
            "applied_limit": limit,
            "applied_subpath": subpath,
            "excluded_kinds": exclude_kind_strs,
        });
        let mut response = tool_result_value(&payload, output_format)?;
        inject_budget_metadata(
            &mut response,
            &budgets.summary(
                "query_candidates_and_seeds.max_candidates",
                limit,
                candidates.len(),
            ),
        );
        return Ok(response);
    }

    let omitted = candidates
        .len()
        .saturating_sub(trimming.dead_code_candidate_preview_limit);

    let preview: Vec<_> = candidates
        .iter()
        .take(trimming.dead_code_candidate_preview_limit)
        .map(|c| {
            serde_json::json!({
                "qn": c.node.qualified_name,
                "kind": c.node.kind.as_str(),
                "file": c.node.file_path,
                "line": c.node.line_start,
                "certainty": format!("{:?}", c.certainty),
                "reasons": c.reasons,
                "blockers": c.blockers,
            })
        })
        .collect();

    let payload = serde_json::json!({
        "candidate_count": candidates.len(),
        "omitted_count": omitted,
        "candidates": preview,
        "applied_limit": limit,
        "applied_subpath": subpath,
        "excluded_kinds": exclude_kind_strs,
    });
    let mut response = tool_result_value(&payload, output_format)?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "query_candidates_and_seeds.max_candidates",
            limit,
            candidates.len(),
        ),
    );
    Ok(response)
}

pub(super) fn tool_analyze_dependency(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let symbol = str_arg(args, "symbol")?
        .ok_or_else(|| anyhow::anyhow!("analyze_dependency requires 'symbol'"))?
        .to_owned();

    let store = open_store(db_path)?;
    let engine = ReasoningEngine::new(&store);
    let ranking = AnalysisRankingPrimitives::default();
    let trimming = AnalysisTrimmingPrimitives::default();
    let mut result = engine
        .check_dependency_removal(&symbol)
        .with_context(|| format!("dependency check for `{symbol}` failed"))?;
    sort_dependency_result(&mut result, &ranking);

    let omitted = result
        .blocking_references
        .len()
        .saturating_sub(trimming.dependency_blocker_preview_limit);

    let blocking_preview: Vec<_> = result
        .blocking_references
        .iter()
        .take(trimming.dependency_blocker_preview_limit)
        .map(|n| {
            serde_json::json!({
                "qn": n.qualified_name,
                "kind": n.kind.as_str(),
                "file": n.file_path,
            })
        })
        .collect();

    let payload = serde_json::json!({
        "symbol": result.target_qname,
        "removable": result.removable,
        "confidence": format!("{:?}", result.confidence),
        "blocking_reference_count": result.blocking_references.len(),
        "blocking_references": blocking_preview,
        "omitted_blocking_count": omitted,
        "suggested_cleanups": result.suggested_cleanups,
        "uncertainty_flags": result.uncertainty_flags,
        "evidence": result.evidence.iter().map(|e| serde_json::json!({ "key": e.key, "value": e.value })).collect::<Vec<_>>(),
    });
    let mut response = tool_result_value(&payload, output_format)?;
    inject_budget_metadata(&mut response, &result.budget);
    Ok(response)
}
