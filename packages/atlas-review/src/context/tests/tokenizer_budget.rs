//! Deterministic tokenizer-backed budget tests (Phase XI.5).
//!
//! Uses the committed character-level WordPiece fixture in `atlas-token-count`
//! whose token counts differ from `bytes.div_ceil(4)`, so any regression back
//! to byte-only accounting fails these tests.

use super::*;
use atlas_token_count::TokenCounter;
use std::path::PathBuf;

fn tokenizer_budget_fixture() -> TokenCounter {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../atlas-token-count/tests/fixtures/tokenizer_budget.json");
    TokenCounter::from_file(&path, "tokenizers", Some("tokenizer_budget".to_owned()))
        .expect("tokenizer_budget fixture should load")
}

fn serialized_payload(result: &ContextResult) -> Vec<u8> {
    let mut clone = result.clone();
    clone.truncation.payload = None;
    serde_json::to_vec(&clone).expect("serialize result")
}

fn apply_with(result: &mut ContextResult, counter: &TokenCounter) {
    super::payload::apply_payload_budgets(
        result,
        &BudgetPolicy::default(),
        counter,
        &super::payload::TokenFallbackInfo::default(),
    );
}

/// Build a symbol context seeded with saved sources, workflow metadata, and
/// ambiguity metadata so every trimming section is populated.
fn seeded_full_result(store: &Store, token_budget: Option<usize>) -> ContextResult {
    let seed = store
        .node_by_qname("src/a.rs::fn_a")
        .unwrap()
        .expect("seed node");
    let mut req = symbol_request("src/a.rs::fn_a");
    req.token_budget = token_budget;
    let mut result = build_symbol_context(store, seed, &req).expect("build symbol context");

    result.saved_context_sources = vec![
        SavedContextSource {
            source_id: "s1".to_owned(),
            label: "artifact-s1".to_owned(),
            source_type: "review_context".to_owned(),
            session_id: Some("sess-1".to_owned()),
            agent_id: None,
            preview: "preview s1".to_owned(),
            retrieval_hint: "source_id=s1".to_owned(),
            relevance_score: 10.0,
            repo_provenance: None,
            context_ranking_evidence: None,
        },
        SavedContextSource {
            source_id: "s2".to_owned(),
            label: "artifact-s2".to_owned(),
            source_type: "review_context".to_owned(),
            session_id: Some("sess-1".to_owned()),
            agent_id: None,
            preview: "preview s2".to_owned(),
            retrieval_hint: "source_id=s2".to_owned(),
            relevance_score: 1.0,
            repo_provenance: None,
            context_ranking_evidence: None,
        },
    ];
    result.workflow = Some(WorkflowSummary {
        headline: Some("headline".to_owned()),
        high_impact_nodes: vec![
            WorkflowFocusNode {
                qualified_name: "src/a.rs::fn_a".to_owned(),
                kind: "function".to_owned(),
                file_path: "src/a.rs".to_owned(),
                relevance_score: 1.0,
                selection_reason: "direct_target".to_owned(),
            },
            WorkflowFocusNode {
                qualified_name: "src/b.rs::fn_b".to_owned(),
                kind: "function".to_owned(),
                file_path: "src/b.rs".to_owned(),
                relevance_score: 0.5,
                selection_reason: "caller".to_owned(),
            },
        ],
        impacted_components: vec![WorkflowComponent {
            label: "comp".to_owned(),
            kind: "module".to_owned(),
            changed_node_count: 1,
            impacted_node_count: 2,
            file_count: 1,
            summary: "comp summary".to_owned(),
        }],
        call_chains: vec![WorkflowCallChain {
            summary: "chain".to_owned(),
            steps: vec!["a".to_owned(), "b".to_owned()],
            edge_kinds: vec!["call".to_owned()],
        }],
        ripple_effects: vec!["ripple".to_owned()],
        noise_reduction: NoiseReductionSummary {
            retained_nodes: 1,
            retained_edges: 0,
            retained_files: 1,
            dropped_nodes: 0,
            dropped_edges: 0,
            dropped_files: 0,
            rules_applied: vec![],
        },
    });
    result.ambiguity = Some(AmbiguityMeta {
        query: "fn_a".to_owned(),
        candidates: vec!["src/a.rs::fn_a".to_owned(), "src/b.rs::fn_b".to_owned()],
        resolved: false,
    });
    result
}

#[test]
fn tokenizer_budget_trims_when_tokenizer_count_exceeds_budget() {
    let mut store = open_store();
    seed_graph(&mut store);

    let full = seeded_full_result(&store, None);
    let heuristic = TokenCounter::heuristic(4).expect("heuristic counter");
    let heuristic_tokens = heuristic
        .count_json_bytes(&serialized_payload(&full))
        .expect("heuristic count")
        .tokens;
    let tokenizer = tokenizer_budget_fixture();
    let tokenizer_tokens = tokenizer
        .count_json_bytes(&serialized_payload(&full))
        .expect("tokenizer count")
        .tokens;
    assert!(
        tokenizer_tokens > heuristic_tokens,
        "fixture must count more tokens than the byte heuristic ({tokenizer_tokens} vs {heuristic_tokens})"
    );

    // Budget at the heuristic count: the heuristic counter considers the
    // payload within budget (no trimming), the tokenizer counter must trim.
    let mut heuristic_result = seeded_full_result(&store, Some(heuristic_tokens));
    apply_with(&mut heuristic_result, &heuristic);
    let heuristic_payload = heuristic_result
        .truncation
        .payload
        .expect("metadata exists from token budget override");
    assert_eq!(heuristic_payload.omitted_byte_count, 0);
    assert_eq!(heuristic_payload.omitted_node_count, 0);

    let mut tokenizer_result = seeded_full_result(&store, Some(heuristic_tokens));
    apply_with(&mut tokenizer_result, &tokenizer);
    let payload = tokenizer_result
        .truncation
        .payload
        .expect("payload metadata");
    assert!(
        payload.omitted_byte_count > 0,
        "tokenizer token count must force trimming while the byte cap stays high"
    );
    assert!(
        payload.omitted_node_count > 0 || payload.omitted_file_count > 0,
        "trimming must drop payload units"
    );
    assert!(
        payload.tokens_estimated < tokenizer_tokens,
        "trimming must reduce the tokenizer-backed count below the full payload count"
    );
}

#[test]
fn tokenizer_budget_skips_trimming_when_count_below_cap() {
    let mut store = open_store();
    seed_graph(&mut store);

    let full = seeded_full_result(&store, None);
    let tokenizer = tokenizer_budget_fixture();
    let tokenizer_tokens = tokenizer
        .count_json_bytes(&serialized_payload(&full))
        .expect("tokenizer count")
        .tokens;

    let mut result = seeded_full_result(&store, Some(tokenizer_tokens + 1));
    apply_with(&mut result, &tokenizer);

    let payload = result
        .truncation
        .payload
        .expect("metadata exists from token budget override");
    assert_eq!(payload.omitted_byte_count, 0);
    assert_eq!(payload.omitted_node_count, 0);
    assert_eq!(payload.omitted_source_count, 0);
    // tokens_estimated must equal the fixture-backed count of the emitted
    // payload (identical to the full payload since nothing was trimmed).
    assert_eq!(payload.tokens_estimated, tokenizer_tokens);
}

#[test]
fn tokenizer_budget_truncation_order_is_deterministic() {
    let mut store = open_store();
    seed_graph(&mut store);
    let mut result = seeded_full_result(&store, None);

    let direct_files = result
        .files
        .iter()
        .filter(|f| f.selection_reason == SelectionReason::DirectTarget)
        .count();
    let non_direct_files = result.files.len() - direct_files;
    assert!(non_direct_files > 0, "needs non-direct files");
    let direct_nodes = result
        .nodes
        .iter()
        .filter(|n| n.selection_reason == SelectionReason::DirectTarget)
        .count();
    assert!(direct_nodes >= 1, "seed node must be a direct target");

    // Phase 1: saved context drops before anything else.
    assert!(super::payload::trim_one_payload_unit(&mut result));
    assert_eq!(result.saved_context_sources.len(), 1);
    assert!(super::payload::trim_one_payload_unit(&mut result));
    assert!(result.saved_context_sources.is_empty());

    // Phase 2: workflow metadata drops before files.
    let files_before = result.files.len();
    assert!(super::payload::trim_one_payload_unit(&mut result));
    assert!(
        result.workflow.as_ref().unwrap().call_chains.is_empty(),
        "call chains drop before files"
    );
    while result.workflow.as_ref().is_some_and(|w| {
        !w.ripple_effects.is_empty()
            || !w.impacted_components.is_empty()
            || w.high_impact_nodes.len() > 1
    }) {
        assert!(super::payload::trim_one_payload_unit(&mut result));
    }
    assert_eq!(result.files.len(), files_before);

    // Phase 3: ambiguity candidates drop before files.
    let ambiguity_len = result.ambiguity.as_ref().unwrap().candidates.len();
    assert!(super::payload::trim_one_payload_unit(&mut result));
    assert_eq!(
        result.ambiguity.as_ref().unwrap().candidates.len(),
        ambiguity_len - 1
    );
    while result.ambiguity.as_ref().unwrap().candidates.len() > 1 {
        assert!(super::payload::trim_one_payload_unit(&mut result));
    }
    assert_eq!(result.files.len(), files_before);

    // Phase 4: files (non-direct first) drop before edges and nodes.
    while result
        .files
        .iter()
        .any(|f| f.selection_reason != SelectionReason::DirectTarget)
    {
        assert!(super::payload::trim_one_payload_unit(&mut result));
        assert_eq!(
            result
                .files
                .iter()
                .filter(|f| f.selection_reason == SelectionReason::DirectTarget)
                .count(),
            direct_files,
            "direct files survive while non-direct files remain"
        );
    }
    let nodes_before_edges = result.nodes.len();
    while !result.files.is_empty() {
        assert!(super::payload::trim_one_payload_unit(&mut result));
        assert_eq!(
            result.nodes.len(),
            nodes_before_edges,
            "files drop before nodes"
        );
    }

    // Phase 5: edges drop before nodes.
    while !result.edges.is_empty() {
        assert!(super::payload::trim_one_payload_unit(&mut result));
    }

    // Phase 6: non-direct nodes drop before direct target nodes.
    while result
        .nodes
        .iter()
        .any(|n| n.selection_reason != SelectionReason::DirectTarget)
    {
        assert!(super::payload::trim_one_payload_unit(&mut result));
        assert_eq!(
            result
                .nodes
                .iter()
                .filter(|n| n.selection_reason == SelectionReason::DirectTarget)
                .count(),
            direct_nodes,
            "direct target nodes remain while removable lower-priority nodes exist"
        );
    }
    assert!(
        result
            .nodes
            .iter()
            .all(|n| n.selection_reason == SelectionReason::DirectTarget)
    );
}

#[test]
fn tokenizer_budget_direct_targets_survive_tight_budget() {
    let mut store = open_store();
    seed_graph(&mut store);

    let mut result = seeded_full_result(&store, Some(1));
    apply_with(&mut result, &tokenizer_budget_fixture());

    // Every removable section is dropped; the direct target seed remains.
    assert!(result.saved_context_sources.is_empty());
    assert!(result.files.is_empty());
    assert!(result.edges.is_empty());
    assert_eq!(result.nodes.len(), 1, "one node must survive");
    assert_eq!(
        result.nodes[0].selection_reason,
        SelectionReason::DirectTarget
    );
}

#[test]
fn tokenizer_budget_source_mix_reflects_dropped_sections() {
    let mut store = open_store();
    seed_graph(&mut store);

    let initial = seeded_full_result(&store, None);
    let initial_saved = initial.saved_context_sources.len();
    let initial_files = initial.files.len();
    let initial_edges = initial.edges.len();
    let initial_nodes = initial.nodes.len();

    let mut result = seeded_full_result(&store, Some(1));
    apply_with(&mut result, &tokenizer_budget_fixture());

    let payload = result
        .truncation
        .payload
        .expect("payload metadata with 1-token budget");
    let saved = payload
        .source_mix
        .iter()
        .find(|m| m.source_kind == "saved_artifacts")
        .expect("saved_artifacts mix entry");
    assert_eq!(saved.items_included, 0);
    assert_eq!(saved.items_dropped, initial_saved);

    let graph = payload
        .source_mix
        .iter()
        .find(|m| m.source_kind == "graph_context")
        .expect("graph_context mix entry");
    let retained = result.nodes.len() + result.edges.len() + result.files.len();
    let dropped = (initial_nodes - result.nodes.len())
        + (initial_edges - result.edges.len())
        + (initial_files - result.files.len());
    assert_eq!(graph.items_included, retained);
    assert_eq!(graph.items_dropped, dropped);
    assert!(dropped > 0, "tight budget must drop graph units");
}
