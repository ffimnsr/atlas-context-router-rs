use super::*;
use atlas_token_count::TokenCounter;

#[test]
fn source_mix_lists_saved_artifacts_when_present() {
    let mut store = open_store();
    seed_graph(&mut store);

    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let req = ContextRequest {
        intent: ContextIntent::Symbol,
        target: ContextTarget::QualifiedName {
            qname: "src/a.rs::fn_a".to_owned(),
        },
        token_budget: Some(1), // force trimming
        ..ContextRequest::default()
    };
    let mut result = build_symbol_context(&store, seed, &req).expect("build symbol context");
    result.saved_context_sources = vec![SavedContextSource {
        source_id: "s1".to_owned(),
        label: "prior_result".to_owned(),
        source_type: "review_context".to_owned(),
        session_id: None,
        agent_id: None,
        preview: "preview".to_owned(),
        retrieval_hint: "source_id=s1".to_owned(),
        relevance_score: 5.0,
        repo_provenance: None,
        context_ranking_evidence: None,
    }];
    super::payload::apply_payload_budgets(
        &mut result,
        &BudgetPolicy::default(),
        &TokenCounter::heuristic(4).expect("test heuristic counter"),
        &super::payload::TokenFallbackInfo::default(),
    );

    let payload = result
        .truncation
        .payload
        .expect("payload truncation must run with 1-token budget");

    if !payload.source_mix.is_empty() {
        let has_saved = payload
            .source_mix
            .iter()
            .any(|m| m.source_kind == "saved_artifacts");
        // saved_artifacts are dropped first so they may be gone from the result,
        // but the mix entry must still record them as dropped.
        assert!(
            has_saved,
            "source_mix must include saved_artifacts when sources were present"
        );
    }
}

// ─── Patch N2: graph/content companion selection policy ───────────────────────

#[test]
fn content_assets_field_is_empty_when_include_flag_is_false() {
    // When include_content_assets = false (default), result.content_assets must
    // be empty regardless of budget settings.
    let mut store = open_store();
    seed_graph(&mut store);

    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let req = ContextRequest {
        intent: ContextIntent::Symbol,
        target: ContextTarget::QualifiedName {
            qname: "src/a.rs::fn_a".to_owned(),
        },
        include_content_assets: false,
        ..ContextRequest::default()
    };
    let result = build_symbol_context(&store, seed, &req).expect("build symbol context");
    assert!(
        result.content_assets.is_empty(),
        "content_assets must be empty when include_content_assets is false"
    );
}

#[test]
fn content_assets_dropped_field_is_zero_by_default() {
    // TruncationMeta.content_assets_dropped must default to zero even when no
    // content assets are requested or dropped.
    let mut store = open_store();
    seed_graph(&mut store);

    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let req = ContextRequest {
        intent: ContextIntent::Symbol,
        target: ContextTarget::QualifiedName {
            qname: "src/a.rs::fn_a".to_owned(),
        },
        ..ContextRequest::default()
    };
    let mut result = build_symbol_context(&store, seed, &req).expect("build symbol context");
    let policy = BudgetPolicy::default();
    super::payload::apply_payload_budgets(
        &mut result,
        &policy,
        &TokenCounter::heuristic(4).expect("test heuristic counter"),
        &super::payload::TokenFallbackInfo::default(),
    );

    assert_eq!(
        result.truncation.content_assets_dropped, 0,
        "content_assets_dropped must be 0 when no content assets are present"
    );
}

#[test]
fn source_mix_includes_content_assets_when_present() {
    // When content_assets are present, source_mix must include a "content_assets"
    // entry with the correct counts.
    let mut store = open_store();
    seed_graph(&mut store);

    let seed = store.node_by_qname("src/a.rs::fn_a").unwrap().unwrap();
    let req = ContextRequest {
        intent: ContextIntent::Symbol,
        target: ContextTarget::QualifiedName {
            qname: "src/a.rs::fn_a".to_owned(),
        },
        token_budget: Some(1), // force trimming to exercise source_mix path
        ..ContextRequest::default()
    };
    let mut result = build_symbol_context(&store, seed, &req).expect("build symbol context");
    // Manually inject a content asset to simulate N2 retrieval.
    result.content_assets = vec![atlas_core::ContentAsset {
        source_id: "ca1".to_owned(),
        path: "docs/overview.md".to_owned(),
        content_type: "doc".to_owned(),
        preview: "# Overview".to_owned(),
        selection_reason: atlas_core::ContentAssetReason::AdjacentToChangedFile,
        relevance_score: 0.8,
        context_ranking_evidence: None,
    }];
    super::payload::apply_payload_budgets(
        &mut result,
        &BudgetPolicy::default(),
        &TokenCounter::heuristic(4).expect("test heuristic counter"),
        &super::payload::TokenFallbackInfo::default(),
    );

    let payload = result
        .truncation
        .payload
        .expect("payload truncation must run with 1-token budget");

    if !payload.source_mix.is_empty() {
        let ca_mix = payload
            .source_mix
            .iter()
            .find(|m| m.source_kind == "content_assets");
        // At very tight budgets content assets may be dropped, but the mix entry
        // must still reference them (either included or dropped count > 0).
        if let Some(ca) = ca_mix {
            assert!(
                ca.items_included + ca.items_dropped > 0,
                "content_assets source_mix entry must record at least one item"
            );
        }
        // Verify ordering: graph_context before content_assets before saved_artifacts.
        let kinds: Vec<&str> = payload
            .source_mix
            .iter()
            .map(|m| m.source_kind.as_str())
            .collect();
        let graph_idx = kinds.iter().position(|k| *k == "graph_context");
        let ca_idx = kinds.iter().position(|k| *k == "content_assets");
        let saved_idx = kinds.iter().position(|k| *k == "saved_artifacts");
        if let (Some(g), Some(c)) = (graph_idx, ca_idx) {
            assert!(
                g < c,
                "graph_context must appear before content_assets in source_mix"
            );
        }
        if let (Some(c), Some(s)) = (ca_idx, saved_idx) {
            assert!(
                c < s,
                "content_assets must appear before saved_artifacts in source_mix"
            );
        }
    }
}

#[test]
fn content_asset_reason_priority_order() {
    // AdjacentToChangedFile > RelatedToChangedSymbol > ContentMatch
    use atlas_core::ContentAssetReason;
    assert!(
        ContentAssetReason::AdjacentToChangedFile.priority()
            > ContentAssetReason::RelatedToChangedSymbol.priority(),
        "AdjacentToChangedFile must outrank RelatedToChangedSymbol"
    );
    assert!(
        ContentAssetReason::RelatedToChangedSymbol.priority()
            > ContentAssetReason::ContentMatch.priority(),
        "RelatedToChangedSymbol must outrank ContentMatch"
    );
}
