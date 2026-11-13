//! Bounded payload helpers shared by the insight analysis tools.
//!
//! These helpers keep architecture/metric payloads useful without turning the
//! response into an unbounded dump: module coupling and metric distributions are
//! listed explicitly (not only as counts) but capped by a deterministic limit.

use atlas_reasoning::{
    ArchitectureModuleEdge, ArchitectureModuleNode, MetricDistribution, MetricGraphSummary,
};
use serde_json::{Value, json};

/// Bounded module listing with coupling/cohesion detail.
pub(super) fn architecture_modules_json(
    modules: &[ArchitectureModuleNode],
    limit: usize,
) -> Vec<Value> {
    modules
        .iter()
        .take(limit)
        .map(|module| {
            json!({
                "module_id": module.module_id,
                "layer": module.layer,
                "node_count": module.node_count,
                "file_count": module.file_paths.len(),
                "coupling_score": round_metric(module.coupling_score),
                "cohesion": round_metric(module.cohesion),
                "internal_edge_count": module.internal_edge_count,
                "outbound_dependency_count": module.external_dependency_edge_count,
                "inbound_dependency_count": module.inbound_dependency_edge_count,
            })
        })
        .collect()
}

/// Bounded module-coupling listing. Evidence rows stay out of the payload; only
/// the aggregated edge count is reported.
pub(super) fn architecture_module_edges_json(
    edges: &[ArchitectureModuleEdge],
    limit: usize,
) -> Vec<Value> {
    edges
        .iter()
        .take(limit)
        .map(|edge| {
            json!({
                "source_module": edge.source_module,
                "target_module": edge.target_module,
                "source_layer": edge.source_layer,
                "target_layer": edge.target_layer,
                "evidence_count": edge.evidence.len(),
            })
        })
        .collect()
}

/// Bounded metric distribution summary. Cyclomatic/cognitive metrics surface
/// here even when no finding crossed a threshold.
pub(super) fn metric_distributions_json(
    distributions: &[MetricDistribution],
    limit: usize,
) -> Vec<Value> {
    distributions
        .iter()
        .take(limit)
        .map(|distribution| {
            json!({
                "metric_name": distribution.metric_name,
                "min": round_metric(distribution.min),
                "max": round_metric(distribution.max),
                "average": round_metric(distribution.average),
                "p50": round_metric(distribution.p50),
                "p90": round_metric(distribution.p90),
                "p95": round_metric(distribution.p95),
                "threshold_value": distribution.threshold_value.map(round_metric),
                "outlier_cutoff": round_metric(distribution.outlier_cutoff),
                "outlier_count": distribution.outliers.len(),
            })
        })
        .collect()
}

/// Document the filtered scope of the graph summary alongside raw counts.
pub(super) fn metric_graph_summary_json(summary: &MetricGraphSummary) -> Value {
    json!({
        "node_count": summary.node_count,
        "edge_count": summary.edge_count,
        "density": summary.density,
        "average_degree": summary.average_degree,
        "diameter": summary.diameter,
        "diameter_scope": summary.diameter_scope,
        "scope": summary.scope,
        "raw_node_count": summary.raw_node_count,
        "raw_edge_count": summary.raw_edge_count,
    })
}

/// Round to one decimal place and normalize negative zero.
pub(super) fn round_one_decimal(value: f64) -> f64 {
    let rounded = (value * 10.0).round() / 10.0;
    if rounded == 0.0 { 0.0 } else { rounded }
}

/// Normalize `-0.0` to `0.0` after rounding so JSON never reports a negative
/// zero metric.
pub(super) fn round_metric(value: f64) -> f64 {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded == 0.0 { 0.0 } else { rounded }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_metric_normalizes_negative_zero() {
        assert_eq!(round_metric(-0.0).to_string(), "0");
        assert_eq!(round_metric(0.0001), 0.0);
        assert_eq!(round_metric(1.23456), 1.235);
    }
}
