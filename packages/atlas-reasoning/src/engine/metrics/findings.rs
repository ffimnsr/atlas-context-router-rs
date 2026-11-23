use super::types::*;
use super::*;

pub(super) fn build_metric_findings(
    engine: &InsightsEngine<'_>,
    node_metrics: &[NodeMetric],
    file_metrics: &[FileMetric],
    module_metrics: &[ModuleMetric],
    distributions: &[MetricDistribution],
) -> Vec<InsightFinding> {
    let mut findings = Vec::new();

    for metric in node_metrics {
        if metric.fan_in >= engine.config().high_fan_in {
            findings.push(node_metric_finding(
                metric,
                "fan_in",
                metric.fan_in as f64,
                engine.config().high_fan_in as f64,
                "high fan-in",
            ));
        }
        if metric.fan_out >= engine.config().high_fan_out {
            findings.push(node_metric_finding(
                metric,
                "fan_out",
                metric.fan_out as f64,
                engine.config().high_fan_out as f64,
                "high fan-out",
            ));
        }
        if metric.large_function_candidate {
            findings.push(node_metric_finding(
                metric,
                "loc",
                metric.loc.unwrap_or_default() as f64,
                engine.config().large_function_loc as f64,
                "large function",
            ));
        }
        for (metric_name, raw_value, threshold, title) in [
            (
                "cyclomatic_complexity",
                metric.cyclomatic_complexity.copied(),
                engine.config().high_cyclomatic_complexity as f64,
                "high cyclomatic complexity",
            ),
            (
                "cognitive_complexity",
                metric.cognitive_complexity.copied(),
                engine.config().high_cognitive_complexity as f64,
                "high cognitive complexity",
            ),
            (
                "branch_count",
                metric.branch_count.copied(),
                engine.config().branch_count as f64,
                "high branch count",
            ),
            (
                "max_nesting_depth",
                metric.max_nesting_depth.copied(),
                engine.config().max_nesting_depth as f64,
                "deep nesting",
            ),
        ] {
            if let Some(raw_value) = raw_value
                && raw_value as f64 >= threshold
            {
                findings.push(node_metric_finding(
                    metric,
                    metric_name,
                    raw_value as f64,
                    threshold,
                    title,
                ));
            }
        }
    }

    for metric in file_metrics.iter().filter(|metric| metric.highly_connected) {
        let raw_value = metric.average_fan_in + metric.average_fan_out;
        let threshold = engine.config().high_coupling as f64;
        findings.push(InsightFinding {
            id: format!("metrics:file:{}", metric.file_path),
            title: format!("highly connected file {}", metric.file_path),
            severity: severity_for_ratio(raw_value, threshold.max(1.0)),
            category: "file_metrics".to_owned(),
            message: format!(
                "file {} has {} nodes, {} edges, connectivity {:.2}, threshold {:.2}",
                metric.file_path, metric.node_count, metric.edge_count, raw_value, threshold
            ),
            evidence: vec![InsightEvidence {
                file_path: Some(metric.file_path.clone()),
                qualified_name: None,
                node_kind: None,
                edge_kind: None,
                line_range: None,
                confidence_tier: None,
            }],
            ranking_reason: format!(
                "file metric connectivity {:.2} exceeded threshold {:.2}",
                raw_value, threshold
            ),
            details: None,
            score: raw_value,
        });
    }

    for metric in module_metrics
        .iter()
        .filter(|metric| metric.high_coupling_candidate)
    {
        findings.push(InsightFinding {
            id: format!("metrics:module:{}", metric.module_id),
            title: format!("high coupling module {}", metric.module_id),
            severity: severity_for_ratio(
                metric.coupling_score,
                engine.config().high_coupling.max(1) as f64,
            ),
            category: "module_metrics".to_owned(),
            message: format!(
                "module {} has coupling {:.2} (outbound {}, inbound {}), threshold {:.2}",
                metric.module_id,
                metric.coupling_score,
                metric.external_dependency_edge_count,
                metric.inbound_dependency_edge_count,
                engine.config().high_coupling as f64,
            ),
            evidence: metric
                .file_paths
                .first()
                .cloned()
                .into_iter()
                .map(|file_path| InsightEvidence {
                    file_path: Some(file_path),
                    qualified_name: None,
                    node_kind: None,
                    edge_kind: None,
                    line_range: None,
                    confidence_tier: None,
                })
                .collect(),
            ranking_reason: format!(
                "module coupling {:.2} exceeded threshold {:.2}",
                metric.coupling_score,
                engine.config().high_coupling as f64,
            ),
            details: None,
            score: metric.coupling_score,
        });
    }

    // Distribution outliers can duplicate a node-metric finding for the same
    // subject and metric (for example high fan-in). Emit the node-metric row
    // once and skip the duplicate so small pages are not wasted on repeats.
    let node_metric_keys: HashSet<(String, String)> = findings
        .iter()
        .filter(|finding| finding.category == "node_metrics")
        .filter_map(|finding| {
            let rest = finding.id.strip_prefix("metrics:")?;
            let (metric_name, qualified_name) = rest.split_once(':')?;
            Some((metric_name.to_owned(), qualified_name.to_owned()))
        })
        .collect();

    for distribution in distributions {
        for outlier in &distribution.outliers {
            if node_metric_keys
                .contains(&(distribution.metric_name.clone(), outlier.subject_id.clone()))
            {
                continue;
            }
            findings.push(InsightFinding {
                id: format!(
                    "metrics:distribution:{}:{}",
                    distribution.metric_name, outlier.subject_id
                ),
                title: format!("{} outlier", distribution.metric_name),
                severity: severity_for_ratio(outlier.value, distribution.outlier_cutoff.max(1.0)),
                category: "distribution".to_owned(),
                message: format!(
                    "metric {} value {:.2} exceeded outlier cutoff {:.2}",
                    distribution.metric_name, outlier.value, distribution.outlier_cutoff
                ),
                evidence: vec![InsightEvidence {
                    file_path: outlier.file_path.clone(),
                    qualified_name: outlier.qualified_name.clone(),
                    node_kind: None,
                    edge_kind: None,
                    line_range: None,
                    confidence_tier: None,
                }],
                ranking_reason: format!(
                    "distribution {} cutoff {:.2}, raw {:.2}",
                    distribution.metric_name, distribution.outlier_cutoff, outlier.value
                ),
                details: None,
                score: outlier.value,
            });
        }
    }

    findings
}

pub(super) fn node_metric_finding(
    metric: &NodeMetric,
    metric_name: &str,
    raw_value: f64,
    threshold: f64,
    title: &str,
) -> InsightFinding {
    InsightFinding {
        id: format!("metrics:{metric_name}:{}", metric.node.qualified_name),
        title: format!("{title} {}", metric.node.name),
        severity: severity_for_ratio(raw_value, threshold.max(1.0)),
        category: "node_metrics".to_owned(),
        message: format!(
            "metric {} for {} is {:.2}; threshold {:.2}",
            metric_name, metric.node.qualified_name, raw_value, threshold
        ),
        evidence: vec![InsightEvidence {
            file_path: Some(metric.node.file_path.clone()),
            qualified_name: Some(metric.node.qualified_name.clone()),
            node_kind: Some(metric.node.kind.as_str().to_owned()),
            edge_kind: None,
            line_range: Some(InsightLineRange {
                start_line: metric.node.line_start,
                end_line: metric.node.line_end,
            }),
            confidence_tier: None,
        }],
        ranking_reason: format!(
            "raw value {:.2} exceeded threshold {:.2} for {}",
            raw_value, threshold, metric_name
        ),
        details: None,
        score: raw_value,
    }
}

pub(super) fn severity_for_ratio(raw_value: f64, threshold: f64) -> InsightSeverity {
    if raw_value >= threshold * 2.0 {
        InsightSeverity::High
    } else if raw_value >= threshold {
        InsightSeverity::Medium
    } else {
        InsightSeverity::Low
    }
}

pub(super) fn compute_test_adjacency(
    node: &Node,
    inbound: &HashMap<String, Vec<&Edge>>,
    test_neighbors: &HashMap<String, BTreeSet<String>>,
    file_tests: Option<&Vec<&Node>>,
) -> (usize, CoverageStrength) {
    if let Some(linked) = test_neighbors.get(&node.qualified_name)
        && !linked.is_empty()
    {
        return (linked.len(), CoverageStrength::Direct);
    }

    let caller_tests = inbound
        .get(&node.qualified_name)
        .into_iter()
        .flat_map(|edges| edges.iter())
        .filter_map(|edge| test_neighbors.get(&edge.source_qn))
        .flat_map(|tests| tests.iter().cloned())
        .collect::<BTreeSet<_>>();
    if !caller_tests.is_empty() {
        return (caller_tests.len(), CoverageStrength::IndirectThroughCallers);
    }

    if let Some(file_tests) = file_tests
        && !file_tests.is_empty()
    {
        return (file_tests.len(), CoverageStrength::SameFile);
    }

    (0, CoverageStrength::None)
}

pub(super) fn build_direct_test_neighbors(
    snapshot: &GraphSnapshot,
    qname_to_node: &HashMap<String, &Node>,
) -> HashMap<String, BTreeSet<String>> {
    let mut neighbors = HashMap::<String, BTreeSet<String>>::new();
    for edge in &snapshot.edges {
        if !matches!(edge.kind, EdgeKind::Tests | EdgeKind::TestedBy) {
            continue;
        }
        let Some(source) = qname_to_node.get(&edge.source_qn) else {
            continue;
        };
        let Some(target) = qname_to_node.get(&edge.target_qn) else {
            continue;
        };
        if source.is_test || source.kind == NodeKind::Test {
            neighbors
                .entry(target.qualified_name.clone())
                .or_default()
                .insert(source.qualified_name.clone());
        }
        if target.is_test || target.kind == NodeKind::Test {
            neighbors
                .entry(source.qualified_name.clone())
                .or_default()
                .insert(target.qualified_name.clone());
        }
    }
    neighbors
}

pub(super) fn build_dependency_depths(snapshot: &GraphSnapshot) -> HashMap<String, usize> {
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

    let mut memo = HashMap::<String, usize>::new();
    let max_depth = snapshot.nodes.len();
    snapshot
        .nodes
        .iter()
        .map(|node| {
            let depth = dependency_depth(
                &node.qualified_name,
                &adjacency,
                &mut HashSet::new(),
                &mut memo,
                max_depth,
            );
            (node.qualified_name.clone(), depth)
        })
        .collect()
}

pub(super) fn dependency_depth(
    qname: &str,
    adjacency: &HashMap<String, Vec<String>>,
    visiting: &mut HashSet<String>,
    memo: &mut HashMap<String, usize>,
    remaining_budget: usize,
) -> usize {
    if remaining_budget == 0 {
        return 0;
    }
    if let Some(value) = memo.get(qname) {
        return *value;
    }
    if !visiting.insert(qname.to_owned()) {
        return 0;
    }

    let mut depth = 0;
    if let Some(targets) = adjacency.get(qname) {
        for target in targets {
            if target == qname || visiting.contains(target.as_str()) {
                continue;
            }
            depth = depth
                .max(1 + dependency_depth(target, adjacency, visiting, memo, remaining_budget - 1));
        }
    }

    visiting.remove(qname);
    memo.insert(qname.to_owned(), depth);
    depth
}
