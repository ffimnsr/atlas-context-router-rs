use super::*;
use atlas_core::model::RepoProvenance;

/// R7.2: remove opt-out metadata and hoist uniform repo provenance.
///
/// Ranking evidence is diagnostics, not required evidence, so it is dropped by
/// default. Provenance is hoisted only when every emitted item shares exactly
/// the same provenance; mixed-repo or partially-provenanced results keep the
/// per-item fields losslessly.
pub(super) fn slim_context_metadata(result: &mut ContextResult, include_ranking_evidence: bool) {
    if !include_ranking_evidence {
        for node in &mut result.nodes {
            node.context_ranking_evidence = None;
        }
        for asset in &mut result.content_assets {
            asset.context_ranking_evidence = None;
        }
        for source in &mut result.saved_context_sources {
            source.context_ranking_evidence = None;
        }
    }

    hoist_uniform_provenance(result);
}

fn hoist_uniform_provenance(result: &mut ContextResult) {
    let mut shared: Option<RepoProvenance> = None;
    let mut item_count = 0usize;

    for provenance in result
        .nodes
        .iter()
        .map(|node| node.node.repo_provenance.as_ref())
        .chain(
            result
                .files
                .iter()
                .map(|file| file.repo_provenance.as_ref()),
        )
        .chain(
            result
                .saved_context_sources
                .iter()
                .map(|source| source.repo_provenance.as_ref()),
        )
    {
        let Some(provenance) = provenance else {
            // A missing provenance means the result is not uniform; keep
            // per-item provenance and the hoisted field empty.
            return;
        };
        match &shared {
            Some(existing) if existing != provenance => return,
            Some(_) => {}
            None => shared = Some(provenance.clone()),
        }
        item_count += 1;
    }

    let Some(provenance) = shared else {
        return;
    };
    if item_count == 0 {
        return;
    }

    result.repo_provenance = Some(provenance);
    for node in &mut result.nodes {
        node.node.repo_provenance = None;
    }
    for file in &mut result.files {
        file.repo_provenance = None;
    }
    for source in &mut result.saved_context_sources {
        source.repo_provenance = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::model::{SelectedFile, SelectionReason, TruncationMeta};
    use atlas_core::{BudgetReport, ContextRequest, ContextResult, Node, NodeId, NodeKind};

    fn provenance(repo_id: &str) -> RepoProvenance {
        RepoProvenance::new(repo_id).with_repo_fingerprint(format!("{repo_id}-fp"))
    }

    fn node(repo_provenance: Option<RepoProvenance>) -> SelectedNode {
        SelectedNode {
            node: Node {
                id: NodeId::UNSET,
                kind: NodeKind::Function,
                name: "seed".to_owned(),
                qualified_name: "src/lib.rs::fn::seed".to_owned(),
                file_path: "src/lib.rs".to_owned(),
                line_start: 1,
                line_end: 3,
                language: "rust".to_owned(),
                parent_name: None,
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: "hash".to_owned(),
                extra_json: serde_json::Value::Null,
                repo_provenance,
            },
            selection_reason: SelectionReason::DirectTarget,
            distance: 0,
            relevance_score: 1.0,
            context_ranking_evidence: Some(ContextRankingEvidence::from_selection_reason(
                SelectionReason::DirectTarget,
            )),
        }
    }

    fn file(repo_provenance: Option<RepoProvenance>) -> SelectedFile {
        SelectedFile {
            path: "src/lib.rs".to_owned(),
            selection_reason: SelectionReason::DirectTarget,
            line_ranges: Vec::new(),
            language: Some("rust".to_owned()),
            node_count_included: 1,
            repo_provenance,
        }
    }

    fn result(nodes: Vec<SelectedNode>, files: Vec<SelectedFile>) -> ContextResult {
        ContextResult {
            request: ContextRequest::default(),
            nodes,
            edges: Vec::new(),
            files,
            content_assets: Vec::new(),
            truncation: TruncationMeta::none(),
            seed_budgets: Vec::new(),
            traversal_budget: None,
            ambiguity: None,
            workflow: None,
            saved_context_sources: Vec::new(),
            overviews: Vec::new(),
            sufficiency: None,
            repo_provenance: None,
            telemetry: None,
            budget: BudgetReport::default(),
        }
    }

    #[test]
    fn hoists_uniform_provenance_and_clears_items() {
        let mut result = result(
            vec![node(Some(provenance("repo-a")))],
            vec![file(Some(provenance("repo-a")))],
        );

        slim_context_metadata(&mut result, false);

        assert_eq!(result.repo_provenance, Some(provenance("repo-a")));
        assert!(result.nodes[0].node.repo_provenance.is_none());
        assert!(result.files[0].repo_provenance.is_none());
        assert!(result.nodes[0].context_ranking_evidence.is_none());
    }

    #[test]
    fn keeps_per_item_provenance_for_multi_repo_results() {
        let mut result = result(
            vec![node(Some(provenance("repo-a")))],
            vec![file(Some(provenance("repo-b")))],
        );

        slim_context_metadata(&mut result, true);

        assert!(result.repo_provenance.is_none());
        assert_eq!(
            result.nodes[0].node.repo_provenance,
            Some(provenance("repo-a"))
        );
        assert_eq!(result.files[0].repo_provenance, Some(provenance("repo-b")));
        assert!(result.nodes[0].context_ranking_evidence.is_some());
    }

    #[test]
    fn keeps_item_provenance_when_one_item_is_missing_it() {
        let mut result = result(vec![node(Some(provenance("repo-a")))], vec![file(None)]);

        slim_context_metadata(&mut result, false);

        assert!(result.repo_provenance.is_none());
        assert_eq!(
            result.nodes[0].node.repo_provenance,
            Some(provenance("repo-a"))
        );
    }

    #[test]
    fn metadata_slimming_meets_corpus_token_threshold() {
        let provenance = provenance("repo-a");
        let nodes: Vec<SelectedNode> = (0..64)
            .map(|index| {
                let mut selected = node(Some(provenance.clone()));
                selected.node.qualified_name = format!("src/lib.rs::fn::item_{index}");
                selected.node.name = format!("item_{index}");
                selected.node.line_start = index as u32 + 1;
                selected.node.line_end = selected.node.line_start + 4;
                selected.context_ranking_evidence = Some(ContextRankingEvidence {
                    base_score: Some(100.0),
                    final_score: Some(100.0),
                    direct_target: true,
                    ..ContextRankingEvidence::from_selection_reason(SelectionReason::DirectTarget)
                });
                selected
            })
            .collect();
        let files = vec![file(Some(provenance.clone()))];
        let legacy = result(nodes, files);
        let legacy_bytes = serde_json::to_vec(&legacy).expect("serialize legacy").len();

        let mut slim = legacy.clone();
        slim_context_metadata(&mut slim, false);
        let slim_bytes = serde_json::to_vec(&slim).expect("serialize slim").len();

        let reduction = 1.0 - (slim_bytes as f64 / legacy_bytes as f64);
        assert!(
            reduction >= 0.15,
            "expected >=15% token reduction, got {reduction:.3} (legacy={legacy_bytes}, slim={slim_bytes})"
        );
        assert_eq!(slim.nodes.len(), legacy.nodes.len());
        assert!(slim.nodes.iter().all(|node| {
            node.node
                .qualified_name
                .starts_with("src/lib.rs::fn::item_")
                && node.node.line_start > 0
                && node.selection_reason == SelectionReason::DirectTarget
        }));
    }

    #[test]
    fn ranking_evidence_is_opt_in() {
        let mut result = result(vec![node(None)], Vec::new());

        slim_context_metadata(&mut result, true);
        assert!(result.nodes[0].context_ranking_evidence.is_some());

        slim_context_metadata(&mut result, false);
        assert!(result.nodes[0].context_ranking_evidence.is_none());
    }
}
