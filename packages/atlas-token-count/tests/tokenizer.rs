//! Integration tests for tokenizer-backed counting from a local fixture.

use std::path::PathBuf;

use atlas_token_count::{TokenCountMethod, TokenCounter};

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn loads_fixture_and_counts_deterministic_text() {
    let counter = TokenCounter::from_file(
        fixture_path("simple-tokenizer.json"),
        "fixture",
        Some("simple-bpe".to_owned()),
    )
    .expect("fixture tokenizer should load");

    let first = counter
        .count_text("hello world")
        .expect("counting should succeed");
    let second = counter
        .count_text("hello world")
        .expect("counting should succeed");

    assert_eq!(first.tokens, 2);
    assert_eq!(first.tokens, second.tokens);
    assert_eq!(
        first.method,
        TokenCountMethod::Tokenizer {
            provider: "fixture".to_owned(),
            model: Some("simple-bpe".to_owned()),
        }
    );
    assert!(first.fallback_reason.is_none());
}

#[test]
fn missing_tokenizer_path_mentions_path_in_error() {
    let missing = fixture_path("does-not-exist.json");
    let err = TokenCounter::from_file(&missing, "fixture", None)
        .expect_err("missing fixture must fail")
        .to_string();
    assert!(
        err.contains("does-not-exist.json"),
        "error should include the tokenizer path: {err}"
    );
}

#[test]
fn tokenizer_budget_fixture_counts_known_samples() {
    let counter = TokenCounter::from_file(
        fixture_path("tokenizer_budget.json"),
        "tokenizers",
        Some("tokenizer_budget".to_owned()),
    )
    .expect("tokenizer_budget fixture should load");

    // Character-level WordPiece: one token per known character.
    assert_eq!(counter.count_text("hello world").expect("count").tokens, 10);
    assert_eq!(
        counter.count_text("hello, world!").expect("count").tokens,
        12
    );
    assert_eq!(
        counter
            .count_text("hello world hello world")
            .expect("count")
            .tokens,
        20
    );
    assert_eq!(counter.count_text("").expect("count").tokens, 0);
    assert_eq!(counter.count_text("a").expect("count").tokens, 1);

    // The tokenizer count must differ from the byte heuristic so regressions
    // back to bytes.div_ceil(4) stay detectable.
    assert_ne!(
        counter.count_text("hello world").expect("count").tokens,
        "hello world".len().div_ceil(4)
    );
}
