use super::boundaries::boundary_signal_sets;
use super::*;

// Base scores by node kind (seed nodes start with this score).
fn base_score_for_kind(kind: NodeKind) -> f64 {
    match kind {
        NodeKind::Function | NodeKind::Method => 1.0,
        NodeKind::Class
        | NodeKind::Struct
        | NodeKind::Enum
        | NodeKind::Trait
        | NodeKind::Interface => 1.2,
        NodeKind::Module | NodeKind::Package => 0.8,
        NodeKind::Constant | NodeKind::Variable => 0.5,
        NodeKind::Test => 0.4,
        _ => 0.6,
    }
}

/// Extra multiplier for public-API nodes (e.g. `pub fn`).
fn api_multiplier(node: &Node) -> f64 {
    let mods = node.modifiers.as_deref().unwrap_or("");
    if mods.contains("pub") { 1.5 } else { 1.0 }
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Score all nodes (changed + impacted) using weighted BFS propagation.
///
/// Seed nodes (changed) start with a kind-based base score × api_multiplier.
/// Each hop through `relevant_edges` decays the score by `HOP_DECAY` times
/// the edge's `traversal_weight`.  Every node is kept at its highest score
/// across all paths (max propagation).
pub(crate) fn score_nodes(
    base: &ImpactResult,
    test_impact: &TestImpactResult,
    violations: &[BoundaryViolation],
) -> Vec<ScoredImpactNode> {
    // Seed scores for changed nodes.
    let mut scores: HashMap<String, f64> = base
        .changed_nodes
        .iter()
        .map(|n| {
            let s = base_score_for_kind(n.kind) * api_multiplier(n);
            (n.qualified_name.clone(), s)
        })
        .collect();

    // Build adjacency: qn → list of (neighbour_qn, edge_weight).
    // Bidirectional so impact propagates both ways.
    let mut adj: HashMap<&str, Vec<(&str, f64)>> = HashMap::new();
    for e in &base.relevant_edges {
        let w = e.kind.traversal_weight() * e.confidence as f64;
        adj.entry(&e.source_qn).or_default().push((&e.target_qn, w));
        adj.entry(&e.target_qn).or_default().push((&e.source_qn, w));
    }

    // BFS from seeds (take highest score, re-queue when improved).
    let mut queue: VecDeque<String> = base
        .changed_nodes
        .iter()
        .map(|n| n.qualified_name.clone())
        .collect();

    while let Some(qn) = queue.pop_front() {
        let current_score = match scores.get(qn.as_str()) {
            Some(&s) => s,
            None => continue,
        };
        if let Some(neighbours) = adj.get(qn.as_str()) {
            for (nbr, weight) in neighbours {
                let propagated = current_score * HOP_DECAY * weight;
                let entry = scores.entry(nbr.to_string()).or_insert(0.0);
                if propagated > *entry {
                    *entry = propagated;
                    queue.push_back(nbr.to_string());
                }
            }
        }
    }

    // Classify changed nodes; impacted nodes get no change_kind.
    let seed_qns: HashSet<&str> = base
        .changed_nodes
        .iter()
        .map(|n| n.qualified_name.as_str())
        .collect();
    let test_adjacent_qns = test_adjacent_qns(base, test_impact);
    let uncovered_qns: HashSet<String> = test_impact
        .uncovered_changed_nodes
        .iter()
        .map(|n| n.qualified_name.clone())
        .collect();
    let (cross_module_qns, cross_package_qns, cross_repo_qns) = boundary_signal_sets(violations);

    let all_nodes = base.changed_nodes.iter().chain(base.impacted_nodes.iter());

    let mut result: Vec<ScoredImpactNode> = all_nodes
        .map(|n| {
            let mut impact_score = scores
                .get(n.qualified_name.as_str())
                .copied()
                .unwrap_or(0.0);
            if test_adjacent_qns.contains(&n.qualified_name) {
                impact_score += TEST_ADJACENCY_BOOST;
            }
            if uncovered_qns.contains(&n.qualified_name) {
                impact_score += UNCOVERED_CHANGE_BOOST;
            }
            if cross_module_qns.contains(&n.qualified_name) {
                impact_score += CROSS_MODULE_BOOST;
            }
            if cross_package_qns.contains(&n.qualified_name) {
                impact_score += CROSS_PACKAGE_BOOST;
            }
            if cross_repo_qns.contains(&n.qualified_name) {
                impact_score += CROSS_REPO_BOOST;
            }
            let change_kind = if seed_qns.contains(n.qualified_name.as_str()) {
                Some(classify_node(n))
            } else {
                None
            };
            ScoredImpactNode {
                node: n.clone(),
                impact_score,
                change_kind,
            }
        })
        .collect();

    // Highest score first.
    result.sort_by(|a, b| {
        b.impact_score
            .partial_cmp(&a.impact_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.node.qualified_name.cmp(&b.node.qualified_name))
    });
    result
}
