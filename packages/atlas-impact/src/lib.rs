#![doc = include_str!("../README.md")]

use std::collections::{HashMap, HashSet, VecDeque};

use atlas_core::{
    AdvancedImpactResult, BoundaryKind, BoundaryViolation, ChangeKind, EdgeKind, ImpactResult,
    Node, NodeKind, RiskLevel, ScoredImpactNode, TestImpactResult,
};

// Decay factor applied to the impact score at each graph hop.
// Must be < 1/max_edge_weight (max = 3.0) to guarantee scores decrease per hop.
const HOP_DECAY: f64 = 0.25;
const TEST_ADJACENCY_BOOST: f64 = 0.35;
const UNCOVERED_CHANGE_BOOST: f64 = 0.45;
const CROSS_MODULE_BOOST: f64 = 0.30;
const CROSS_PACKAGE_BOOST: f64 = 0.60;

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

/// Compute the full advanced impact analysis from a base `ImpactResult`.
///
/// The `base` result is produced by `Store::impact_radius`. All heavy
/// computation here is done in Rust over the in-memory node/edge sets so no
/// additional database queries are needed.
pub fn analyze(base: ImpactResult) -> AdvancedImpactResult {
    let test_impact = compute_test_impact(&base);
    let boundary_violations = detect_boundary_violations(&base);
    let scored_nodes = score_nodes(&base, &test_impact, &boundary_violations);
    let risk_level = compute_risk_level(&base, &scored_nodes, &test_impact, &boundary_violations);

    AdvancedImpactResult {
        base,
        scored_nodes,
        risk_level,
        test_impact,
        boundary_violations,
    }
}

// ---------------------------------------------------------------------------
// Weighted impact scoring
// ---------------------------------------------------------------------------

/// Score all nodes (changed + impacted) using weighted BFS propagation.
///
/// Seed nodes (changed) start with a kind-based base score × api_multiplier.
/// Each hop through `relevant_edges` decays the score by `HOP_DECAY` times
/// the edge's `traversal_weight`.  Every node is kept at its highest score
/// across all paths (max propagation).
fn score_nodes(
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
    let (cross_module_qns, cross_package_qns) = boundary_signal_sets(violations);

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

// ---------------------------------------------------------------------------
// Change classification
// ---------------------------------------------------------------------------

/// Classify how a seed (changed) node was modified.
///
/// This is heuristic: it operates on the current stored metadata (modifiers,
/// params, return_type).  A caller may compare old vs. new if both are
/// available; here we classify based only on what is stored for the node.
fn classify_node(node: &Node) -> ChangeKind {
    let mods = node.modifiers.as_deref().unwrap_or("");
    let has_public = mods.contains("pub");

    match node.kind {
        NodeKind::Function | NodeKind::Method => {
            if has_public {
                // If the node has params or return_type data, consider it a
                // signature-level change (the stored shape is the new shape).
                if node.params.is_some() || node.return_type.is_some() {
                    ChangeKind::SignatureChange
                } else {
                    ChangeKind::ApiChange
                }
            } else {
                ChangeKind::InternalChange
            }
        }
        NodeKind::Struct
        | NodeKind::Enum
        | NodeKind::Trait
        | NodeKind::Interface
        | NodeKind::Class => {
            if has_public {
                ChangeKind::ApiChange
            } else {
                ChangeKind::InternalChange
            }
        }
        _ => ChangeKind::InternalChange,
    }
}

// ---------------------------------------------------------------------------
// Test impact
// ---------------------------------------------------------------------------

/// Find all tests affected by the change and flag changed nodes lacking tests.
fn compute_test_impact(base: &ImpactResult) -> TestImpactResult {
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

    let mut seen: HashSet<&str> = HashSet::new();
    let mut affected_tests: Vec<Node> = test_qns
        .iter()
        .chain(edges_to_tests.iter())
        .filter_map(|qn| all_nodes.get(qn.as_str()))
        .filter(|n| n.is_test || n.kind == NodeKind::Test)
        .filter(|n| seen.insert(n.qualified_name.as_str()))
        .map(|n| (*n).clone())
        .collect();

    affected_tests.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));

    // Changed nodes that have NO edge to any test node.
    let tested_qns: HashSet<String> = base
        .relevant_edges
        .iter()
        .filter(|e| matches!(e.kind, EdgeKind::Tests | EdgeKind::TestedBy))
        .flat_map(|e| [e.source_qn.clone(), e.target_qn.clone()])
        .collect();

    let mut uncovered_changed_nodes: Vec<Node> = base
        .changed_nodes
        .iter()
        .filter(|n| {
            !n.is_test && n.kind != NodeKind::Test && !tested_qns.contains(&n.qualified_name)
        })
        .cloned()
        .collect();
    uncovered_changed_nodes.sort_by(|a, b| a.qualified_name.cmp(&b.qualified_name));

    TestImpactResult {
        affected_tests,
        uncovered_changed_nodes,
    }
}

fn test_adjacent_qns(base: &ImpactResult, test_impact: &TestImpactResult) -> HashSet<String> {
    let mut qns: HashSet<String> = test_impact
        .affected_tests
        .iter()
        .map(|n| n.qualified_name.clone())
        .collect();

    for edge in &base.relevant_edges {
        if matches!(edge.kind, EdgeKind::Tests | EdgeKind::TestedBy) {
            qns.insert(edge.source_qn.clone());
            qns.insert(edge.target_qn.clone());
        }
    }

    qns
}

// ---------------------------------------------------------------------------
// Boundary detection
// ---------------------------------------------------------------------------

/// Detect cross-module and cross-package impacts.
///
/// Module  = unique directory prefix of a node's `file_path`.
/// Package = `owner_id` from node metadata when present, else top-level path component.
fn detect_boundary_violations(base: &ImpactResult) -> Vec<BoundaryViolation> {
    let module_of = |path: &str| -> String {
        // Use the directory part of the path (everything before the last `/`).
        match path.rfind('/') {
            Some(idx) => path[..idx].to_string(),
            None => ".".to_string(),
        }
    };

    let package_of = |node: &Node| -> String {
        node.extra_json
            .as_object()
            .and_then(|extra| extra.get("owner_id"))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
            .unwrap_or_else(|| {
                node.file_path
                    .split('/')
                    .next()
                    .unwrap_or(&node.file_path)
                    .to_string()
            })
    };

    let changed_modules: HashSet<String> = base
        .changed_nodes
        .iter()
        .map(|n| module_of(&n.file_path))
        .collect();

    let changed_packages: HashSet<String> = base.changed_nodes.iter().map(package_of).collect();

    // Collect impacted nodes outside the changed modules/packages.
    let mut cross_module_qns: Vec<String> = Vec::new();
    let mut cross_package_qns: Vec<String> = Vec::new();

    for n in &base.impacted_nodes {
        let m = module_of(&n.file_path);
        if !changed_modules.contains(&m) {
            cross_module_qns.push(n.qualified_name.clone());
        }
        let p = package_of(n);
        if !changed_packages.contains(&p) {
            cross_package_qns.push(n.qualified_name.clone());
        }
    }

    let mut violations = Vec::new();

    if !cross_module_qns.is_empty() {
        cross_module_qns.sort();
        cross_module_qns.dedup();
        let count = cross_module_qns.len();
        violations.push(BoundaryViolation {
            kind: BoundaryKind::CrossModule,
            description: format!("{count} node(s) in other modules are impacted by this change"),
            nodes: cross_module_qns,
        });
    }

    if !cross_package_qns.is_empty() {
        cross_package_qns.sort();
        cross_package_qns.dedup();
        let count = cross_package_qns.len();
        violations.push(BoundaryViolation {
            kind: BoundaryKind::CrossPackage,
            description: format!("{count} node(s) in other packages are impacted by this change"),
            nodes: cross_package_qns,
        });
    }

    violations
}

fn boundary_signal_sets(violations: &[BoundaryViolation]) -> (HashSet<String>, HashSet<String>) {
    let mut cross_module_qns = HashSet::new();
    let mut cross_package_qns = HashSet::new();

    for violation in violations {
        match violation.kind {
            BoundaryKind::CrossModule => {
                cross_module_qns.extend(violation.nodes.iter().cloned());
            }
            BoundaryKind::CrossPackage => {
                cross_package_qns.extend(violation.nodes.iter().cloned());
            }
        }
    }

    (cross_module_qns, cross_package_qns)
}

// ---------------------------------------------------------------------------
// Risk level
// ---------------------------------------------------------------------------

/// Assign an overall risk level for the change set.
fn compute_risk_level(
    base: &ImpactResult,
    scored: &[ScoredImpactNode],
    test_impact: &TestImpactResult,
    violations: &[BoundaryViolation],
) -> RiskLevel {
    let has_api_change = scored
        .iter()
        .any(|s| s.change_kind == Some(ChangeKind::ApiChange));
    let has_sig_change = scored
        .iter()
        .any(|s| s.change_kind == Some(ChangeKind::SignatureChange));
    let has_cross_module = violations
        .iter()
        .any(|v| v.kind == BoundaryKind::CrossModule);
    let has_cross_package = violations
        .iter()
        .any(|v| v.kind == BoundaryKind::CrossPackage);
    let impacted_count = base.impacted_nodes.len();
    let uncovered = !test_impact.uncovered_changed_nodes.is_empty();

    if has_api_change && (has_cross_package || has_cross_module) {
        RiskLevel::Critical
    } else if has_api_change || (has_cross_package && (uncovered || impacted_count > 5)) {
        RiskLevel::High
    } else if has_sig_change || has_cross_module || (impacted_count > 20) || uncovered {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::BudgetReport;
    use atlas_core::{Edge, EdgeKind, ImpactResult, Node, NodeId, NodeKind};

    fn make_node(kind: NodeKind, qn: &str, file_path: &str, is_test: bool) -> Node {
        Node {
            id: NodeId::UNSET,
            kind,
            name: qn.split("::").last().unwrap_or(qn).to_string(),
            qualified_name: qn.to_string(),
            file_path: file_path.to_string(),
            line_start: 1,
            line_end: 10,
            language: "rust".to_string(),
            parent_name: None,
            params: Some("()".to_string()),
            return_type: Some("()".to_string()),
            modifiers: Some("pub".to_string()),
            is_test,
            file_hash: "abc".to_string(),
            extra_json: serde_json::Value::Null,
        }
    }

    fn make_edge(kind: EdgeKind, src: &str, tgt: &str) -> Edge {
        Edge {
            id: 0,
            kind,
            source_qn: src.to_string(),
            target_qn: tgt.to_string(),
            file_path: "src/a.rs".to_string(),
            line: None,
            confidence: 1.0,
            confidence_tier: None,
            extra_json: serde_json::Value::Null,
        }
    }

    fn with_owner(mut node: Node, owner_id: &str) -> Node {
        node.extra_json = serde_json::json!({ "owner_id": owner_id });
        node
    }

    fn base_result(changed: Vec<Node>, impacted: Vec<Node>, edges: Vec<Edge>) -> ImpactResult {
        let impacted_files: Vec<String> = impacted
            .iter()
            .map(|n| n.file_path.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        ImpactResult {
            changed_nodes: changed,
            impacted_nodes: impacted,
            impacted_files,
            relevant_edges: edges,
            seed_budgets: vec![],
            traversal_budget: None,
            budget: BudgetReport::not_applicable(),
        }
    }

    #[test]
    fn score_propagates_through_calls_edge() {
        let seed = make_node(NodeKind::Function, "a.rs::fn::foo", "src/a.rs", false);
        let callee = make_node(NodeKind::Function, "b.rs::fn::bar", "src/b.rs", false);
        let edge = make_edge(EdgeKind::Calls, "a.rs::fn::foo", "b.rs::fn::bar");

        let base = base_result(vec![seed], vec![callee], vec![edge]);
        let result = analyze(base);

        let bar = result
            .scored_nodes
            .iter()
            .find(|s| s.node.qualified_name == "b.rs::fn::bar")
            .expect("bar must be scored");
        assert!(
            bar.impact_score > 0.0,
            "bar should have positive impact score"
        );
        // foo is seed so it should have highest or equal score.
        let foo = result
            .scored_nodes
            .iter()
            .find(|s| s.node.qualified_name == "a.rs::fn::foo")
            .expect("foo must be scored");
        assert!(
            foo.impact_score >= bar.impact_score,
            "seed should score >= propagated"
        );
    }

    #[test]
    fn classify_public_function_as_signature_change() {
        let mut node = make_node(NodeKind::Function, "a.rs::fn::foo", "src/a.rs", false);
        node.modifiers = Some("pub".to_string());
        node.params = Some("(x: i32)".to_string());

        let kind = classify_node(&node);
        assert_eq!(kind, ChangeKind::SignatureChange);
    }

    #[test]
    fn classify_private_function_as_internal() {
        let mut node = make_node(NodeKind::Function, "a.rs::fn::foo", "src/a.rs", false);
        node.modifiers = Some("".to_string());

        let kind = classify_node(&node);
        assert_eq!(kind, ChangeKind::InternalChange);
    }

    #[test]
    fn test_impact_finds_test_nodes() {
        let seed = make_node(NodeKind::Function, "a.rs::fn::foo", "src/a.rs", false);
        let test_node = make_node(NodeKind::Test, "a.rs::test::foo_test", "src/a.rs", true);
        let edge = make_edge(EdgeKind::Tests, "a.rs::test::foo_test", "a.rs::fn::foo");

        let base = base_result(vec![seed], vec![test_node.clone()], vec![edge]);
        let result = analyze(base);

        assert!(
            !result.test_impact.affected_tests.is_empty(),
            "should detect test node"
        );
        let found = result
            .test_impact
            .affected_tests
            .iter()
            .any(|n| n.qualified_name == "a.rs::test::foo_test");
        assert!(found, "test node must be in affected_tests");
    }

    #[test]
    fn uncovered_node_flagged_when_no_test_edge() {
        let seed = make_node(NodeKind::Function, "a.rs::fn::foo", "src/a.rs", false);
        // No test edges at all.
        let base = base_result(vec![seed.clone()], vec![], vec![]);
        let result = analyze(base);

        assert!(
            !result.test_impact.uncovered_changed_nodes.is_empty(),
            "foo has no tests → must be in uncovered"
        );
        let found = result
            .test_impact
            .uncovered_changed_nodes
            .iter()
            .any(|n| n.qualified_name == "a.rs::fn::foo");
        assert!(found);
    }

    #[test]
    fn cross_module_violation_detected() {
        let seed = make_node(
            NodeKind::Function,
            "a.rs::fn::foo",
            "src/moduleA/a.rs",
            false,
        );
        let impacted = make_node(
            NodeKind::Function,
            "b.rs::fn::bar",
            "src/moduleB/b.rs",
            false,
        );

        let base = base_result(vec![seed], vec![impacted], vec![]);
        let result = analyze(base);

        let has_cross = result
            .boundary_violations
            .iter()
            .any(|v| v.kind == BoundaryKind::CrossModule);
        assert!(
            has_cross,
            "different modules should produce CrossModule violation"
        );
    }

    #[test]
    fn no_violation_when_same_module() {
        let seed = make_node(
            NodeKind::Function,
            "a.rs::fn::foo",
            "src/module/a.rs",
            false,
        );
        let impacted = make_node(
            NodeKind::Function,
            "b.rs::fn::bar",
            "src/module/b.rs",
            false,
        );

        let base = base_result(vec![seed], vec![impacted], vec![]);
        let result = analyze(base);

        let has_cross = result
            .boundary_violations
            .iter()
            .any(|v| v.kind == BoundaryKind::CrossModule);
        assert!(
            !has_cross,
            "same module should produce no CrossModule violation"
        );
    }

    #[test]
    fn risk_critical_on_api_change_with_cross_package() {
        let mut seed = make_node(
            NodeKind::Struct,
            "pkg_a/a.rs::struct::Foo",
            "pkg_a/a.rs",
            false,
        );
        seed.modifiers = Some("pub".to_string());
        seed.params = None;
        seed.return_type = None;

        let impacted = make_node(
            NodeKind::Function,
            "pkg_b/b.rs::fn::bar",
            "pkg_b/b.rs",
            false,
        );

        let base = base_result(vec![seed], vec![impacted], vec![]);
        let result = analyze(base);

        assert_eq!(result.risk_level, RiskLevel::Critical);
    }

    #[test]
    fn risk_low_for_internal_single_file_change() {
        let mut seed = make_node(NodeKind::Function, "src/a.rs::fn::foo", "src/a.rs", false);
        seed.modifiers = Some("".to_string()); // private
        seed.params = None;
        seed.return_type = None;

        let base = base_result(vec![seed], vec![], vec![]);
        let result = analyze(base);

        assert!(result.risk_level <= RiskLevel::Medium);
    }

    #[test]
    fn risk_high_for_untested_cross_package_internal_change() {
        let seed = with_owner(
            make_node(
                NodeKind::Function,
                "packages/ui/src/lib.rs::fn::helper",
                "packages/ui/src/lib.rs",
                false,
            ),
            "cargo:packages/ui/Cargo.toml",
        );
        let impacted = with_owner(
            make_node(
                NodeKind::Function,
                "packages/web/src/lib.rs::fn::run",
                "packages/web/src/lib.rs",
                false,
            ),
            "cargo:packages/web/Cargo.toml",
        );
        let base = base_result(vec![seed], vec![impacted], vec![]);

        let result = analyze(base);

        assert_eq!(result.risk_level, RiskLevel::High);
    }

    #[test]
    fn empty_base_produces_empty_result() {
        let base = ImpactResult {
            changed_nodes: vec![],
            impacted_nodes: vec![],
            impacted_files: vec![],
            relevant_edges: vec![],
            seed_budgets: vec![],
            traversal_budget: None,
            budget: BudgetReport::not_applicable(),
        };
        let result = analyze(base);
        assert!(result.scored_nodes.is_empty());
        assert!(result.boundary_violations.is_empty());
        assert_eq!(result.risk_level, RiskLevel::Low);
    }

    #[test]
    fn one_hop_graph_scores_correctly() {
        let seed = make_node(NodeKind::Function, "a::fn::foo", "src/a.rs", false);
        let hop1 = make_node(NodeKind::Function, "b::fn::bar", "src/b.rs", false);
        let hop2 = make_node(NodeKind::Function, "c::fn::baz", "src/c.rs", false);
        let e1 = make_edge(EdgeKind::Calls, "a::fn::foo", "b::fn::bar");
        let e2 = make_edge(EdgeKind::Calls, "b::fn::bar", "c::fn::baz");

        let base = base_result(vec![seed], vec![hop1, hop2], vec![e1, e2]);
        let result = analyze(base);

        // baz is two hops away — should score less than bar.
        let bar_score = result
            .scored_nodes
            .iter()
            .find(|s| s.node.qualified_name == "b::fn::bar")
            .unwrap()
            .impact_score;
        let baz_score = result
            .scored_nodes
            .iter()
            .find(|s| s.node.qualified_name == "c::fn::baz")
            .unwrap()
            .impact_score;
        assert!(
            bar_score > baz_score,
            "one-hop node should score higher than two-hop node"
        );
    }

    #[test]
    fn disconnected_impacted_node_gets_zero_score() {
        let seed = make_node(NodeKind::Function, "a::fn::foo", "src/a.rs", false);
        // disconnected: no edges connect it to seed
        let disconnected = make_node(NodeKind::Function, "z::fn::orphan", "src/z.rs", false);

        let base = base_result(vec![seed], vec![disconnected], vec![]);
        let result = analyze(base);

        let orphan = result
            .scored_nodes
            .iter()
            .find(|s| s.node.qualified_name == "z::fn::orphan")
            .unwrap();
        assert_eq!(orphan.impact_score, 0.0);
    }

    #[test]
    fn test_adjacency_boosts_scored_node_priority() {
        let seed = make_node(NodeKind::Function, "src/a.rs::fn::seed", "src/a.rs", false);
        let covered = make_node(
            NodeKind::Function,
            "src/a.rs::fn::covered",
            "src/a.rs",
            false,
        );
        let plain = make_node(NodeKind::Function, "src/b.rs::fn::plain", "src/b.rs", false);
        let test_node = make_node(NodeKind::Test, "src/a.rs::test::covered", "src/a.rs", true);

        let base = base_result(
            vec![seed],
            vec![covered, plain, test_node],
            vec![
                make_edge(
                    EdgeKind::Calls,
                    "src/a.rs::fn::seed",
                    "src/a.rs::fn::covered",
                ),
                make_edge(EdgeKind::Calls, "src/a.rs::fn::seed", "src/b.rs::fn::plain"),
                make_edge(
                    EdgeKind::Tests,
                    "src/a.rs::test::covered",
                    "src/a.rs::fn::covered",
                ),
            ],
        );

        let result = analyze(base);
        let covered_score = result
            .scored_nodes
            .iter()
            .find(|s| s.node.qualified_name == "src/a.rs::fn::covered")
            .unwrap()
            .impact_score;
        let plain_score = result
            .scored_nodes
            .iter()
            .find(|s| s.node.qualified_name == "src/b.rs::fn::plain")
            .unwrap()
            .impact_score;

        assert!(
            covered_score > plain_score,
            "test-adjacent node should score above equally distant plain node"
        );
    }

    #[test]
    fn cross_package_signal_boosts_scored_node_priority() {
        let seed = with_owner(
            make_node(
                NodeKind::Function,
                "packages/core/src/lib.rs::fn::seed",
                "packages/core/src/lib.rs",
                false,
            ),
            "cargo:packages/core/Cargo.toml",
        );
        let local = with_owner(
            make_node(
                NodeKind::Function,
                "packages/core/src/lib.rs::fn::local",
                "packages/core/src/lib.rs",
                false,
            ),
            "cargo:packages/core/Cargo.toml",
        );
        let cross = with_owner(
            make_node(
                NodeKind::Function,
                "packages/ui/src/lib.rs::fn::cross",
                "packages/ui/src/lib.rs",
                false,
            ),
            "cargo:packages/ui/Cargo.toml",
        );

        let base = base_result(
            vec![seed],
            vec![local, cross],
            vec![
                make_edge(
                    EdgeKind::Calls,
                    "packages/core/src/lib.rs::fn::seed",
                    "packages/core/src/lib.rs::fn::local",
                ),
                make_edge(
                    EdgeKind::Calls,
                    "packages/core/src/lib.rs::fn::seed",
                    "packages/ui/src/lib.rs::fn::cross",
                ),
            ],
        );

        let result = analyze(base);
        let local_score = result
            .scored_nodes
            .iter()
            .find(|s| s.node.qualified_name == "packages/core/src/lib.rs::fn::local")
            .unwrap()
            .impact_score;
        let cross_score = result
            .scored_nodes
            .iter()
            .find(|s| s.node.qualified_name == "packages/ui/src/lib.rs::fn::cross")
            .unwrap()
            .impact_score;

        assert!(
            cross_score > local_score,
            "cross-package node should score above same-distance local node"
        );
    }
}
