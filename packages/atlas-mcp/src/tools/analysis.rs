use anyhow::{Context, Result};
use atlas_core::{BudgetManager, BudgetPolicy, BudgetStatus, NodeKind, kinds::parse_kind_alias};
use atlas_reasoning::{
    AnalysisRankingPrimitives, AnalysisTrimmingPrimitives, ComponentLabelRequest,
    DuplicateDetectionRequest, InsightsEngine, LargeFunctionMode, LargeFunctionRequest,
    ReasoningEngine, RiskAssessmentTarget, SimilarFunctionRequest, sort_dead_code_candidates,
    sort_dependency_result, sort_refactor_safety_result, sort_removal_result,
};

use super::shared::{
    bool_arg, inject_budget_metadata, open_store, str_arg, string_array_arg, tool_result_value,
    u64_arg,
};
use serde_json::json;

use super::analysis_bounds::{
    collapse_repeated_chain_details, compact_insight_report, compact_modules,
    insight_report_response, limit_modules, paginate_findings, payload_byte_limit,
};

pub(super) fn tool_analyze_architecture(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let limit = u64_arg(args, "limit").map(|value| value as usize);
    let verbose = bool_arg(args, "verbose").unwrap_or(false);
    let store = open_store(db_path)?;
    let atlas_dir = atlas_engine::paths::atlas_dir(repo_root);
    let config =
        atlas_engine::Config::load(&atlas_dir).context("cannot load .atlas/config.toml")?;
    let insights = config
        .insights
        .with_loaded_layer_rules(&atlas_dir)
        .context("cannot load insights layer rules")?;
    let engine =
        InsightsEngine::new(&store, insights).context("cannot initialize insights engine")?;
    let mut analysis = engine
        .analyze_architecture(repo_root)
        .context("architecture analysis failed")?;
    let (total_available, offset) = paginate_findings(
        &mut analysis.report.findings,
        &mut analysis.report.summary,
        0,
        limit,
    );

    let compact = compact_insight_report(
        &analysis.report.summary,
        &analysis.report.findings,
        &analysis.report.atlas_provenance,
        analysis.report.atlas_freshness.as_ref(),
        json!({
            "tool": "analyze_architecture",
            "module_count": analysis.modules.len(),
            "module_edge_count": analysis.edges.len(),
            "counts": analysis.counts,
            "findings_offset": offset,
            "findings_total_available": total_available,
        }),
    );
    let mut full_payload = serde_json::to_value(&analysis.report)?;
    if let Some(object) = full_payload.as_object_mut() {
        object.insert("counts".to_owned(), json!(analysis.counts));
    }
    insight_report_response(
        &full_payload,
        compact,
        output_format,
        verbose,
        payload_byte_limit(args),
    )
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
    let (total_available, offset) = paginate_findings(
        &mut analysis.report.findings,
        &mut analysis.report.summary,
        0,
        limit,
    );

    let compact = compact_insight_report(
        &analysis.report.summary,
        &analysis.report.findings,
        &analysis.report.atlas_provenance,
        analysis.report.atlas_freshness.as_ref(),
        json!({
            "tool": "analyze_metrics",
            "graph_summary": {
                "node_count": analysis.metrics.graph_summary.node_count,
                "edge_count": analysis.metrics.graph_summary.edge_count,
                "density": analysis.metrics.graph_summary.density,
                "average_degree": analysis.metrics.graph_summary.average_degree,
                "diameter": analysis.metrics.graph_summary.diameter,
                "diameter_scope": analysis.metrics.graph_summary.diameter_scope,
            },
            "node_metric_count": analysis.metrics.node_metrics.len(),
            "file_metric_count": analysis.metrics.file_metrics.len(),
            "module_metric_count": analysis.metrics.module_metrics.len(),
            "findings_offset": offset,
            "findings_total_available": total_available,
        }),
    );
    let mut full_payload = serde_json::to_value(&analysis.report)?;
    if let Some(object) = full_payload.as_object_mut() {
        object.insert(
            "graph_summary".to_owned(),
            json!({
                "node_count": analysis.metrics.graph_summary.node_count,
                "edge_count": analysis.metrics.graph_summary.edge_count,
                "density": analysis.metrics.graph_summary.density,
                "average_degree": analysis.metrics.graph_summary.average_degree,
                "diameter": analysis.metrics.graph_summary.diameter,
                "diameter_scope": analysis.metrics.graph_summary.diameter_scope,
            }),
        );
    }
    insight_report_response(
        &full_payload,
        compact,
        output_format,
        verbose,
        payload_byte_limit(args),
    )
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

    let compact = compact_insight_report(
        &analysis.report.summary,
        &analysis.report.findings,
        &analysis.report.atlas_provenance,
        analysis.report.atlas_freshness.as_ref(),
        json!({
            "tool": "assess_risk",
            "target": analysis.target.qualified_name,
            "score": analysis.score,
            "classification": analysis.classification,
            "top_factors": analysis.factor_contributions.iter().take(5).collect::<Vec<_>>(),
        }),
    );
    // `verbose: true` must be a strict superset of the compact payload: the
    // raw report alone drops the target identity, score, and classification.
    let mut full_payload = serde_json::to_value(&analysis.report)?;
    if let Some(object) = full_payload.as_object_mut() {
        object.insert("tool".to_owned(), json!("assess_risk"));
        object.insert("target".to_owned(), json!(analysis.target.qualified_name));
        object.insert("score".to_owned(), json!(analysis.score));
        object.insert("classification".to_owned(), json!(analysis.classification));
        object.insert(
            "top_factors".to_owned(),
            json!(analysis.factor_contributions),
        );
    }
    insight_report_response(
        &full_payload,
        compact,
        output_format,
        verbose,
        payload_byte_limit(args),
    )
}

pub(super) fn tool_analyze_patterns(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let verbose = bool_arg(args, "verbose").unwrap_or(false);
    let store = open_store(db_path)?;
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    let engine = InsightsEngine::new(&store, config.insights.clone())
        .context("cannot initialize insights engine")?;
    let mut report = engine
        .analyze_patterns(repo_root)
        .context("pattern analysis failed")?;
    // Repeated-chain findings carry one occurrence payload per match; collapse
    // them before paging so even verbose callers get a bounded report.
    collapse_repeated_chain_details(&mut report.findings);
    let offset = u64_arg(args, "offset").unwrap_or(0) as usize;
    let limit = u64_arg(args, "max_findings")
        .or_else(|| u64_arg(args, "limit"))
        .map(|value| value as usize);
    let (total_available, offset) =
        paginate_findings(&mut report.findings, &mut report.summary, offset, limit);

    let compact = compact_insight_report(
        &report.summary,
        &report.findings,
        &report.atlas_provenance,
        report.atlas_freshness.as_ref(),
        json!({
            "tool": "analyze_patterns",
            "findings_offset": offset,
            "findings_total_available": total_available,
        }),
    );
    insight_report_response(
        &report,
        compact,
        output_format,
        verbose,
        payload_byte_limit(args),
    )
}

fn normalize_files_arg(repo_root: &str, files: &[String]) -> Result<Vec<String>> {
    files
        .iter()
        .map(|path| {
            atlas_repo::normalize_repo_file_path(camino::Utf8Path::new(repo_root), path)
                .map(|resolved| resolved.canonical)
                .map_err(|error| anyhow::anyhow!("invalid file path '{path}': {error}"))
        })
        .collect()
}

fn tool_find_large_functions_impl(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
    forced_mode: Option<LargeFunctionMode>,
    result_kind: &str,
) -> Result<serde_json::Value> {
    let files = normalize_files_arg(repo_root, &string_array_arg(args, "files")?)?;
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

    let compact = compact_insight_report(
        &analysis.report.summary,
        &analysis.report.findings,
        &analysis.report.atlas_provenance,
        analysis.report.atlas_freshness.as_ref(),
        json!({
            "tool": result_kind,
            "mode": analysis.request.mode,
            "candidate_count": analysis.candidates.len(),
            "candidates": analysis.candidates
                .iter()
                .take(50)
                .cloned()
                .collect::<Vec<_>>(),
        }),
    );
    insight_report_response(
        &analysis.report_result(),
        compact,
        output_format,
        verbose,
        payload_byte_limit(args),
    )
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
        "complex_functions",
    )
}

pub(super) fn tool_find_similar_functions(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let symbol = str_arg(args, "symbol")?
        .ok_or_else(|| anyhow::anyhow!("find_similar_functions requires 'symbol'"))?
        .to_owned();
    let min_score = args
        .and_then(|value| value.get("min_score"))
        .and_then(|value| value.as_f64());
    let limit = u64_arg(args, "limit").map(|value| value as usize);
    let include_same_file = bool_arg(args, "include_same_file").unwrap_or(false);
    let verbose = bool_arg(args, "verbose").unwrap_or(false);
    let store = open_store(db_path)?;
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    let engine = InsightsEngine::new(&store, config.insights.clone())
        .context("cannot initialize insights engine")?;
    let analysis = engine
        .find_similar_functions(
            repo_root,
            SimilarFunctionRequest {
                symbol,
                limit,
                min_score,
                include_same_file,
            },
        )
        .context("similar-function analysis failed")?;
    let compact = compact_insight_report(
        &analysis.report.summary,
        &analysis.report.findings,
        &analysis.report.atlas_provenance,
        analysis.report.atlas_freshness.as_ref(),
        json!({
            "tool": "find_similar_functions",
            "source": analysis.source,
            "thresholds": analysis.thresholds,
            "matches": analysis.matches,
            "no_candidates_hint": analysis.no_candidates_hint(),
        }),
    );
    insight_report_response(
        &analysis.report_result(),
        compact,
        output_format,
        verbose,
        payload_byte_limit(args),
    )
}

pub(super) fn tool_find_duplicates(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let files = normalize_files_arg(repo_root, &string_array_arg(args, "files")?)?;
    let min_score = args
        .and_then(|value| value.get("min_score"))
        .and_then(|value| value.as_f64());
    let limit = u64_arg(args, "limit").map(|value| value as usize);
    let include_tests = bool_arg(args, "include_tests").unwrap_or(false);
    let suppressions = string_array_arg(args, "suppressions")?;
    let verbose = bool_arg(args, "verbose").unwrap_or(false);
    let store = open_store(db_path)?;
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    let engine = InsightsEngine::new(&store, config.insights.clone())
        .context("cannot initialize insights engine")?;
    let analysis = engine
        .find_duplicates(
            repo_root,
            DuplicateDetectionRequest {
                files: (!files.is_empty()).then_some(files),
                limit,
                min_score,
                include_tests,
                suppressions,
            },
        )
        .context("duplicate detection failed")?;
    let compact = compact_insight_report(
        &analysis.report.summary,
        &analysis.report.findings,
        &analysis.report.atlas_provenance,
        analysis.report.atlas_freshness.as_ref(),
        json!({
            "tool": "find_duplicates",
            "thresholds": analysis.thresholds,
            "group_count": analysis.groups.len(),
            "groups": analysis.groups,
        }),
    );
    insight_report_response(
        &analysis.report_result(),
        compact,
        output_format,
        verbose,
        payload_byte_limit(args),
    )
}

pub(super) fn tool_infer_modules(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let limit = u64_arg(args, "limit").map(|value| value as usize);
    let verbose = bool_arg(args, "verbose").unwrap_or(false);
    let store = open_store(db_path)?;
    let atlas_dir = atlas_engine::paths::atlas_dir(repo_root);
    let config =
        atlas_engine::Config::load(&atlas_dir).context("cannot load .atlas/config.toml")?;
    let insights = config
        .insights
        .with_loaded_layer_rules(&atlas_dir)
        .context("cannot load insights layer rules")?;
    let engine =
        InsightsEngine::new(&store, insights).context("cannot initialize insights engine")?;
    let mut analysis = engine
        .infer_modules(repo_root)
        .context("module inference failed")?;
    let (total_available, offset) = paginate_findings(
        &mut analysis.report.findings,
        &mut analysis.report.summary,
        0,
        limit,
    );
    let total_modules = analysis.modules.len();
    limit_modules(&mut analysis.modules, limit);
    let compact = compact_insight_report(
        &analysis.report.summary,
        &analysis.report.findings,
        &analysis.report.atlas_provenance,
        analysis.report.atlas_freshness.as_ref(),
        json!({
            "tool": "infer_modules",
            "module_count": total_modules,
            "modules": compact_modules(&analysis.modules),
            "findings_offset": offset,
            "findings_total_available": total_available,
        }),
    );
    insight_report_response(
        &analysis.report_result(),
        compact,
        output_format,
        verbose,
        payload_byte_limit(args),
    )
}

pub(super) fn tool_label_components(
    args: Option<&serde_json::Value>,
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let files = normalize_files_arg(repo_root, &string_array_arg(args, "files")?)?;
    let symbols = string_array_arg(args, "symbols")?;
    let limit = u64_arg(args, "limit").map(|value| value as usize);
    let verbose = bool_arg(args, "verbose").unwrap_or(false);
    let store = open_store(db_path)?;
    let config =
        atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).unwrap_or_default();
    let engine = InsightsEngine::new(&store, config.insights.clone())
        .context("cannot initialize insights engine")?;
    let analysis = engine
        .label_components(
            repo_root,
            ComponentLabelRequest {
                files: (!files.is_empty()).then_some(files),
                symbols: (!symbols.is_empty()).then_some(symbols),
                limit,
            },
        )
        .context("component labeling failed")?;
    let compact = compact_insight_report(
        &analysis.report.summary,
        &analysis.report.findings,
        &analysis.report.atlas_provenance,
        analysis.report.atlas_freshness.as_ref(),
        json!({
            "tool": "label_components",
            "assignment_count": analysis.assignments.len(),
            "assignments": analysis.assignments
                .iter()
                .take(50)
                .cloned()
                .collect::<Vec<_>>(),
        }),
    );
    insight_report_response(
        &analysis.report_result(),
        compact,
        output_format,
        verbose,
        payload_byte_limit(args),
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

    let cross_module_callers = result
        .evidence
        .iter()
        .find(|e| e.key == "cross_module_callers")
        .and_then(|e| e.value.parse::<usize>().ok())
        .unwrap_or(0);
    let payload = serde_json::json!({
        "tool": "analyze_safety",
        "symbol": {
            "qname": result.node.qualified_name,
            "kind": result.node.kind.as_str(),
            "file": result.node.file_path,
            "line": result.node.line_start,
        },
        "fan_in": result.fan_in,
        "fan_out": result.fan_out,
        "test_adjacency": {
            "linked_test_count": result.linked_test_count,
            "coverage_strength": format!("{:?}", result.coverage_strength),
            "has_test_coverage": result.linked_test_count > 0,
        },
        "cross_module_callers": cross_module_callers,
        "safety_score": result.safety.score,
        "safety_band": result.safety.band.to_string(),
        "suggested_validations": result.safety.suggested_validations,
        "factor_evidence": result.evidence.iter().map(|e| serde_json::json!({ "key": e.key, "value": e.value })).collect::<Vec<_>>(),
        "summary": {
            "status": "ok",
            "reason_count": result.safety.reasons.len(),
            "unresolved_edge_count": result.unresolved_edge_count,
        },
        "warnings": result.safety.reasons,
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

    let impact_entry = |im: &atlas_core::ImpactedNode| {
        serde_json::json!({
            "symbol": {
                "qname": im.node.qualified_name,
                "kind": im.node.kind.as_str(),
                "file": im.node.file_path,
                "line": im.node.line_start,
            },
            "depth": im.depth,
            "impact_class": im.impact_class.to_string(),
            "via_edge_kind": im.via_edge_kind.map(|kind| kind.as_str().to_owned()),
        })
    };
    let definite_impacts: Vec<_> = result
        .impacted_symbols
        .iter()
        .filter(|im| matches!(im.impact_class, atlas_core::ImpactClass::Definite))
        .take(trimming.removal_symbol_preview_limit)
        .map(impact_entry)
        .collect();
    let probable_impacts: Vec<_> = result
        .impacted_symbols
        .iter()
        .filter(|im| matches!(im.impact_class, atlas_core::ImpactClass::Probable))
        .take(trimming.removal_symbol_preview_limit)
        .map(impact_entry)
        .collect();
    let weak_impacts: Vec<_> = result
        .impacted_symbols
        .iter()
        .filter(|im| matches!(im.impact_class, atlas_core::ImpactClass::Weak))
        .take(trimming.removal_symbol_preview_limit)
        .map(impact_entry)
        .collect();

    let omitted_files = result.impacted_files.len().saturating_sub(max_files);
    let omitted_edges = result.relevant_edges.len().saturating_sub(max_edges);

    let payload = serde_json::json!({
        "tool": "analyze_remove",
        "symbols": result.seed.iter().map(|node| serde_json::json!({
            "qname": node.qualified_name,
            "kind": node.kind.as_str(),
            "file": node.file_path,
            "line": node.line_start,
        })).collect::<Vec<_>>(),
        "definite_impacts": definite_impacts,
        "probable_impacts": probable_impacts,
        "weak_impacts": weak_impacts,
        "tests": result.impacted_tests.iter().map(|node| serde_json::json!({
            "qname": node.qualified_name,
            "kind": node.kind.as_str(),
            "file": node.file_path,
            "line": node.line_start,
        })).collect::<Vec<_>>(),
        "impacted_files": result
            .impacted_files
            .iter()
            .take(max_files)
            .cloned()
            .collect::<Vec<_>>(),
        "uncertainty_flags": result.uncertainty_flags,
        "summary": {
            "status": "ok",
            "seed_count": result.seed.len(),
            "impacted_symbol_count": result.impacted_symbols.len(),
            "impacted_file_count": result.impacted_files.len(),
            "impacted_test_count": result.impacted_tests.len(),
            "omitted_symbol_count": omitted,
            "omitted_file_count": omitted_files,
            "omitted_edge_count": omitted_edges,
        },
        "warnings": result.warnings.iter().map(|w| serde_json::json!({
            "message": w.message,
            "confidence": w.confidence.to_string(),
            "error_code": w.error_code,
            "suggestions": w.suggestions,
        })).collect::<Vec<_>>(),
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
    repo_root: &str,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let allowlist = string_array_arg(args, "allowlist").unwrap_or_default();
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    // Best-effort normalization: root-prefixed and absolute-under-root forms
    // collapse to canonical repo-relative prefixes; unmatched inputs stay as
    // typed so the prefix filter can still match literally.
    let subpath = subpath.map(|raw| {
        atlas_repo::normalize_repo_file_path(camino::Utf8Path::new(repo_root), &raw)
            .map(|resolved| resolved.canonical)
            .unwrap_or(raw)
    });
    // Compact default: 100 candidates, matching the CLI default so both
    // surfaces page identically. Caller may raise with `limit`.
    let requested_limit = u64_arg(args, "limit").unwrap_or(100) as usize;
    let summary = bool_arg(args, "summary").unwrap_or(false);
    let exclude_kind_strs = string_array_arg(args, "exclude_kind").unwrap_or_default();
    // Accept the same kind aliases as `list_symbols` (`fn`, `record`, ...);
    // `excluded_kinds` in the response keeps the caller-provided values.
    let exclude_kinds: Vec<NodeKind> = exclude_kind_strs
        .iter()
        .filter_map(|k| parse_kind_alias(k))
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
        .detect_dead_code(&allowlist_refs, subpath.as_deref(), None, &exclude_kinds)
        .context("detect_dead_code failed")?;
    sort_dead_code_candidates(&mut candidates, &ranking);
    // Summary totals must describe the full candidate set, not the paged
    // preview. Compute them before the caller-facing limit is applied.
    let total_available = candidates.len();
    let mut certainty_counts = std::collections::BTreeMap::<String, usize>::new();
    for candidate in &candidates {
        *certainty_counts
            .entry(candidate.certainty.to_string())
            .or_default() += 1;
    }
    let total_blocker_count = candidates
        .iter()
        .map(|candidate| candidate.blockers.len())
        .sum::<usize>();
    if candidates.len() > limit {
        candidates = std::mem::take(&mut candidates)
            .into_iter()
            .take(limit)
            .collect();
    }

    if summary {
        let payload = serde_json::json!({
            "tool": "analyze_dead_code",
            "scope": {
                "code_only": true,
                "summary_only": true,
                "subpath": subpath,
                "excluded_kinds": exclude_kind_strs,
                "limit": limit,
            },
            "candidates": [],
            "blockers": [],
            "summary": {
                "status": "ok",
                "candidate_count": total_available,
                "blocker_count": total_blocker_count,
                "certainty_counts": certainty_counts,
            },
            "truncated": total_available > limit,
            "warnings": [],
        });
        let mut response = tool_result_value(&payload, output_format)?;
        inject_budget_metadata(
            &mut response,
            &budgets.summary(
                "query_candidates_and_seeds.max_candidates",
                limit,
                total_available,
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
                "symbol": {
                    "qname": c.node.qualified_name,
                    "kind": c.node.kind.as_str(),
                    "file": c.node.file_path,
                    "line": c.node.line_start,
                },
                "certainty": c.certainty.to_string(),
                "certainty_class": dead_code_certainty_class(c.certainty),
                "reasons": c.reasons,
                "blockers": c.blockers,
            })
        })
        .collect();
    let blockers: Vec<_> = candidates
        .iter()
        .take(trimming.dead_code_candidate_preview_limit)
        .flat_map(|c| {
            c.blockers.iter().map(|blocker| {
                json!({
                    "symbol": c.node.qualified_name,
                    "message": blocker,
                })
            })
        })
        .collect();

    let payload = serde_json::json!({
        "tool": "analyze_dead_code",
        "scope": {
            "code_only": true,
            "summary_only": false,
            "subpath": subpath,
            "excluded_kinds": exclude_kind_strs,
            "limit": limit,
        },
        "candidates": preview,
        "blockers": blockers,
        "summary": {
            "status": "ok",
            "candidate_count": total_available,
            "blocker_count": total_blocker_count,
            "omitted_count": omitted,
            "certainty_counts": certainty_counts,
        },
        "truncated": omitted > 0 || total_available > limit,
        "warnings": [],
    });
    let mut response = tool_result_value(&payload, output_format)?;
    inject_budget_metadata(
        &mut response,
        &budgets.summary(
            "query_candidates_and_seeds.max_candidates",
            limit,
            total_available,
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
        "tool": "analyze_dependency",
        "symbol": result.target_qname,
        "removable": result.removable,
        "blocking_references": blocking_preview.clone(),
        "blocking_callers": blocking_preview,
        "confidence_tier": result.confidence.to_string(),
        "suggested_cleanups": result.suggested_cleanups,
        "summary": {
            "status": if result.removable { "removable" } else { "blocked" },
            "blocking_reference_count": result.blocking_references.len(),
            "omitted_blocking_count": omitted,
        },
        "warnings": result.uncertainty_flags,
    });
    let mut response = tool_result_value(&payload, output_format)?;
    inject_budget_metadata(&mut response, &result.budget);
    Ok(response)
}

/// Canonical certainty vocabulary used by the test guide; the raw
/// `certainty` field keeps the `ConfidenceTier` spelling for compatibility.
fn dead_code_certainty_class(tier: atlas_core::ConfidenceTier) -> &'static str {
    match tier {
        atlas_core::ConfidenceTier::High => "definite",
        atlas_core::ConfidenceTier::Medium => "probable",
        atlas_core::ConfidenceTier::Low => "speculative",
    }
}
