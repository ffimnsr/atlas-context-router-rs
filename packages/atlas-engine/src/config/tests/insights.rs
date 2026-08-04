//! Insights config validation, layer-rules, and external file handling tests.

use super::super::*;
use super::*;
use std::fs;

#[test]
fn insights_config_rejects_non_positive_thresholds() {
    let mut config = Config::default();
    config.insights.max_findings = 0;

    let err = config
        .insights_config()
        .expect_err("invalid insights config");
    assert!(
        err.to_string()
            .contains("insights.max_findings must be greater than 0")
    );
}

#[test]
fn insights_config_rejects_outlier_percentile_above_100() {
    let mut config = Config::default();
    config.insights.outlier_percentile_cutoff = 101;

    let err = config
        .insights_config()
        .expect_err("invalid insights percentile cutoff");
    assert!(
        err.to_string()
            .contains("insights.outlier_percentile_cutoff=101 exceeds safe maximum 100")
    );
}

#[test]
fn insights_config_rejects_invalid_risk_threshold_order() {
    let mut config = Config::default();
    config.insights.risk_medium_threshold = 80.0;
    config.insights.risk_high_threshold = 70.0;

    let err = config
        .insights_config()
        .expect_err("invalid risk threshold order");
    assert!(err.to_string().contains(
        "insights.risk_medium_threshold (80) must be less than insights.risk_high_threshold (70)"
    ));
}

#[test]
fn insights_config_rejects_non_positive_risk_weight() {
    let mut config = Config::default();
    config.insights.risk_unresolved_edge_weight = 0.0;

    let err = config.insights_config().expect_err("invalid risk weight");
    assert!(
        err.to_string()
            .contains("insights.risk_unresolved_edge_weight must be a finite value greater than 0")
    );
}

#[test]
fn load_rejects_invalid_insights_layer_rule() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[insights]\nmax_findings = 10\n\n[[insights.layer_rules]]\nname = \"app\"\n",
    )
    .expect("write config");

    let err = Config::load(dir.path()).expect_err("invalid layer rule");
    assert!(
        err.to_string()
            .contains("insights.layer_rules[0] must define path_prefixes or module_prefixes")
    );
}

#[test]
fn load_accepts_valid_external_redaction_rules_file() {
    let dir = tempdir().expect("tempdir");
    fs::write(
            dir.path().join("redaction-rules.toml"),
            "token_prefixes = [\"zz-\"]\nsecret_key_patterns = [\"sessionid\"]\ntoken_min_len = 3\nhex_secret_min_len = 16\nbase64_secret_min_len = 20\n",
        )
        .expect("write rules");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[sanitization]\nredaction_rules_file = \"redaction-rules.toml\"\n",
    )
    .expect("write config");

    let config = Config::load(dir.path()).expect("config should load");
    let resolved = config
        .resolve_redaction_rules_file(dir.path())
        .expect("resolve path")
        .expect("configured path");
    assert!(resolved.ends_with("redaction-rules.toml"));
}

#[test]
fn load_rejects_missing_external_redaction_rules_file() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[sanitization]\nredaction_rules_file = \"missing-rules.toml\"\n",
    )
    .expect("write config");

    let err = Config::load(dir.path()).expect_err("missing rules must fail");
    assert!(
        err.to_string()
            .contains("sanitization.redaction_rules_file points to missing file")
    );
}

#[test]
fn load_rejects_unreadable_external_redaction_rules_file() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("rules-dir")).expect("create rules dir");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[sanitization]\nredaction_rules_file = \"rules-dir\"\n",
    )
    .expect("write config");

    let err = Config::load(dir.path()).expect_err("directory path must fail");
    assert!(
        err.to_string()
            .contains("sanitization.redaction_rules_file must point to a readable file")
    );
}

#[test]
fn load_rejects_malformed_external_redaction_rules_file() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("redaction-rules.toml"),
        "token_prefixes = [",
    )
    .expect("write malformed rules");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[sanitization]\nredaction_rules_file = \"redaction-rules.toml\"\n",
    )
    .expect("write config");

    let err = Config::load(dir.path()).expect_err("malformed rules must fail");
    let message = err.to_string();
    assert!(message.contains("sanitization.redaction_rules_file"));
    assert!(
        message.contains("cannot parse redaction rules file")
            || message.contains("failed validation")
    );
}

#[test]
fn load_accepts_valid_external_layer_rules_file() {
    let dir = tempdir().expect("tempdir");
    fs::write(
            dir.path().join("layer-rules.toml"),
            "[[layer_rules]]\nname = \"api\"\npath_prefixes = [\"src/api\"]\nmodule_prefixes = []\n\n[[layer_rules]]\nname = \"domain\"\npath_prefixes = [\"src/domain\"]\nmodule_prefixes = []\n",
        )
        .expect("write rules");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[insights]\nmax_findings = 10\nlayer_rules_file = \"layer-rules.toml\"\n",
    )
    .expect("write config");

    let config = Config::load(dir.path()).expect("config should load");
    let resolved = config
        .insights
        .resolve_layer_rules_file(dir.path())
        .expect("resolve path")
        .expect("configured path");
    assert!(resolved.ends_with("layer-rules.toml"));
    let rules = config
        .insights
        .effective_layer_rules(dir.path())
        .expect("effective rules");
    assert_eq!(rules.len(), 2);
    assert_eq!(rules[0].name, "api");
    assert_eq!(rules[1].name, "domain");
}

#[test]
fn with_loaded_layer_rules_prefers_external_file_over_inline() {
    let dir = tempdir().expect("tempdir");
    fs::write(
            dir.path().join("layer-rules.toml"),
            "[[layer_rules]]\nname = \"external\"\npath_prefixes = [\"src/ext\"]\nmodule_prefixes = []\n",
        )
        .expect("write rules");

    let mut config = Config::default();
    config.insights.layer_rules = vec![InsightsLayerRule {
        name: "inline".to_owned(),
        path_prefixes: vec!["src/inline".to_owned()],
        module_prefixes: vec![],
    }];
    config.insights.layer_rules_file = Some("layer-rules.toml".to_owned());

    let loaded = config
        .insights
        .with_loaded_layer_rules(dir.path())
        .expect("loaded rules");
    assert_eq!(loaded.layer_rules.len(), 1);
    assert_eq!(loaded.layer_rules[0].name, "external");
}

#[test]
fn load_external_layer_rules_file_ignores_invalid_inline_rules() {
    let dir = tempdir().expect("tempdir");
    fs::write(
            dir.path().join("layer-rules.toml"),
            "[[layer_rules]]\nname = \"external\"\npath_prefixes = [\"src/ext\"]\nmodule_prefixes = []\n",
        )
        .expect("write rules");
    fs::write(
            dir.path().join(crate::paths::ATLAS_CONFIG),
            "[insights]\nlayer_rules_file = \"layer-rules.toml\"\n\n[[insights.layer_rules]]\nname = \"\"\npath_prefixes = []\nmodule_prefixes = []\n",
        )
        .expect("write config");

    let config = Config::load(dir.path()).expect("external rules should replace inline rules");
    let rules = config
        .insights
        .effective_layer_rules(dir.path())
        .expect("effective rules");
    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].name, "external");
}

#[test]
fn with_loaded_layer_rules_keeps_inline_rules_when_file_unset() {
    let dir = tempdir().expect("tempdir");
    let mut config = Config::default();
    config.insights.layer_rules = vec![InsightsLayerRule {
        name: "inline".to_owned(),
        path_prefixes: vec!["src/inline".to_owned()],
        module_prefixes: vec![],
    }];

    let loaded = config
        .insights
        .with_loaded_layer_rules(dir.path())
        .expect("loaded rules");
    assert_eq!(loaded.layer_rules.len(), 1);
    assert_eq!(loaded.layer_rules[0].name, "inline");
}

#[test]
fn load_rejects_missing_external_layer_rules_file() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[insights]\nlayer_rules_file = \"missing-rules.toml\"\n",
    )
    .expect("write config");

    let err = Config::load(dir.path()).expect_err("missing rules must fail");
    assert!(
        err.to_string()
            .contains("insights.layer_rules_file points to missing file")
    );
}

#[test]
fn load_rejects_directory_external_layer_rules_file() {
    let dir = tempdir().expect("tempdir");
    fs::create_dir_all(dir.path().join("rules-dir")).expect("create rules dir");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[insights]\nlayer_rules_file = \"rules-dir\"\n",
    )
    .expect("write config");

    let err = Config::load(dir.path()).expect_err("directory path must fail");
    assert!(
        err.to_string()
            .contains("insights.layer_rules_file must point to a readable file")
    );
}

#[cfg(unix)]
#[test]
fn load_rejects_unreadable_external_layer_rules_file() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempdir().expect("tempdir");
    let rules_path = dir.path().join("layer-rules.toml");
    fs::write(
        &rules_path,
        "[[layer_rules]]\nname = \"api\"\npath_prefixes = [\"src/api\"]\nmodule_prefixes = []\n",
    )
    .expect("write rules");
    let mut permissions = fs::metadata(&rules_path)
        .expect("rules metadata")
        .permissions();
    permissions.set_mode(0o000);
    fs::set_permissions(&rules_path, permissions).expect("remove read permission");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[insights]\nlayer_rules_file = \"layer-rules.toml\"\n",
    )
    .expect("write config");

    let err = Config::load(dir.path()).expect_err("unreadable rules must fail");
    assert!(err.to_string().contains("insights.layer_rules_file"));
    assert!(
        err.chain()
            .any(|cause| { cause.to_string().contains("cannot read layer-rules file") })
    );
}

#[test]
fn load_rejects_malformed_external_layer_rules_file() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("layer-rules.toml"),
        "[[layer_rules]]\nname = [",
    )
    .expect("write malformed rules");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[insights]\nlayer_rules_file = \"layer-rules.toml\"\n",
    )
    .expect("write config");

    let err = Config::load(dir.path()).expect_err("malformed rules must fail");
    let message = err.to_string();
    assert!(message.contains("insights.layer_rules_file"));
    assert!(
        message.contains("cannot parse layer-rules file") || message.contains("failed validation")
    );
}

#[test]
fn load_rejects_blank_external_layer_rules_file() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[insights]\nlayer_rules_file = \"\"\n",
    )
    .expect("write config");

    let err = Config::load(dir.path()).expect_err("blank path must fail");
    assert!(
        err.to_string()
            .contains("insights.layer_rules_file must not be empty")
    );
}

#[test]
fn load_rejects_external_layer_rules_file_with_duplicate_names() {
    let dir = tempdir().expect("tempdir");
    fs::write(
            dir.path().join("layer-rules.toml"),
            "[[layer_rules]]\nname = \"api\"\npath_prefixes = [\"src/api\"]\nmodule_prefixes = []\n\n[[layer_rules]]\nname = \"api\"\npath_prefixes = [\"src/other\"]\nmodule_prefixes = []\n",
        )
        .expect("write rules");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[insights]\nlayer_rules_file = \"layer-rules.toml\"\n",
    )
    .expect("write config");

    let err = Config::load(dir.path()).expect_err("duplicate names must fail");
    assert!(err.to_string().contains("insights.layer_rules_file"));
    assert!(err.chain().any(|cause| {
        cause
            .to_string()
            .contains("insights.layer_rules[1].name duplicates layer `api`")
    }));
}

#[test]
fn load_rejects_external_layer_rules_file_with_empty_matchers() {
    let dir = tempdir().expect("tempdir");
    fs::write(
        dir.path().join("layer-rules.toml"),
        "[[layer_rules]]\nname = \"app\"\npath_prefixes = []\nmodule_prefixes = []\n",
    )
    .expect("write rules");
    fs::write(
        dir.path().join(crate::paths::ATLAS_CONFIG),
        "[insights]\nlayer_rules_file = \"layer-rules.toml\"\n",
    )
    .expect("write config");

    let err = Config::load(dir.path()).expect_err("empty matchers must fail");
    assert!(err.to_string().contains("insights.layer_rules_file"));
    assert!(err.chain().any(|cause| {
        cause
            .to_string()
            .contains("insights.layer_rules[0] must define path_prefixes or module_prefixes")
    }));
}
