use super::factors::*;
use super::model::*;
use super::*;

impl<'s> InsightsEngine<'s> {
    pub fn assess_risk(
        &self,
        repo_root: impl AsRef<Path>,
        target: RiskAssessmentTarget,
    ) -> Result<RiskAssessmentAnalysis> {
        let store = self.store().ok_or_else(|| {
            atlas_core::AtlasError::Other(
                "risk assessment requires a store-backed insights engine".to_owned(),
            )
        })?;
        let resolved = resolve_risk_target(store, target)?;
        let snapshot = self.load_graph_snapshot(store, repo_root.as_ref())?;
        let rust_complexity = load_rust_complexity(repo_root.as_ref(), &snapshot.nodes)?;
        let node_metrics = build_node_metrics(self, &snapshot, &rust_complexity);
        let metric = node_metrics
            .iter()
            .find(|metric| metric.node.qualified_name == resolved.node.qualified_name)
            .ok_or_else(|| {
                atlas_core::AtlasError::Other(format!(
                    "risk target not present in active insights snapshot: {}",
                    resolved.node.qualified_name
                ))
            })?;
        let relationships =
            build_risk_relationship_context(store, &snapshot, &node_metrics, metric)?;
        let factor_contributions = finalize_factor_contributions(build_risk_factor_contributions(
            self,
            metric,
            &relationships,
        ));
        let score = score_risk(self, &factor_contributions);
        let classification = classify_risk(self, score);
        let finding = build_risk_finding(metric, score, classification, &factor_contributions);
        let report = self.risk_report(vec![finding]);

        Ok(RiskAssessmentAnalysis {
            target: resolved.node,
            requested_symbol: resolved.requested_symbol,
            score,
            classification,
            report,
            factor_contributions,
        })
    }
}

fn resolve_risk_target(store: &Store, target: RiskAssessmentTarget) -> Result<ResolvedRiskTarget> {
    match target {
        RiskAssessmentTarget::ResolvedNode { node } => {
            let qname = normalize_qn_kind_tokens(&node.qualified_name);
            let resolved = store.node_by_qname(&qname)?.ok_or_else(|| {
                atlas_core::AtlasError::Other(format!(
                    "risk target could not be resolved from node {}",
                    node.qualified_name
                ))
            })?;
            Ok(ResolvedRiskTarget {
                node: resolved,
                requested_symbol: Some(node.qualified_name),
            })
        }
        RiskAssessmentTarget::Symbol { symbol } => {
            let normalized = normalize_qn_kind_tokens(&symbol);
            if let Some(node) = store.node_by_qname(&normalized)? {
                return Ok(ResolvedRiskTarget {
                    node,
                    requested_symbol: Some(symbol),
                });
            }

            let candidates = store.nodes_by_name(symbol.trim(), EDGE_QUERY_LIMIT)?;
            if candidates.is_empty() {
                return Err(atlas_core::AtlasError::Other(format!(
                    "risk target could not be resolved: {symbol}"
                )));
            }
            if candidates.len() > 1 {
                let rendered = candidates
                    .iter()
                    .map(|candidate| candidate.qualified_name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(atlas_core::AtlasError::Other(format!(
                    "ambiguous risk target `{symbol}`; candidates: {rendered}"
                )));
            }

            Ok(ResolvedRiskTarget {
                node: candidates.into_iter().next().expect("single candidate"),
                requested_symbol: Some(symbol),
            })
        }
    }
}
