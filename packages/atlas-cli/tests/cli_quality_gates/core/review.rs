use super::*;

#[test]
fn review_context_includes_workflow_summary_and_call_chains() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    rewrite_fixture_helper(repo.path());

    let review_ctx = read_json_data_output(
        "review_context",
        run_atlas(repo.path(), &["--json", "review-context", "--base", "HEAD"]),
    );

    assert!(review_ctx["workflow"].is_object(), "workflow block missing");
    assert!(
        review_ctx["workflow"]["high_impact_nodes"]
            .as_array()
            .expect("high impact nodes array")
            .iter()
            .any(|node| node["qualified_name"].is_string()),
        "expected high-impact nodes in workflow summary: {review_ctx:?}"
    );
    assert!(
        review_ctx["workflow"]["call_chains"]
            .as_array()
            .expect("call chains array")
            .iter()
            .any(|chain| chain["summary"].as_str().unwrap_or_default().contains("->")),
        "expected call chain summary in review context: {review_ctx:?}"
    );
    assert!(review_ctx["workflow"]["noise_reduction"]["rules_applied"].is_array());
}

#[test]
fn review_context_changed_file_flow_stays_bounded_and_keeps_useful_neighbors() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    rewrite_fixture_helper(repo.path());

    let review_ctx = read_json_data_output(
        "review_context",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "review-context",
                "--base",
                "HEAD",
                "--max-nodes",
                "4",
            ],
        ),
    );

    let nodes = review_ctx["nodes"].as_array().expect("review nodes array");
    assert!(
        nodes.len() <= 4,
        "review-context must stay bounded by requested max-nodes: {review_ctx:?}"
    );
    assert!(
        nodes.iter().any(|node| {
            node["selection_reason"] == json!("direct_target")
                && node["node"]["qualified_name"] == json!("src/lib.rs::fn::helper")
        }),
        "review-context must keep changed symbol helper as direct target: {review_ctx:?}"
    );
    assert!(
        nodes.iter().any(|node| {
            node["selection_reason"] == json!("impact_neighbor")
                && node["node"]["qualified_name"] == json!("src/main.rs::fn::main")
        }),
        "review-context must retain useful caller neighbor from changed-file flow: {review_ctx:?}"
    );
    assert!(
        review_ctx["files"]
            .as_array()
            .expect("review files array")
            .iter()
            .any(|file| file["path"] == json!("src/main.rs")),
        "review-context files must include impacted caller file: {review_ctx:?}"
    );
    assert!(
        review_ctx["workflow"]["call_chains"]
            .as_array()
            .expect("call chains array")
            .iter()
            .any(|chain| {
                chain["summary"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("src/main.rs::fn::main -> src/lib.rs::fn::helper")
            }),
        "review-context workflow must surface useful changed-file call chain: {review_ctx:?}"
    );
    assert!(review_ctx["context_ranking_evidence_legend"].is_object());
    assert!(
        review_ctx["nodes"]
            .as_array()
            .expect("review nodes array")
            .iter()
            .find(|node| node["selection_reason"] == json!("direct_target"))
            .and_then(|node| node["context_ranking_evidence"]["changed_symbol"].as_bool())
            == Some(true),
        "review-context must expose changed-symbol evidence for direct targets: {review_ctx:?}"
    );
}

#[test]
fn review_context_cross_package_changed_file_flow_surfaces_useful_focus() {
    let repo = setup_repo(&[
        (
            "package.json",
            r#"{"private":true,"workspaces":["apps/*","packages/*"]}"#,
        ),
        (
            "tsconfig.json",
            r#"{
    "compilerOptions": {
        "baseUrl": ".",
        "paths": {
            "@ui/*": ["packages/ui/src/*"]
        }
    }
}
"#,
        ),
        ("apps/web/package.json", r#"{"name":"web","version":"0.1.0"}"#),
        (
            "apps/web/src/app.ts",
            "import { helper } from '@ui/helper';\nexport function run(): string {\n    return helper();\n}\n",
        ),
        ("packages/ui/package.json", r#"{"name":"ui","version":"0.1.0"}"#),
        (
            "packages/ui/src/helper.ts",
            "export function helper(): string {\n    return 'v1';\n}\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    write_repo_file(
        repo.path(),
        "packages/ui/src/helper.ts",
        "export function helper(): string {\n    return 'v2';\n}\n",
    );

    let review_ctx = read_json_data_output(
        "review_context",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "review-context",
                "--base",
                "HEAD",
                "--max-nodes",
                "5",
            ],
        ),
    );

    assert!(
        review_ctx["nodes"]
            .as_array()
            .expect("review nodes array")
            .iter()
            .any(|node| {
                node["selection_reason"] == json!("direct_target")
                    && node["node"]["qualified_name"]
                        == json!("packages/ui/src/helper.ts::fn::helper")
            }),
        "review-context must keep changed cross-package target helper: {review_ctx:?}"
    );
    assert!(
        review_ctx["nodes"]
            .as_array()
            .expect("review nodes array")
            .iter()
            .any(|node| {
                node["selection_reason"] == json!("impact_neighbor")
                    && node["node"]["qualified_name"] == json!("apps/web/src/app.ts::fn::run")
            }),
        "review-context must surface impacted cross-package caller: {review_ctx:?}"
    );
    assert!(
        review_ctx["files"]
            .as_array()
            .expect("review files array")
            .iter()
            .any(|file| file["path"] == json!("apps/web/src/app.ts")),
        "review-context must include impacted application file: {review_ctx:?}"
    );
    assert!(
        review_ctx["workflow"]["high_impact_nodes"]
            .as_array()
            .expect("high impact nodes array")
            .iter()
            .any(|node| node["qualified_name"] == json!("apps/web/src/app.ts::fn::run")),
        "workflow summary must keep impacted cross-package caller in focus: {review_ctx:?}"
    );
    assert!(
        review_ctx["workflow"]["ripple_effects"]
            .as_array()
            .expect("ripple effects array")
            .iter()
            .any(|item| {
                let text = item.as_str().unwrap_or_default();
                text.contains("Impact spans") || text.contains("neighboring file")
            }),
        "workflow summary must explain why cross-package change matters: {review_ctx:?}"
    );
}

#[test]
fn review_context_markdown_profile_is_pr_comment_friendly() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    rewrite_fixture_helper(repo.path());

    let review = stdout_text(&run_atlas(
        repo.path(),
        &[
            "review-context",
            "--base",
            "HEAD",
            "--format",
            "markdown",
        ],
    ));

    assert_contains_all(
        &review,
        &[
            "## Atlas Review Context",
            "### Summary",
            "<details>",
            "<summary>Changed files</summary>",
            "### Changed Symbols",
            "### Critical Paths",
            "```text",
            "src/main.rs::fn::main -> src/lib.rs::fn::helper",
        ],
    );
}

#[test]
fn impact_reports_test_signal_for_changed_symbol() {
    let repo = setup_repo(&[
        (
            "Cargo.toml",
            "[package]\nname = 'demo'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        (
            "src/lib.rs",
            "pub fn helper() -> i32 {\n    1\n}\n\n#[cfg(test)]\nmod tests {\n    use super::helper;\n\n    #[test]\n    fn helper_works() {\n        assert_eq!(helper(), 1);\n    }\n}\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    write_repo_file(
        repo.path(),
        "src/lib.rs",
        "pub fn helper() -> i32 {\n    2\n}\n\n#[cfg(test)]\nmod tests {\n    use super::helper;\n\n    #[test]\n    fn helper_works() {\n        assert_eq!(helper(), 2);\n    }\n}\n",
    );

    let impact = read_json_data_output(
        "impact",
        run_atlas(repo.path(), &["--json", "impact", "--base", "HEAD"]),
    );

    assert!(
        impact["analysis"]["test_impact"]["affected_tests"]
            .as_array()
            .expect("affected tests array")
            .iter()
            .any(|node| node["is_test"] == json!(true)),
        "impact must report affected tests for covered changed symbol: {impact:?}"
    );
    assert!(
        impact["analysis"]["scored_nodes"]
            .as_array()
            .expect("scored nodes array")
            .iter()
            .any(|node| node["node"]["is_test"] == json!(true)),
        "impact scores must carry test-adjacent nodes: {impact:?}"
    );
}

#[test]
fn impact_reports_boundary_and_uncovered_signals_for_cross_package_change() {
    let repo = setup_repo(&[
        (
            "package.json",
            r#"{"private":true,"workspaces":["apps/*","packages/*"]}"#,
        ),
        (
            "tsconfig.json",
            r#"{
    "compilerOptions": {
        "baseUrl": ".",
        "paths": {
            "@ui/*": ["packages/ui/src/*"]
        }
    }
}
"#,
        ),
        ("apps/web/package.json", r#"{"name":"web","version":"0.1.0"}"#),
        (
            "apps/web/src/app.ts",
            "import { helper } from '@ui/helper';\nexport function run(): string {\n    return helper();\n}\n",
        ),
        ("packages/ui/package.json", r#"{"name":"ui","version":"0.1.0"}"#),
        (
            "packages/ui/src/helper.ts",
            "export function helper(): string {\n    return 'v1';\n}\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    write_repo_file(
        repo.path(),
        "packages/ui/src/helper.ts",
        "export function helper(): string {\n    return 'v2';\n}\n",
    );

    let impact = read_json_data_output(
        "impact",
        run_atlas(repo.path(), &["--json", "impact", "--base", "HEAD"]),
    );

    assert!(
        impact["analysis"]["boundary_violations"]
            .as_array()
            .expect("boundary violations array")
            .iter()
            .any(|violation| violation["kind"] == json!("cross_package")),
        "impact must surface cross-package boundary signal: {impact:?}"
    );
    assert!(
        impact["analysis"]["test_impact"]["uncovered_changed_nodes"]
            .as_array()
            .expect("uncovered changed nodes array")
            .iter()
            .any(|node| node["qualified_name"] == json!("packages/ui/src/helper.ts::fn::helper")),
        "changed helper without tests must be flagged uncovered: {impact:?}"
    );
    assert!(
        matches!(impact["analysis"]["risk_level"].as_str(), Some("high") | Some("critical")),
        "cross-package untested change must elevate risk: {impact:?}"
    );
}

#[test]
fn analyze_dead_code_subpath_filters_candidates() {
    let repo = setup_repo(&[
        (
            "src/lib.rs",
            "mod a;\n\npub fn caller() {\n    a::live();\n}\n",
        ),
        ("src/a.rs", "pub fn live() {}\n\nfn unused_in_src() {}\n"),
        ("examples/demo.rs", "fn unused_in_examples() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let candidates = read_json_data_output(
        "analyze_dead_code",
        run_atlas(
            repo.path(),
            &["--json", "analyze", "dead-code", "--subpath", "src"],
        ),
    );
    let candidates = candidates.as_array().expect("dead-code candidates array");

    assert!(!candidates.is_empty(), "expected dead-code candidates");
    assert!(candidates.iter().all(|candidate| {
        candidate["node"]["file_path"]
            .as_str()
            .unwrap_or_default()
            .starts_with("src")
    }));
    assert!(candidates.iter().any(|candidate| {
        candidate["node"]["qualified_name"] == json!("src/a.rs::fn::unused_in_src")
    }));
    assert!(candidates.iter().all(|candidate| {
        candidate["node"]["qualified_name"] != json!("examples/demo.rs::fn::unused_in_examples")
    }));
}

#[test]
fn refactor_rename_named_flags_support_dry_run() {
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn helper() -> i32 {\n    1\n}\n\npub fn caller() -> i32 {\n    helper()\n}\n",
    )]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let result = read_json_data_output(
        "refactor_rename",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "refactor",
                "rename",
                "--symbol",
                "src/lib.rs::fn::helper",
                "--to",
                "helper_renamed",
                "--dry-run",
            ],
        ),
    );

    assert_eq!(result["dry_run"], json!(true));
    assert_eq!(result["files_changed"], json!(1));
    assert!(
        result["patches"]
            .as_array()
            .expect("patch array")
            .iter()
            .any(|patch| patch["unified_diff"]
                .as_str()
                .unwrap_or_default()
                .contains("helper_renamed")),
        "rename dry-run must include renamed identifier in patch: {result:?}"
    );

    let after = fs::read_to_string(repo.path().join("src/lib.rs")).expect("read source after dry-run");
    assert!(after.contains("pub fn helper() -> i32"));
    assert!(after.contains("helper()"));
}

#[test]
fn refactor_rename_accepts_alias_qname() {
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn helper() -> i32 {\n    1\n}\n\npub fn caller() -> i32 {\n    helper()\n}\n",
    )]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let result = read_json_data_output(
        "refactor_rename",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "refactor",
                "rename",
                "--symbol",
                "src/lib.rs::function::helper",
                "--to",
                "helper_renamed",
                "--dry-run",
            ],
        ),
    );

    assert_eq!(result["plan"]["operation"]["kind"], json!("rename_symbol"));
    assert_eq!(result["plan"]["operation"]["old_qname"], json!("src/lib.rs::fn::helper"));
}

#[test]
fn refactor_remove_dead_accepts_alias_qname() {
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn caller() -> i32 {\n    1\n}\n\nfn unused_helper() -> i32 {\n    2\n}\n",
    )]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let result = read_json_data_output(
        "refactor_remove_dead",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "refactor",
                "remove-dead",
                "src/lib.rs::function::unused_helper",
                "--dry-run",
            ],
        ),
    );

    assert_eq!(result["dry_run"], json!(true));
    assert_eq!(result["plan"]["operation"]["kind"], json!("remove_dead_code"));
    assert_eq!(result["plan"]["operation"]["target_qname"], json!("src/lib.rs::fn::unused_helper"));
    assert!(
        result["patches"]
            .as_array()
            .expect("patch array")
            .iter()
            .any(|patch| patch["unified_diff"]
                .as_str()
                .unwrap_or_default()
                .contains("unused_helper")),
        "remove-dead dry-run must include removed symbol in patch: {result:?}"
    );
}

#[test]
fn impact_review_context_and_query_cover_phase_14_5_cli_flow() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    rewrite_fixture_helper(repo.path());
    run_atlas(repo.path(), &["update", "--base", "HEAD"]);

    let impact = stdout_text(&run_atlas(repo.path(), &["impact", "--base", "HEAD"]));
    assert_contains_all(
        &impact,
        &[
            "Changed files : 1",
            "Changed nodes :",
            "Relevant edges:",
            "Risk level    : high",
            "struct src/lib.rs::struct::Greeter [api_change]",
            "method src/lib.rs::method::Greeter::greet_twice [signature_change]",
            "function src/lib.rs::fn::helper [signature_change]",
        ],
    );

    let review = stdout_text(&run_atlas(repo.path(), &["review-context", "--base", "HEAD"]));
    assert_contains_all(
        &review,
        &[
            "Changed files (1):",
            "  src/lib.rs",
            "Context summary:",
            "Changed symbols:",
            "function src/lib.rs::fn::helper",
            "High-impact nodes:",
            "Call chains:",
            "Noise reduction:",
        ],
    );

    let query = stdout_text(&run_atlas(repo.path(), &["query", "greet_twice"]));
    let query_lines: Vec<&str> = query.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(query_lines.len(), 2, "expected one ranked result and count");
    assert!(
        query_lines[0].contains("method src/lib.rs::method::Greeter::greet_twice"),
        "unexpected first query result: {query}"
    );
    assert!(query_lines[1].starts_with("1 result(s)."));
}
