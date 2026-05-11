use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::path::Path;

use atlas_core::{
    ArchitectureReport, AtlasError, InsightEvidence, InsightFinding, InsightSeverity, Result,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::InsightsEngine;
use super::metrics::{FileMetric, ModuleMetric, NodeMetric};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureModuleNode {
    pub module_id: String,
    pub file_paths: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
    pub node_count: usize,
    pub coupling_score: f64,
    pub cohesion: f64,
    pub internal_edge_count: usize,
    pub external_dependency_edge_count: usize,
    pub inbound_dependency_edge_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureEdgeEvidence {
    pub file_path: String,
    pub source_qn: String,
    pub target_qn: String,
    pub edge_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureModuleEdge {
    pub source_module: String,
    pub target_module: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_layer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_layer: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<ArchitectureEdgeEvidence>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureAnalysis {
    pub report: ArchitectureReport,
    pub modules: Vec<ArchitectureModuleNode>,
    pub edges: Vec<ArchitectureModuleEdge>,
}

#[derive(Debug, Clone)]
struct ModuleContents {
    file_paths: Vec<String>,
    qualified_names: Vec<String>,
}

impl<'s> InsightsEngine<'s> {
    pub fn analyze_architecture(
        &self,
        repo_root: impl AsRef<Path>,
    ) -> Result<ArchitectureAnalysis> {
        let repo_root = repo_root.as_ref();
        let metrics = self.analyze_metrics(repo_root)?;
        let store = self.store().ok_or_else(|| {
            AtlasError::Other(
                "architecture analysis requires a store-backed insights engine".to_owned(),
            )
        })?;
        let snapshot = self.load_graph_snapshot(store)?;

        let module_contents = build_module_contents(&metrics.metrics.node_metrics);
        let layer_by_module = assign_layers(self, &module_contents)?;
        let mut modules =
            build_architecture_modules(&metrics.metrics.module_metrics, &layer_by_module);
        let mut edges = build_module_edges(
            &snapshot.edges,
            &metrics.metrics.node_metrics,
            &layer_by_module,
        );
        let findings =
            build_architecture_findings(self, &modules, &edges, &metrics.metrics.file_metrics);
        let report = self.architecture_report(findings);

        modules.sort_by(|left, right| left.module_id.cmp(&right.module_id));
        edges.sort_by(|left, right| {
            left.source_module
                .cmp(&right.source_module)
                .then_with(|| left.target_module.cmp(&right.target_module))
        });

        Ok(ArchitectureAnalysis {
            report,
            modules,
            edges,
        })
    }
}

fn build_module_contents(node_metrics: &[NodeMetric]) -> BTreeMap<String, ModuleContents> {
    let mut file_paths = BTreeMap::<String, BTreeSet<String>>::new();
    let mut qualified_names = BTreeMap::<String, BTreeSet<String>>::new();

    for metric in node_metrics {
        file_paths
            .entry(metric.module_id.clone())
            .or_default()
            .insert(metric.node.file_path.clone());
        qualified_names
            .entry(metric.module_id.clone())
            .or_default()
            .insert(metric.node.qualified_name.clone());
    }

    file_paths
        .into_iter()
        .map(|(module_id, module_files)| {
            let qnames = qualified_names
                .remove(&module_id)
                .unwrap_or_default()
                .into_iter()
                .collect();
            (
                module_id,
                ModuleContents {
                    file_paths: module_files.into_iter().collect(),
                    qualified_names: qnames,
                },
            )
        })
        .collect()
}

fn assign_layers(
    engine: &InsightsEngine<'_>,
    module_contents: &BTreeMap<String, ModuleContents>,
) -> Result<BTreeMap<String, String>> {
    if engine.config().layer_rules.is_empty() {
        return Ok(BTreeMap::new());
    }

    let mut assignments = BTreeMap::new();
    for (module_id, contents) in module_contents {
        let mut matches = Vec::new();
        for rule in &engine.config().layer_rules {
            let path_match = contents.file_paths.iter().any(|file_path| {
                rule.path_prefixes.iter().any(|prefix| {
                    file_path == prefix
                        || file_path
                            .strip_prefix(prefix.as_str())
                            .is_some_and(|suffix| suffix.starts_with('/'))
                })
            });
            let module_match = contents.qualified_names.iter().any(|qualified_name| {
                rule.module_prefixes.iter().any(|prefix| {
                    qualified_name == prefix
                        || qualified_name
                            .strip_prefix(prefix.as_str())
                            .is_some_and(|suffix| suffix.starts_with("::"))
                })
            });
            if path_match || module_match {
                matches.push(rule.name.clone());
            }
        }
        if matches.len() > 1 {
            return Err(AtlasError::Other(format!(
                "invalid insights layer configuration: module {module_id} matches multiple layers: {}",
                matches.join(", ")
            )));
        }
        if let Some(layer) = matches.into_iter().next() {
            assignments.insert(module_id.clone(), layer);
        }
    }

    Ok(assignments)
}

fn build_architecture_modules(
    module_metrics: &[ModuleMetric],
    layer_by_module: &BTreeMap<String, String>,
) -> Vec<ArchitectureModuleNode> {
    module_metrics
        .iter()
        .map(|metric| ArchitectureModuleNode {
            module_id: metric.module_id.clone(),
            file_paths: metric.file_paths.clone(),
            layer: layer_by_module.get(&metric.module_id).cloned(),
            node_count: metric.node_count,
            coupling_score: metric.coupling_score,
            cohesion: metric.cohesion,
            internal_edge_count: metric.internal_edge_count,
            external_dependency_edge_count: metric.external_dependency_edge_count,
            inbound_dependency_edge_count: metric.inbound_dependency_edge_count,
        })
        .collect()
}

fn build_module_edges(
    edges: &[atlas_core::Edge],
    node_metrics: &[NodeMetric],
    layer_by_module: &BTreeMap<String, String>,
) -> Vec<ArchitectureModuleEdge> {
    let node_module_by_qname = node_metrics
        .iter()
        .map(|metric| (metric.node.qualified_name.clone(), metric.module_id.clone()))
        .collect::<HashMap<_, _>>();
    let mut module_edges = BTreeMap::<(String, String), Vec<ArchitectureEdgeEvidence>>::new();

    for edge in edges {
        let Some(source_module) = node_module_by_qname.get(&edge.source_qn) else {
            continue;
        };
        let Some(target_module) = node_module_by_qname.get(&edge.target_qn) else {
            continue;
        };
        if source_module == target_module && edge.source_qn != edge.target_qn {
            continue;
        }
        module_edges
            .entry((source_module.clone(), target_module.clone()))
            .or_default()
            .push(ArchitectureEdgeEvidence {
                file_path: edge.file_path.clone(),
                source_qn: edge.source_qn.clone(),
                target_qn: edge.target_qn.clone(),
                edge_kind: edge.kind.as_str().to_owned(),
                line: edge.line,
            });
    }

    module_edges
        .into_iter()
        .map(|((source_module, target_module), mut evidence)| {
            evidence.sort_by(|left, right| {
                left.file_path
                    .cmp(&right.file_path)
                    .then_with(|| left.line.cmp(&right.line))
                    .then_with(|| left.source_qn.cmp(&right.source_qn))
                    .then_with(|| left.target_qn.cmp(&right.target_qn))
                    .then_with(|| left.edge_kind.cmp(&right.edge_kind))
            });
            evidence.dedup();
            ArchitectureModuleEdge {
                source_module: source_module.clone(),
                target_module: target_module.clone(),
                source_layer: layer_by_module.get(&source_module).cloned(),
                target_layer: layer_by_module.get(&target_module).cloned(),
                evidence,
            }
        })
        .collect()
}

fn build_architecture_findings(
    engine: &InsightsEngine<'_>,
    modules: &[ArchitectureModuleNode],
    edges: &[ArchitectureModuleEdge],
    file_metrics: &[FileMetric],
) -> Vec<InsightFinding> {
    let mut findings = Vec::new();
    let module_by_id = modules
        .iter()
        .map(|module| (module.module_id.clone(), module))
        .collect::<BTreeMap<_, _>>();
    let edge_by_pair = edges
        .iter()
        .map(|edge| {
            (
                (edge.source_module.clone(), edge.target_module.clone()),
                edge,
            )
        })
        .collect::<BTreeMap<_, _>>();
    let adjacency = build_module_adjacency(modules, edges);
    let sccs = strongly_connected_components(&adjacency);
    let threshold = engine.config().high_coupling as f64;

    for component in sccs {
        if component.len() > 1 {
            let cycle_path = deterministic_cycle_path(&component, &adjacency)
                .unwrap_or_else(|| fallback_cycle_path(&component));
            let cycle_edges = cycle_path_pairs(&cycle_path)
                .into_iter()
                .filter_map(|(source, target)| edge_by_pair.get(&(source, target)).copied())
                .collect::<Vec<_>>();
            let classification = cycle_classification(&component, &module_by_id);
            let evidence = cycle_edges
                .iter()
                .filter_map(|edge| edge.evidence.first())
                .map(edge_evidence_to_insight)
                .collect::<Vec<_>>();
            findings.push(InsightFinding {
                id: format!("architecture:cycle:{}", component.join("|")),
                title: format!("{classification} cycle"),
                severity: if classification == "cross-module" {
                    InsightSeverity::High
                } else {
                    InsightSeverity::Medium
                },
                category: "architecture_cycle".to_owned(),
                message: format!(
                    "{} dependency cycle spans {} modules",
                    classification,
                    component.len()
                ),
                evidence,
                ranking_reason: format!(
                    "{} cycle path {}",
                    classification,
                    cycle_path.join(" -> ")
                ),
                details: Some(json!({
                    "classification": classification,
                    "modules": component,
                    "cycle_path": cycle_path,
                    "edges": cycle_edges,
                })),
                score: component.len() as f64,
            });
        }
    }

    for edge in edges
        .iter()
        .filter(|edge| edge.source_module == edge.target_module)
    {
        findings.push(InsightFinding {
            id: format!("architecture:self-cycle:{}", edge.source_module),
            title: format!("self cycle {}", edge.source_module),
            severity: InsightSeverity::Medium,
            category: "architecture_cycle".to_owned(),
            message: format!(
                "module {} contains an explicit self-cycle",
                edge.source_module
            ),
            evidence: edge
                .evidence
                .first()
                .into_iter()
                .map(edge_evidence_to_insight)
                .collect(),
            ranking_reason: "explicit recursive edge collapsed to module graph self-cycle"
                .to_owned(),
            details: Some(json!({
                "classification": "local",
                "modules": [edge.source_module.clone()],
                "cycle_path": [edge.source_module.clone(), edge.target_module.clone()],
                "edges": [edge],
            })),
            score: 1.0,
        });
    }

    let layer_order = engine
        .config()
        .layer_rules
        .iter()
        .enumerate()
        .map(|(index, rule)| (rule.name.clone(), index))
        .collect::<HashMap<_, _>>();
    for edge in edges
        .iter()
        .filter(|edge| edge.source_module != edge.target_module)
    {
        let Some(source_layer) = edge.source_layer.as_deref() else {
            continue;
        };
        let Some(target_layer) = edge.target_layer.as_deref() else {
            continue;
        };
        let Some(source_index) = layer_order.get(source_layer) else {
            continue;
        };
        let Some(target_index) = layer_order.get(target_layer) else {
            continue;
        };
        if source_index <= target_index {
            continue;
        }

        findings.push(InsightFinding {
            id: format!(
                "architecture:layer:{}:{}",
                edge.source_module, edge.target_module
            ),
            title: format!("layer violation {source_layer} -> {target_layer}"),
            severity: InsightSeverity::High,
            category: "layer_violation".to_owned(),
            message: format!(
                "module {} in layer {} depends on higher layer {} via {}",
                edge.source_module, source_layer, target_layer, edge.target_module
            ),
            evidence: edge
                .evidence
                .iter()
                .take(5)
                .map(edge_evidence_to_insight)
                .collect(),
            ranking_reason: format!(
                "configured layer order forbids {} depending on {}",
                source_layer, target_layer
            ),
            details: Some(json!({
                "source_layer": source_layer,
                "target_layer": target_layer,
                "source_module": edge.source_module,
                "target_module": edge.target_module,
                "edge_count": edge.evidence.len(),
            })),
            score: (*source_index - *target_index + 1) as f64,
        });
    }

    for module in modules
        .iter()
        .filter(|module| module.coupling_score >= threshold)
    {
        findings.push(InsightFinding {
            id: format!("architecture:module:{}", module.module_id),
            title: format!("high coupling module {}", module.module_id),
            severity: severity_for_ratio(module.coupling_score, threshold.max(1.0)),
            category: "architecture_module_health".to_owned(),
            message: format!(
                "module {} coupling {:.2} exceeds threshold {:.2}",
                module.module_id, module.coupling_score, threshold
            ),
            evidence: module
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
                "module coupling {:.2} crossed architecture threshold {:.2}",
                module.coupling_score, threshold
            ),
            details: Some(json!({
                "module_id": module.module_id,
                "layer": module.layer,
                "coupling_score": module.coupling_score,
                "cohesion": module.cohesion,
                "outbound_dependencies": module.external_dependency_edge_count,
                "inbound_dependencies": module.inbound_dependency_edge_count,
            })),
            score: module.coupling_score,
        });
    }

    for component in strongly_connected_components(&adjacency)
        .into_iter()
        .filter(|component| component.len() > 1)
    {
        let component_modules = component
            .iter()
            .filter_map(|module_id| module_by_id.get(module_id).copied())
            .collect::<Vec<_>>();
        if component_modules.is_empty() {
            continue;
        }
        let average_coupling = component_modules
            .iter()
            .map(|module| module.coupling_score)
            .sum::<f64>()
            / component_modules.len() as f64;
        if average_coupling < threshold {
            continue;
        }
        findings.push(InsightFinding {
            id: format!("architecture:cluster:{}", component.join("|")),
            title: format!("tightly coupled cluster ({})", component.len()),
            severity: severity_for_ratio(average_coupling, threshold.max(1.0)),
            category: "architecture_cluster".to_owned(),
            message: format!(
                "cycle cluster of {} modules has average coupling {:.2}",
                component.len(),
                average_coupling
            ),
            evidence: component_modules
                .iter()
                .filter_map(|module| module.file_paths.first())
                .cloned()
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
                "SCC size {} with average coupling {:.2} exceeded threshold {:.2}",
                component.len(),
                average_coupling,
                threshold
            ),
            details: Some(json!({
                "modules": component,
                "average_coupling": average_coupling,
            })),
            score: average_coupling,
        });
    }

    for metric in file_metrics.iter().filter(|metric| metric.highly_connected) {
        let connectivity = metric.average_fan_in + metric.average_fan_out;
        findings.push(InsightFinding {
            id: format!("architecture:file:{}", metric.file_path),
            title: format!("architecture hotspot {}", metric.file_path),
            severity: severity_for_ratio(
                connectivity.max(metric.node_count as f64),
                threshold.max(1.0),
            ),
            category: "architecture_file_health".to_owned(),
            message: format!(
                "file {} is highly connected with connectivity {:.2} across {} nodes",
                metric.file_path, connectivity, metric.node_count
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
                "file connectivity {:.2} or node count {} exceeded architecture hotspot threshold",
                connectivity, metric.node_count
            ),
            details: Some(json!({
                "file_path": metric.file_path,
                "module_id": metric.module_id,
                "connectivity": connectivity,
                "node_count": metric.node_count,
                "edge_count": metric.edge_count,
            })),
            score: connectivity.max(metric.node_count as f64),
        });
    }

    findings
}

fn build_module_adjacency(
    modules: &[ArchitectureModuleNode],
    edges: &[ArchitectureModuleEdge],
) -> BTreeMap<String, Vec<String>> {
    let mut adjacency = modules
        .iter()
        .map(|module| (module.module_id.clone(), Vec::new()))
        .collect::<BTreeMap<_, _>>();
    for edge in edges
        .iter()
        .filter(|edge| edge.source_module != edge.target_module)
    {
        adjacency
            .entry(edge.source_module.clone())
            .or_default()
            .push(edge.target_module.clone());
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort();
        neighbors.dedup();
    }
    adjacency
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

fn deterministic_cycle_path(
    component: &[String],
    adjacency: &BTreeMap<String, Vec<String>>,
) -> Option<Vec<String>> {
    let component_set = component.iter().cloned().collect::<HashSet<_>>();
    let start = component.first()?.clone();
    let neighbors = adjacency
        .get(&start)
        .into_iter()
        .flat_map(|neighbors| neighbors.iter())
        .filter(|neighbor| component_set.contains(*neighbor))
        .cloned()
        .collect::<Vec<_>>();

    for neighbor in neighbors {
        let mut queue = VecDeque::from([(neighbor.clone(), vec![neighbor.clone()])]);
        let mut visited = HashSet::from([neighbor.clone()]);
        while let Some((current, path)) = queue.pop_front() {
            let mut next_nodes = adjacency.get(&current).cloned().unwrap_or_default();
            next_nodes.retain(|candidate| component_set.contains(candidate));
            next_nodes.sort();
            for next in next_nodes {
                if next == start {
                    let mut cycle = vec![start.clone()];
                    cycle.extend(path.clone());
                    cycle.push(start.clone());
                    return Some(cycle);
                }
                if visited.insert(next.clone()) {
                    let mut next_path = path.clone();
                    next_path.push(next.clone());
                    queue.push_back((next, next_path));
                }
            }
        }
    }

    None
}

fn fallback_cycle_path(component: &[String]) -> Vec<String> {
    let mut path = component.to_vec();
    if let Some(first) = component.first() {
        path.push(first.clone());
    }
    path
}

fn cycle_path_pairs(path: &[String]) -> Vec<(String, String)> {
    path.windows(2)
        .map(|pair| (pair[0].clone(), pair[1].clone()))
        .collect()
}

fn cycle_classification(
    component: &[String],
    module_by_id: &BTreeMap<String, &ArchitectureModuleNode>,
) -> &'static str {
    let roots = component
        .iter()
        .filter_map(|module_id| module_by_id.get(module_id).copied())
        .map(module_root_key)
        .collect::<BTreeSet<_>>();
    if roots.len() <= 1 {
        "local"
    } else {
        "cross-module"
    }
}

fn module_root_key(module: &ArchitectureModuleNode) -> String {
    if let Some(rest) = module.module_id.strip_prefix("cargo:") {
        return rest
            .rsplit_once('/')
            .map(|(prefix, _)| prefix.to_owned())
            .unwrap_or_else(|| rest.to_owned());
    }
    if let Some(rest) = module.module_id.strip_prefix("npm:") {
        return rest
            .rsplit_once('/')
            .map(|(prefix, _)| prefix.to_owned())
            .unwrap_or_else(|| rest.to_owned());
    }
    if let Some(rest) = module.module_id.strip_prefix("go:") {
        return rest
            .rsplit_once('/')
            .map(|(prefix, _)| prefix.to_owned())
            .unwrap_or_else(|| rest.to_owned());
    }
    if let Some(rest) = module.module_id.strip_prefix("module:") {
        return rest.split('/').next().unwrap_or("<root>").to_owned();
    }
    module
        .file_paths
        .first()
        .and_then(|file_path| file_path.split('/').next())
        .unwrap_or("<root>")
        .to_owned()
}

fn edge_evidence_to_insight(evidence: &ArchitectureEdgeEvidence) -> InsightEvidence {
    InsightEvidence {
        file_path: Some(evidence.file_path.clone()),
        qualified_name: Some(evidence.source_qn.clone()),
        node_kind: None,
        edge_kind: Some(evidence.edge_kind.clone()),
        line_range: evidence.line.map(|line| atlas_core::InsightLineRange {
            start_line: line,
            end_line: line,
        }),
        confidence_tier: None,
    }
}

fn severity_for_ratio(raw_value: f64, threshold: f64) -> InsightSeverity {
    if raw_value >= threshold * 2.0 {
        InsightSeverity::High
    } else if raw_value >= threshold {
        InsightSeverity::Medium
    } else {
        InsightSeverity::Low
    }
}
