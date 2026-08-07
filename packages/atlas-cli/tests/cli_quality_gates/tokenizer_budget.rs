//! Phase XI.5: tokenizer-backed budget accounting through the CLI.
//!
//! Uses the committed character-level WordPiece fixture in
//! `atlas-token-count` (token count ≈ char count, much higher than
//! `bytes.div_ceil(4)`) so these tests fail if accounting regresses to the
//! byte heuristic.

use super::*;

fn tokenizer_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../atlas-token-count/tests/fixtures/tokenizer_budget.json")
}

/// Temp repo with a small graph and a `[context.tokenizer]` config pointing
/// at a local tokenizer file under `.atlas/tokenizers/`.
fn setup_tokenizer_repo(tokenizer_file: &str, fallback: &str, place_fixture: bool) -> TempDir {
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn alpha() {\n    beta();\n}\n\npub fn beta() {}\n",
    )]);
    run_atlas(repo.path(), &["init"]);

    let atlas_dir = repo.path().join(".atlas");
    let tokenizers_dir = atlas_dir.join("tokenizers");
    fs::create_dir_all(&tokenizers_dir).expect("create tokenizers dir");
    if place_fixture {
        fs::copy(
            tokenizer_fixture_path(),
            tokenizers_dir.join(tokenizer_file),
        )
        .expect("copy tokenizer fixture");
    }
    fs::write(
        atlas_dir.join("config.toml"),
        format!(
            "[context.tokenizer]\nprovider = \"tokenizers\"\ntokenizer_file = \"tokenizers/{tokenizer_file}\"\nfallback = \"{fallback}\"\n"
        ),
    )
    .expect("write tokenizer config");

    repo
}

#[test]
fn tokenizer_budget_cli_context_reports_tokenizer_accounting() {
    let repo = setup_tokenizer_repo("tokenizer_budget.json", "fail_closed", true);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &["--json", "context", "alpha", "--token-budget", "100"],
        ),
    );

    let payload = &data["truncation"]["payload"];
    let accounting = &payload["token_accounting"];
    assert_eq!(accounting["provider"], json!("tokenizers"));
    assert_eq!(accounting["fallback_used"], json!(false));

    let tokens_estimated = payload["tokens_estimated"]
        .as_u64()
        .expect("tokens_estimated must remain present");
    let bytes_emitted = payload["bytes_emitted"].as_u64().expect("bytes_emitted");
    assert!(
        tokens_estimated != bytes_emitted.div_ceil(4),
        "tokenizer-backed count must differ from the byte heuristic (tokens={tokens_estimated}, bytes={bytes_emitted})"
    );
    assert!(
        tokens_estimated > 0,
        "tokenizer-backed count must be non-zero"
    );
}

#[test]
fn tokenizer_budget_cli_heuristic_fallback_reports_fallback() {
    let repo = setup_tokenizer_repo("missing.json", "heuristic", false);
    run_atlas(repo.path(), &["build"]);

    let output = run_atlas(
        repo.path(),
        &["--json", "context", "alpha", "--token-budget", "100"],
    );
    assert!(
        output.status.success(),
        "heuristic fallback must keep the command successful: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let data = read_json_data_output("context", output);
    let accounting = &data["truncation"]["payload"]["token_accounting"];
    assert_eq!(accounting["provider"], json!("heuristic"));
    assert_eq!(accounting["fallback_used"], json!(true));
    let reason = accounting["fallback_reason"]
        .as_str()
        .expect("fallback reason must be present");
    assert!(!reason.is_empty(), "fallback reason must be non-empty");
}

#[test]
fn tokenizer_budget_cli_fail_closed_errors_with_config_key() {
    let repo = setup_tokenizer_repo("missing.json", "fail_closed", false);
    run_atlas(repo.path(), &["build"]);

    let output = run_command_capture(
        repo.path(),
        env!("CARGO_BIN_EXE_atlas"),
        &["--json", "context", "alpha"],
    );
    assert!(!output.status.success(), "fail-closed must error");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        combined.contains("context.tokenizer.tokenizer_file"),
        "error must name the config key: {combined}"
    );
}
