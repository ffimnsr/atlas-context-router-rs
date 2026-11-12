use super::*;
use atlas_core::model::ContextOutcome;

fn telemetry_request(qname: &str, include_telemetry: bool) -> ContextRequest {
    ContextRequest {
        include_telemetry,
        ..symbol_request(qname)
    }
}

#[test]
fn telemetry_counts_parity_and_shape() {
    let mut store = open_store();
    seed_graph(&mut store);
    let engine = ContextEngine::new(&store);

    let result = engine
        .build(&telemetry_request("src/a.rs::fn_a", true))
        .expect("build context");
    let telemetry = result.telemetry.as_ref().expect("telemetry requested");

    assert_eq!(telemetry.intent, ContextIntent::Symbol);
    assert!(telemetry.requested_nodes >= telemetry.emitted_nodes);
    assert_eq!(telemetry.emitted_nodes, result.nodes.len());
    assert_eq!(telemetry.emitted_edges, result.edges.len());
    assert_eq!(telemetry.emitted_files, result.files.len());
    assert_eq!(
        telemetry.emitted_saved_sources,
        result.saved_context_sources.len()
    );
    assert_eq!(
        telemetry.emitted_content_assets,
        result.content_assets.len()
    );
    assert_eq!(telemetry.emitted_overviews, result.overviews.len());
    assert!(telemetry.requested_bytes >= telemetry.emitted_bytes);
    assert!(telemetry.emitted_bytes > 0);
    assert!(telemetry.requested_tokens >= telemetry.emitted_tokens);
    assert_eq!(telemetry.outcome, ContextOutcome::Ok);

    let value = serde_json::to_value(&result).expect("serialize result");
    let json = value.get("telemetry").expect("telemetry json");
    for field in [
        "intent",
        "requested_nodes",
        "emitted_nodes",
        "requested_edges",
        "emitted_edges",
        "requested_files",
        "emitted_files",
        "requested_saved_sources",
        "emitted_saved_sources",
        "requested_content_assets",
        "emitted_content_assets",
        "requested_overviews",
        "emitted_overviews",
        "requested_bytes",
        "emitted_bytes",
        "requested_tokens",
        "emitted_tokens",
        "budget_hit",
        "duration_ms",
        "outcome",
    ] {
        assert!(json.get(field).is_some(), "missing telemetry field {field}");
    }
}

#[test]
fn telemetry_is_opt_in_and_default_output_has_no_telemetry() {
    let mut store = open_store();
    seed_graph(&mut store);
    let engine = ContextEngine::new(&store);

    let result = engine
        .build(&telemetry_request("src/a.rs::fn_a", false))
        .expect("build context");
    assert!(result.telemetry.is_none());

    let value = serde_json::to_value(&result).expect("serialize default result");
    assert!(value.get("telemetry").is_none());
    assert!(
        !value["request"]
            .as_object()
            .unwrap()
            .contains_key("include_telemetry")
    );
}

#[test]
fn telemetry_classifies_empty_and_budget_exhausted() {
    let mut store = open_store();
    seed_graph(&mut store);
    let engine = ContextEngine::new(&store);

    let missing = engine
        .build(&telemetry_request("src/a.rs::fn_missing", true))
        .expect("build empty context");
    assert_eq!(
        missing
            .telemetry
            .as_ref()
            .map(|telemetry| telemetry.outcome),
        Some(ContextOutcome::Empty)
    );

    let mut request = telemetry_request("src/a.rs::fn_a", true);
    request.max_nodes = Some(1);
    let trimmed = engine.build(&request).expect("build trimmed context");
    let telemetry = trimmed.telemetry.as_ref().expect("telemetry requested");
    assert_eq!(telemetry.outcome, ContextOutcome::BudgetExhausted);
    assert!(telemetry.budget_hit);
}

#[test]
fn telemetry_records_process_metrics() {
    let before = atlas_metrics::snapshot()
        .context_builds
        .get("symbol:ok")
        .copied()
        .unwrap_or(0);

    let mut store = open_store();
    seed_graph(&mut store);
    let engine = ContextEngine::new(&store);
    let _ = engine
        .build(&telemetry_request("src/a.rs::fn_a", false))
        .expect("build context");

    let after = atlas_metrics::snapshot()
        .context_builds
        .get("symbol:ok")
        .copied()
        .unwrap_or(0);
    assert!(after > before);
    assert!(
        atlas_metrics::snapshot()
            .context_duration_ms
            .contains_key("symbol")
    );
    assert!(
        atlas_metrics::snapshot()
            .context_payload_bytes
            .contains_key("symbol")
    );
    assert!(
        atlas_metrics::snapshot()
            .context_payload_tokens
            .contains_key("symbol")
    );
}

#[test]
fn ranking_evidence_is_opt_in_on_engine_results() {
    let mut store = open_store();
    seed_graph(&mut store);
    let engine = ContextEngine::new(&store);

    let without = engine
        .build(&telemetry_request("src/a.rs::fn_a", false))
        .expect("build default context");
    assert!(
        without
            .nodes
            .iter()
            .all(|node| node.context_ranking_evidence.is_none())
    );

    let mut request = telemetry_request("src/a.rs::fn_a", false);
    request.include_ranking_evidence = true;
    let with = engine.build(&request).expect("build evidence context");
    assert!(
        with.nodes
            .iter()
            .any(|node| node.context_ranking_evidence.is_some())
    );
}
