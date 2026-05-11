use super::*;

fn write_atlas_config(repo: &Path, contents: &str) {
    let atlas_dir = repo.join(".atlas");
    std::fs::create_dir_all(&atlas_dir).expect("create .atlas dir");
    std::fs::write(atlas_dir.join("config.toml"), contents).expect("write atlas config");
}

#[test]
fn insights_large_functions_returns_structured_report() {
    let repo = setup_repo(&[
        (
            "src/lib.rs",
            "pub fn giant() {\n    let mut total = 0;\n    total += 1;\n    total += 2;\n    total += 3;\n    total += 4;\n    total += 5;\n    total += 6;\n    total += 7;\n    total += 8;\n    total += 9;\n    total += 10;\n}\n",
        ),
        (
            "tests/lib_test.rs",
            "#[test]\nfn giant_test() { assert_eq!(1, 1); }\n",
        ),
    ]);

    run_atlas(repo.path(), &["build"]);

    let output = run_atlas(
        repo.path(),
        &[
            "--json",
            "insights",
            "large-functions",
            "--threshold",
            "5",
            "--mode",
            "large",
        ],
    );
    let value = read_json_output(output);
    let data = &value["data"];

    assert_eq!(value["command"], json!("insights_large_functions"));
    assert_eq!(data["summary"]["total_findings"], json!(1));
    assert_eq!(data["findings"][0]["category"], json!("large_functions"));
    assert_eq!(data["findings"][0]["details"]["loc"], json!(13));
    assert_eq!(
        data["findings"][0]["details"]["qualified_name"],
        json!("src/lib.rs::fn::giant")
    );
}

#[test]
fn insights_complex_functions_returns_structured_report() {
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn knot(x: i32) -> i32 {\n    if x > 0 {\n        if x % 2 == 0 {\n            for value in 0..x {\n                if value == 3 || value == 4 {\n                    return value;\n                }\n            }\n        }\n    }\n    0\n}\n",
    )]);

    run_atlas(repo.path(), &["build"]);

    let output = run_atlas(
        repo.path(),
        &[
            "--json",
            "insights",
            "complex-functions",
            "--complexity-threshold",
            "3",
        ],
    );
    let value = read_json_output(output);
    let data = &value["data"];

    assert_eq!(value["command"], json!("insights_complex_functions"));
    assert_eq!(data["summary"]["total_findings"], json!(1));
    assert_eq!(data["findings"][0]["category"], json!("large_functions"));
    assert_eq!(
        data["findings"][0]["details"]["qualified_name"],
        json!("src/lib.rs::fn::knot")
    );
}

#[test]
fn insights_architecture_reports_layer_violations_from_config() {
    let repo = setup_repo(&[
        ("src/api/index.js", "export function dto() { return 1; }\n"),
        (
            "src/domain/index.js",
            "import { dto } from \"../api/index.js\";\nexport function service() { return dto(); }\n",
        ),
    ]);
    write_atlas_config(
        repo.path(),
        "[insights]\nmax_findings = 10\n\n[[insights.layer_rules]]\nname = \"api\"\npath_prefixes = [\"src/api\"]\nmodule_prefixes = []\n\n[[insights.layer_rules]]\nname = \"domain\"\npath_prefixes = [\"src/domain\"]\nmodule_prefixes = []\n",
    );

    run_atlas(repo.path(), &["build"]);

    let output = run_atlas(repo.path(), &["--json", "insights", "architecture"]);
    let value = read_json_output(output);
    let data = &value["data"];

    assert_eq!(value["command"], json!("insights_architecture"));
    assert!(
        data["findings"]
            .as_array()
            .expect("findings array")
            .iter()
            .any(|finding| finding["category"] == json!("layer_violation"))
    );
}

#[test]
fn insights_metrics_reports_threshold_findings_from_config() {
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn giant() {\n    let mut total = 0;\n    total += 1;\n    total += 2;\n    total += 3;\n    total += 4;\n    total += 5;\n    total += 6;\n}\n",
    )]);
    write_atlas_config(
        repo.path(),
        "[insights]\nlarge_function_loc = 5\nmax_findings = 10\n",
    );

    run_atlas(repo.path(), &["build"]);

    let output = run_atlas(repo.path(), &["--json", "insights", "metrics"]);
    let value = read_json_output(output);
    let data = &value["data"];

    assert_eq!(value["command"], json!("insights_metrics"));
    assert!(
        data["findings"]
            .as_array()
            .expect("findings array")
            .iter()
            .any(|finding| finding["category"] == json!("node_metrics"))
    );
}

#[test]
fn insights_risk_returns_structured_report() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["build"]);

    let output = run_atlas(
        repo.path(),
        &["--json", "insights", "risk", "src/lib.rs::fn::helper"],
    );
    let value = read_json_output(output);
    let data = &value["data"];

    assert_eq!(value["command"], json!("insights_risk"));
    assert_eq!(data["summary"]["total_findings"], json!(1));
    assert_eq!(data["findings"][0]["category"], json!("risk"));
    assert_eq!(
        data["findings"][0]["details"]["qualified_name"],
        json!("src/lib.rs::fn::helper")
    );
}

#[test]
fn insights_patterns_returns_structured_report() {
    let repo = setup_repo(&[
        (
            "src/a.rs",
            "pub fn entry_a() { parse(); }\npub fn parse() { save(); }\npub fn save() {}\n",
        ),
        (
            "src/b.rs",
            "pub fn entry_b() { parse(); }\npub fn parse() { save(); }\npub fn save() {}\n",
        ),
    ]);

    run_atlas(repo.path(), &["build"]);

    let output = run_atlas(repo.path(), &["--json", "insights", "patterns"]);
    let value = read_json_output(output);
    let data = &value["data"];

    assert_eq!(value["command"], json!("insights_patterns"));
    assert!(
        data["findings"]
            .as_array()
            .expect("findings array")
            .iter()
            .any(|finding| finding["category"] == json!("pattern_repeated_chain"))
    );
}

#[test]
fn insights_large_functions_cli_and_mcp_share_report() {
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn giant() {\n    let mut total = 0;\n    total += 1;\n    total += 2;\n    total += 3;\n    total += 4;\n    total += 5;\n    total += 6;\n    total += 7;\n    total += 8;\n    total += 9;\n    total += 10;\n}\n",
    )]);

    run_atlas(repo.path(), &["build"]);

    let cli_report = read_json_data_output(
        "insights_large_functions",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "insights",
                "large-functions",
                "--threshold",
                "5",
                "--mode",
                "large",
            ],
        ),
    );

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"find_large_functions\",\"arguments\":{\"threshold\":5,\"mode\":\"large\",\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(output.status.success(), "atlas serve find_large_functions failed");

    let mcp_report = read_json_tool_result(&output, 2);
    assert_eq!(
        cli_report["summary"]["total_findings"],
        mcp_report["summary"]["total_findings"]
    );
    assert_eq!(
        cli_report["findings"][0]["details"]["qualified_name"],
        mcp_report["findings"][0]["details"]["qualified_name"]
    );

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn insights_risk_cli_and_mcp_share_report() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["build"]);

    let cli_report = read_json_data_output(
        "insights_risk",
        run_atlas(
            repo.path(),
            &["--json", "insights", "risk", "src/lib.rs::fn::helper"],
        ),
    );

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"assess_risk\",\"arguments\":{\"symbol\":\"src/lib.rs::fn::helper\",\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(output.status.success(), "atlas serve assess_risk failed");

    let mcp_report = read_json_tool_result(&output, 2);
    assert_eq!(
        cli_report["summary"]["total_findings"],
        mcp_report["summary"]["total_findings"]
    );
    assert_eq!(
        cli_report["findings"][0]["details"]["qualified_name"],
        mcp_report["findings"][0]["details"]["qualified_name"]
    );

    cleanup_mcp_daemons(repo.path());
}
