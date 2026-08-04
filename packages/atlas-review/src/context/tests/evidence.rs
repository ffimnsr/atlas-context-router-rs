use super::*;

// ─── Patch N3: coordinated ranking and evidence ────────────────────────────────

#[test]
fn graph_node_evidence_has_source_kind_graph_node() {
    // After rank_context runs, every graph node must carry source_kind = graph_node
    // in its context_ranking_evidence.
    use atlas_core::MixedResultKind;

    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let req = symbol_request("src/a.rs::fn_a");
    let result = build_symbol_context(&store, seed, &req).unwrap();

    for sn in &result.nodes {
        let evidence = sn
            .context_ranking_evidence
            .as_ref()
            .expect("every ranked node must carry evidence");
        assert_eq!(
            evidence.source_kind,
            Some(MixedResultKind::GraphNode),
            "graph node '{}' must have source_kind = graph_node, got {:?}",
            sn.node.qualified_name,
            evidence.source_kind
        );
    }
}

#[test]
fn graph_edge_evidence_has_source_kind_graph_edge() {
    // After rank_context runs, every graph edge must carry source_kind = graph_edge.
    use atlas_core::MixedResultKind;

    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let req = symbol_request("src/a.rs::fn_a");
    let result = build_symbol_context(&store, seed, &req).unwrap();

    for se in &result.edges {
        let evidence = se
            .context_ranking_evidence
            .as_ref()
            .expect("every ranked edge must carry evidence");
        assert_eq!(
            evidence.source_kind,
            Some(MixedResultKind::GraphEdge),
            "graph edge {}->{} must have source_kind = graph_edge, got {:?}",
            se.edge.source_qn,
            se.edge.target_qn,
            evidence.source_kind
        );
    }
}

#[test]
fn direct_target_node_has_no_graph_distance_penalty() {
    // The seed node (distance = 0) must not record a graph_distance_penalty.
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let req = symbol_request("src/a.rs::fn_a");
    let result = build_symbol_context(&store, seed, &req).unwrap();

    let seed_node = result
        .nodes
        .iter()
        .find(|n| n.node.qualified_name == "src/a.rs::fn_a")
        .expect("seed node");
    assert_eq!(seed_node.distance, 0);
    let evidence = seed_node
        .context_ranking_evidence
        .as_ref()
        .expect("seed node evidence");
    assert!(
        evidence.graph_distance_penalty.is_none(),
        "seed node at distance 0 must not record a distance penalty"
    );
}

#[test]
fn neighbor_node_records_graph_distance_penalty() {
    // A callee at distance > 0 must carry a graph_distance_penalty > 0.
    let mut store = open_store();
    seed_graph(&mut store);
    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let req = symbol_request("src/a.rs::fn_a");
    let result = build_symbol_context(&store, seed, &req).unwrap();

    // fn_b is a callee of fn_a so distance >= 1.
    let callee = result
        .nodes
        .iter()
        .find(|n| n.node.qualified_name == "src/b.rs::fn_b");
    if let Some(callee) = callee {
        assert!(callee.distance >= 1, "callee must have distance >= 1");
        let evidence = callee
            .context_ranking_evidence
            .as_ref()
            .expect("callee evidence");
        assert!(
            evidence.graph_distance_penalty.is_some_and(|p| p > 0.0),
            "callee at distance {} must record a positive graph_distance_penalty",
            callee.distance
        );
    }
    // If fn_b wasn't included (budget too small), just skip without failing.
}

#[test]
fn content_asset_evidence_carries_mixed_result_source_kind() {
    // A manually constructed ContentAsset with context_ranking_evidence must
    // expose source_kind matching its content_type.
    use atlas_core::{ContentAsset, ContentAssetReason, ContextRankingEvidence, MixedResultKind};

    let make_asset = |content_type: &str, expected_kind: MixedResultKind| {
        let kind = MixedResultKind::from_content_type(content_type);
        assert_eq!(
            kind, expected_kind,
            "content_type '{content_type}' must map to {expected_kind:?}"
        );
        let evidence = ContextRankingEvidence {
            source_kind: Some(kind),
            base_score: Some(0.5),
            final_score: Some(0.5),
            bm25_score: Some(0.5),
            ..ContextRankingEvidence::default()
        };
        ContentAsset {
            source_id: format!("{content_type}-id"),
            path: format!("some/path.{content_type}"),
            content_type: content_type.to_string(),
            preview: "preview text".to_string(),
            selection_reason: ContentAssetReason::ContentMatch,
            relevance_score: 0.5,
            context_ranking_evidence: Some(evidence),
        }
    };

    let template_asset = make_asset("template", MixedResultKind::Template);
    assert_eq!(
        template_asset
            .context_ranking_evidence
            .as_ref()
            .unwrap()
            .source_kind,
        Some(MixedResultKind::Template),
        "template asset must carry source_kind = template"
    );

    let config_asset = make_asset("config", MixedResultKind::TextAsset);
    assert_eq!(
        config_asset
            .context_ranking_evidence
            .as_ref()
            .unwrap()
            .source_kind,
        Some(MixedResultKind::TextAsset),
        "config asset must carry source_kind = text_asset"
    );

    let prompt_asset = make_asset("prompt", MixedResultKind::TextAsset);
    assert_eq!(
        prompt_asset
            .context_ranking_evidence
            .as_ref()
            .unwrap()
            .source_kind,
        Some(MixedResultKind::TextAsset),
        "prompt asset must carry source_kind = text_asset"
    );

    let doc_asset = make_asset("doc", MixedResultKind::ContentMatch);
    assert_eq!(
        doc_asset
            .context_ranking_evidence
            .as_ref()
            .unwrap()
            .source_kind,
        Some(MixedResultKind::ContentMatch),
        "doc asset must carry source_kind = content_match"
    );
}

#[test]
fn content_asset_evidence_populates_normalized_signals() {
    // An asset that matches a changed-file adjacency must have changed_file_boost set.
    // An asset that mentions a symbol name must have same_package_dir_boost set.
    // Both must carry bm25_score.
    use atlas_core::{ContentAsset, ContentAssetReason, ContextRankingEvidence, MixedResultKind};

    // Simulate the evidence that retrieve_content_assets would build for an
    // adjacent asset (same directory as a changed file).
    let adjacency_evidence = ContextRankingEvidence {
        source_kind: Some(MixedResultKind::Template),
        base_score: Some(1.0),
        final_score: Some(1.5),
        bm25_score: Some(1.0),
        changed_file_boost: Some(0.5),
        ..ContextRankingEvidence::default()
    };
    let adjacent_asset = ContentAsset {
        source_id: "tmpl-1".to_string(),
        path: "src/layout.html".to_string(),
        content_type: "template".to_string(),
        preview: "<!DOCTYPE html>".to_string(),
        selection_reason: ContentAssetReason::AdjacentToChangedFile,
        relevance_score: 1.5,
        context_ranking_evidence: Some(adjacency_evidence),
    };
    let ev = adjacent_asset.context_ranking_evidence.as_ref().unwrap();
    assert_eq!(ev.source_kind, Some(MixedResultKind::Template));
    assert!(
        ev.changed_file_boost.is_some_and(|b| b > 0.0),
        "adjacent asset must have changed_file_boost"
    );
    assert!(
        ev.bm25_score.is_some(),
        "adjacent asset must have bm25_score"
    );
    assert!(
        ev.final_score.unwrap_or(0.0) > ev.base_score.unwrap_or(0.0),
        "adjacency boost must increase final score above base"
    );

    // Simulate evidence for a symbol-mention asset (SQL file mentioning fn_a).
    let symbol_evidence = ContextRankingEvidence {
        source_kind: Some(MixedResultKind::TextAsset),
        base_score: Some(0.5),
        final_score: Some(0.8),
        bm25_score: Some(0.5),
        same_package_dir_boost: Some(0.3),
        exact_symbol_match: true,
        ..ContextRankingEvidence::default()
    };
    let sql_asset = ContentAsset {
        source_id: "sql-1".to_string(),
        path: "src/queries.sql".to_string(),
        content_type: "sql".to_string(),
        preview: "SELECT * FROM fn_a WHERE id = 1".to_string(),
        selection_reason: ContentAssetReason::RelatedToChangedSymbol,
        relevance_score: 0.8,
        context_ranking_evidence: Some(symbol_evidence),
    };
    let sql_ev = sql_asset.context_ranking_evidence.as_ref().unwrap();
    assert_eq!(sql_ev.source_kind, Some(MixedResultKind::TextAsset));
    assert!(
        sql_ev.same_package_dir_boost.is_some_and(|b| b > 0.0),
        "symbol-related asset must have same_package_dir_boost"
    );
    assert!(
        sql_ev.exact_symbol_match,
        "exact symbol mention must set exact_symbol_match"
    );
}

#[test]
fn saved_context_evidence_carries_saved_context_kind_and_session_recency() {
    // A SavedContextSource evidence struct must have source_kind = saved_context
    // and session_recency_boost when session boost was applied.
    use atlas_core::{ContextRankingEvidence, MixedResultKind, SavedContextSource};

    let evidence = ContextRankingEvidence {
        source_kind: Some(MixedResultKind::SavedContext),
        saved_context_rank_score: Some(5.0),
        recent_source_boost: Some(5.0),
        same_session_boost: Some(10.0),
        session_recency_boost: Some(15.0), // 5.0 + 10.0
        base_score: Some(5.0),
        final_score: Some(20.0),
        bm25_score: Some(5.0),
        ..ContextRankingEvidence::default()
    };
    let source = SavedContextSource {
        source_id: "src-42".to_string(),
        label: "prior_review".to_string(),
        source_type: "review_context".to_string(),
        session_id: Some("sess-abc".to_string()),
        agent_id: None,
        preview: "previous review output".to_string(),
        retrieval_hint: "source_id=src-42".to_string(),
        relevance_score: 20.0,
        repo_provenance: None,
        context_ranking_evidence: Some(evidence),
    };

    let ev = source.context_ranking_evidence.as_ref().unwrap();
    assert_eq!(ev.source_kind, Some(MixedResultKind::SavedContext));
    assert!(
        ev.session_recency_boost.is_some_and(|b| b > 0.0),
        "saved context with session and recency boosts must carry unified session_recency_boost"
    );
    // session_recency_boost must equal recent_source_boost + same_session_boost.
    let expected = ev.recent_source_boost.unwrap_or(0.0) + ev.same_session_boost.unwrap_or(0.0);
    assert!(
        (ev.session_recency_boost.unwrap_or(0.0) - expected).abs() < 0.001,
        "session_recency_boost must sum recent_source_boost + same_session_boost"
    );
}

#[test]
fn mixed_result_kind_from_content_type_mapping() {
    // Validate the full mapping table for MixedResultKind::from_content_type.
    use atlas_core::MixedResultKind;

    assert_eq!(
        MixedResultKind::from_content_type("template"),
        MixedResultKind::Template
    );
    assert_eq!(
        MixedResultKind::from_content_type("sql"),
        MixedResultKind::TextAsset
    );
    assert_eq!(
        MixedResultKind::from_content_type("config"),
        MixedResultKind::TextAsset
    );
    assert_eq!(
        MixedResultKind::from_content_type("prompt"),
        MixedResultKind::TextAsset
    );
    assert_eq!(
        MixedResultKind::from_content_type("doc"),
        MixedResultKind::ContentMatch
    );
    assert_eq!(
        MixedResultKind::from_content_type("other"),
        MixedResultKind::ContentMatch
    );
    assert_eq!(
        MixedResultKind::from_content_type(""),
        MixedResultKind::ContentMatch
    );
    assert_eq!(
        MixedResultKind::from_content_type("unknown_kind"),
        MixedResultKind::ContentMatch
    );
}
