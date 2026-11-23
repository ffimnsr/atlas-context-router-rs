use super::*;

// ---------------------------------------------------------------------------
// Test impact
// ---------------------------------------------------------------------------

/// Find all tests affected by the change and flag changed nodes lacking tests.
pub(crate) fn compute_test_impact(base: &ImpactResult) -> TestImpactResult {
    // Collect qualified names of all test nodes reachable via relevant_edges.
    let test_qns: HashSet<String> = base
        .changed_nodes
        .iter()
        .chain(base.impacted_nodes.iter())
        .filter(|n| n.is_test || n.kind == NodeKind::Test)
        .map(|n| n.qualified_name.clone())
        .collect();

    // Also pull in nodes connected to seeds via Tests/TestedBy edges.
    let edges_to_tests: HashSet<String> = base
        .relevant_edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Tests | EdgeKind::TestedBy))
        .flat_map(|e| [e.source_qn.clone(), e.target_qn.clone()])
        .collect();

    // Build a lookup of all nodes.
    let all_nodes: HashMap<&str, &Node> = base
        .changed_nodes
        .iter()
        .chain(base.impacted_nodes.iter())
        .map(|n| (n.qualified_name.as_str(), n))
        .collect();

    // Call sites inside test functions count as test adjacency. Rust in-file
    // `#[cfg(test)]` modules emit `calls` edges but no explicit `tests` edge, so
    // without this fallback every symbol touched only by an inline test would be
    // reported as uncovered.
    let test_node_qns: HashSet<&str> = all_nodes
        .values()
        .filter(|n| n.is_test || n.kind == NodeKind::Test)
        .map(|n| n.qualified_name.as_str())
        .collect();
    let tested_by_call_sites: HashSet<String> = base
        .relevant_edges
        .iter()
        .filter(|e| e.kind == EdgeKind::Calls && test_node_qns.contains(e.source_qn.as_str()))
        .map(|e| e.target_qn.clone())
        .collect();

    let mut seen: HashSet<&str> = HashSet::new();
    let mut affected_tests: Vec<Node> = test_qns
        .iter()
        .chain(edges_to_tests.iter())
        .chain(tested_by_call_sites.iter())
        .filter_map(|qn| all_nodes.get(qn.as_str()))
        .filter(|n| n.is_test || n.kind == NodeKind::Test)
        .filter(|n| seen.insert(n.qualified_name.as_str()))
        .map(|n| (*n).clone())
        .collect();

    affected_tests.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));

    // Changed nodes that have NO edge to any test node.
    let mut tested_qns: HashSet<String> = base
        .relevant_edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Tests | EdgeKind::TestedBy))
        .flat_map(|e| [e.source_qn.clone(), e.target_qn.clone()])
        .collect();
    tested_qns.extend(tested_by_call_sites);

    let mut uncovered_changed_nodes: Vec<Node> = base
        .changed_nodes
        .iter()
        // Only callable/type symbols can carry test coverage. Container nodes
        // (file, package, module/heading, import) would otherwise report every
        // docs-only delta as an uncovered code change and escalate risk.
        .filter(|n| is_testable_change_kind(n.kind))
        .filter(|n| !n.is_test && !tested_qns.contains(&n.qualified_name))
        .cloned()
        .collect();
    uncovered_changed_nodes.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));

    TestImpactResult {
        affected_tests,
        uncovered_changed_nodes,
    }
}

/// Node kinds whose changes can be covered by tests.
fn is_testable_change_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Class
            | NodeKind::Interface
            | NodeKind::Struct
            | NodeKind::Enum
            | NodeKind::Function
            | NodeKind::Method
            | NodeKind::Trait
    )
}
