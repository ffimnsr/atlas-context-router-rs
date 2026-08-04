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
