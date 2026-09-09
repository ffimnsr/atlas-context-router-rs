//! Budget-policy mapping and partial-section loading tests.

use super::super::*;
use super::*;
use atlas_core::NodeKind;
use std::fs;

#[test]
fn parsers_external_section_loads_and_validates() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path();
    fs::write(
        atlas_dir.join(crate::paths::ATLAS_CONFIG),
        r#"
[[parsers.external]]
language_name = "zig"
extensions = ["zig"]
grammar_dir = "grammars/tree-sitter-zig"

[[parsers.external.symbols]]
tree_kind = "function_declaration"
node_kind = "function"
"#,
    )
    .expect("write config");

    let config = Config::load(atlas_dir).expect("load config");
    assert_eq!(config.parsers.external.len(), 1);
    assert_eq!(config.parsers.external[0].language_name, "zig");
    assert_eq!(config.parsers.external[0].extensions, ["zig"]);
    assert_eq!(config.parsers.external[0].symbols.len(), 1);
    assert_eq!(
        config.parsers.external[0].symbols[0].node_kind,
        NodeKind::Function
    );
    // symbol name_field defaults to "name".
    assert_eq!(config.parsers.external[0].symbols[0].name_field, "name");
    // relative grammar paths resolve from the atlas dir (like tokenizer files).
    assert_eq!(
        config.parsers.external[0].grammar_dir.as_deref(),
        Some(atlas_dir.join("grammars/tree-sitter-zig").to_str().unwrap())
    );
}

#[test]
fn parsers_external_rejects_ambiguous_source_and_duplicate_extensions() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path();
    fs::write(
        atlas_dir.join(crate::paths::ATLAS_CONFIG),
        r#"
[[parsers.external]]
language_name = "zig"
extensions = ["zig"]
grammar_dir = "/opt/grammars/a"
lib_path = "/opt/grammars/libtree-sitter-zig.so"

[[parsers.external.symbols]]
tree_kind = "function_declaration"
node_kind = "function"
"#,
    )
    .expect("write config");
    let error = Config::load(atlas_dir).expect_err("must reject grammar_dir + lib_path");
    assert!(
        error
            .to_string()
            .contains("exactly one of grammar_dir or lib_path")
    );

    fs::write(
        atlas_dir.join(crate::paths::ATLAS_CONFIG),
        r#"
[[parsers.external]]
language_name = "a"
extensions = ["zig"]
grammar_dir = "/opt/grammars/a"
[[parsers.external.symbols]]
tree_kind = "x"
node_kind = "function"

[[parsers.external]]
language_name = "b"
extensions = ["zig"]
grammar_dir = "/opt/grammars/b"
[[parsers.external.symbols]]
tree_kind = "x"
node_kind = "function"
"#,
    )
    .expect("write config");
    let error = Config::load(atlas_dir).expect_err("must reject duplicate extension");
    assert!(error.to_string().contains("registered more than once"));
}

#[test]
fn budget_policy_maps_payload_budget_fields() {
    let mut config = Config::default();
    config.context.max_review_source_bytes = 2048;
    config.context.max_context_payload_bytes = 4096;
    config.context.max_context_tokens_estimate = 512;
    config.context.max_file_excerpt_bytes = 256;
    config.context.max_saved_context_bytes = 128;
    config.mcp.max_mcp_response_bytes = 8192;

    let policy = config.budget_policy().expect("budget policy");

    assert_eq!(
        policy
            .mcp_cli_payload_serialization
            .review_source_bytes
            .default_limit,
        2048
    );
    assert_eq!(
        policy
            .mcp_cli_payload_serialization
            .context_payload_bytes
            .default_limit,
        4096
    );
    assert_eq!(
        policy
            .mcp_cli_payload_serialization
            .context_tokens_estimate
            .default_limit,
        512
    );
    assert_eq!(
        policy
            .mcp_cli_payload_serialization
            .file_excerpt_bytes
            .default_limit,
        256
    );
    assert_eq!(
        policy
            .mcp_cli_payload_serialization
            .saved_context_bytes
            .default_limit,
        128
    );
    assert_eq!(
        policy
            .mcp_cli_payload_serialization
            .mcp_response_bytes
            .default_limit,
        8192
    );
}

#[test]
fn load_accepts_partial_nested_sections() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path();
    fs::write(
            atlas_dir.join(crate::paths::ATLAS_CONFIG),
            "[mcp]\nmax_mcp_response_bytes = 4096\n\n[context]\nmax_saved_context_bytes = 256\n\n[search.embedding]\nurl = \"http://embed.test\"\n",
        )
        .expect("write config");

    let config = Config::load(atlas_dir).expect("load config");

    assert_eq!(config.mcp.max_mcp_response_bytes, 4096);
    assert_eq!(config.context.max_saved_context_bytes, 256);
    assert_eq!(
        config.search.embedding.url.as_deref(),
        Some("http://embed.test")
    );
    assert_eq!(config.mcp.worker_threads, DEFAULT_MCP_WORKER_THREADS);
    assert!(config.mcp.tool_timeout_ms_by_tool.is_empty());
}

// ── ICM-B1 — memory decay config ──────────────────────────────────────────────

#[test]
fn memory_decay_defaults_load_without_a_memory_section() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path();
    fs::write(
        atlas_dir.join(crate::paths::ATLAS_CONFIG),
        "[build]\nparse_batch_size = 32\n",
    )
    .expect("write config");

    let config = Config::load(atlas_dir).expect("load config");
    let decay = &config.memory.decay;
    assert!(decay.enabled, "decay must default to enabled");
    assert_eq!(decay.low_days, 30);
    assert_eq!(decay.normal_days, 90);
    assert_eq!(decay.high_days, 365);
    assert!(
        decay.critical_never_prune,
        "critical memories must be protected by default"
    );
    assert!(!config.memory.allow_custom_frontends);
}

#[test]
fn memory_decay_partial_section_fills_missing_fields_with_defaults() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path();
    fs::write(
        atlas_dir.join(crate::paths::ATLAS_CONFIG),
        "[memory.decay]\nlow_days = 7\n",
    )
    .expect("write config");

    let config = Config::load(atlas_dir).expect("load config");
    let decay = &config.memory.decay;
    assert!(decay.enabled);
    assert_eq!(decay.low_days, 7);
    assert_eq!(
        decay.normal_days, 90,
        "absent fields must fall back to defaults"
    );
    assert_eq!(decay.high_days, 365);
    assert!(decay.critical_never_prune);
}

#[test]
fn memory_decay_rejects_non_positive_retention_days() {
    for (key, value, expected) in [
        ("low_days", "0", "memory.decay.low_days"),
        ("normal_days", "0", "memory.decay.normal_days"),
        ("high_days", "0", "memory.decay.high_days"),
    ] {
        let dir = tempdir().expect("tempdir");
        let atlas_dir = dir.path();
        fs::write(
            atlas_dir.join(crate::paths::ATLAS_CONFIG),
            format!("[memory.decay]\n{key} = {value}\n"),
        )
        .expect("write config");

        let error = Config::load(atlas_dir).expect_err("invalid retention days must fail");
        assert!(
            error.to_string().contains(expected),
            "error must name the field: {error}"
        );
    }
}

#[test]
fn memory_decay_disabled_skips_retention_validation() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path();
    fs::write(
        atlas_dir.join(crate::paths::ATLAS_CONFIG),
        "[memory.decay]\nenabled = false\nlow_days = 0\n",
    )
    .expect("write config");

    // Disabled decay is a valid configuration even with degenerate values.
    let config = Config::load(atlas_dir).expect("load config");
    assert!(!config.memory.decay.enabled);
}

// ── ICM-D1 — memory.wake_up budget config ────────────────────────────────────

#[test]
fn wake_up_defaults_load_without_a_memory_section() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path();
    fs::write(
        atlas_dir.join(crate::paths::ATLAS_CONFIG),
        "[build]\nparse_batch_size = 32\n",
    )
    .expect("write config");

    let config = Config::load(atlas_dir).expect("load config");
    let wake_up = &config.memory.wake_up;
    assert_eq!(wake_up.max_items, 10);
    assert_eq!(wake_up.max_feedback_items, 3);
    assert_eq!(wake_up.max_pending_changes, 20);
}

#[test]
fn wake_up_partial_section_fills_missing_fields_with_defaults() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path();
    fs::write(
        atlas_dir.join(crate::paths::ATLAS_CONFIG),
        "[memory.wake_up]\nmax_items = 5\n",
    )
    .expect("write config");

    let config = Config::load(atlas_dir).expect("load config");
    let wake_up = &config.memory.wake_up;
    assert_eq!(wake_up.max_items, 5);
    assert_eq!(
        wake_up.max_feedback_items, 3,
        "absent fields must fall back to defaults"
    );
    assert_eq!(wake_up.max_pending_changes, 20);
}

#[test]
fn wake_up_rejects_out_of_range_budget_values() {
    for (key, value, expected) in [
        ("max_items", "0", "memory.wake_up.max_items"),
        ("max_items", "26", "memory.wake_up.max_items"),
        (
            "max_feedback_items",
            "0",
            "memory.wake_up.max_feedback_items",
        ),
        (
            "max_feedback_items",
            "11",
            "memory.wake_up.max_feedback_items",
        ),
        (
            "max_pending_changes",
            "0",
            "memory.wake_up.max_pending_changes",
        ),
        (
            "max_pending_changes",
            "101",
            "memory.wake_up.max_pending_changes",
        ),
    ] {
        let dir = tempdir().expect("tempdir");
        let atlas_dir = dir.path();
        fs::write(
            atlas_dir.join(crate::paths::ATLAS_CONFIG),
            format!("[memory.wake_up]\n{key} = {value}\n"),
        )
        .expect("write config");

        let error = Config::load(atlas_dir).expect_err("invalid budget must fail");
        assert!(
            error.to_string().contains(expected),
            "error must name the field: {error}"
        );
    }
}

// ── ICM-C3 — analysis.feedback_adjustment ─────────────────────────────────────

#[test]
fn feedback_adjustment_defaults_to_enabled_without_an_analysis_section() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path();
    fs::write(
        atlas_dir.join(crate::paths::ATLAS_CONFIG),
        "[build]\nparse_batch_size = 16\n",
    )
    .expect("write config");

    let config = Config::load(atlas_dir).expect("load config");
    assert!(
        config.analysis.feedback_adjustment.enabled,
        "feedback adjustment must default to enabled; empty feedback keeps outputs stable"
    );
}

#[test]
fn feedback_adjustment_flag_can_be_disabled_explicitly() {
    let dir = tempdir().expect("tempdir");
    let atlas_dir = dir.path();
    fs::write(
        atlas_dir.join(crate::paths::ATLAS_CONFIG),
        "[analysis.feedback_adjustment]\nenabled = false\n",
    )
    .expect("write config");

    let config = Config::load(atlas_dir).expect("load config");
    assert!(!config.analysis.feedback_adjustment.enabled);
    // Other analysis defaults still apply.
    assert_eq!(config.analysis.dead_code_certainty_threshold, "low");
}
