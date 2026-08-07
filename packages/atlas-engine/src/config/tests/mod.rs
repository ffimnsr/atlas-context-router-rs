//! Config unit tests, grouped by surface: transport (mcp/http-auth/embedding),
//! loading, template rendering, tokenizer config, and insights/layer-rules
//! validation.

use std::fs;

use camino::Utf8Path;
use tempfile::tempdir;

use super::{Config, ConfigTemplateProfile, TokenizerFallbackMode, TokenizerProvider};

mod insights;
mod load;
mod mcp;
mod template;

/// Minimal valid WordPiece tokenizer JSON used as a local fixture.
const SIMPLE_TOKENIZER_JSON: &str = r##"{
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
}"##;

fn write_config(atlas_dir: &std::path::Path, body: &str) {
    fs::create_dir_all(atlas_dir).expect("create atlas dir");
    fs::write(atlas_dir.join(crate::paths::ATLAS_CONFIG), body).expect("write config");
}

#[test]
fn load_default_context_tokenizer_config() {
    let dir = tempdir().expect("tempdir");
    let config = Config::load(dir.path()).expect("default config");

    let tokenizer = &config.context.tokenizer;
    assert_eq!(tokenizer.provider, TokenizerProvider::Heuristic);
    assert_eq!(tokenizer.model, None);
    assert_eq!(tokenizer.tokenizer_file, None);
    assert_eq!(tokenizer.fallback, TokenizerFallbackMode::Heuristic);
    assert_eq!(tokenizer.bytes_per_token, 4);

    let result = config
        .token_counter(Utf8Path::from_path(dir.path()).expect("utf8 atlas dir"))
        .expect("heuristic counter");
    assert!(!result.fallback_used);
    assert!(result.fallback_reason.is_none());
    // Default heuristic preserves bytes.div_ceil(4).
    let count = result.counter.count_text("abcd").expect("count");
    assert_eq!(count.tokens, 1);
}

#[test]
fn load_tokenizers_provider_with_relative_file() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    fs::create_dir_all(&atlas_dir).expect("create atlas dir");
    fs::write(atlas_dir.join("tokenizer.json"), SIMPLE_TOKENIZER_JSON).expect("write tokenizer");
    write_config(
        &atlas_dir,
        "[context.tokenizer]\nprovider = \"tokenizers\"\ntokenizer_file = \"tokenizer.json\"\nmodel = \"fixture\"\nfallback = \"fail_closed\"\n",
    );

    let config = Config::load(&atlas_dir).expect("load config");
    assert_eq!(
        config.context.tokenizer.provider,
        TokenizerProvider::Tokenizers
    );

    let result = config
        .token_counter(Utf8Path::from_path(&atlas_dir).expect("utf8 atlas dir"))
        .expect("tokenizer counter");
    assert!(!result.fallback_used);
    assert!(result.fallback_reason.is_none());
    let count = result.counter.count_text("hello world").expect("count");
    assert_eq!(count.tokens, 2);
    assert_eq!(
        count.method,
        atlas_token_count::TokenCountMethod::Tokenizer {
            provider: "tokenizers".to_owned(),
            model: Some("fixture".to_owned()),
        }
    );
}

#[test]
fn tokenizer_provider_missing_file_falls_back_to_heuristic() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(
        &atlas_dir,
        "[context.tokenizer]\nprovider = \"tokenizers\"\ntokenizer_file = \"missing-tokenizer.json\"\nfallback = \"heuristic\"\n",
    );

    let config = Config::load(&atlas_dir).expect("load config");
    let result = config
        .token_counter(Utf8Path::from_path(&atlas_dir).expect("utf8 atlas dir"))
        .expect("fallback counter");
    assert!(result.fallback_used);
    let reason = result.fallback_reason.expect("fallback reason");
    assert!(
        reason.contains("missing-tokenizer.json"),
        "reason should mention the tokenizer path: {reason}"
    );
    let count = result.counter.count_text("abcd").expect("count");
    assert_eq!(count.tokens, 1);
}

#[test]
fn tokenizer_provider_missing_file_fail_closed_errors() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(
        &atlas_dir,
        "[context.tokenizer]\nprovider = \"tokenizers\"\ntokenizer_file = \"missing-tokenizer.json\"\nfallback = \"fail_closed\"\n",
    );

    let config = Config::load(&atlas_dir).expect("load config");
    let err = config
        .token_counter(Utf8Path::from_path(&atlas_dir).expect("utf8 atlas dir"))
        .expect_err("fail-closed must error")
        .to_string();
    assert!(
        err.contains("context.tokenizer.tokenizer_file"),
        "error should name the config key: {err}"
    );
    assert!(
        err.contains("missing-tokenizer.json"),
        "error should include the resolved path: {err}"
    );
}

#[test]
fn tokenizer_provider_missing_file_rejected_at_load() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(
        &atlas_dir,
        "[context.tokenizer]\nprovider = \"tokenizers\"\n",
    );

    let err = Config::load(&atlas_dir)
        .expect_err("load must reject tokenizers provider without file")
        .to_string();
    assert!(
        err.contains("context.tokenizer.tokenizer_file"),
        "error should name the config key: {err}"
    );
}

#[test]
fn tokenizer_blank_tokenizer_file_rejected() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(
        &atlas_dir,
        "[context.tokenizer]\nprovider = \"tokenizers\"\ntokenizer_file = \"  \"\n",
    );

    let err = Config::load(&atlas_dir)
        .expect_err("load must reject blank tokenizer_file")
        .to_string();
    assert!(
        err.contains("context.tokenizer.tokenizer_file"),
        "error should name the config key: {err}"
    );
}

#[test]
fn tokenizer_blank_model_rejected() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(
        &atlas_dir,
        "[context.tokenizer]\nprovider = \"tokenizers\"\ntokenizer_file = \"tokenizer.json\"\nmodel = \"\"\n",
    );

    let err = Config::load(&atlas_dir)
        .expect_err("load must reject blank model")
        .to_string();
    assert!(
        err.contains("context.tokenizer.model"),
        "error should name the config key: {err}"
    );
}

#[test]
fn tokenizer_zero_bytes_per_token_rejected() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(&atlas_dir, "[context.tokenizer]\nbytes_per_token = 0\n");

    let err = Config::load(&atlas_dir)
        .expect_err("load must reject zero bytes_per_token")
        .to_string();
    assert!(
        err.contains("context.tokenizer.bytes_per_token"),
        "error should name the config key: {err}"
    );
}

#[test]
fn tokenizer_file_rejected_for_heuristic_provider() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(
        &atlas_dir,
        "[context.tokenizer]\ntokenizer_file = \"tokenizer.json\"\n",
    );

    let err = Config::load(&atlas_dir)
        .expect_err("load must reject tokenizer_file for heuristic provider")
        .to_string();
    assert!(
        err.contains("context.tokenizer.tokenizer_file"),
        "error should name the config key: {err}"
    );
}

#[test]
fn tokenizer_template_contains_context_tokenizer_section() {
    let template = Config::render_template(ConfigTemplateProfile::Minimal).expect("template");

    assert!(template.contains("[context.tokenizer]"));
    assert!(template.contains("# provider = \"heuristic\""));
    assert!(template.contains("# fallback = \"heuristic\""));
    assert!(template.contains("# bytes_per_token = 4"));
    // Commented example values must never activate a missing file.
    assert!(template.contains("# tokenizer_file = \"tokenizer.json\""));
    assert!(
        !template
            .lines()
            .any(|line| line.starts_with("tokenizer_file = ")),
        "no active tokenizer_file line may point at a missing file"
    );
}
