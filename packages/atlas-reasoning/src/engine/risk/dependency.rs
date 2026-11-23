use super::*;

pub(super) fn related_node_evidence(edges: &[(Node, Edge)], limit: usize) -> Vec<InsightEvidence> {
    edges
        .iter()
        .take(limit)
        .map(|(neighbor, edge)| edge_evidence(neighbor, edge))
        .collect()
}

pub(super) fn node_evidence(node: &Node) -> InsightEvidence {
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

pub(super) fn edge_evidence(neighbor: &Node, edge: &Edge) -> InsightEvidence {
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

pub(super) fn parse_confidence_tier(value: Option<&str>) -> Option<ConfidenceTier> {
    match value {
        Some("high") => Some(ConfidenceTier::High),
        Some("medium") => Some(ConfidenceTier::Medium),
        Some("low") => Some(ConfidenceTier::Low),
        _ => None,
    }
}

pub(super) fn sort_and_dedup_evidence(mut evidence: Vec<InsightEvidence>) -> Vec<InsightEvidence> {
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

pub(super) fn longest_dependency_path(snapshot: &GraphSnapshot, start_qname: &str) -> Vec<String> {
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

pub(super) fn longest_dependency_path_inner(
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

pub(super) fn cycle_context(
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
                repo_provenance: None,
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
