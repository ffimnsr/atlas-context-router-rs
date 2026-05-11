use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use atlas_core::{
    BudgetReport, ChangeRiskResult, ConfidenceTier, CoverageStrength, DependencyRemovalResult,
    Edge, EdgeKind, InsightEvidence, InsightFinding, InsightLineRange, InsightSeverity, Node,
    NodeKind, ReasoningEvidence, RefactorSafetyResult, Result, RiskReport, SafetyScore,
    TestAdjacencyResult,
};
use atlas_store_sqlite::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    InsightsEngine, ReasoningEngine,
    helpers::{
        EDGE_QUERY_LIMIT, RiskInputs, build_review_focus, compute_risk_level, compute_safety_score,
        file_paths_cross_package, is_public_node, normalize_qn_kind_tokens,
    },
    metrics::{
        GraphSnapshot, NodeMetric, build_node_metrics, load_rust_complexity, module_id_for_file,
    },
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RiskAssessmentTarget {
    Symbol { symbol: String },
    ResolvedNode { node: Box<Node> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskClassification {
    Low,
    Medium,
    High,
}

impl std::fmt::Display for RiskClassification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RiskFactorContribution {
    pub factor: String,
    pub raw_value: Value,
    pub normalized: f64,
    pub weight: f64,
    pub contribution: f64,
    pub mitigates_risk: bool,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<InsightEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessmentAnalysis {
    pub target: Node,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_symbol: Option<String>,
    pub score: f64,
    pub classification: RiskClassification,
    pub report: RiskReport,
    pub factor_contributions: Vec<RiskFactorContribution>,
}

#[derive(Debug, Clone)]
struct ResolvedRiskTarget {
    node: Node,
    requested_symbol: Option<String>,
}

#[derive(Debug, Clone)]
struct RiskRelationshipContext {
    inbound: Vec<(Node, Edge)>,
    outbound: Vec<(Node, Edge)>,
    direct_tests: Vec<(Node, Edge)>,
    file_tests: Vec<Node>,
    cross_module_neighbors: Vec<Node>,
    unresolved_edges: Vec<(Node, Edge)>,
    dependency_path: Vec<Node>,
    cycle_evidence: Vec<InsightEvidence>,
    cycle_member_count: usize,
}

struct RiskFactorInput {
    factor: &'static str,
    raw_value: Value,
    normalized: f64,
    weight: f64,
    mitigates_risk: bool,
    reason: String,
    evidence: Vec<InsightEvidence>,
}

impl<'s> InsightsEngine<'s> {
    pub fn assess_risk(
        &self,
        repo_root: impl AsRef<Path>,
        target: RiskAssessmentTarget,
    ) -> Result<RiskAssessmentAnalysis> {
        let store = self.store().ok_or_else(|| {
            atlas_core::AtlasError::Other(
                "risk assessment requires a store-backed insights engine".to_owned(),
            )
        })?;
        let resolved = resolve_risk_target(store, target)?;
        let snapshot = self.load_graph_snapshot(store)?;
        let rust_complexity = load_rust_complexity(repo_root.as_ref(), &snapshot.nodes)?;
        let node_metrics = build_node_metrics(self, &snapshot, &rust_complexity);
        let metric = node_metrics
            .iter()
            .find(|metric| metric.node.qualified_name == resolved.node.qualified_name)
            .ok_or_else(|| {
                atlas_core::AtlasError::Other(format!(
                    "risk target not present in active insights snapshot: {}",
                    resolved.node.qualified_name
                ))
            })?;
        let relationships =
            build_risk_relationship_context(store, &snapshot, &node_metrics, metric)?;
        let factor_contributions = finalize_factor_contributions(build_risk_factor_contributions(
            self,
            metric,
            &relationships,
        ));
        let score = score_risk(self, &factor_contributions);
        let classification = classify_risk(self, score);
        let finding = build_risk_finding(metric, score, classification, &factor_contributions);
        let report = self.risk_report(vec![finding]);

        Ok(RiskAssessmentAnalysis {
            target: resolved.node,
            requested_symbol: resolved.requested_symbol,
            score,
            classification,
            report,
            factor_contributions,
        })
    }
}

fn resolve_risk_target(store: &Store, target: RiskAssessmentTarget) -> Result<ResolvedRiskTarget> {
    match target {
        RiskAssessmentTarget::ResolvedNode { node } => {
            let qname = normalize_qn_kind_tokens(&node.qualified_name);
            let resolved = store.node_by_qname(&qname)?.ok_or_else(|| {
                atlas_core::AtlasError::Other(format!(
                    "risk target could not be resolved from node {}",
                    node.qualified_name
                ))
            })?;
            Ok(ResolvedRiskTarget {
                node: resolved,
                requested_symbol: Some(node.qualified_name),
            })
        }
        RiskAssessmentTarget::Symbol { symbol } => {
            let normalized = normalize_qn_kind_tokens(&symbol);
            if let Some(node) = store.node_by_qname(&normalized)? {
                return Ok(ResolvedRiskTarget {
                    node,
                    requested_symbol: Some(symbol),
                });
            }

            let candidates = store.nodes_by_name(symbol.trim(), EDGE_QUERY_LIMIT)?;
            if candidates.is_empty() {
                return Err(atlas_core::AtlasError::Other(format!(
                    "risk target could not be resolved: {symbol}"
                )));
            }
            if candidates.len() > 1 {
                let rendered = candidates
                    .iter()
                    .map(|candidate| candidate.qualified_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(atlas_core::AtlasError::Other(format!(
                    "ambiguous risk target `{symbol}`; candidates: {rendered}"
                )));
            }

            Ok(ResolvedRiskTarget {
                node: candidates.into_iter().next().expect("single candidate"),
                requested_symbol: Some(symbol),
            })
        }
    }
}

fn build_risk_relationship_context(
    store: &Store,
    snapshot: &GraphSnapshot,
    node_metrics: &[NodeMetric],
    metric: &NodeMetric,
) -> Result<RiskRelationshipContext> {
    let qname = &metric.node.qualified_name;
    let inbound = store.inbound_edges(qname, EDGE_QUERY_LIMIT)?;
    let outbound = store.outbound_edges(qname, EDGE_QUERY_LIMIT)?;
    let direct_tests = store.test_neighbors(qname, EDGE_QUERY_LIMIT)?;
    let file_tests = store
        .nodes_by_file(&metric.node.file_path)?
        .into_iter()
        .filter(|node| node.is_test || node.kind == NodeKind::Test)
        .collect::<Vec<_>>();

    let module_by_qname = node_metrics
        .iter()
        .map(|candidate| {
            (
                candidate.node.qualified_name.clone(),
                candidate.module_id.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    let target_module = metric.module_id.as_str();

    let mut seen_modules = HashSet::new();
    let mut cross_module_neighbors = inbound
        .iter()
        .chain(outbound.iter())
        .filter_map(|(neighbor, _)| {
            let neighbor_module = module_by_qname
                .get(&neighbor.qualified_name)
                .cloned()
                .unwrap_or_else(|| {
                    module_id_for_file(
                        &neighbor.file_path,
                        store
                            .file_owner_id(&neighbor.file_path)
                            .ok()
                            .flatten()
                            .as_deref(),
                    )
                });
            if neighbor_module == target_module || !seen_modules.insert(neighbor_module) {
                return None;
            }
            Some(neighbor.clone())
        })
        .collect::<Vec<_>>();
    cross_module_neighbors.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.line_start.cmp(&right.line_start))
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
    });

    let mut unresolved_edges = inbound
        .iter()
        .chain(outbound.iter())
        .filter(|(_, edge)| {
            edge.confidence_tier.as_deref().unwrap_or("") == "low" || edge.confidence < 0.5
        })
        .cloned()
        .collect::<Vec<_>>();
    unresolved_edges.sort_by(|left, right| {
        left.0
            .file_path
            .cmp(&right.0.file_path)
            .then_with(|| left.1.line.cmp(&right.1.line))
            .then_with(|| left.0.qualified_name.cmp(&right.0.qualified_name))
    });

    let node_by_qname = snapshot
        .nodes
        .iter()
        .map(|node| (node.qualified_name.clone(), node.clone()))
        .collect::<HashMap<_, _>>();
    let dependency_path_qnames = longest_dependency_path(snapshot, qname);
    let dependency_path = dependency_path_qnames
        .into_iter()
        .filter_map(|qname| node_by_qname.get(&qname).cloned())
        .collect::<Vec<_>>();

    let (cycle_evidence, cycle_member_count) = cycle_context(store, snapshot, node_metrics, metric);

    Ok(RiskRelationshipContext {
        inbound,
        outbound,
        direct_tests,
        file_tests,
        cross_module_neighbors,
        unresolved_edges,
        dependency_path,
        cycle_evidence,
        cycle_member_count,
    })
}

fn build_risk_factor_contributions(
    engine: &InsightsEngine<'_>,
    metric: &NodeMetric,
    relationships: &RiskRelationshipContext,
) -> Vec<RiskFactorContribution> {
    let config = engine.config();
    let target_evidence = vec![node_evidence(&metric.node)];
    let mut factors = Vec::new();

    push_risk_factor(
        &mut factors,
        RiskFactorInput {
            factor: "public_api_exposure",
            raw_value: json!(is_public_node(&metric.node)),
            normalized: if is_public_node(&metric.node) {
                1.0
            } else {
                0.0
            },
            weight: config.risk_public_api_weight,
            mitigates_risk: false,
            reason: format!("public api exposure = {}", is_public_node(&metric.node)),
            evidence: target_evidence.clone(),
        },
    );
    push_risk_factor(
        &mut factors,
        RiskFactorInput {
            factor: "fan_in",
            raw_value: json!(metric.fan_in),
            normalized: normalized_ratio(metric.fan_in, config.high_fan_in),
            weight: config.risk_fan_in_weight,
            mitigates_risk: false,
            reason: format!(
                "fan-in {} against threshold {}",
                metric.fan_in, config.high_fan_in
            ),
            evidence: related_node_evidence(&relationships.inbound, 5),
        },
    );
    push_risk_factor(
        &mut factors,
        RiskFactorInput {
            factor: "fan_out",
            raw_value: json!(metric.fan_out),
            normalized: normalized_ratio(metric.fan_out, config.high_fan_out),
            weight: config.risk_fan_out_weight,
            mitigates_risk: false,
            reason: format!(
                "fan-out {} against threshold {}",
                metric.fan_out, config.high_fan_out
            ),
            evidence: related_node_evidence(&relationships.outbound, 5),
        },
    );
    push_risk_factor(
        &mut factors,
        RiskFactorInput {
            factor: "cross_module_dependency_count",
            raw_value: json!(relationships.cross_module_neighbors.len()),
            normalized: normalized_ratio(
                relationships.cross_module_neighbors.len(),
                config.high_coupling,
            ),
            weight: config.risk_cross_module_dependency_weight,
            mitigates_risk: false,
            reason: format!(
                "cross-module dependencies {} against threshold {}",
                relationships.cross_module_neighbors.len(),
                config.high_coupling
            ),
            evidence: relationships
                .cross_module_neighbors
                .iter()
                .take(5)
                .map(node_evidence)
                .collect(),
        },
    );

    let test_normalized = normalized_test_adjacency(metric, relationships);
    let test_evidence = if !relationships.direct_tests.is_empty() {
        related_node_evidence(&relationships.direct_tests, 5)
    } else {
        relationships
            .file_tests
            .iter()
            .take(5)
            .map(node_evidence)
            .collect()
    };
    push_risk_factor(
        &mut factors,
        RiskFactorInput {
            factor: "test_adjacency",
            raw_value: json!({
                "linked_test_count": metric.linked_test_count,
                "coverage_strength": metric.coverage_strength,
            }),
            normalized: test_normalized,
            weight: config.risk_test_adjacency_mitigation_weight,
            mitigates_risk: true,
            reason: format!(
                "test adjacency {:?} with {} linked tests",
                metric.coverage_strength, metric.linked_test_count
            ),
            evidence: test_evidence,
        },
    );

    push_risk_factor(
        &mut factors,
        RiskFactorInput {
            factor: "dependency_depth",
            raw_value: json!(metric.dependency_depth),
            normalized: normalized_ratio(metric.dependency_depth, config.deep_chain_length),
            weight: config.risk_dependency_depth_weight,
            mitigates_risk: false,
            reason: format!(
                "dependency depth {} against threshold {}",
                metric.dependency_depth, config.deep_chain_length
            ),
            evidence: relationships
                .dependency_path
                .iter()
                .take(5)
                .map(node_evidence)
                .collect(),
        },
    );
    push_risk_factor(
        &mut factors,
        RiskFactorInput {
            factor: "unresolved_edge_count",
            raw_value: json!(relationships.unresolved_edges.len()),
            normalized: unresolved_ratio(relationships.unresolved_edges.len()),
            weight: config.risk_unresolved_edge_weight,
            mitigates_risk: false,
            reason: format!(
                "{} low-confidence edges",
                relationships.unresolved_edges.len()
            ),
            evidence: relationships
                .unresolved_edges
                .iter()
                .take(5)
                .map(|(node, edge)| edge_evidence(node, edge))
                .collect(),
        },
    );

    if let Some(loc) = metric.loc {
        push_risk_factor(
            &mut factors,
            RiskFactorInput {
                factor: "large_function_flag",
                raw_value: json!(loc),
                normalized: normalized_ratio(loc, config.large_function_loc),
                weight: config.risk_large_function_weight,
                mitigates_risk: false,
                reason: format!(
                    "loc {} against threshold {}",
                    loc, config.large_function_loc
                ),
                evidence: target_evidence.clone(),
            },
        );
        push_risk_factor(
            &mut factors,
            RiskFactorInput {
                factor: "loc",
                raw_value: json!(loc),
                normalized: normalized_ratio(loc, config.large_function_loc),
                weight: config.risk_loc_weight,
                mitigates_risk: false,
                reason: format!(
                    "loc {} against threshold {}",
                    loc, config.large_function_loc
                ),
                evidence: target_evidence.clone(),
            },
        );
    }

    if let Some(cyclomatic) = metric.cyclomatic_complexity.copied() {
        push_risk_factor(
            &mut factors,
            RiskFactorInput {
                factor: "cyclomatic_complexity",
                raw_value: json!(cyclomatic),
                normalized: normalized_ratio(cyclomatic, config.high_cyclomatic_complexity),
                weight: config.risk_cyclomatic_complexity_weight,
                mitigates_risk: false,
                reason: format!(
                    "cyclomatic complexity {} against threshold {}",
                    cyclomatic, config.high_cyclomatic_complexity
                ),
                evidence: target_evidence.clone(),
            },
        );
    }
    if let Some(cognitive) = metric.cognitive_complexity.copied() {
        push_risk_factor(
            &mut factors,
            RiskFactorInput {
                factor: "cognitive_complexity",
                raw_value: json!(cognitive),
                normalized: normalized_ratio(cognitive, config.high_cognitive_complexity),
                weight: config.risk_cognitive_complexity_weight,
                mitigates_risk: false,
                reason: format!(
                    "cognitive complexity {} against threshold {}",
                    cognitive, config.high_cognitive_complexity
                ),
                evidence: target_evidence.clone(),
            },
        );
    }
    if let Some(nesting) = metric.max_nesting_depth.copied() {
        push_risk_factor(
            &mut factors,
            RiskFactorInput {
                factor: "max_nesting_depth",
                raw_value: json!(nesting),
                normalized: normalized_ratio(nesting, config.max_nesting_depth),
                weight: config.risk_nesting_depth_weight,
                mitigates_risk: false,
                reason: format!(
                    "nesting depth {} against threshold {}",
                    nesting, config.max_nesting_depth
                ),
                evidence: target_evidence.clone(),
            },
        );
    }

    push_risk_factor(
        &mut factors,
        RiskFactorInput {
            factor: "cycle_participation",
            raw_value: json!({
                "participates": relationships.cycle_member_count > 0,
                "member_count": relationships.cycle_member_count,
            }),
            normalized: if relationships.cycle_member_count > 0 {
                1.0
            } else {
                0.0
            },
            weight: config.risk_cycle_participation_weight,
            mitigates_risk: false,
            reason: format!(
                "cycle participation members {}",
                relationships.cycle_member_count
            ),
            evidence: relationships.cycle_evidence.clone(),
        },
    );

    factors
}

fn push_risk_factor(factors: &mut Vec<RiskFactorContribution>, input: RiskFactorInput) {
    if input.normalized <= 0.0 {
        return;
    }
    factors.push(RiskFactorContribution {
        factor: input.factor.to_owned(),
        raw_value: input.raw_value,
        normalized: input.normalized,
        weight: input.weight,
        contribution: 0.0,
        mitigates_risk: input.mitigates_risk,
        reason: input.reason,
        evidence: sort_and_dedup_evidence(input.evidence),
    });
}

fn score_risk(engine: &InsightsEngine<'_>, factors: &[RiskFactorContribution]) -> f64 {
    let positive_weight_total = positive_risk_weight_total(engine.config());
    if positive_weight_total == 0.0 {
        return 0.0;
    }

    let raw_score = factors.iter().fold(0.0, |total, factor| {
        let component = factor.normalized * factor.weight;
        if factor.mitigates_risk {
            total - component
        } else {
            total + component
        }
    });
    ((raw_score / positive_weight_total) * 100.0).clamp(0.0, 100.0)
}

fn classify_risk(engine: &InsightsEngine<'_>, score: f64) -> RiskClassification {
    if score >= engine.config().risk_high_threshold {
        RiskClassification::High
    } else if score >= engine.config().risk_medium_threshold {
        RiskClassification::Medium
    } else {
        RiskClassification::Low
    }
}

fn build_risk_finding(
    metric: &NodeMetric,
    score: f64,
    classification: RiskClassification,
    factor_contributions: &[RiskFactorContribution],
) -> InsightFinding {
    let finding_factors = factor_contributions.to_vec();
    let evidence = sort_and_dedup_evidence(
        finding_factors
            .iter()
            .flat_map(|factor| factor.evidence.clone())
            .collect(),
    );

    InsightFinding {
        id: format!("risk:{}", metric.node.qualified_name),
        title: format!("{} risk for {}", classification, metric.node.name),
        severity: match classification {
            RiskClassification::Low => InsightSeverity::Low,
            RiskClassification::Medium => InsightSeverity::Medium,
            RiskClassification::High => InsightSeverity::High,
        },
        category: "risk".to_owned(),
        message: format!(
            "{} scored {:.2}/100 risk for {}",
            metric.node.qualified_name, score, metric.node.qualified_name
        ),
        evidence,
        ranking_reason: format!(
            "risk score {:.2} classified as {} from {} contributing factors",
            score,
            classification,
            finding_factors.len()
        ),
        details: Some(json!({
            "qualified_name": metric.node.qualified_name,
            "classification": classification,
            "score": score,
            "factor_contributions": finding_factors,
        })),
        score,
    }
}

fn finalize_factor_contributions(
    mut factors: Vec<RiskFactorContribution>,
) -> Vec<RiskFactorContribution> {
    let positive_weight_total = factors
        .iter()
        .filter(|factor| !factor.mitigates_risk)
        .map(|factor| factor.weight)
        .sum::<f64>()
        .max(1.0);
    for factor in &mut factors {
        let component = factor.normalized * factor.weight;
        factor.contribution = if factor.mitigates_risk {
            -((component / positive_weight_total) * 100.0)
        } else {
            (component / positive_weight_total) * 100.0
        };
    }
    factors
}

fn positive_risk_weight_total(config: &atlas_engine::config::InsightsConfig) -> f64 {
    config.risk_public_api_weight
        + config.risk_fan_in_weight
        + config.risk_fan_out_weight
        + config.risk_cross_module_dependency_weight
        + config.risk_dependency_depth_weight
        + config.risk_unresolved_edge_weight
        + config.risk_large_function_weight
        + config.risk_loc_weight
        + config.risk_cyclomatic_complexity_weight
        + config.risk_cognitive_complexity_weight
        + config.risk_nesting_depth_weight
        + config.risk_cycle_participation_weight
}

fn normalized_ratio(value: usize, threshold: usize) -> f64 {
    if threshold == 0 {
        return 0.0;
    }
    (value as f64 / threshold as f64).clamp(0.0, 1.0)
}

fn unresolved_ratio(count: usize) -> f64 {
    (count as f64 / 3.0).clamp(0.0, 1.0)
}

fn normalized_test_adjacency(metric: &NodeMetric, relationships: &RiskRelationshipContext) -> f64 {
    let base = match metric.coverage_strength {
        CoverageStrength::Direct => 1.0,
        CoverageStrength::IndirectThroughCallers => 0.75,
        CoverageStrength::SameModule => 0.65,
        CoverageStrength::SameFile => 0.5,
        CoverageStrength::None => 0.0,
    };
    let count = if !relationships.direct_tests.is_empty() {
        relationships.direct_tests.len()
    } else {
        metric.linked_test_count.max(relationships.file_tests.len())
    };
    base * (count as f64 / 3.0).clamp(0.0, 1.0)
}

fn related_node_evidence(edges: &[(Node, Edge)], limit: usize) -> Vec<InsightEvidence> {
    edges
        .iter()
        .take(limit)
        .map(|(neighbor, edge)| edge_evidence(neighbor, edge))
        .collect()
}

fn node_evidence(node: &Node) -> InsightEvidence {
    InsightEvidence {
        file_path: Some(node.file_path.clone()),
        qualified_name: Some(node.qualified_name.clone()),
        node_kind: Some(node.kind.as_str().to_owned()),
        edge_kind: None,
        line_range: Some(InsightLineRange {
            start_line: node.line_start,
            end_line: node.line_end,
        }),
        confidence_tier: None,
    }
}

fn edge_evidence(neighbor: &Node, edge: &Edge) -> InsightEvidence {
    InsightEvidence {
        file_path: Some(edge.file_path.clone()),
        qualified_name: Some(neighbor.qualified_name.clone()),
        node_kind: Some(neighbor.kind.as_str().to_owned()),
        edge_kind: Some(edge.kind.as_str().to_owned()),
        line_range: edge.line.map(|line| InsightLineRange {
            start_line: line,
            end_line: line,
        }),
        confidence_tier: parse_confidence_tier(edge.confidence_tier.as_deref()),
    }
}

fn parse_confidence_tier(value: Option<&str>) -> Option<ConfidenceTier> {
    match value {
        Some("high") => Some(ConfidenceTier::High),
        Some("medium") => Some(ConfidenceTier::Medium),
        Some("low") => Some(ConfidenceTier::Low),
        _ => None,
    }
}

fn sort_and_dedup_evidence(mut evidence: Vec<InsightEvidence>) -> Vec<InsightEvidence> {
    evidence.sort_by(|left, right| {
        left.file_path
            .cmp(&right.file_path)
            .then_with(|| left.qualified_name.cmp(&right.qualified_name))
            .then_with(|| left.node_kind.cmp(&right.node_kind))
            .then_with(|| left.edge_kind.cmp(&right.edge_kind))
            .then_with(|| {
                left.line_range
                    .as_ref()
                    .map(|range| (range.start_line, range.end_line))
                    .cmp(
                        &right
                            .line_range
                            .as_ref()
                            .map(|range| (range.start_line, range.end_line)),
                    )
            })
            .then_with(|| left.confidence_tier.cmp(&right.confidence_tier))
    });
    evidence.dedup();
    evidence
}

fn longest_dependency_path(snapshot: &GraphSnapshot, start_qname: &str) -> Vec<String> {
    let adjacency =
        snapshot
            .edges
            .iter()
            .fold(HashMap::<String, Vec<String>>::new(), |mut acc, edge| {
                acc.entry(edge.source_qn.clone())
                    .or_default()
                    .push(edge.target_qn.clone());
                acc
            });
    let mut memo = HashMap::<String, Vec<String>>::new();
    longest_dependency_path_inner(
        start_qname,
        &adjacency,
        &mut HashSet::new(),
        &mut memo,
        snapshot.nodes.len(),
    )
}

fn longest_dependency_path_inner(
    qname: &str,
    adjacency: &HashMap<String, Vec<String>>,
    visiting: &mut HashSet<String>,
    memo: &mut HashMap<String, Vec<String>>,
    remaining_budget: usize,
) -> Vec<String> {
    if remaining_budget == 0 {
        return vec![qname.to_owned()];
    }
    if let Some(path) = memo.get(qname) {
        return path.clone();
    }
    if !visiting.insert(qname.to_owned()) {
        return vec![qname.to_owned()];
    }

    let mut best = vec![qname.to_owned()];
    if let Some(targets) = adjacency.get(qname) {
        for target in targets {
            if target == qname || visiting.contains(target) {
                continue;
            }
            let candidate = longest_dependency_path_inner(
                target,
                adjacency,
                visiting,
                memo,
                remaining_budget - 1,
            );
            if candidate.len() + 1 > best.len() {
                let mut path = vec![qname.to_owned()];
                path.extend(candidate);
                best = path;
            }
        }
    }

    visiting.remove(qname);
    memo.insert(qname.to_owned(), best.clone());
    best
}

fn cycle_context(
    store: &Store,
    snapshot: &GraphSnapshot,
    node_metrics: &[NodeMetric],
    metric: &NodeMetric,
) -> (Vec<InsightEvidence>, usize) {
    let module_by_qname = node_metrics
        .iter()
        .map(|candidate| {
            (
                candidate.node.qualified_name.clone(),
                candidate.module_id.clone(),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut adjacency = BTreeMap::<String, Vec<String>>::new();
    let mut evidence_by_edge = BTreeMap::<(String, String), Vec<InsightEvidence>>::new();
    for edge in &snapshot.edges {
        let Some(source_module) = module_by_qname.get(&edge.source_qn) else {
            continue;
        };
        let Some(target_module) = module_by_qname.get(&edge.target_qn) else {
            continue;
        };
        if source_module == target_module && edge.source_qn != edge.target_qn {
            continue;
        }
        adjacency
            .entry(source_module.clone())
            .or_default()
            .push(target_module.clone());

        let neighbor_qname = if source_module == &metric.module_id {
            edge.target_qn.as_str()
        } else {
            edge.source_qn.as_str()
        };
        let neighbor = snapshot
            .nodes
            .iter()
            .find(|node| node.qualified_name == neighbor_qname)
            .cloned()
            .unwrap_or_else(|| Node {
                id: atlas_core::NodeId(0),
                kind: NodeKind::Function,
                name: neighbor_qname
                    .rsplit("::")
                    .next()
                    .unwrap_or(neighbor_qname)
                    .to_owned(),
                qualified_name: neighbor_qname.to_owned(),
                file_path: edge.file_path.clone(),
                line_start: edge.line.unwrap_or(1),
                line_end: edge.line.unwrap_or(1),
                language: String::new(),
                parent_name: None,
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: String::new(),
                extra_json: Value::Null,
            });
        evidence_by_edge
            .entry((source_module.clone(), target_module.clone()))
            .or_default()
            .push(edge_evidence(&neighbor, edge));
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort();
        neighbors.dedup();
    }

    let components = strongly_connected_components(&adjacency);
    let target_component = components.into_iter().find(|component| {
        component
            .iter()
            .any(|module_id| module_id == &metric.module_id)
            && (component.len() > 1
                || adjacency.get(&metric.module_id).is_some_and(|neighbors| {
                    neighbors
                        .iter()
                        .any(|neighbor| neighbor == &metric.module_id)
                }))
    });
    let Some(component) = target_component else {
        return (Vec::new(), 0);
    };

    let component_set = component.iter().cloned().collect::<HashSet<_>>();
    let mut evidence = evidence_by_edge
        .into_iter()
        .filter(|((source, target), _)| {
            component_set.contains(source)
                && component_set.contains(target)
                && (source != target || source == &metric.module_id)
        })
        .flat_map(|(_, evidence)| evidence)
        .collect::<Vec<_>>();
    if evidence.is_empty() {
        let owner_id = store
            .file_owner_id(&metric.node.file_path)
            .ok()
            .and_then(|owner| owner);
        evidence.push(InsightEvidence {
            file_path: Some(metric.node.file_path.clone()),
            qualified_name: Some(metric.node.qualified_name.clone()),
            node_kind: Some(metric.node.kind.as_str().to_owned()),
            edge_kind: owner_id.map(|_| "module_cycle".to_owned()),
            line_range: Some(InsightLineRange {
                start_line: metric.node.line_start,
                end_line: metric.node.line_end,
            }),
            confidence_tier: None,
        });
    }

    (sort_and_dedup_evidence(evidence), component.len())
}

fn strongly_connected_components(adjacency: &BTreeMap<String, Vec<String>>) -> Vec<Vec<String>> {
    struct TarjanState {
        index: usize,
        stack: Vec<String>,
        on_stack: HashSet<String>,
        indices: HashMap<String, usize>,
        lowlink: HashMap<String, usize>,
        components: Vec<Vec<String>>,
    }

    fn strong_connect(
        node: &str,
        adjacency: &BTreeMap<String, Vec<String>>,
        state: &mut TarjanState,
    ) {
        let current_index = state.index;
        state.indices.insert(node.to_owned(), current_index);
        state.lowlink.insert(node.to_owned(), current_index);
        state.index += 1;
        state.stack.push(node.to_owned());
        state.on_stack.insert(node.to_owned());

        let neighbors = adjacency.get(node).cloned().unwrap_or_default();
        for neighbor in neighbors {
            if !state.indices.contains_key(&neighbor) {
                strong_connect(&neighbor, adjacency, state);
                let low_neighbor = state.lowlink[&neighbor];
                let low_node = state.lowlink[node];
                state
                    .lowlink
                    .insert(node.to_owned(), low_node.min(low_neighbor));
            } else if state.on_stack.contains(&neighbor) {
                let neighbor_index = state.indices[&neighbor];
                let low_node = state.lowlink[node];
                state
                    .lowlink
                    .insert(node.to_owned(), low_node.min(neighbor_index));
            }
        }

        if state.lowlink[node] == state.indices[node] {
            let mut component = Vec::new();
            while let Some(item) = state.stack.pop() {
                state.on_stack.remove(&item);
                component.push(item.clone());
                if item == node {
                    break;
                }
            }
            component.sort();
            state.components.push(component);
        }
    }

    let mut state = TarjanState {
        index: 0,
        stack: Vec::new(),
        on_stack: HashSet::new(),
        indices: HashMap::new(),
        lowlink: HashMap::new(),
        components: Vec::new(),
    };

    for node in adjacency.keys() {
        if !state.indices.contains_key(node) {
            strong_connect(node, adjacency, &mut state);
        }
    }
    state
        .components
        .sort_by(|left, right| left.first().cmp(&right.first()));
    state.components
}

impl<'s> ReasoningEngine<'s> {
    /// Score how safe it is to refactor `qname`.
    ///
    /// Factors: fan-in, fan-out, visibility (public API), test adjacency,
    /// self-containment, unresolved edges.
    pub fn score_refactor_safety(&self, qname: &str) -> Result<RefactorSafetyResult> {
        let qname = normalize_qn_kind_tokens(qname);
        let node = match self.store.node_by_qname(&qname)? {
            Some(node) => node,
            None => {
                return Err(atlas_core::AtlasError::Other(format!(
                    "node not found: {qname}"
                )));
            }
        };

        let inbound = self.store.inbound_edges(&qname, EDGE_QUERY_LIMIT)?;
        let outbound = self.store.outbound_edges(&qname, EDGE_QUERY_LIMIT)?;
        let tests = self.store.test_neighbors(&qname, EDGE_QUERY_LIMIT)?;

        let fan_in = inbound.len();
        let fan_out = outbound.len();
        let linked_test_count = tests.len();

        let coverage_strength = if linked_test_count > 0 {
            CoverageStrength::Direct
        } else {
            let caller_has_tests = inbound.iter().any(|(caller, _)| {
                self.store
                    .test_neighbors(&caller.qualified_name, 1)
                    .ok()
                    .map(|tests| !tests.is_empty())
                    .unwrap_or(false)
            });
            if caller_has_tests {
                CoverageStrength::IndirectThroughCallers
            } else {
                CoverageStrength::None
            }
        };

        let is_public = is_public_node(&node);
        let cross_module_callers = inbound
            .iter()
            .filter(|(caller, _)| caller.file_path != node.file_path)
            .count();

        let unresolved_edge_count = inbound
            .iter()
            .chain(outbound.iter())
            .filter(|(_, edge)| {
                edge.confidence_tier.as_deref().unwrap_or("") == "low" || edge.confidence < 0.5
            })
            .count();

        let (score, band, reasons, suggested_validations) = compute_safety_score(
            &node,
            fan_in,
            fan_out,
            linked_test_count,
            is_public,
            cross_module_callers,
            unresolved_edge_count,
        );

        let evidence = vec![
            ReasoningEvidence {
                key: "fan_in".to_owned(),
                value: fan_in.to_string(),
            },
            ReasoningEvidence {
                key: "fan_out".to_owned(),
                value: fan_out.to_string(),
            },
            ReasoningEvidence {
                key: "linked_tests".to_owned(),
                value: linked_test_count.to_string(),
            },
            ReasoningEvidence {
                key: "cross_module_callers".to_owned(),
                value: cross_module_callers.to_string(),
            },
            ReasoningEvidence {
                key: "unresolved_edges".to_owned(),
                value: unresolved_edge_count.to_string(),
            },
        ];

        Ok(RefactorSafetyResult {
            node,
            safety: SafetyScore {
                score,
                band,
                reasons,
                suggested_validations,
            },
            fan_in,
            fan_out,
            linked_test_count,
            unresolved_edge_count,
            coverage_strength,
            evidence,
            budget: BudgetReport::within_budget(
                "analysis.refactor_safety",
                EDGE_QUERY_LIMIT,
                fan_in + fan_out,
            ),
        })
    }

    /// Check whether removing `qname` is safe (no remaining references).
    ///
    /// Verifies zero references in graph. Flags dynamic/reflective uncertainty
    /// for low-confidence inbound edges.
    pub fn check_dependency_removal(&self, qname: &str) -> Result<DependencyRemovalResult> {
        let qname = normalize_qn_kind_tokens(qname);
        let inbound = self.store.inbound_edges(&qname, EDGE_QUERY_LIMIT)?;

        let blocking: Vec<Node> = inbound
            .iter()
            .filter(|(_, edge)| {
                matches!(
                    edge.kind,
                    EdgeKind::Calls
                        | EdgeKind::Imports
                        | EdgeKind::References
                        | EdgeKind::Extends
                        | EdgeKind::Implements
                )
            })
            .map(|(node, _)| node.clone())
            .collect();

        let has_low_confidence = inbound.iter().any(|(_, edge)| edge.confidence < 0.5);

        let confidence = if blocking.is_empty() && !has_low_confidence {
            ConfidenceTier::High
        } else if has_low_confidence {
            ConfidenceTier::Medium
        } else {
            ConfidenceTier::Low
        };

        let blocking_count = blocking.len();
        let removable = blocking_count == 0;

        let mut suggested_cleanups: Vec<String> = blocking
            .iter()
            .take(5)
            .map(|node| format!("remove reference in `{}`", node.file_path))
            .collect();
        if has_low_confidence && removable {
            suggested_cleanups
                .push("verify no dynamic/reflective usage before removing".to_owned());
        }

        let evidence = vec![
            ReasoningEvidence {
                key: "inbound_semantic_references".to_owned(),
                value: inbound.len().to_string(),
            },
            ReasoningEvidence {
                key: "blocking_references".to_owned(),
                value: blocking.len().to_string(),
            },
        ];

        let mut uncertainty_flags: Vec<String> = Vec::new();
        if has_low_confidence {
            uncertainty_flags.push(
                "low-confidence edges present; dynamic or reflective usage cannot be excluded"
                    .to_owned(),
            );
        }

        Ok(DependencyRemovalResult {
            target_qname: qname,
            removable,
            blocking_references: blocking,
            evidence_edges: inbound.into_iter().map(|(_, edge)| edge).collect(),
            confidence,
            suggested_cleanups,
            evidence,
            uncertainty_flags,
            budget: BudgetReport::within_budget(
                "analysis.dependency_removal",
                EDGE_QUERY_LIMIT,
                blocking_count,
            ),
        })
    }

    /// Estimate test coverage adjacency for `qname`.
    pub fn find_test_adjacency(&self, qname: &str) -> Result<TestAdjacencyResult> {
        let qname = normalize_qn_kind_tokens(qname);
        let symbol = match self.store.node_by_qname(&qname)? {
            Some(node) => node,
            None => {
                return Err(atlas_core::AtlasError::Other(format!(
                    "node not found: {qname}"
                )));
            }
        };

        let test_pairs = self.store.test_neighbors(&qname, EDGE_QUERY_LIMIT)?;
        let mut linked_tests: Vec<Node> = test_pairs.into_iter().map(|(node, _)| node).collect();

        let coverage_strength = if !linked_tests.is_empty() {
            CoverageStrength::Direct
        } else {
            let callers = self.store.inbound_edges(&qname, EDGE_QUERY_LIMIT)?;
            let caller_has_tests = callers.iter().any(|(caller, _)| {
                self.store
                    .test_neighbors(&caller.qualified_name, 1)
                    .ok()
                    .map(|tests| !tests.is_empty())
                    .unwrap_or(false)
            });

            if caller_has_tests {
                for (caller, _) in &callers {
                    if let Ok(tests) = self.store.test_neighbors(&caller.qualified_name, 4) {
                        linked_tests.extend(tests.into_iter().map(|(node, _)| node));
                    }
                }
                linked_tests.dedup_by_key(|node| node.qualified_name.clone());
                CoverageStrength::IndirectThroughCallers
            } else {
                let file_nodes = self.store.nodes_by_file(&symbol.file_path)?;
                let file_tests: Vec<Node> = file_nodes
                    .into_iter()
                    .filter(|node| node.is_test || node.kind == NodeKind::Test)
                    .collect();

                if !file_tests.is_empty() {
                    linked_tests = file_tests;
                    CoverageStrength::SameFile
                } else {
                    CoverageStrength::None
                }
            }
        };

        let recommendation = match coverage_strength {
            CoverageStrength::None => {
                Some("no tests found for this symbol — consider adding a dedicated test".to_owned())
            }
            CoverageStrength::IndirectThroughCallers => Some(
                "coverage is indirect through callers — consider adding a direct unit test"
                    .to_owned(),
            ),
            CoverageStrength::SameFile => Some(
                "tests are co-located in the same file but not directly linked via edge — \
                 verify coverage"
                    .to_owned(),
            ),
            _ => None,
        };

        Ok(TestAdjacencyResult {
            symbol,
            linked_tests,
            coverage_strength,
            recommendation,
        })
    }

    /// Classify the risk of changing `qname` by aggregating graph factors.
    pub fn classify_change_risk(&self, qname: &str) -> Result<ChangeRiskResult> {
        let qname = normalize_qn_kind_tokens(qname);
        let node = match self.store.node_by_qname(&qname)? {
            Some(node) => node,
            None => {
                return Err(atlas_core::AtlasError::Other(format!(
                    "node not found: {qname}"
                )));
            }
        };

        let inbound = self.store.inbound_edges(&qname, EDGE_QUERY_LIMIT)?;
        let outbound = self.store.outbound_edges(&qname, EDGE_QUERY_LIMIT)?;
        let tests = self.store.test_neighbors(&qname, EDGE_QUERY_LIMIT)?;

        let is_public = is_public_node(&node);
        let fan_in = inbound.len();
        let fan_out = outbound.len();
        let test_adj = !tests.is_empty();

        let cross_module = inbound
            .iter()
            .any(|(caller, _)| caller.file_path != node.file_path);
        let cross_package = inbound.iter().any(|(caller, _)| {
            file_paths_cross_package(self.store, &caller.file_path, &node.file_path)
                .unwrap_or(false)
        });

        let unresolved = inbound
            .iter()
            .chain(outbound.iter())
            .filter(|(_, edge)| edge.confidence < 0.5)
            .count();

        let impacted_files: HashSet<&str> = inbound
            .iter()
            .chain(outbound.iter())
            .map(|(node, _)| node.file_path.as_str())
            .collect();

        let (risk_level, factors) = compute_risk_level(
            &node,
            RiskInputs {
                fan_in,
                fan_out,
                is_public,
                test_adj,
                cross_module,
                cross_package,
                unresolved,
                impacted_file_count: impacted_files.len(),
            },
        );

        let suggested_review_focus =
            build_review_focus(is_public, cross_module, cross_package, fan_in, &tests);

        Ok(ChangeRiskResult {
            risk_level,
            contributing_factors: factors,
            suggested_review_focus,
        })
    }
}
