//! `ReasoningEngine` — Phase 23 implementation.
//!
//! All methods operate on graph data from the Store; no external inference.
//! Results are deterministic given the same graph state.

use std::collections::{HashMap, HashSet, VecDeque};

use atlas_core::{
    ChangeRiskResult, ConfidenceTier, CoverageStrength, DeadCodeCandidate, DependencyRemovalResult,
    Edge, EdgeKind, ImpactClass, ImpactedNode, Node, NodeKind, ReasoningEvidence, ReasoningWarning,
    RefactorSafetyResult, ReferenceScope, RemovalImpactResult, RenamePreviewResult,
    RenameReference, Result, RiskLevel, SafetyBand, SafetyScore, TestAdjacencyResult,
};
use atlas_store_sqlite::Store;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default traversal depth for removal impact.
const DEFAULT_IMPACT_DEPTH: u32 = 3;
/// Default node cap for impact traversal.
const DEFAULT_IMPACT_NODES: usize = 200;
/// Edge query cap for per-node lookups.
const EDGE_QUERY_LIMIT: usize = 500;

// ---------------------------------------------------------------------------
// Internal helper types
// ---------------------------------------------------------------------------

type ImpactNodeInfo = (Node, u32, Option<EdgeKind>);
type BfsImpact = (Vec<ImpactNodeInfo>, Vec<Edge>);

// ---------------------------------------------------------------------------
// Allowlist suppressions for dead-code detection
// ---------------------------------------------------------------------------

/// Simple-name patterns that are always suppressed as entrypoints even when
/// they have no inbound edges in the graph.
const ENTRYPOINT_NAMES: &[&str] = &[
    "main", "new", "init", "setup", "configure", "run", "start", "handler", "middleware",
];

// ---------------------------------------------------------------------------
// ReasoningEngine
// ---------------------------------------------------------------------------

/// Provides autonomous code-reasoning queries backed by the Atlas graph store.
///
/// Wrap a `Store` reference; all methods borrow it immutably.
pub struct ReasoningEngine<'s> {
    store: &'s Store,
}

impl<'s> ReasoningEngine<'s> {
    /// Create a new engine from a shared store reference.
    pub fn new(store: &'s Store) -> Self {
        Self { store }
    }

    // -----------------------------------------------------------------------
    // 23.2 — Removal impact analysis
    // -----------------------------------------------------------------------

    /// Analyse the impact of removing `seed_qnames` from the codebase.
    ///
    /// Traverses the graph bidirectionally up to `max_depth` hops (default 3)
    /// and separates impacted nodes by confidence: `Definite` (direct call /
    /// import / test), `Probable` (same-file/package inferred), `Weak` (textual
    /// reference only).
    pub fn analyze_removal(
        &self,
        seed_qnames: &[&str],
        max_depth: Option<u32>,
        max_nodes: Option<usize>,
    ) -> Result<RemovalImpactResult> {
        let depth = max_depth.unwrap_or(DEFAULT_IMPACT_DEPTH);
        let cap = max_nodes.unwrap_or(DEFAULT_IMPACT_NODES);

        // Load seed nodes.
        let seed_nodes = self.load_nodes(seed_qnames)?;
        if seed_nodes.is_empty() {
            return Ok(RemovalImpactResult {
                seed: vec![],
                impacted_symbols: vec![],
                impacted_files: vec![],
                impacted_tests: vec![],
                relevant_edges: vec![],
                evidence_nodes: vec![],
                warnings: vec![ReasoningWarning {
                    message: format!(
                        "none of {} seed qualified names resolved to graph nodes",
                        seed_qnames.len()
                    ),
                    confidence: ConfidenceTier::High,
                }],
                evidence: vec![],
                uncertainty_flags: vec![
                    "seed qualified names not found in graph — run `atlas build` first".to_owned(),
                ],
            });
        }

        // BFS from seeds through inbound & outbound edges up to `depth`.
        let (impacted, relevant_edges) =
            self.bfs_impact(seed_qnames, depth, cap)?;

        let seed_set: HashSet<&str> = seed_qnames.iter().copied().collect();

        // Classify each reachable node.
        let impacted_symbols: Vec<ImpactedNode> = impacted
            .into_iter()
            .filter(|(n, _, _)| !seed_set.contains(n.qualified_name.as_str()))
            .map(|(node, depth_val, edge_kind)| {
                let impact_class = classify_impact(&node, depth_val, edge_kind);
                ImpactedNode {
                    node,
                    depth: depth_val,
                    impact_class,
                    via_edge_kind: edge_kind,
                }
            })
            .collect();

        let impacted_tests: Vec<Node> = impacted_symbols
            .iter()
            .filter(|im| im.node.is_test || im.node.kind == NodeKind::Test)
            .map(|im| im.node.clone())
            .collect();

        let mut impacted_files: Vec<String> = impacted_symbols
            .iter()
            .map(|im| im.node.file_path.clone())
            .collect();
        impacted_files.sort();
        impacted_files.dedup();

        let evidence = vec![
            ReasoningEvidence {
                key: "seed_count".to_owned(),
                value: seed_nodes.len().to_string(),
            },
            ReasoningEvidence {
                key: "impacted_symbol_count".to_owned(),
                value: impacted_symbols.len().to_string(),
            },
            ReasoningEvidence {
                key: "max_depth".to_owned(),
                value: depth.to_string(),
            },
        ];

        Ok(RemovalImpactResult {
            seed: seed_nodes,
            impacted_symbols,
            impacted_files,
            impacted_tests,
            relevant_edges,
            evidence_nodes: vec![],
            warnings: vec![],
            evidence,
            uncertainty_flags: vec![],
        })
    }

    // -----------------------------------------------------------------------
    // 23.3 — Dead code detection
    // -----------------------------------------------------------------------

    /// Detect dead-code candidates: nodes with no inbound semantic edges, not
    /// public/exported, not a test, not in the entrypoint allowlist.
    ///
    /// The store pre-filters on visibility and edge absence; this method
    /// applies the remaining suppression logic plus certainty assignment.
    pub fn detect_dead_code(
        &self,
        extra_allowlist: &[&str],
        limit: Option<usize>,
    ) -> Result<Vec<DeadCodeCandidate>> {
        let cap = limit.unwrap_or(500);
        let raw = self.store.dead_code_candidates(cap)?;

        let allowlist_set: HashSet<&str> = extra_allowlist.iter().copied().collect();

        let candidates = raw
            .into_iter()
            .filter_map(|node| {
                // Suppress entrypoints by simple name.
                if ENTRYPOINT_NAMES.contains(&node.name.as_str()) {
                    return None;
                }
                // Suppress caller-provided allowlist.
                if allowlist_set.contains(node.qualified_name.as_str()) {
                    return None;
                }

                let (reasons, certainty, blockers) = dead_code_reasons(&node);
                Some(DeadCodeCandidate { node, reasons, certainty, blockers })
            })
            .collect();

        Ok(candidates)
    }

    // -----------------------------------------------------------------------
    // 23.3 — Refactor safety scoring
    // -----------------------------------------------------------------------

    /// Score how safe it is to refactor `qname`.
    ///
    /// Factors: fan-in, fan-out, visibility (public API), test adjacency,
    /// self-containment, unresolved edges.
    pub fn score_refactor_safety(&self, qname: &str) -> Result<RefactorSafetyResult> {
        let node = match self.store.node_by_qname(qname)? {
            Some(n) => n,
            None => {
                return Err(atlas_core::AtlasError::Other(format!(
                    "node not found: {qname}"
                )));
            }
        };

        let inbound = self.store.inbound_edges(qname, EDGE_QUERY_LIMIT)?;
        let outbound = self.store.outbound_edges(qname, EDGE_QUERY_LIMIT)?;
        let tests = self.store.test_neighbors(qname, EDGE_QUERY_LIMIT)?;

        let fan_in = inbound.len();
        let fan_out = outbound.len();
        let linked_test_count = tests.len();

        let is_public = is_public_node(&node);
        let cross_module_callers = inbound
            .iter()
            .filter(|(n, _)| n.file_path != node.file_path)
            .count();

        let unresolved_edge_count = inbound
            .iter()
            .chain(outbound.iter())
            .filter(|(_, e)| {
                e.confidence_tier.as_deref().unwrap_or("") == "low"
                    || e.confidence < 0.5
            })
            .count();

        let (score, band, reasons, suggested_validations) =
            compute_safety_score(&node, fan_in, fan_out, linked_test_count, is_public,
                                 cross_module_callers, unresolved_edge_count);

        let evidence = vec![
            ReasoningEvidence { key: "fan_in".to_owned(), value: fan_in.to_string() },
            ReasoningEvidence { key: "fan_out".to_owned(), value: fan_out.to_string() },
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
            safety: SafetyScore { score, band, reasons, suggested_validations },
            fan_in,
            fan_out,
            linked_test_count,
            unresolved_edge_count,
            evidence,
        })
    }

    // -----------------------------------------------------------------------
    // 23.3 — Dependency removal validation
    // -----------------------------------------------------------------------

    /// Check whether removing `qname` is safe (no remaining references).
    ///
    /// Verifies zero references in graph. Flags dynamic/reflective uncertainty
    /// for low-confidence inbound edges.
    pub fn check_dependency_removal(&self, qname: &str) -> Result<DependencyRemovalResult> {
        let inbound = self.store.inbound_edges(qname, EDGE_QUERY_LIMIT)?;

        // Filter to semantic reference edges only (not test/contains hierarchy).
        let blocking: Vec<Node> = inbound
            .iter()
            .filter(|(_, e)| {
                matches!(
                    e.kind,
                    EdgeKind::Calls
                        | EdgeKind::Imports
                        | EdgeKind::References
                        | EdgeKind::Extends
                        | EdgeKind::Implements
                )
            })
            .map(|(n, _)| n.clone())
            .collect();

        let has_low_confidence = inbound.iter().any(|(_, e)| e.confidence < 0.5);

        let confidence = if blocking.is_empty() && !has_low_confidence {
            ConfidenceTier::High
        } else if has_low_confidence {
            ConfidenceTier::Medium
        } else {
            ConfidenceTier::Low
        };

        let removable = blocking.is_empty();

        let mut suggested_cleanups: Vec<String> = blocking
            .iter()
            .take(5)
            .map(|n| format!("remove reference in `{}`", n.file_path))
            .collect();
        if has_low_confidence && removable {
            suggested_cleanups.push(
                "verify no dynamic/reflective usage before removing".to_owned(),
            );
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

        // Uncertainty flag when low-confidence edges are present.
        let mut uncertainty_flags: Vec<String> = Vec::new();
        if has_low_confidence {
            uncertainty_flags.push(
                "low-confidence edges present; dynamic or reflective usage cannot be excluded"
                    .to_owned(),
            );
        }

        Ok(DependencyRemovalResult {
            target_qname: qname.to_owned(),
            removable,
            blocking_references: blocking,
            evidence_edges: inbound.into_iter().map(|(_, e)| e).collect(),
            confidence,
            suggested_cleanups,
            evidence,
            uncertainty_flags,
        })
    }

    // -----------------------------------------------------------------------
    // 23.4 — Rename blast radius
    // -----------------------------------------------------------------------

    /// Preview all references that would need updating when renaming `qname`
    /// to `new_name`.
    pub fn preview_rename_radius(
        &self,
        qname: &str,
        new_name: &str,
    ) -> Result<RenamePreviewResult> {
        let target = match self.store.node_by_qname(qname)? {
            Some(n) => n,
            None => {
                return Err(atlas_core::AtlasError::Other(format!(
                    "node not found: {qname}"
                )));
            }
        };

        let inbound = self.store.inbound_edges(qname, EDGE_QUERY_LIMIT)?;

        let mut affected_references: Vec<RenameReference> = Vec::new();
        let mut affected_files: HashSet<String> = HashSet::new();
        let mut manual_review_flags: Vec<String> = Vec::new();

        for (ref_node, edge) in inbound {
            if ref_node.is_test || ref_node.kind == NodeKind::Test {
                let scope = ReferenceScope::Test;
                affected_files.insert(ref_node.file_path.clone());
                affected_references.push(RenameReference { node: ref_node, edge, scope });
                continue;
            }

            let scope = if ref_node.file_path == target.file_path {
                ReferenceScope::SameFile
            } else if same_module(&ref_node, &target) {
                ReferenceScope::SameModule
            } else {
                ReferenceScope::CrossModule
            };

            // Low-confidence edges need manual review.
            if edge.confidence < 0.5 {
                manual_review_flags.push(format!(
                    "unresolved reference in `{}` — verify manually",
                    ref_node.file_path
                ));
            }

            affected_files.insert(ref_node.file_path.clone());
            affected_references.push(RenameReference { node: ref_node, edge, scope });
        }

        // Collision check: does anything else share the new simple name in the
        // same module/file? (best-effort name uniqueness check)
        let collision_warnings =
            self.detect_name_collisions(new_name, &target)?;

        let risk_level = rename_risk(
            &target,
            affected_references.len(),
            manual_review_flags.len(),
            !collision_warnings.is_empty(),
        );

        let mut affected_file_list: Vec<String> = affected_files.into_iter().collect();
        affected_file_list.sort();

        Ok(RenamePreviewResult {
            target,
            new_name: new_name.to_owned(),
            affected_references,
            affected_files: affected_file_list,
            risk_level,
            collision_warnings,
            manual_review_flags,
        })
    }

    // -----------------------------------------------------------------------
    // 23.4 — Test adjacency
    // -----------------------------------------------------------------------

    /// Estimate test coverage adjacency for `qname`.
    pub fn find_test_adjacency(&self, qname: &str) -> Result<TestAdjacencyResult> {
        let symbol = match self.store.node_by_qname(qname)? {
            Some(n) => n,
            None => {
                return Err(atlas_core::AtlasError::Other(format!(
                    "node not found: {qname}"
                )));
            }
        };

        let test_pairs = self.store.test_neighbors(qname, EDGE_QUERY_LIMIT)?;
        let mut linked_tests: Vec<Node> =
            test_pairs.into_iter().map(|(n, _)| n).collect();

        // If no direct test edge, look for same-file test nodes.
        let coverage_strength = if !linked_tests.is_empty() {
            CoverageStrength::Direct
        } else {
            // Same-file test scan.
            let file_nodes = self.store.nodes_by_file(&symbol.file_path)?;
            let file_tests: Vec<Node> = file_nodes
                .into_iter()
                .filter(|n| n.is_test || n.kind == NodeKind::Test)
                .collect();

            if !file_tests.is_empty() {
                linked_tests = file_tests;
                CoverageStrength::SameFile
            } else {
                CoverageStrength::None
            }
        };

        let recommendation = match coverage_strength {
            CoverageStrength::None => Some(
                "no tests found for this symbol — consider adding a dedicated test".to_owned(),
            ),
            CoverageStrength::SameFile => Some(
                "tests are co-located in the same file but not directly linked via edge — \
                 verify coverage"
                    .to_owned(),
            ),
            _ => None,
        };

        Ok(TestAdjacencyResult { symbol, linked_tests, coverage_strength, recommendation })
    }

    // -----------------------------------------------------------------------
    // 23.4 — Change risk classification
    // -----------------------------------------------------------------------

    /// Classify the risk of changing `qname` by aggregating graph factors.
    pub fn classify_change_risk(&self, qname: &str) -> Result<ChangeRiskResult> {
        let node = match self.store.node_by_qname(qname)? {
            Some(n) => n,
            None => {
                return Err(atlas_core::AtlasError::Other(format!(
                    "node not found: {qname}"
                )));
            }
        };

        let inbound = self.store.inbound_edges(qname, EDGE_QUERY_LIMIT)?;
        let outbound = self.store.outbound_edges(qname, EDGE_QUERY_LIMIT)?;
        let tests = self.store.test_neighbors(qname, EDGE_QUERY_LIMIT)?;

        let is_public = is_public_node(&node);
        let fan_in = inbound.len();
        let fan_out = outbound.len();
        let test_adj = !tests.is_empty();

        let cross_module = inbound.iter().any(|(n, _)| n.file_path != node.file_path);
        let cross_package =
            inbound.iter().any(|(n, _)| different_package(&n.file_path, &node.file_path));

        let unresolved = inbound
            .iter()
            .chain(outbound.iter())
            .filter(|(_, e)| e.confidence < 0.5)
            .count();

        let impacted_files: HashSet<&str> = inbound
            .iter()
            .chain(outbound.iter())
            .map(|(n, _)| n.file_path.as_str())
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

        Ok(ChangeRiskResult { risk_level, contributing_factors: factors, suggested_review_focus })
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    /// Load nodes for the given qualified names. Skips any QN that does not
    /// resolve to a node (caller sees them missing from `seed`).
    fn load_nodes(&self, qnames: &[&str]) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();
        for qn in qnames {
            if let Some(n) = self.store.node_by_qname(qn)? {
                nodes.push(n);
            }
        }
        Ok(nodes)
    }

    /// Breadth-first traversal seeded from `seed_qnames`, bidirectional,
    /// up to `max_depth` hops, capped at `max_nodes` total.
    ///
    /// Returns `(reachable_nodes_with_depth_and_edge_kind, relevant_edges)`.
    fn bfs_impact(
        &self,
        seed_qnames: &[&str],
        max_depth: u32,
        max_nodes: usize,
    ) -> Result<BfsImpact> {
        let mut visited: HashMap<String, (u32, Option<EdgeKind>)> = HashMap::new();
        for qn in seed_qnames {
            visited.insert(qn.to_string(), (0, None));
        }

        let mut queue: VecDeque<(String, u32)> =
            seed_qnames.iter().map(|qn| (qn.to_string(), 0)).collect();

        let mut all_edges: Vec<Edge> = Vec::new();

        while let Some((qn, depth)) = queue.pop_front() {
            if depth >= max_depth || visited.len() >= max_nodes {
                break;
            }

            let inbound = self.store.inbound_edges(&qn, EDGE_QUERY_LIMIT)?;
            let outbound = self.store.outbound_edges(&qn, EDGE_QUERY_LIMIT)?;

            for (neighbor, edge) in inbound.iter().chain(outbound.iter()) {
                let nqn = &neighbor.qualified_name;
                if !visited.contains_key(nqn) {
                    visited.insert(nqn.clone(), (depth + 1, Some(edge.kind)));
                    queue.push_back((nqn.clone(), depth + 1));
                }
                all_edges.push(edge.clone());
            }
        }

        // Dedup edges by id.
        all_edges.sort_by_key(|e| e.id);
        all_edges.dedup_by_key(|e| e.id);

        // Load nodes for all visited QNs.
        let mut results = Vec::new();
        for (qn, (depth, ek)) in &visited {
            if let Some(node) = self.store.node_by_qname(qn)? {
                results.push((node, *depth, *ek));
            }
        }
        results.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.qualified_name.cmp(&b.0.qualified_name)));

        Ok((results, all_edges))
    }

    /// Check whether any node in the same file has `name == new_name` and
    /// could collide with the rename target.
    fn detect_name_collisions(&self, new_name: &str, target: &Node) -> Result<Vec<String>> {
        let file_nodes = self.store.nodes_by_file(&target.file_path)?;
        let warnings: Vec<String> = file_nodes
            .iter()
            .filter(|n| n.name == new_name && n.qualified_name != target.qualified_name)
            .map(|n| {
                format!(
                    "name `{new_name}` already exists in `{}` as `{}`",
                    target.file_path, n.qualified_name
                )
            })
            .collect();
        Ok(warnings)
    }
}

// ---------------------------------------------------------------------------
// Pure scoring/classification helpers (no I/O)
// ---------------------------------------------------------------------------

struct RiskInputs {
    fan_in: usize,
    fan_out: usize,
    is_public: bool,
    test_adj: bool,
    cross_module: bool,
    cross_package: bool,
    unresolved: usize,
    impacted_file_count: usize,
}

fn classify_impact(_node: &Node, depth: u32, edge_kind: Option<EdgeKind>) -> ImpactClass {
    match edge_kind {
        Some(EdgeKind::Calls | EdgeKind::Imports | EdgeKind::Tests | EdgeKind::TestedBy) => {
            ImpactClass::Definite
        }
        Some(EdgeKind::Implements | EdgeKind::Extends) => ImpactClass::Definite,
        Some(EdgeKind::References) if depth <= 1 => ImpactClass::Definite,
        Some(EdgeKind::References) => ImpactClass::Probable,
        Some(EdgeKind::Contains | EdgeKind::Defines) => ImpactClass::Probable,
        None if depth == 0 => ImpactClass::Definite,
        _ => ImpactClass::Weak,
    }
}

fn is_public_node(node: &Node) -> bool {
    let mods = node.modifiers.as_deref().unwrap_or("");
    mods.contains("pub") || mods.contains("export") || mods.contains("public")
}

fn same_module(a: &Node, b: &Node) -> bool {
    // Same parent directory = same module heuristic.
    let a_dir = parent_dir(&a.file_path);
    let b_dir = parent_dir(&b.file_path);
    a_dir == b_dir
}

fn parent_dir(path: &str) -> &str {
    match path.rfind('/') {
        Some(idx) => &path[..idx],
        None => "",
    }
}

fn different_package(a: &str, b: &str) -> bool {
    // Heuristic: top-level directory differs → different package.
    let a_top = a.split('/').next().unwrap_or("");
    let b_top = b.split('/').next().unwrap_or("");
    a_top != b_top && !a_top.is_empty() && !b_top.is_empty()
}

fn dead_code_reasons(node: &Node) -> (Vec<String>, ConfidenceTier, Vec<String>) {
    let mut reasons = vec!["no inbound call, reference, or import edges".to_owned()];
    let mut blockers: Vec<String> = vec![];

    if matches!(
        node.kind,
        NodeKind::Function | NodeKind::Method | NodeKind::Constant
    ) {
        reasons.push(format!("{} with zero callers", node.kind.as_str()));
    }

    // Lower certainty for constants and variables (may be used via reflection).
    let certainty = match node.kind {
        NodeKind::Constant | NodeKind::Variable => {
            blockers.push("may be used via reflection or dynamic dispatch".to_owned());
            ConfidenceTier::Medium
        }
        NodeKind::Class | NodeKind::Struct | NodeKind::Enum | NodeKind::Trait
        | NodeKind::Interface => {
            blockers.push(
                "may be instantiated via reflection, macro, or config".to_owned(),
            );
            ConfidenceTier::Medium
        }
        _ => ConfidenceTier::High,
    };

    (reasons, certainty, blockers)
}

fn compute_safety_score(
    _node: &Node,
    fan_in: usize,
    fan_out: usize,
    linked_tests: usize,
    is_public: bool,
    cross_module_callers: usize,
    unresolved: usize,
) -> (f64, SafetyBand, Vec<String>, Vec<String>) {
    let mut score: f64 = 1.0;
    let mut reasons: Vec<String> = vec![];
    let mut validations: Vec<String> = vec![];

    // Public API penalty.
    if is_public {
        score -= 0.25;
        reasons.push("public/exported API — breaking change risk".to_owned());
        validations.push("run all integration tests after refactor".to_owned());
    }

    // Fan-in penalty.
    let fi_penalty = (fan_in as f64 * 0.04).min(0.3);
    if fan_in > 5 {
        score -= fi_penalty;
        reasons.push(format!("high fan-in: {fan_in} inbound references"));
        validations.push("search all call sites before changing signature".to_owned());
    } else if fan_in > 0 {
        score -= fi_penalty;
    }

    // Cross-module callers add extra risk.
    if cross_module_callers > 0 {
        let penalty = (cross_module_callers as f64 * 0.05).min(0.25);
        score -= penalty;
        reasons.push(format!("{cross_module_callers} cross-module callers"));
        validations.push("verify cross-module consumers compile after change".to_owned());
    }

    // Fan-out penalty (dependency depth).
    if fan_out > 10 {
        score -= 0.1;
        reasons.push(format!("high fan-out: {fan_out} outbound references"));
    }

    // Test coverage bonus.
    if linked_tests == 0 {
        score -= 0.15;
        reasons.push("no linked tests".to_owned());
        validations.push("add tests before refactoring".to_owned());
    } else if linked_tests >= 3 {
        score += 0.05;
        reasons.push(format!("strong test adjacency ({linked_tests} tests)"));
    }

    // Unresolved edge penalty.
    if unresolved > 0 {
        let penalty = (unresolved as f64 * 0.03).min(0.2);
        score -= penalty;
        reasons.push(format!("{unresolved} low-confidence/unresolved edges — dynamic usage risk"));
        validations.push("verify no dynamic dispatch before removing".to_owned());
    }

    // Clamp to [0, 1].
    score = score.clamp(0.0, 1.0);

    let band = if score >= 0.7 {
        SafetyBand::Safe
    } else if score >= 0.4 {
        SafetyBand::Caution
    } else {
        SafetyBand::Risky
    };

    (score, band, reasons, validations)
}

fn rename_risk(
    node: &Node,
    reference_count: usize,
    manual_flags: usize,
    has_collisions: bool,
) -> RiskLevel {
    if has_collisions || (is_public_node(node) && reference_count > 20) {
        return RiskLevel::High;
    }
    if manual_flags > 0 || reference_count > 10 || is_public_node(node) {
        return RiskLevel::Medium;
    }
    RiskLevel::Low
}

fn compute_risk_level(
    _node: &Node,
    inputs: RiskInputs,
) -> (RiskLevel, Vec<String>) {
    let mut score: i32 = 0;
    let mut factors: Vec<String> = vec![];

    if inputs.is_public {
        score += 3;
        factors.push("public/exported API touched".to_owned());
    }
    if !inputs.test_adj {
        score += 2;
        factors.push("no test adjacency".to_owned());
    }
    if inputs.cross_package {
        score += 3;
        factors.push("cross-package impact".to_owned());
    } else if inputs.cross_module {
        score += 2;
        factors.push("cross-module impact".to_owned());
    }
    if inputs.fan_in > 10 {
        score += 2;
        factors.push(format!("high inbound caller count ({})", inputs.fan_in));
    } else if inputs.fan_in > 3 {
        score += 1;
    }
    if inputs.unresolved > 0 {
        score += 1;
        factors.push(format!("{} unresolved/dynamic references", inputs.unresolved));
    }
    if inputs.fan_out > 15 {
        score += 1;
        factors.push(format!("high dependency fan-out ({})", inputs.fan_out));
    }
    if inputs.impacted_file_count > 10 {
        score += 1;
        factors.push(format!("{} impacted files", inputs.impacted_file_count));
    }

    let level = if score >= 8 {
        RiskLevel::Critical
    } else if score >= 5 {
        RiskLevel::High
    } else if score >= 2 {
        RiskLevel::Medium
    } else {
        RiskLevel::Low
    };

    (level, factors)
}

fn build_review_focus(
    is_public: bool,
    cross_module: bool,
    cross_package: bool,
    fan_in: usize,
    tests: &[(Node, Edge)],
) -> Vec<String> {
    let mut focus: Vec<String> = vec![];
    if is_public {
        focus.push("review public API contract and downstream consumers".to_owned());
    }
    if cross_package {
        focus.push("audit cross-package dependencies for breakage".to_owned());
    } else if cross_module {
        focus.push("check cross-module call sites for compatibility".to_owned());
    }
    if fan_in > 5 {
        focus.push(format!("verify all {fan_in} call sites handle change correctly"));
    }
    if tests.is_empty() {
        focus.push("add tests before merging — symbol is uncovered".to_owned());
    }
    focus
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind};
    use atlas_store_sqlite::Store;

    fn make_store() -> Store {
        let mut s = Store::open(":memory:").unwrap();
        s.migrate().unwrap();
        s
    }

    fn node(id: i64, name: &str, qname: &str, file: &str, kind: NodeKind) -> Node {
        Node {
            id: NodeId(id),
            kind,
            name: name.to_owned(),
            qualified_name: qname.to_owned(),
            file_path: file.to_owned(),
            line_start: 1,
            line_end: 10,
            language: "rust".to_owned(),
            parent_name: None,
            params: None,
            return_type: None,
            modifiers: None,
            is_test: false,
            file_hash: String::new(),
            extra_json: serde_json::Value::Null,
        }
    }

    fn edge(src: &str, tgt: &str, kind: EdgeKind, file: &str) -> Edge {
        Edge {
            id: 0,
            kind,
            source_qn: src.to_owned(),
            target_qn: tgt.to_owned(),
            file_path: file.to_owned(),
            line: None,
            confidence: 1.0,
            confidence_tier: Some("high".to_owned()),
            extra_json: serde_json::Value::Null,
        }
    }

    fn seed_graph(store: &mut Store, nodes: Vec<Node>, edges: Vec<Edge>) {
        // Collect unique file paths.
        let mut files: std::collections::HashMap<String, (Vec<Node>, Vec<Edge>)> =
            Default::default();
        for n in nodes {
            files.entry(n.file_path.clone()).or_default().0.push(n);
        }
        for e in edges {
            files.entry(e.file_path.clone()).or_default().1.push(e);
        }
        for (path, (ns, es)) in files {
            let lang = ns.first().map(|n| n.language.clone());
            store
                .replace_file_graph(
                    &path,
                    "hash",
                    lang.as_deref(),
                    None,
                    &ns,
                    &es,
                )
                .unwrap();
        }
    }

    // -----------------------------------------------------------------------
    // analyze_removal: simple call graph
    // -----------------------------------------------------------------------
    #[test]
    fn removal_simple_call_graph() {
        let mut store = make_store();
        let nodes = vec![
            node(0, "fn_a", "src/a.rs::fn_a", "src/a.rs", NodeKind::Function),
            node(0, "fn_b", "src/b.rs::fn_b", "src/b.rs", NodeKind::Function),
        ];
        // fn_b calls fn_a
        let edges = vec![edge("src/b.rs::fn_b", "src/a.rs::fn_a", EdgeKind::Calls, "src/b.rs")];
        seed_graph(&mut store, nodes, edges);

        let engine = ReasoningEngine::new(&store);
        let result =
            engine.analyze_removal(&["src/a.rs::fn_a"], None, None).unwrap();

        assert!(!result.seed.is_empty(), "seed should resolve");
        assert!(
            result.impacted_symbols.iter().any(|im| im.node.qualified_name == "src/b.rs::fn_b"),
            "fn_b should be in impacted symbols"
        );
    }

    // -----------------------------------------------------------------------
    // analyze_removal: cyclic graph — should not hang
    // -----------------------------------------------------------------------
    #[test]
    fn removal_cyclic_graph() {
        let mut store = make_store();
        let nodes = vec![
            node(0, "fn_a", "src/a.rs::fn_a", "src/a.rs", NodeKind::Function),
            node(0, "fn_b", "src/b.rs::fn_b", "src/b.rs", NodeKind::Function),
        ];
        let edges = vec![
            edge("src/a.rs::fn_a", "src/b.rs::fn_b", EdgeKind::Calls, "src/a.rs"),
            edge("src/b.rs::fn_b", "src/a.rs::fn_a", EdgeKind::Calls, "src/b.rs"),
        ];
        seed_graph(&mut store, nodes, edges);

        let engine = ReasoningEngine::new(&store);
        let result =
            engine.analyze_removal(&["src/a.rs::fn_a"], Some(5), Some(100)).unwrap();
        // Should terminate and include fn_b.
        assert!(result.impacted_symbols.iter().any(|im| im.node.qualified_name == "src/b.rs::fn_b"));
    }

    // -----------------------------------------------------------------------
    // detect_dead_code: private function with no callers is flagged
    // -----------------------------------------------------------------------
    #[test]
    fn dead_code_private_function_flagged() {
        let mut store = make_store();
        let mut priv_node =
            node(0, "unused_fn", "src/a.rs::unused_fn", "src/a.rs", NodeKind::Function);
        priv_node.modifiers = None; // private
        seed_graph(&mut store, vec![priv_node], vec![]);

        let engine = ReasoningEngine::new(&store);
        let candidates = engine.detect_dead_code(&[], None).unwrap();
        assert!(
            candidates.iter().any(|c| c.node.qualified_name == "src/a.rs::unused_fn"),
            "private unused_fn should be dead-code candidate"
        );
    }

    // -----------------------------------------------------------------------
    // detect_dead_code: exported function NOT flagged
    // -----------------------------------------------------------------------
    #[test]
    fn dead_code_exported_function_not_flagged() {
        let mut store = make_store();
        let mut pub_node =
            node(0, "pub_fn", "src/a.rs::pub_fn", "src/a.rs", NodeKind::Function);
        pub_node.modifiers = Some("pub".to_owned());
        seed_graph(&mut store, vec![pub_node], vec![]);

        let engine = ReasoningEngine::new(&store);
        let candidates = engine.detect_dead_code(&[], None).unwrap();
        assert!(
            !candidates.iter().any(|c| c.node.qualified_name == "src/a.rs::pub_fn"),
            "pub function should not be flagged"
        );
    }

    // -----------------------------------------------------------------------
    // detect_dead_code: entrypoint suppression
    // -----------------------------------------------------------------------
    #[test]
    fn dead_code_entrypoint_suppressed() {
        let mut store = make_store();
        let main_node =
            node(0, "main", "src/main.rs::main", "src/main.rs", NodeKind::Function);
        seed_graph(&mut store, vec![main_node], vec![]);

        let engine = ReasoningEngine::new(&store);
        let candidates = engine.detect_dead_code(&[], None).unwrap();
        assert!(
            !candidates.iter().any(|c| c.node.name == "main"),
            "main entrypoint should be suppressed"
        );
    }

    // -----------------------------------------------------------------------
    // preview_rename_radius: same file
    // -----------------------------------------------------------------------
    #[test]
    fn rename_same_file_radius() {
        let mut store = make_store();
        let nodes = vec![
            node(0, "fn_a", "src/a.rs::fn_a", "src/a.rs", NodeKind::Function),
            node(0, "fn_caller", "src/a.rs::fn_caller", "src/a.rs", NodeKind::Function),
        ];
        let edges =
            vec![edge("src/a.rs::fn_caller", "src/a.rs::fn_a", EdgeKind::Calls, "src/a.rs")];
        seed_graph(&mut store, nodes, edges);

        let engine = ReasoningEngine::new(&store);
        let result = engine.preview_rename_radius("src/a.rs::fn_a", "fn_a_renamed").unwrap();
        assert!(
            result.affected_references.iter().any(|r| r.scope == ReferenceScope::SameFile),
            "caller in same file should appear as SameFile reference"
        );
    }

    // -----------------------------------------------------------------------
    // preview_rename_radius: cross-module
    // -----------------------------------------------------------------------
    #[test]
    fn rename_cross_module_radius() {
        let mut store = make_store();
        let nodes = vec![
            node(0, "fn_a", "module_a/lib.rs::fn_a", "module_a/lib.rs", NodeKind::Function),
            node(0, "fn_b", "module_b/lib.rs::fn_b", "module_b/lib.rs", NodeKind::Function),
        ];
        let edges = vec![edge(
            "module_b/lib.rs::fn_b",
            "module_a/lib.rs::fn_a",
            EdgeKind::Calls,
            "module_b/lib.rs",
        )];
        seed_graph(&mut store, nodes, edges);

        let engine = ReasoningEngine::new(&store);
        let result =
            engine.preview_rename_radius("module_a/lib.rs::fn_a", "fn_a_v2").unwrap();
        assert!(
            result
                .affected_references
                .iter()
                .any(|r| r.scope == ReferenceScope::CrossModule),
            "caller in different module dir should be CrossModule"
        );
    }

    // -----------------------------------------------------------------------
    // check_dependency_removal: blocked by reference
    // -----------------------------------------------------------------------
    #[test]
    fn dependency_removal_blocked_by_reference() {
        let mut store = make_store();
        let nodes = vec![
            node(0, "dep_a", "src/a.rs::dep_a", "src/a.rs", NodeKind::Function),
            node(0, "consumer", "src/b.rs::consumer", "src/b.rs", NodeKind::Function),
        ];
        let edges = vec![edge(
            "src/b.rs::consumer",
            "src/a.rs::dep_a",
            EdgeKind::Calls,
            "src/b.rs",
        )];
        seed_graph(&mut store, nodes, edges);

        let engine = ReasoningEngine::new(&store);
        let result = engine.check_dependency_removal("src/a.rs::dep_a").unwrap();
        assert!(!result.removable, "dep_a is still referenced — not removable");
        assert!(!result.blocking_references.is_empty());
    }

    // -----------------------------------------------------------------------
    // find_test_adjacency: missing test signal
    // -----------------------------------------------------------------------
    #[test]
    fn test_adjacency_missing_for_changed_symbol() {
        let mut store = make_store();
        let no_test_node =
            node(0, "fn_x", "src/lib.rs::fn_x", "src/lib.rs", NodeKind::Function);
        seed_graph(&mut store, vec![no_test_node], vec![]);

        let engine = ReasoningEngine::new(&store);
        let result = engine.find_test_adjacency("src/lib.rs::fn_x").unwrap();
        assert_eq!(result.coverage_strength, CoverageStrength::None);
        assert!(result.recommendation.is_some());
    }

    // -----------------------------------------------------------------------
    // score_refactor_safety: risk scoring sanity
    // -----------------------------------------------------------------------
    #[test]
    fn refactor_safety_sanity_checks() {
        let mut store = make_store();
        // Private fn, no callers, no tests — should be Safe band.
        let solo =
            node(0, "solo_fn", "src/a.rs::solo_fn", "src/a.rs", NodeKind::Function);
        seed_graph(&mut store, vec![solo], vec![]);

        let engine = ReasoningEngine::new(&store);
        let result = engine.score_refactor_safety("src/a.rs::solo_fn").unwrap();
        // No callers, no tests → score penalized but stored; band should not be Risky
        // (only ~0.15 deducted for no tests from 1.0 start → 0.85 → Safe).
        assert_eq!(result.safety.band, SafetyBand::Safe);
    }
}
