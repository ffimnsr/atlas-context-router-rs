//! Integration tests for `tiktoken` counting from a local rank fixture.

use std::path::PathBuf;

use atlas_token_count::{TokenCountMethod, TokenCounter};
use atlas_tokenizer::EncodingName;

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join(name)
}

#[test]
fn loads_fixture_and_counts_deterministic_text() {
    let counter = TokenCounter::from_tiktoken_file_with_encoding(
        fixture_path("tiny.tiktoken"),
        EncodingName::Cl100kBase,
        Some("fixture-model".to_owned()),
    )
    .expect("fixture encoding should load");

    let first = counter
        .count_text("hello world")
        .expect("counting should succeed");
    let second = counter
        .count_text("hello world")
        .expect("counting should succeed");

    // The synthetic fixture has one rank per byte and no merges.
    assert_eq!(first.tokens, "hello world".len());
    assert_eq!(first.tokens, second.tokens);
    assert_eq!(
        first.method,
        TokenCountMethod::Tiktoken {
            encoding: "cl100k_base".to_owned(),
            model: Some("fixture-model".to_owned()),
        }
    );
    assert!(first.fallback_reason.is_none());
}

#[test]
fn missing_rank_path_mentions_path_in_error() {
    let missing = fixture_path("does-not-exist.tiktoken");
    let err = TokenCounter::from_tiktoken_file(&missing, None)
        .expect_err("missing fixture must fail")
        .to_string();
    assert!(
        err.contains("does-not-exist.tiktoken"),
        "error should include the rank file path: {err}"
    );
}

#[test]
fn fixture_counts_known_samples() {
    let counter = TokenCounter::from_tiktoken_file_with_encoding(
        fixture_path("tiny.tiktoken"),
        EncodingName::Cl100kBase,
        None,
    )
    .expect("tiny fixture should load");

    assert_eq!(counter.count_text("hello world").expect("count").tokens, 11);
    assert_eq!(
        counter.count_text("hello, world!").expect("count").tokens,
        13
    );
    assert_eq!(
        counter
            .count_text("hello world hello world")
            .expect("count")
            .tokens,
        23
    );
    assert_eq!(counter.count_text("").expect("count").tokens, 0);
    assert_eq!(counter.count_text("a").expect("count").tokens, 1);

    // The fixture token count must differ from the byte heuristic so
    // regressions back to bytes.div_ceil(4) stay detectable.
    assert_ne!(
        counter.count_text("hello world").expect("count").tokens,
        "hello world".len().div_ceil(4)
    );
}
