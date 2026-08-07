//! Config template rendering and profile write tests.

use super::super::*;
use super::*;
use std::fs;

#[test]
fn render_template_minimal_comments_all_keys() {
    let template = Config::render_template(ConfigTemplateProfile::Minimal).expect("template");

    assert!(template.contains("# parse_batch_size = 64"));
    assert!(template.contains("[search.embedding]\n# url = \"http://localhost:11434\""));
    assert!(template.contains("[insights]\n# large_function_loc = 80"));
    assert!(template.contains("# repeated_call_chain_min_length = 3"));
    assert!(template.contains("# outlier_percentile_cutoff = 95"));
    assert!(template.contains("# [[insights.layer_rules]]\n# name = \"layer_1\""));
    assert!(template.contains("# layer_rules_file = \"layer-rules.toml\""));
    assert!(template.contains("[sanitization]\n# redaction_rules_file = \"redaction-rules.toml\""));
    assert!(template.contains("# worker_threads = 2"));
    assert!(template.contains("[mcp.http_auth]\n# enabled = false"));
    assert!(!template.contains("\nparse_batch_size = 64\n"));
}

#[test]
fn rendered_minimal_template_loads_without_layer_rule_validation_error() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        Config::render_template(ConfigTemplateProfile::Minimal).expect("template"),
    )
    .expect("write config");

    Config::load(dir.path()).expect("minimal template should load");
}

#[test]
fn render_template_full_activates_keys() {
    let template = Config::render_template(ConfigTemplateProfile::Full).expect("template");

    assert!(template.contains("[build]\nparse_batch_size = 64"));
    assert!(template.contains(
        "tool_timeout_ms_by_tool = { build_or_update_graph = 900000, get_review_context = 120000 }"
    ));
    assert!(template.contains("hybrid_enabled = true"));
    assert!(template.contains("[search.embedding]\nurl = \"http://localhost:11434\""));
    assert!(template.contains("[insights]\nlarge_function_loc = 60"));
    assert!(template.contains("repeated_call_chain_min_length = 4"));
    assert!(template.contains("outlier_percentile_cutoff = 90"));
    assert!(template.contains("ignore_node_kinds = [\"import\"]"));
    assert!(template.contains("[[insights.layer_rules]]\nname = \"api\""));
    assert!(template.contains("# layer_rules_file = \"layer-rules.toml\""));
    assert!(template.contains("[sanitization]\nredaction_rules_file = \"\""));
    assert!(template.contains("[mcp.http_auth]\nenabled = true"));
    assert!(template.contains("required_scopes = { mcp = [\"atlas:mcp\", \"atlas:read\"] }"));
    // Tokenizer accounting: active heuristic defaults in the full profile.
    assert!(template.contains(
        "[context.tokenizer]\nprovider = \"heuristic\"\nmodel = \"\"\ntokenizer_file = \"\"\nfallback = \"heuristic\"\nbytes_per_token = 4"
    ));
}

#[test]
fn tokenizer_template_block_snapshot_matches_minimal_profile() {
    // Snapshot of the generated [context.tokenizer] block: active heuristic
    // defaults commented out, tokenizer-backed example values never active.
    let template = Config::render_template(ConfigTemplateProfile::Minimal).expect("template");
    assert!(template.contains(
        "[context.tokenizer]\n# provider = \"heuristic\"\n# model = \"cl100k_base\"\n# tokenizer_file = \"tokenizer.json\"\n# fallback = \"heuristic\"\n# bytes_per_token = 4"
    ));
}

#[test]
fn readme_describes_tokenizer_budget_accounting() {
    // Docs drift guard: the README must keep describing tokenizer-backed
    // budget accounting whenever the config surface changes.
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../README.md"))
        .expect("read README");
    assert!(
        readme.contains("### Payload budgets"),
        "README payload budgets section"
    );
    assert!(
        readme.contains("[context.tokenizer]"),
        "README tokenizer config block"
    );
    assert!(
        readme.contains("provider = \"tokenizers\""),
        "README tokenizer mode"
    );
    assert!(
        readme.contains("fail_closed"),
        "README fail-closed behavior"
    );
    assert!(
        readme.contains("token_accounting"),
        "README fallback metadata"
    );
    assert!(
        readme.contains("never downloads"),
        "README local-file-only rule"
    );
}

#[test]
fn write_template_uses_selected_profile() {
    let dir = tempdir().expect("tempdir");
    let created =
        Config::write_template(dir.path(), ConfigTemplateProfile::Full).expect("write template");

    assert!(created);
    let text =
        fs::read_to_string(dir.path().join(crate::paths::ATLAS_CONFIG)).expect("read config");
    assert!(text.contains("# profile = \"full\""));
    assert!(text.contains("hybrid_enabled = true"));
    assert!(text.contains("url = \"http://localhost:11434\""));
    assert!(text.contains("large_function_loc = 60"));
    assert!(text.contains("repeated_call_chain_min_length = 4"));
}
