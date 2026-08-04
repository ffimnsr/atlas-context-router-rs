use super::*;

#[test]
fn merge_scored_nodes_preserves_exact_evidence() {
    let mut exact = make_test_node("compute", "src/lib.rs::fn::compute", "src/lib.rs", "rust");
    exact.ranking_evidence = Some({
        let mut evidence = RankingEvidence::new(RetrievalMode::Fts5, 80.0);
        evidence.exact_name_match = true;
        evidence.add_matched_field(SearchMatchedField::Name);
        evidence
    });
    exact.score = 80.0;

    let mut fuzzy = make_test_node("compute", "src/lib.rs::fn::compute", "src/lib.rs", "rust");
    fuzzy.ranking_evidence = Some(RankingEvidence {
        base_mode: RetrievalMode::Fts5,
        raw_score: Some(90.0),
        final_score: 90.0,
        matched_fields: vec![SearchMatchedField::Name],
        exact_name_match: false,
        exact_qualified_name_match: false,
        prefix_match: false,
        fuzzy: Some(atlas_core::FuzzyCorrectionEvidence {
            corrected_term: Some("compute".to_string()),
            edit_distance: Some(1),
            fuzzy_threshold: Some(1),
        }),
        kind_boost: None,
        public_exported_boost: None,
        same_directory_boost: None,
        same_language_boost: None,
        recent_file_boost: None,
        changed_file_boost: None,
        graph_expansion: None,
        hybrid_rrf: None,
    });
    fuzzy.score = 90.0;

    let merged = merge_scored_nodes(vec![exact], vec![fuzzy]);
    let evidence = merged[0]
        .ranking_evidence
        .as_ref()
        .expect("ranking evidence");
    assert!(
        evidence.exact_name_match,
        "exact evidence should survive merge"
    );
    assert!(
        evidence.fuzzy.is_some(),
        "fuzzy evidence should survive merge"
    );
}

#[test]
fn graph_expand_records_hop_distance_and_seed_source() {
    let dir = tempfile::tempdir().expect("tempdir");
    let db_path = dir.path().join("atlas.db");
    let db_path = db_path.to_string_lossy().to_string();
    let mut store = Store::open(&db_path).expect("open store");

    let node_a = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "a".to_string(),
        qualified_name: "src/a.rs::fn::a".to_string(),
        file_path: "src/a.rs".to_string(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_string(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "ha".to_string(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    };
    let node_b = Node {
        id: NodeId::UNSET,
        kind: NodeKind::Function,
        name: "b".to_string(),
        qualified_name: "src/b.rs::fn::b".to_string(),
        file_path: "src/b.rs".to_string(),
        line_start: 1,
        line_end: 5,
        language: "rust".to_string(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: "hb".to_string(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    };
    let edge_ab = Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: "src/a.rs::fn::a".to_string(),
        target_qn: "src/b.rs::fn::b".to_string(),
        file_path: "src/a.rs".to_string(),
        line: Some(1),
        confidence: 1.0,
        confidence_tier: Some("high".to_string()),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    };
    store
        .replace_file_graph(
            "src/a.rs",
            "ha",
            Some("rust"),
            Some(5),
            &[node_a],
            &[edge_ab],
        )
        .expect("replace a graph");
    store
        .replace_file_graph("src/b.rs", "hb", Some("rust"), Some(5), &[node_b], &[])
        .expect("replace b graph");

    let expanded = graph_expand(
        &store,
        vec![make_test_node("a", "src/a.rs::fn::a", "src/a.rs", "rust")],
        1,
        10,
    )
    .expect("expanded results");

    let neighbor = expanded
        .iter()
        .find(|result| result.node.qualified_name == "src/b.rs::fn::b")
        .expect("neighbor result");
    let graph = neighbor
        .ranking_evidence
        .as_ref()
        .and_then(|e| e.graph_expansion.as_ref())
        .expect("graph expansion evidence");
    assert_eq!(graph.hop_distance, 1);
    assert_eq!(
        graph.seed_qualified_name.as_deref(),
        Some("src/a.rs::fn::a")
    );
}

#[test]
fn reciprocal_rank_fusion_records_rank_and_score_contributions() {
    let mut fts = make_test_node("compute", "src/lib.rs::fn::compute", "src/lib.rs", "rust");
    fts.ranking_evidence = Some({
        let mut evidence = RankingEvidence::new(RetrievalMode::Fts5, 10.0);
        evidence.exact_name_match = true;
        evidence.add_matched_field(SearchMatchedField::Name);
        evidence
    });
    fts.score = 10.0;

    let mut vector = make_test_node("compute", "src/lib.rs::fn::compute", "src/lib.rs", "rust");
    vector.ranking_evidence = Some(
        RankingEvidence::new(RetrievalMode::Vector, 0.9)
            .with_matched_field(SearchMatchedField::Embedding),
    );
    vector.score = 0.9;

    let fused = reciprocal_rank_fusion(&[fts], &[vector], 60);
    let evidence = fused[0]
        .ranking_evidence
        .as_ref()
        .expect("ranking evidence");
    let hybrid = evidence.hybrid_rrf.as_ref().expect("hybrid evidence");
    assert_eq!(evidence.base_mode, RetrievalMode::Hybrid);
    assert!(
        evidence.exact_name_match,
        "fts evidence should survive fusion"
    );
    assert!(
        evidence
            .matched_fields
            .contains(&SearchMatchedField::Embedding),
        "vector evidence should survive fusion"
    );
    assert_eq!(hybrid.sources.len(), 2);
    assert!(hybrid.sources.iter().any(|source| {
        source.source == HybridRankingSource::Fts5
            && source.rank == 1
            && source.score_contribution > 0.0
    }));
    assert!(hybrid.sources.iter().any(|source| {
        source.source == HybridRankingSource::Vector
            && source.rank == 1
            && source.score_contribution > 0.0
    }));
}
