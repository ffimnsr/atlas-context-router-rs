//! Phase XI.5: MCP parity for tokenizer-backed budget accounting.
//!
//! Mirrors the CLI `tokenizer_budget` quality-gate tests with the same
//! committed fixture and config values, so CLI and MCP JSON agree on token
//! provider and fallback flag for the same input.

use super::*;

fn tokenizer_fixture_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../atlas-token-count/tests/fixtures/tokenizer_budget.json")
}

fn setup_tokenizer_config(
    repo_root: &std::path::Path,
    tokenizer_file: &str,
    fallback: &str,
    place_fixture: bool,
) {
    let atlas_dir = repo_root.join(".atlas");
    let tokenizers_dir = atlas_dir.join("tokenizers");
    std::fs::create_dir_all(&tokenizers_dir).expect("create tokenizers dir");
    if place_fixture {
        std::fs::copy(
            tokenizer_fixture_path(),
            tokenizers_dir.join(tokenizer_file),
        )
        .expect("copy tokenizer fixture");
    }
    std::fs::write(
        atlas_dir.join("config.toml"),
        format!(
            "[context.tokenizer]\nprovider = \"tokenizers\"\ntokenizer_file = \"tokenizers/{tokenizer_file}\"\nfallback = \"{fallback}\"\n"
        ),
    )
    .expect("write tokenizer config");
}

#[test]
fn tokenizer_budget_mcp_reports_tokenizer_accounting() {
    let fixture = setup_mcp_fixture();
    setup_tokenizer_config(
        fixture._dir.path(),
        "tokenizer_budget.json",
        "fail_closed",
        true,
    );
    let repo_root = fixture._dir.path().to_string_lossy().to_string();

    let args = serde_json::json!({
        "target": { "kind": "query", "query": "compute" },
        "token_budget": 100,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), &repo_root, &fixture.db_path).expect("call");

    let value = resp["structuredContent"].clone();
    let accounting = value["payload_truncation"]["token_accounting"].clone();
    assert_eq!(accounting["provider"], serde_json::json!("tokenizers"));
    assert_eq!(accounting["fallback_used"], serde_json::json!(false));
    let tokens_estimated = value["payload_truncation"]["tokens_estimated"]
        .as_u64()
        .expect("tokens_estimated must remain present");
    let bytes_emitted = value["payload_truncation"]["bytes_emitted"]
        .as_u64()
        .expect("bytes_emitted");
    assert!(
        tokens_estimated != bytes_emitted.div_ceil(4),
        "tokenizer-backed count must differ from the byte heuristic (tokens={tokens_estimated}, bytes={bytes_emitted})"
    );
}

#[test]
fn tokenizer_budget_mcp_heuristic_fallback_matches_cli() {
    let fixture = setup_mcp_fixture();
    setup_tokenizer_config(fixture._dir.path(), "missing.json", "heuristic", false);
    let repo_root = fixture._dir.path().to_string_lossy().to_string();

    let args = serde_json::json!({
        "target": { "kind": "query", "query": "compute" },
        "token_budget": 100,
        "output_format": "json"
    });
    let resp = call("get_context", Some(&args), &repo_root, &fixture.db_path).expect("call");

    let accounting = resp["structuredContent"]["payload_truncation"]["token_accounting"].clone();
    // Same provider and fallback flag as the CLI fallback quality gate.
    assert_eq!(accounting["provider"], serde_json::json!("heuristic"));
    assert_eq!(accounting["fallback_used"], serde_json::json!(true));
    let reason = accounting["fallback_reason"]
        .as_str()
        .expect("fallback reason must be present");
    assert!(!reason.is_empty(), "fallback reason must be non-empty");
}

#[test]
fn tokenizer_budget_mcp_fail_closed_errors_with_config_key() {
    let fixture = setup_mcp_fixture();
    setup_tokenizer_config(fixture._dir.path(), "missing.json", "fail_closed", false);
    let repo_root = fixture._dir.path().to_string_lossy().to_string();

    let args = serde_json::json!({
        "target": { "kind": "query", "query": "compute" },
        "output_format": "json"
    });
    let err = call("get_context", Some(&args), &repo_root, &fixture.db_path)
        .expect("fail-closed must return a tool error result");
    assert_eq!(err["isError"], serde_json::json!(true));
    let message = err["structuredContent"]["message"]
        .as_str()
        .expect("error message");
    assert!(
        message.contains("context.tokenizer.tokenizer_file"),
        "error must name the config key: {message}"
    );
}
