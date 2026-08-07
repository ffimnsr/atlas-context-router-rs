//! Token accounting metadata emission tests.

use super::*;
use atlas_token_count::TokenCounter;
use std::fs;

/// Minimal valid WordPiece tokenizer JSON used as a local fixture.
const SIMPLE_TOKENIZER_JSON: &str = r#"{
  "version": "1.0",
  "truncation": null,
  "padding": null,
  "added_tokens": [],
  "normalizer": null,
  "pre_tokenizer": {
    "type": "Whitespace"
  },
  "post_processor": null,
  "decoder": null,
  "model": {
    "type": "WordPiece",
    "vocab": {
      "[UNK]": 0,
      "hello": 1,
      "world": 2
    },
    "unk_token": "[UNK]",
    "continuing_subword_prefix": "??",
    "max_input_chars_per_word": 100
  }
}"#;

fn apply_with(
    result: &mut ContextResult,
    counter: &TokenCounter,
    fallback: &super::payload::TokenFallbackInfo,
) {
    super::payload::apply_payload_budgets(result, &BudgetPolicy::default(), counter, fallback);
}

fn seed_result(store: &Store, token_budget: Option<usize>) -> ContextResult {
    let seed = store
        .node_by_qname("src/a.rs::fn_a")
        .unwrap()
        .expect("seed node");
    let req = ContextRequest {
        intent: ContextIntent::Symbol,
        target: ContextTarget::QualifiedName {
            qname: "src/a.rs::fn_a".to_owned(),
        },
        token_budget,
        ..ContextRequest::default()
    };
    build_symbol_context(store, seed, &req).expect("build symbol context")
}

#[test]
fn heuristic_mode_emits_provider_and_bytes_per_token() {
    let mut store = open_store();
    seed_graph(&mut store);
    let mut result = seed_result(&store, Some(1));

    apply_with(
        &mut result,
        &TokenCounter::heuristic(4).expect("heuristic counter"),
        &super::payload::TokenFallbackInfo::default(),
    );

    let payload = result
        .truncation
        .payload
        .expect("payload metadata with 1-token budget");
    let accounting = payload.token_accounting.expect("token accounting metadata");
    assert_eq!(accounting.provider, "heuristic");
    assert_eq!(accounting.bytes_per_token, Some(4));
    assert!(!accounting.fallback_used);
    assert!(accounting.model.is_none());
    assert!(accounting.fallback_reason.is_none());
    // Compatibility field still present with the counted value.
    assert!(payload.tokens_estimated > 0);
}

#[test]
fn tokenizer_mode_emits_provider_and_model() {
    let mut store = open_store();
    seed_graph(&mut store);
    let mut result = seed_result(&store, Some(1));

    let dir = tempfile::tempdir().expect("tempdir");
    let tokenizer_path = dir.path().join("simple-tokenizer.json");
    fs::write(&tokenizer_path, SIMPLE_TOKENIZER_JSON).expect("write fixture");
    let counter =
        TokenCounter::from_file(&tokenizer_path, "tokenizers", Some("simple-bpe".to_owned()))
            .expect("load tokenizer");

    apply_with(
        &mut result,
        &counter,
        &super::payload::TokenFallbackInfo::default(),
    );

    let accounting = result
        .truncation
        .payload
        .expect("payload metadata")
        .token_accounting
        .expect("token accounting metadata");
    assert_eq!(accounting.provider, "tokenizers");
    assert_eq!(accounting.model.as_deref(), Some("simple-bpe"));
    assert_eq!(accounting.bytes_per_token, None);
    assert!(!accounting.fallback_used);
}

#[test]
fn fallback_mode_emits_used_and_reason() {
    let mut store = open_store();
    seed_graph(&mut store);
    let mut result = seed_result(&store, Some(1));

    let fallback = super::payload::TokenFallbackInfo {
        used: true,
        reason: Some("failed to load tokenizer from .atlas/missing-tokenizer.json".to_owned()),
    };
    apply_with(
        &mut result,
        &TokenCounter::heuristic(4).expect("heuristic counter"),
        &fallback,
    );

    let accounting = result
        .truncation
        .payload
        .expect("payload metadata")
        .token_accounting
        .expect("token accounting metadata");
    assert!(accounting.fallback_used);
    let reason = accounting.fallback_reason.expect("fallback reason");
    assert!(
        reason.contains("missing-tokenizer.json"),
        "reason should carry the load error summary: {reason}"
    );
    assert_eq!(accounting.provider, "heuristic");
}

#[test]
fn metadata_appears_when_token_budget_applies_without_drops() {
    let mut store = open_store();
    seed_graph(&mut store);

    // Small request: payload stays under the 2000-token override, so no
    // payload units are dropped, but the override must still surface
    // token accounting metadata.
    let seed = store
        .node_by_qname("src/a.rs::fn_a")
        .unwrap()
        .expect("seed node");
    let req = ContextRequest {
        intent: ContextIntent::Symbol,
        target: ContextTarget::QualifiedName {
            qname: "src/a.rs::fn_a".to_owned(),
        },
        max_nodes: Some(3),
        max_edges: Some(3),
        max_files: Some(2),
        token_budget: Some(2000),
        ..ContextRequest::default()
    };
    let mut result = build_symbol_context(&store, seed, &req).expect("build symbol context");

    apply_with(
        &mut result,
        &TokenCounter::heuristic(4).expect("heuristic counter"),
        &super::payload::TokenFallbackInfo::default(),
    );

    let payload = result
        .truncation
        .payload
        .expect("metadata must exist when a token budget override applies");
    assert_eq!(payload.token_budget_applied, Some(2000));
    assert_eq!(payload.omitted_byte_count, 0);
    assert_eq!(payload.omitted_node_count, 0);
    let accounting = payload.token_accounting.expect("token accounting metadata");
    assert_eq!(accounting.provider, "heuristic");
    assert!(!accounting.fallback_used);
    assert!(payload.tokens_estimated > 0);
}
