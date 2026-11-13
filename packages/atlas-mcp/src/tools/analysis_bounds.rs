//! Bounded serialization helpers for insight reports.
//!
//! Insight reports can carry thousands of findings, each with nested
//! `evidence` and `details` payloads. A single unbounded `analyze_patterns`
//! response is enough to overflow an agent context window, so this module
//! owns three concerns:
//!
//! * compact default rendering (`verbose: false`) that drops evidence/detail
//!   bodies while keeping identity, severity, score, and counts;
//! * deterministic paging over findings (`offset` / `limit` / `max_findings`);
//! * a hard serialized byte ceiling with `truncated` / `budget_hit` flags.
use anyhow::Result;
use atlas_core::{FreshnessWarning, InsightFinding, InsightSummary, ProvenanceMeta};
use atlas_reasoning::InferredModule;
use serde::Serialize;
use serde_json::{Map, Value, json};

use super::shared::{tool_result_value, u64_arg};
use crate::output::OutputFormat;

/// Default serialized ceiling for one insight payload.
///
/// Mirrors `mcp_cli_payload_serialization.mcp_response_bytes.default_limit`
/// so MCP callers get the same bound without threading policy through every
/// analysis entry point.
pub(super) const DEFAULT_INSIGHT_PAYLOAD_BYTE_LIMIT: usize = 32 * 1024;

/// Maximum occurrences/evidence entries kept per repeated-chain finding.
pub(super) const MAX_PATTERN_OCCURRENCES: usize = 5;

/// Maximum owned symbols/root paths kept per module in compact mode.
const MAX_COMPACT_MODULE_PATHS: usize = 5;

/// Resolve the caller-provided `max_bytes` override, if any.
pub(super) fn payload_byte_limit(args: Option<&Value>) -> usize {
    u64_arg(args, "max_bytes")
        .map(|value| value as usize)
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_INSIGHT_PAYLOAD_BYTE_LIMIT)
}

/// Render an insight report, choosing compact or full output by verbosity and
/// applying the serialized byte ceiling.
pub(super) fn insight_report_response<T: Serialize>(
    full_payload: &T,
    compact_payload: Value,
    output_format: OutputFormat,
    verbose: bool,
    max_bytes: usize,
) -> Result<Value> {
    let payload = if verbose {
        serde_json::to_value(full_payload)?
    } else {
        compact_payload
    };
    let payload = enforce_payload_ceiling(payload, max_bytes)?;
    tool_result_value(&payload, output_format)
}

/// Build the compact finding representation. Evidence and `details` bodies are
/// replaced by counts unless `verbose: true`.
pub(super) fn compact_finding(finding: &InsightFinding) -> Value {
    let files = finding
        .evidence
        .iter()
        .filter_map(|evidence| evidence.file_path.as_deref())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(MAX_COMPACT_MODULE_PATHS)
        .collect::<Vec<_>>();
    json!({
        "id": finding.id,
        "title": finding.title,
        "severity": finding.severity,
        "category": finding.category,
        "message": finding.message,
        "ranking_reason": finding.ranking_reason,
        "score": finding.score,
        "evidence_count": finding.evidence.len(),
        "details_present": finding.details.is_some(),
        "files": files,
    })
}

pub(super) fn compact_findings(findings: &[InsightFinding]) -> Vec<Value> {
    findings.iter().map(compact_finding).collect()
}

/// Bound the module list for `infer_modules`; `limit` applies to modules as
/// well as findings.
pub(super) fn limit_modules(modules: &mut Vec<InferredModule>, limit: Option<usize>) {
    if let Some(limit) = limit {
        *modules = std::mem::take(modules).into_iter().take(limit).collect();
    }
}

/// Compact module shape for `infer_modules`: drops the owned-symbol and
/// root-path lists while keeping counts, dependencies, confidence, and evidence.
pub(super) fn compact_modules(modules: &[InferredModule]) -> Vec<Value> {
    modules
        .iter()
        .map(|module| {
            json!({
                "module_id": module.module_id,
                "display_name": module.display_name,
                "node_count": module.node_count,
                "public_symbol_count": module.public_symbol_count,
                "internal_symbol_count": module.internal_symbol_count,
                "export_ratio": module.export_ratio,
                "file_count": module.file_count,
                "root_paths_preview": module
                    .root_paths
                    .iter()
                    .take(MAX_COMPACT_MODULE_PATHS)
                    .cloned()
                    .collect::<Vec<_>>(),
                "root_paths_omitted": module.root_paths.len().saturating_sub(MAX_COMPACT_MODULE_PATHS),
                "outbound_dependencies": module.outbound_dependencies,
                "inbound_dependencies": module.inbound_dependencies,
                "confidence": module.confidence,
                "evidence": module.evidence,
                "explicit": module.explicit,
            })
        })
        .collect()
}

/// Assemble a compact insight report envelope shared by the analysis tools.
pub(super) fn compact_insight_report(
    summary: &InsightSummary,
    findings: &[InsightFinding],
    provenance: &ProvenanceMeta,
    freshness: Option<&FreshnessWarning>,
    extra: Value,
) -> Value {
    let mut payload = Map::new();
    payload.insert("summary".to_owned(), json!(summary));
    payload.insert(
        "findings".to_owned(),
        Value::Array(compact_findings(findings)),
    );
    payload.insert("atlas_provenance".to_owned(), json!(provenance));
    if let Some(freshness) = freshness {
        payload.insert("atlas_freshness".to_owned(), json!(freshness));
    }
    payload.insert("evidence_omitted".to_owned(), json!(true));
    payload.insert("details_omitted".to_owned(), json!(true));
    if let Value::Object(extra) = extra {
        for (key, value) in extra {
            payload.insert(key, value);
        }
    }
    Value::Object(payload)
}

/// Page findings in place. `offset` skips leading findings; `limit` caps the
/// page. Returns `(total_available, offset)` so callers can report paging state.
pub(super) fn paginate_findings(
    findings: &mut Vec<InsightFinding>,
    summary: &mut InsightSummary,
    offset: usize,
    limit: Option<usize>,
) -> (usize, usize) {
    let total_available = findings.len();
    let offset = offset.min(total_available);
    if offset > 0 {
        *findings = std::mem::take(findings).into_iter().skip(offset).collect();
    }
    if let Some(limit) = limit {
        findings.truncate(limit);
    }
    summary.total_findings = findings.len();
    summary.highest_severity = findings.iter().map(|finding| finding.severity).max();
    (total_available, offset)
}

/// Collapse repeated-call-chain findings: keep one finding per chain but bound
/// the occurrence and evidence previews, recording the omitted counts.
pub(super) fn collapse_repeated_chain_details(findings: &mut [InsightFinding]) {
    for finding in findings.iter_mut() {
        if finding.category != "pattern_repeated_chain" {
            continue;
        }
        if let Some(details) = finding.details.as_mut().and_then(Value::as_object_mut) {
            let total = details
                .get("occurrences")
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            if total > MAX_PATTERN_OCCURRENCES {
                if let Some(occurrences) =
                    details.get_mut("occurrences").and_then(Value::as_array_mut)
                {
                    occurrences.truncate(MAX_PATTERN_OCCURRENCES);
                }
                details.insert("occurrence_total".to_owned(), json!(total));
                details.insert(
                    "occurrences_omitted".to_owned(),
                    json!(total - MAX_PATTERN_OCCURRENCES),
                );
            }
        }
        let evidence_total = finding.evidence.len();
        if evidence_total > MAX_PATTERN_OCCURRENCES {
            finding.evidence.truncate(MAX_PATTERN_OCCURRENCES);
            if let Some(details) = finding.details.as_mut().and_then(Value::as_object_mut) {
                details.insert("evidence_total".to_owned(), json!(evidence_total));
                details.insert(
                    "evidence_omitted".to_owned(),
                    json!(evidence_total - MAX_PATTERN_OCCURRENCES),
                );
            }
        }
    }
}

/// Enforce a serialized byte ceiling on a JSON payload.
///
/// Top-level arrays are halved repeatedly until the payload fits. If the
/// payload still exceeds the ceiling (for example a single gigantic scalar),
/// the response collapses to its summary plus truncation metadata. Truncation
/// is always reported through `truncated` / `budget_hit`.
pub(super) fn enforce_payload_ceiling(mut payload: Value, max_bytes: usize) -> Result<Value> {
    let mut serialized = serde_json::to_vec(&payload)?;
    if serialized.len() <= max_bytes {
        return Ok(payload);
    }

    let mut dropped = 0usize;
    loop {
        let mut changed = false;
        if let Value::Object(map) = &mut payload {
            for value in map.values_mut() {
                if let Value::Array(items) = value
                    && !items.is_empty()
                {
                    let keep = items.len() / 2;
                    dropped += items.len() - keep;
                    items.truncate(keep);
                    changed = true;
                }
            }
        }
        serialized = serde_json::to_vec(&payload)?;
        if serialized.len() <= max_bytes || !changed {
            break;
        }
    }

    if serialized.len() > max_bytes {
        let summary = payload.get("summary").cloned();
        let provenance = payload.get("atlas_provenance").cloned();
        let freshness = payload.get("atlas_freshness").cloned();
        let tool = payload.get("tool").cloned();
        payload = json!({
            "summary": summary,
            "tool": tool,
            "atlas_provenance": provenance,
            "atlas_freshness": freshness,
            "truncated": true,
            "budget_hit": true,
            "dropped_item_count": dropped,
            "payload_bytes": serialized.len(),
            "payload_byte_limit": max_bytes,
        });
    } else if let Some(map) = payload.as_object_mut() {
        map.insert("truncated".to_owned(), json!(true));
        map.insert("budget_hit".to_owned(), json!(true));
        map.insert("dropped_item_count".to_owned(), json!(dropped));
        map.insert("payload_bytes".to_owned(), json!(serialized.len()));
        map.insert("payload_byte_limit".to_owned(), json!(max_bytes));
    }

    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::{InsightEvidence, InsightSeverity};

    fn finding(index: usize, evidence: usize) -> InsightFinding {
        InsightFinding {
            id: format!("pattern:repeated_chain:chain-{index}"),
            title: format!("repeated call chain {index}"),
            severity: InsightSeverity::High,
            category: "pattern_repeated_chain".to_owned(),
            message: format!("sequence repeats {} times", index + 2),
            evidence: (0..evidence)
                .map(|offset| InsightEvidence {
                    file_path: Some(format!("src/file_{offset}.rs")),
                    qualified_name: Some(format!("src/file_{offset}.rs::fn::call_{offset}")),
                    node_kind: Some("function".to_owned()),
                    edge_kind: Some("calls".to_owned()),
                    line_range: None,
                    confidence_tier: None,
                })
                .collect(),
            ranking_reason: "sequence length 3; repeated 3 times".to_owned(),
            details: Some(json!({
                "sequence": ["a", "b", "c"],
                "occurrence_count": index + 2,
                "occurrences": (0..12).map(|slot| json!({ "slot": slot })).collect::<Vec<_>>(),
            })),
            score: index as f64,
        }
    }

    #[test]
    fn compact_finding_omits_evidence_and_details() {
        let value = compact_finding(&finding(1, 4));
        assert!(value.get("evidence").is_none());
        assert!(value.get("details").is_none());
        assert_eq!(value["evidence_count"], json!(4));
        assert_eq!(value["details_present"], json!(true));
    }

    #[test]
    fn collapse_repeated_chain_details_bounds_nested_payloads() {
        let mut findings = vec![finding(1, 20)];
        collapse_repeated_chain_details(&mut findings);
        let finding = &findings[0];
        assert_eq!(finding.evidence.len(), MAX_PATTERN_OCCURRENCES);
        let details = finding.details.as_ref().expect("details retained");
        assert_eq!(
            details["occurrences"].as_array().map(Vec::len),
            Some(MAX_PATTERN_OCCURRENCES)
        );
        assert_eq!(details["occurrence_total"], json!(12));
        assert_eq!(
            details["occurrences_omitted"],
            json!(12 - MAX_PATTERN_OCCURRENCES)
        );
        assert_eq!(
            details["evidence_omitted"],
            json!(20 - MAX_PATTERN_OCCURRENCES)
        );
    }

    #[test]
    fn paginate_findings_reports_total_and_applies_window() {
        let mut findings = (0..10).map(|index| finding(index, 1)).collect::<Vec<_>>();
        let mut summary = InsightSummary::from_findings(&findings, "2026-01-01T00:00:00Z");
        let (total, offset) = paginate_findings(&mut findings, &mut summary, 2, Some(3));
        assert_eq!(total, 10);
        assert_eq!(offset, 2);
        assert_eq!(findings.len(), 3);
        assert_eq!(findings[0].score, 2.0);
        assert_eq!(summary.total_findings, 3);
    }

    #[test]
    fn enforce_payload_ceiling_reports_truncation() {
        let payload = json!({
            "summary": { "total_findings": 64 },
            "findings": (0..64).map(|index| json!({ "id": index, "message": "x".repeat(256) })).collect::<Vec<_>>(),
        });
        let bounded = enforce_payload_ceiling(payload, 4 * 1024).expect("bounded payload");
        let bytes = serde_json::to_vec(&bounded).expect("serialize").len();
        assert!(bytes <= 4 * 1024, "payload {bytes} exceeded ceiling");
        assert_eq!(bounded["truncated"], json!(true));
        assert_eq!(bounded["budget_hit"], json!(true));
        assert!(bounded["dropped_item_count"].as_u64().unwrap_or(0) > 0);
    }
}
