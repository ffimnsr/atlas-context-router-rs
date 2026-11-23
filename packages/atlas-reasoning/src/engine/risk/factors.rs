use super::dependency::*;
use super::model::*;
use super::*;

pub(super) fn build_risk_relationship_context(
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

pub(super) fn build_risk_factor_contributions(
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

pub(super) fn score_risk(engine: &InsightsEngine<'_>, factors: &[RiskFactorContribution]) -> f64 {
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

pub(super) fn classify_risk(engine: &InsightsEngine<'_>, score: f64) -> RiskClassification {
    if score >= engine.config().risk_high_threshold {
        RiskClassification::High
    } else if score >= engine.config().risk_medium_threshold {
        RiskClassification::Medium
    } else {
        RiskClassification::Low
    }
}

pub(super) fn build_risk_finding(
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

pub(super) fn finalize_factor_contributions(
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
