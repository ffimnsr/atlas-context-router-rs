//! Config unit tests, grouped by surface: transport (mcp/http-auth/embedding),
//! loading, template rendering, tokenizer config, and insights/layer-rules
//! validation.

use std::fs;

use camino::Utf8Path;
use tempfile::tempdir;

use super::{Config, ConfigTemplateProfile, TokenizerFallbackMode};

mod auto;
mod insights;
mod load;
mod mcp;
mod overview;
mod template;

fn write_config(atlas_dir: &std::path::Path, body: &str) {
    fs::create_dir_all(atlas_dir).expect("create atlas dir");
    fs::write(atlas_dir.join(crate::paths::ATLAS_CONFIG), body).expect("write config");
}

/// Tiny synthetic `.tiktoken` fixture: every needed ASCII byte plus `ab` and
/// `cd` merges, so `abcd` counts as two tokens.
const TIKTOKEN_FIXTURE: &str = "YQ== 0\nYg== 1\nYw== 2\nZA== 3\nYWI= 4\nY2Q= 5\n";

fn write_tiktoken_fixture(atlas_dir: &std::path::Path, name: &str) {
    fs::create_dir_all(atlas_dir).expect("create atlas dir");
    fs::write(atlas_dir.join(name), TIKTOKEN_FIXTURE).expect("write tiktoken fixture");
}

#[test]
fn load_default_context_tokenizer_config() {
    let dir = tempdir().expect("tempdir");
    let config = Config::load(dir.path()).expect("default config");

    let tokenizer = &config.context.tokenizer;
    assert_eq!(tokenizer.encoding.as_deref(), Some("o200k_base"));
    assert_eq!(tokenizer.model, None);
    assert_eq!(tokenizer.tokenizer_file, None);
    assert_eq!(tokenizer.fallback, TokenizerFallbackMode::Heuristic);
    assert_eq!(tokenizer.bytes_per_token, 4);
    tokenizer.validate().expect("default tokenizer config");
}

#[test]
fn empty_tokenizer_section_inherits_o200k_default() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(&atlas_dir, "[context.tokenizer]\n");

    let config = Config::load(&atlas_dir).expect("load config");
    assert_eq!(
        config.context.tokenizer.encoding.as_deref(),
        Some("o200k_base")
    );
}

#[test]
fn load_tiktoken_provider_with_local_file() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_tiktoken_fixture(&atlas_dir, "fixture.tiktoken");
    write_config(
        &atlas_dir,
        "[context.tokenizer]\nencoding = \"cl100k_base\"\ntokenizer_file = \"fixture.tiktoken\"\nfallback = \"fail_closed\"\n",
    );

    let config = Config::load(&atlas_dir).expect("load config");
    let result = config
        .token_counter(Utf8Path::from_path(&atlas_dir).expect("utf8 atlas dir"))
        .expect("tiktoken counter");
    assert!(!result.fallback_used);
    let count = result.counter.count_text("abcd").expect("count");
    assert_eq!(count.tokens, 2);
    assert_eq!(
        count.method,
        atlas_token_count::TokenCountMethod::Tiktoken {
            encoding: "cl100k_base".to_owned(),
            model: None,
        }
    );
}

#[test]
fn load_tiktoken_preserves_matching_model_metadata() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_tiktoken_fixture(&atlas_dir, "fixture.tiktoken");
    write_config(
        &atlas_dir,
        "[context.tokenizer]\nencoding = \"o200k_base\"\nmodel = \"gpt-4o\"\ntokenizer_file = \"fixture.tiktoken\"\n",
    );

    let config = Config::load(&atlas_dir).expect("load config");
    let result = config
        .token_counter(Utf8Path::from_path(&atlas_dir).expect("utf8 atlas dir"))
        .expect("tiktoken counter");
    let count = result.counter.count_text("abcd").expect("count");
    assert_eq!(
        count.method,
        atlas_token_count::TokenCountMethod::Tiktoken {
            encoding: "o200k_base".to_owned(),
            model: Some("gpt-4o".to_owned()),
        }
    );
}

#[test]
fn tiktoken_missing_file_falls_back_to_heuristic() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(
        &atlas_dir,
        "[context.tokenizer]\nencoding = \"cl100k_base\"\ntokenizer_file = \"missing.tiktoken\"\nfallback = \"heuristic\"\n",
    );

    let config = Config::load(&atlas_dir).expect("load config");
    let result = config
        .token_counter(Utf8Path::from_path(&atlas_dir).expect("utf8 atlas dir"))
        .expect("fallback counter");
    assert!(result.fallback_used);
    let reason = result.fallback_reason.expect("fallback reason");
    assert!(
        reason.contains("missing.tiktoken"),
        "reason should mention the rank file: {reason}"
    );
    let count = result.counter.count_text("abcd").expect("count");
    assert_eq!(count.tokens, 1);
}

#[test]
fn tiktoken_missing_file_fail_closed_errors() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(
        &atlas_dir,
        "[context.tokenizer]\ntokenizer_file = \"missing.tiktoken\"\nfallback = \"fail_closed\"\n",
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
        err.contains("missing.tiktoken"),
        "error should include the resolved path: {err}"
    );
}

#[test]
fn tiktoken_unknown_encoding_rejected_at_load() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(
        &atlas_dir,
        "[context.tokenizer]\nencoding = \"nope_base\"\n",
    );

    let err = Config::load(&atlas_dir)
        .expect_err("unknown encoding")
        .to_string();
    assert!(
        err.contains("context.tokenizer.encoding"),
        "error should name the config key: {err}"
    );
}

#[test]
fn tiktoken_encoding_model_mismatch_rejected() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(
        &atlas_dir,
        "[context.tokenizer]\nencoding = \"cl100k_base\"\nmodel = \"gpt-4o\"\n",
    );

    let err = Config::load(&atlas_dir)
        .expect_err("encoding/model mismatch")
        .to_string();
    assert!(
        err.contains("context.tokenizer.encoding") && err.contains("context.tokenizer.model"),
        "error should name both keys: {err}"
    );
}

#[test]
fn tokenizer_blank_tokenizer_file_rejected() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path().join(crate::paths::ATLAS_DIR);
    write_config(&atlas_dir, "[context.tokenizer]\ntokenizer_file = \"  \"\n");

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
    write_config(&atlas_dir, "[context.tokenizer]\nmodel = \"\"\n");

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
fn tokenizer_template_contains_context_tokenizer_section() {
    let template = Config::render_template(ConfigTemplateProfile::Minimal).expect("template");

    assert!(template.contains("[context.tokenizer]"));
    assert!(template.contains("# encoding = \"o200k_base\""));
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
