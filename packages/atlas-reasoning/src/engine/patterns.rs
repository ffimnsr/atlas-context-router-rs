use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};

use atlas_core::{
    AtlasError, Edge, EdgeKind, InsightEvidence, InsightFinding, InsightLineRange, InsightSeverity,
    Node, NodeKind, PatternReport, Result,
};
use serde_json::json;

use super::InsightsEngine;
use super::helpers::{ENTRYPOINT_NAMES, dead_code_reasons, is_public_node};
use super::metrics::{
    GraphSnapshot, NodeMetric, RustComplexityMetrics, build_node_metrics, is_callable,
    is_reference_edge,
};

impl<'s> InsightsEngine<'s> {
    pub fn analyze_patterns(&self) -> Result<PatternReport> {
        let store = self.store().ok_or_else(|| {
            AtlasError::Other("pattern analysis requires a store-backed insights engine".to_owned())
        })?;
        let snapshot = self.load_graph_snapshot(store)?;
        let rust_complexity: HashMap<String, RustComplexityMetrics> = HashMap::new();
        let node_metrics = build_node_metrics(self, &snapshot, &rust_complexity);

        let mut findings = Vec::new();
        findings.extend(build_repeated_chain_findings(self, &snapshot));
        findings.extend(build_unused_structure_findings(&snapshot, &node_metrics));
        findings.extend(build_centrality_findings(self, &node_metrics));
        findings.extend(build_deep_chain_findings(self, &snapshot));

        Ok(self.pattern_report(findings))
    }
}

fn build_repeated_chain_findings(
    engine: &InsightsEngine<'_>,
    snapshot: &GraphSnapshot,
) -> Vec<InsightFinding> {
    let min_node_length = engine.config().repeated_call_chain_min_length;
    let max_node_length = engine.config().deep_chain_length.max(min_node_length + 1);
    let callable_nodes = snapshot
        .nodes
        .iter()
        .filter(|node| is_callable(node))
        .map(|node| (node.qualified_name.clone(), node))
        .collect::<BTreeMap<_, _>>();

    let mut call_edges_by_source = BTreeMap::<String, Vec<&Edge>>::new();
    for edge in &snapshot.edges {
        if edge.kind != EdgeKind::Calls {
            continue;
        }
        if callable_nodes.contains_key(&edge.source_qn)
            && callable_nodes.contains_key(&edge.target_qn)
        {
            call_edges_by_source
                .entry(edge.source_qn.clone())
                .or_default()
                .push(edge);
        }
    }
    for edges in call_edges_by_source.values_mut() {
        edges.sort_by(|left, right| {
            left.target_qn
                .cmp(&right.target_qn)
                .then_with(|| left.file_path.cmp(&right.file_path))
                .then_with(|| left.line.cmp(&right.line))
        });
    }

    let mut groups = BTreeMap::<Vec<String>, Vec<serde_json::Value>>::new();
    let mut seen_occurrences = BTreeSet::<Vec<String>>::new();
    for start_qname in callable_nodes.keys() {
        let mut path = vec![start_qname.clone()];
        let mut edge_path = Vec::new();
        let mut visiting = BTreeSet::from([start_qname.clone()]);
        collect_repeated_call_chains(
            start_qname,
            &callable_nodes,
            &call_edges_by_source,
            min_node_length,
            max_node_length,
            &mut path,
            &mut edge_path,
            &mut visiting,
            &mut groups,
            &mut seen_occurrences,
        );
    }

    groups
        .into_iter()
        .filter(|(_, occurrences)| occurrences.len() > 1)
        .map(|(sequence, occurrences)| {
            let evidence = occurrences
                .iter()
                .flat_map(|occurrence| {
                    occurrence["edges"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .take(1)
                        .map(json_edge_to_evidence)
                })
                .collect::<Vec<_>>();
            let score = (sequence.len() * occurrences.len()) as f64;
            let severity = if occurrences.len() >= 3 || sequence.len() > min_node_length - 1 {
                InsightSeverity::High
            } else {
                InsightSeverity::Medium
            };

            InsightFinding {
                id: format!("pattern:repeated_chain:{}", sequence.join("->")),
                title: format!("repeated call chain {}", sequence.join(" -> ")),
                severity,
                category: "pattern_repeated_chain".to_owned(),
                message: format!(
                    "simple-name sequence {} repeats {} times",
                    sequence.join(" -> "),
                    occurrences.len()
                ),
                evidence,
                ranking_reason: format!(
                    "sequence length {}; repeated {} times",
                    sequence.len(),
                    occurrences.len()
                ),
                details: Some(json!({
                    "sequence": sequence,
                    "occurrence_count": occurrences.len(),
                    "occurrences": occurrences,
                })),
                score,
            }
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn collect_repeated_call_chains(
    current_qname: &str,
    callable_nodes: &BTreeMap<String, &Node>,
    call_edges_by_source: &BTreeMap<String, Vec<&Edge>>,
    min_node_length: usize,
    max_node_length: usize,
    path: &mut Vec<String>,
    edge_path: &mut Vec<serde_json::Value>,
    visiting: &mut BTreeSet<String>,
    groups: &mut BTreeMap<Vec<String>, Vec<serde_json::Value>>,
    seen_occurrences: &mut BTreeSet<Vec<String>>,
) {
    if path.len() >= min_node_length {
        let occurrence_key = path.clone();
        if seen_occurrences.insert(occurrence_key) {
            let sequence = path
                .iter()
                .skip(1)
                .filter_map(|qname| callable_nodes.get(qname).map(|node| node.name.clone()))
                .collect::<Vec<_>>();
            let files = path
                .iter()
                .filter_map(|qname| callable_nodes.get(qname).map(|node| node.file_path.clone()))
                .collect::<Vec<_>>();
            groups.entry(sequence).or_default().push(json!({
                "qualified_names": path.clone(),
                "files": files,
                "edges": edge_path.clone(),
            }));
        }
    }
    if path.len() == max_node_length {
        return;
    }

    for edge in call_edges_by_source
        .get(current_qname)
        .into_iter()
        .flatten()
        .copied()
    {
        if visiting.contains(&edge.target_qn) {
            continue;
        }
        path.push(edge.target_qn.clone());
        edge_path.push(json!({
            "file_path": edge.file_path,
            "qualified_name": edge.source_qn,
            "target_qualified_name": edge.target_qn,
            "edge_kind": edge.kind.as_str(),
            "line": edge.line,
        }));
        visiting.insert(edge.target_qn.clone());
        collect_repeated_call_chains(
            &edge.target_qn,
            callable_nodes,
            call_edges_by_source,
            min_node_length,
            max_node_length,
            path,
            edge_path,
            visiting,
            groups,
            seen_occurrences,
        );
        visiting.remove(&edge.target_qn);
        edge_path.pop();
        path.pop();
    }
}

fn build_unused_structure_findings(
    snapshot: &GraphSnapshot,
    node_metrics: &[NodeMetric],
) -> Vec<InsightFinding> {
    let module_by_qname = node_metrics
        .iter()
        .map(|metric| (metric.node.qualified_name.clone(), metric.module_id.clone()))
        .collect::<HashMap<_, _>>();
    let node_metric_by_qname = node_metrics
        .iter()
        .map(|metric| (metric.node.qualified_name.clone(), metric))
        .collect::<HashMap<_, _>>();
    let mut nodes_by_module = BTreeMap::<String, Vec<&NodeMetric>>::new();
    for metric in node_metrics {
        nodes_by_module
            .entry(metric.module_id.clone())
            .or_default()
            .push(metric);
    }

    let mut external_inbound = HashMap::<String, usize>::new();
    for edge in &snapshot.edges {
        if !is_reference_edge(edge.kind) || edge.source_qn == edge.target_qn {
            continue;
        }
        let Some(source_module) = module_by_qname.get(&edge.source_qn) else {
            continue;
        };
        let Some(target_module) = module_by_qname.get(&edge.target_qn) else {
            continue;
        };
        if source_module != target_module {
            *external_inbound.entry(target_module.clone()).or_default() += 1;
        }
    }

    let mut findings = Vec::new();
    for (module_id, metrics) in &nodes_by_module {
        if metrics
            .iter()
            .all(|metric| metric.node.is_test || metric.node.kind == NodeKind::Test)
        {
            continue;
        }
        if external_inbound.get(module_id).copied().unwrap_or_default() != 0 {
            continue;
        }

        let blockers = structure_blockers(metrics);
        let evidence = metrics
            .iter()
            .take(3)
            .map(|metric| node_evidence(&metric.node))
            .collect();
        findings.push(InsightFinding {
            id: format!("pattern:unused_module:{module_id}"),
            title: format!("unused module candidate {module_id}"),
            severity: if blockers.is_empty() {
                InsightSeverity::Medium
            } else {
                InsightSeverity::Low
            },
            category: "pattern_unused_module".to_owned(),
            message: format!("module {module_id} has no inbound edges from outside its module"),
            evidence,
            ranking_reason: format!(
                "external inbound edges 0; blocker count {}",
                blockers.len()
            ),
            details: Some(json!({
                "module_id": module_id,
                "file_paths": metrics.iter().map(|metric| metric.node.file_path.clone()).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>(),
                "qualified_names": metrics.iter().map(|metric| metric.node.qualified_name.clone()).collect::<Vec<_>>(),
                "blockers": blockers,
            })),
            score: metrics.len() as f64,
        });
    }

    let adjacency = undirected_reference_adjacency(snapshot);
    let components = connected_components(snapshot, &adjacency);
    if components.len() > 1 {
        for component in components
            .into_iter()
            .filter(|component| component.len() > 1)
        {
            let component_metrics = component
                .iter()
                .filter_map(|qname| node_metric_by_qname.get(qname).copied())
                .collect::<Vec<_>>();
            if component_metrics.is_empty() {
                continue;
            }
            let blockers = structure_blockers(&component_metrics);
            findings.push(InsightFinding {
                id: format!("pattern:isolated_component:{}", component.join("|")),
                title: format!("isolated component of {} nodes", component.len()),
                severity: if blockers.is_empty() {
                    InsightSeverity::Medium
                } else {
                    InsightSeverity::Low
                },
                category: "pattern_isolated_component".to_owned(),
                message: "component has no incoming or outgoing edges outside its own connected subgraph"
                    .to_owned(),
                evidence: component_metrics
                    .iter()
                    .take(4)
                    .map(|metric| node_evidence(&metric.node))
                    .collect(),
                ranking_reason: format!(
                    "component size {}; blocker count {}",
                    component.len(),
                    blockers.len()
                ),
                details: Some(json!({
                    "qualified_names": component,
                    "file_paths": component_metrics.iter().map(|metric| metric.node.file_path.clone()).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>(),
                    "module_ids": component_metrics.iter().map(|metric| metric.module_id.clone()).collect::<BTreeSet<_>>().into_iter().collect::<Vec<_>>(),
                    "node_count": component_metrics.len(),
                    "blockers": blockers,
                })),
                score: component_metrics.len() as f64,
            });
        }
    }

    let meaningful_inbound = meaningful_inbound_counts(snapshot);
    for metric in node_metrics {
        if metric.node.is_test
            || metric.node.kind == NodeKind::Test
            || is_public_node(&metric.node)
            || is_entrypoint_name(&metric.node.name)
            || !is_orphan_candidate_kind(metric.node.kind)
            || metric.linked_test_count > 0
            || meaningful_inbound
                .get(&metric.node.qualified_name)
                .copied()
                .unwrap_or_default()
                > 0
        {
            continue;
        }

        let (reasons, confidence, mut blockers) = dead_code_reasons(&metric.node);
        let outbound_refs = snapshot
            .edges
            .iter()
            .filter(|edge| {
                edge.source_qn == metric.node.qualified_name && is_reference_edge(edge.kind)
            })
            .count();
        if outbound_refs > 0 {
            blockers.push(format!("outbound dependency count {outbound_refs}"));
        }
        findings.push(InsightFinding {
            id: format!("pattern:orphan:{}", metric.node.qualified_name),
            title: format!("orphan node {}", metric.node.name),
            severity: InsightSeverity::Low,
            category: "pattern_orphan_node".to_owned(),
            message: format!(
                "{} has no meaningful inbound references and no test adjacency",
                metric.node.qualified_name
            ),
            evidence: vec![node_evidence(&metric.node)],
            ranking_reason: format!(
                "reason {}; blocker count {}",
                reasons.join("; "),
                blockers.len()
            ),
            details: Some(json!({
                "qualified_name": metric.node.qualified_name,
                "file_path": metric.node.file_path,
                "node_kind": metric.node.kind.as_str(),
                "reasons": reasons,
                "blockers": blockers,
                "confidence_tier": confidence,
            })),
            score: 1.0,
        });
    }

    findings
}

fn build_centrality_findings(
    engine: &InsightsEngine<'_>,
    node_metrics: &[NodeMetric],
) -> Vec<InsightFinding> {
    let mut degrees = node_metrics
        .iter()
        .map(|metric| (metric.fan_in + metric.fan_out) as f64)
        .filter(|value| *value > 0.0)
        .collect::<Vec<_>>();
    degrees.sort_by(|left, right| left.total_cmp(right));
    let cutoff = percentile(&degrees, engine.config().outlier_percentile_cutoff as f64);

    node_metrics
        .iter()
        .filter_map(|metric| {
            let degree = metric.fan_in + metric.fan_out;
            if degree == 0 {
                return None;
            }
            let hub = degree as f64 >= cutoff && cutoff > 0.0;
            let bottleneck = metric.fan_in >= engine.config().high_fan_in
                && metric.fan_out >= engine.config().high_fan_out;
            if !hub && !bottleneck {
                return None;
            }

            Some(InsightFinding {
                id: format!("pattern:centrality:{}", metric.node.qualified_name),
                title: format!("high-centrality node {}", metric.node.name),
                severity: if bottleneck {
                    InsightSeverity::High
                } else {
                    InsightSeverity::Medium
                },
                category: "pattern_centrality".to_owned(),
                message: format!(
                    "{} has degree centrality {} (fan-in {}, fan-out {})",
                    metric.node.qualified_name, degree, metric.fan_in, metric.fan_out,
                ),
                evidence: vec![node_evidence(&metric.node)],
                ranking_reason: format!(
                    "degree {}; hub cutoff {:.2}; bottleneck {}",
                    degree,
                    cutoff,
                    if bottleneck { "yes" } else { "no" }
                ),
                details: Some(json!({
                    "qualified_name": metric.node.qualified_name,
                    "file_path": metric.node.file_path,
                    "module_id": metric.module_id,
                    "hub": hub,
                    "bottleneck": bottleneck,
                    "degree_centrality": degree,
                    "fan_in": metric.fan_in,
                    "fan_out": metric.fan_out,
                })),
                score: degree as f64,
            })
        })
        .collect()
}

fn build_deep_chain_findings(
    engine: &InsightsEngine<'_>,
    snapshot: &GraphSnapshot,
) -> Vec<InsightFinding> {
    let node_by_qname = snapshot
        .nodes
        .iter()
        .map(|node| (node.qualified_name.clone(), node))
        .collect::<HashMap<_, _>>();
    let adjacency = reference_adjacency(snapshot);
    let max_depth = engine.config().deep_chain_length.saturating_mul(2).max(4);
    let max_nodes = snapshot
        .nodes
        .len()
        .min(max_depth.saturating_mul(8).max(max_depth));
    let depth_map = reference_depths(&adjacency, max_depth);

    snapshot
        .nodes
        .iter()
        .filter_map(|node| {
            let depth = depth_map.get(&node.qualified_name).copied().unwrap_or_default();
            if depth <= engine.config().deep_chain_length {
                return None;
            }
            let (chain, capped) = longest_reference_chain(
                &node.qualified_name,
                &adjacency,
                &depth_map,
                max_depth + 1,
                max_nodes,
            );
            if chain.len() <= engine.config().deep_chain_length + 1 {
                return None;
            }
            let file_paths = chain
                .iter()
                .filter_map(|qname| node_by_qname.get(qname).map(|node| node.file_path.clone()))
                .collect::<Vec<_>>();
            let evidence = chain
                .iter()
                .filter_map(|qname| node_by_qname.get(qname).copied())
                .take(4)
                .map(node_evidence)
                .collect::<Vec<_>>();
            Some(InsightFinding {
                id: format!("pattern:deep_chain:{}", chain.join("->")),
                title: format!("deep dependency chain from {}", node.name),
                severity: if depth >= engine.config().deep_chain_length.saturating_mul(2) {
                    InsightSeverity::High
                } else {
                    InsightSeverity::Medium
                },
                category: "pattern_deep_chain".to_owned(),
                message: format!(
                    "chain depth {} exceeds configured threshold {}",
                    depth,
                    engine.config().deep_chain_length,
                ),
                evidence,
                ranking_reason: format!(
                    "depth {}; traversal cap depth {}; traversal cap nodes {}; cycle guard enabled{}",
                    depth,
                    max_depth,
                    max_nodes,
                    if capped { "; cap hit" } else { "" }
                ),
                details: Some(json!({
                    "chain": chain,
                    "file_paths": file_paths,
                    "depth": depth,
                    "threshold": engine.config().deep_chain_length,
                    "capped": capped,
                })),
                score: depth as f64,
            })
        })
        .collect()
}

fn structure_blockers(metrics: &[&NodeMetric]) -> Vec<String> {
    let mut blockers = BTreeSet::new();
    if metrics.iter().any(|metric| is_public_node(&metric.node)) {
        blockers.insert("contains public API symbols".to_owned());
    }
    if metrics
        .iter()
        .any(|metric| metric.node.is_test || metric.node.kind == NodeKind::Test)
    {
        blockers.insert("contains tests".to_owned());
    }
    if metrics
        .iter()
        .any(|metric| is_entrypoint_name(&metric.node.name))
    {
        blockers.insert("contains entrypoint-like symbol".to_owned());
    }
    if metrics.iter().any(|metric| metric.linked_test_count > 0) {
        blockers.insert("linked tests reference this structure".to_owned());
    }
    blockers.into_iter().collect()
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

fn json_edge_to_evidence(value: &serde_json::Value) -> InsightEvidence {
    InsightEvidence {
        file_path: value["file_path"].as_str().map(str::to_owned),
        qualified_name: value["qualified_name"].as_str().map(str::to_owned),
        node_kind: None,
        edge_kind: value["edge_kind"].as_str().map(str::to_owned),
        line_range: value["line"].as_u64().map(|line| InsightLineRange {
            start_line: line as u32,
            end_line: line as u32,
        }),
        confidence_tier: None,
    }
}

fn is_entrypoint_name(name: &str) -> bool {
    ENTRYPOINT_NAMES
        .iter()
        .any(|candidate| candidate.eq_ignore_ascii_case(name))
}

fn is_orphan_candidate_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Function
            | NodeKind::Method
            | NodeKind::Class
            | NodeKind::Struct
            | NodeKind::Enum
            | NodeKind::Trait
            | NodeKind::Interface
            | NodeKind::Constant
            | NodeKind::Variable
    )
}

fn meaningful_inbound_counts(snapshot: &GraphSnapshot) -> HashMap<String, usize> {
    let mut inbound = HashMap::<String, usize>::new();
    for edge in &snapshot.edges {
        if edge.source_qn == edge.target_qn || !is_meaningful_inbound_edge(edge.kind) {
            continue;
        }
        *inbound.entry(edge.target_qn.clone()).or_default() += 1;
    }
    inbound
}

fn is_meaningful_inbound_edge(kind: EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Calls
            | EdgeKind::Imports
            | EdgeKind::References
            | EdgeKind::Extends
            | EdgeKind::Implements
    )
}

fn undirected_reference_adjacency(snapshot: &GraphSnapshot) -> BTreeMap<String, BTreeSet<String>> {
    let mut adjacency = BTreeMap::<String, BTreeSet<String>>::new();
    for node in &snapshot.nodes {
        adjacency.entry(node.qualified_name.clone()).or_default();
    }
    for edge in &snapshot.edges {
        if !is_reference_edge(edge.kind) || edge.source_qn == edge.target_qn {
            continue;
        }
        adjacency
            .entry(edge.source_qn.clone())
            .or_default()
            .insert(edge.target_qn.clone());
        adjacency
            .entry(edge.target_qn.clone())
            .or_default()
            .insert(edge.source_qn.clone());
    }
    adjacency
}

fn connected_components(
    snapshot: &GraphSnapshot,
    adjacency: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<Vec<String>> {
    let mut remaining = snapshot
        .nodes
        .iter()
        .map(|node| node.qualified_name.clone())
        .collect::<BTreeSet<_>>();
    let mut components = Vec::new();

    while let Some(start) = remaining.pop_first() {
        let mut queue = VecDeque::from([start.clone()]);
        let mut component = vec![start.clone()];
        while let Some(current) = queue.pop_front() {
            for next in adjacency.get(&current).into_iter().flatten() {
                if remaining.remove(next) {
                    queue.push_back(next.clone());
                    component.push(next.clone());
                }
            }
        }
        component.sort();
        components.push(component);
    }

    components.sort();
    components
}

fn reference_adjacency(snapshot: &GraphSnapshot) -> BTreeMap<String, Vec<String>> {
    let mut adjacency = BTreeMap::<String, Vec<String>>::new();
    for edge in &snapshot.edges {
        if !is_reference_edge(edge.kind) || edge.source_qn == edge.target_qn {
            continue;
        }
        adjacency
            .entry(edge.source_qn.clone())
            .or_default()
            .push(edge.target_qn.clone());
    }
    for targets in adjacency.values_mut() {
        targets.sort();
        targets.dedup();
    }
    adjacency
}

fn reference_depths(
    adjacency: &BTreeMap<String, Vec<String>>,
    max_depth: usize,
) -> HashMap<String, usize> {
    let mut memo = HashMap::<String, usize>::new();
    for qname in adjacency.keys() {
        let depth = reference_depth(qname, adjacency, &mut HashSet::new(), &mut memo, max_depth);
        memo.entry(qname.clone()).or_insert(depth);
    }
    memo
}

fn reference_depth(
    qname: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
    visiting: &mut HashSet<String>,
    memo: &mut HashMap<String, usize>,
    remaining_depth: usize,
) -> usize {
    if remaining_depth == 0 {
        return 0;
    }
    if let Some(depth) = memo.get(qname) {
        return *depth;
    }
    if !visiting.insert(qname.to_owned()) {
        return 0;
    }

    let mut depth = 0;
    for next in adjacency.get(qname).into_iter().flatten() {
        if visiting.contains(next) {
            continue;
        }
        depth =
            depth.max(1 + reference_depth(next, adjacency, visiting, memo, remaining_depth - 1));
    }

    visiting.remove(qname);
    memo.insert(qname.to_owned(), depth);
    depth
}

fn longest_reference_chain(
    start_qname: &str,
    adjacency: &BTreeMap<String, Vec<String>>,
    depth_map: &HashMap<String, usize>,
    max_depth: usize,
    max_nodes: usize,
) -> (Vec<String>, bool) {
    let mut chain = vec![start_qname.to_owned()];
    let mut visiting = BTreeSet::from([start_qname.to_owned()]);
    let mut current = start_qname.to_owned();

    while chain.len() < max_depth && chain.len() < max_nodes {
        let mut candidates = adjacency
            .get(&current)
            .into_iter()
            .flatten()
            .filter(|next| !visiting.contains(*next))
            .map(|next| {
                (
                    next.clone(),
                    depth_map.get(next).copied().unwrap_or_default(),
                )
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        let Some((next, next_depth)) = candidates.into_iter().next() else {
            break;
        };
        if next_depth == 0 {
            break;
        }
        visiting.insert(next.clone());
        chain.push(next.clone());
        current = next;
    }

    let capped = chain.len() == max_depth || chain.len() == max_nodes;
    (chain, capped)
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let rank = ((percentile / 100.0) * (values.len().saturating_sub(1) as f64)).round() as usize;
    values[rank.min(values.len() - 1)]
}
