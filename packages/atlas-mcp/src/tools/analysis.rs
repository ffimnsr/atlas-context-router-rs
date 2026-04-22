use anyhow::{Context, Result};
use atlas_core::NodeKind;
use atlas_reasoning::ReasoningEngine;

use super::shared::{bool_arg, open_store, str_arg, string_array_arg, tool_result_value, u64_arg};

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
    let result = engine
        .score_refactor_safety(&symbol)
        .with_context(|| format!("safety scoring for `{symbol}` failed"))?;

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
    tool_result_value(&payload, output_format)
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
    let max_files = u64_arg(args, "max_files").unwrap_or(20) as usize;
    let max_edges = u64_arg(args, "max_edges").unwrap_or(50) as usize;

    let store = open_store(db_path)?;
    let engine = ReasoningEngine::new(&store);
    let symbol_refs: Vec<&str> = symbols.iter().map(String::as_str).collect();
    let result = engine
        .analyze_removal(&symbol_refs, Some(max_depth), Some(max_nodes))
        .context("analyze_removal failed")?;

    const SYMBOL_CAP: usize = 50;
    let omitted = result.impacted_symbols.len().saturating_sub(SYMBOL_CAP);

    let impacted_preview: Vec<_> = result
        .impacted_symbols
        .iter()
        .take(SYMBOL_CAP)
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
    tool_result_value(&payload, output_format)
}

pub(super) fn tool_analyze_dead_code(
    args: Option<&serde_json::Value>,
    db_path: &str,
    output_format: crate::output::OutputFormat,
) -> Result<serde_json::Value> {
    let allowlist = string_array_arg(args, "allowlist").unwrap_or_default();
    let subpath = str_arg(args, "subpath")?.map(str::to_owned);
    // Compact default: 50 candidates. Caller may raise with `limit`.
    let limit = u64_arg(args, "limit").unwrap_or(50) as usize;
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
    let allowlist_refs: Vec<&str> = allowlist.iter().map(String::as_str).collect();
    let candidates = engine
        .detect_dead_code(
            &allowlist_refs,
            subpath.as_deref(),
            Some(limit),
            &exclude_kinds,
        )
        .context("detect_dead_code failed")?;

    if summary {
        let payload = serde_json::json!({
            "candidate_count": candidates.len(),
            "applied_limit": limit,
            "applied_subpath": subpath,
            "excluded_kinds": exclude_kind_strs,
        });
        return tool_result_value(&payload, output_format);
    }

    const CANDIDATE_CAP: usize = 50;
    let omitted = candidates.len().saturating_sub(CANDIDATE_CAP);

    let preview: Vec<_> = candidates
        .iter()
        .take(CANDIDATE_CAP)
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
    tool_result_value(&payload, output_format)
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
    let result = engine
        .check_dependency_removal(&symbol)
        .with_context(|| format!("dependency check for `{symbol}` failed"))?;

    const BLOCKER_CAP: usize = 20;
    let omitted = result.blocking_references.len().saturating_sub(BLOCKER_CAP);

    let blocking_preview: Vec<_> = result
        .blocking_references
        .iter()
        .take(BLOCKER_CAP)
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
    tool_result_value(&payload, output_format)
}
