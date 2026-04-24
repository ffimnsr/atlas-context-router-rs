use std::collections::HashSet;

use atlas_core::{
    BudgetReport, ChangeRiskResult, ConfidenceTier, CoverageStrength, DependencyRemovalResult,
    EdgeKind, Node, NodeKind, ReasoningEvidence, RefactorSafetyResult, Result, SafetyScore,
    TestAdjacencyResult,
};

use super::{
    ReasoningEngine,
    helpers::{
        EDGE_QUERY_LIMIT, RiskInputs, build_review_focus, compute_risk_level, compute_safety_score,
        file_paths_cross_package, is_public_node, normalize_qn_kind_tokens,
    },
};

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
