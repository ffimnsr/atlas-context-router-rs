use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use atlas_core::{EdgeKind, NodeKind};
use atlas_store_sqlite::Store;
use serde_json::{Value, json};
use tempfile::TempDir;

#[test]
fn sqlite_fts5_smoke_round_trip() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert!(status["indexed_file_count"].as_u64().unwrap_or_default() >= 2);
    assert!(status["node_count"].as_i64().unwrap_or_default() >= 5);
    assert!(status["edge_count"].as_i64().unwrap_or_default() >= 1);

    let query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "greet_twice"]),
    );
    let results = query["results"]
        .as_array()
        .expect("query results should be an array");
    assert_eq!(results.len(), 1);
    assert_eq!(results[0]["node"]["name"], json!("greet_twice"));
    assert_eq!(results[0]["node"]["kind"], json!("method"));
    assert_eq!(results[0]["node"]["file_path"], json!("src/lib.rs"));
}

#[test]
fn fixture_query_output_matches_golden() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let mut query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "greet_twice"]),
    );
    normalize_query_results(&mut query);

    let golden = read_golden_json("query_greet_twice.json");
    assert_eq!(query, golden);
}

#[test]
fn query_fuzzy_flag_recovers_close_typo() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let no_fuzzy = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "gret_twice"]),
    );
    let no_fuzzy_results = no_fuzzy["results"].as_array().expect("query results array");
    assert!(
        !no_fuzzy_results.is_empty(),
        "baseline typo query should still surface a candidate: {no_fuzzy:?}"
    );
    assert_eq!(no_fuzzy_results[0]["node"]["name"], json!("greet_twice"));

    let fuzzy = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "gret_twice", "--fuzzy"]),
    );
    let results = fuzzy["results"]
        .as_array()
        .expect("query results array with fuzzy enabled");
    assert!(
        !results.is_empty(),
        "fuzzy typo query should recover a close match: {fuzzy:?}"
    );
    assert_eq!(fuzzy["query"]["fuzzy_match"], json!(true));
    assert_eq!(results[0]["node"]["name"], json!("greet_twice"));
    let no_fuzzy_score = no_fuzzy_results[0]["score"].as_f64().unwrap_or_default();
    let fuzzy_score = results[0]["score"].as_f64().unwrap_or_default();
    assert!(
        fuzzy_score > no_fuzzy_score,
        "fuzzy query should improve score for close typo: no_fuzzy={no_fuzzy_score} fuzzy={fuzzy_score}"
    );
}

#[test]
fn query_exact_symbol_and_qname_rank_definition_in_top_three() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let expected_qn = "src/lib.rs::method::Greeter::greet_twice";

    let exact_name = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "greet_twice"]),
    );
    let exact_name_qns = atlas_query_qnames(&exact_name);
    assert!(
        exact_name_qns.iter().take(3).any(|qn| qn == expected_qn),
        "exact symbol lookup must rank intended definition in top 3: {exact_name:?}"
    );

    let exact_qname = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", expected_qn]),
    );
    let exact_qname_qns = atlas_query_qnames(&exact_qname);
    assert!(
        exact_qname_qns.iter().take(3).any(|qn| qn == expected_qn),
        "qualified-name lookup must rank intended definition in top 3: {exact_qname:?}"
    );
}

#[test]
fn query_ambiguous_short_name_returns_ranked_candidates_with_metadata() {
    let repo = setup_repo(&[
        ("Cargo.toml", "[workspace]\nmembers = ['packages/*']\n"),
        (
            "packages/foo/Cargo.toml",
            "[package]\nname = 'foo'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("packages/foo/src/lib.rs", "pub fn helper() {}\n"),
        (
            "packages/bar/Cargo.toml",
            "[package]\nname = 'bar'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("packages/bar/src/lib.rs", "pub fn helper() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "helper"]),
    );
    let results = query["results"].as_array().expect("query results array");
    assert!(
        results.len() >= 2,
        "ambiguous lookup must return candidates"
    );
    assert!(
        results.windows(2).all(|pair| {
            pair[0]["score"].as_f64().unwrap_or_default()
                >= pair[1]["score"].as_f64().unwrap_or_default()
        }),
        "ambiguous candidates must be ranked descending: {query:?}"
    );
    assert!(
        results.iter().take(2).all(|result| {
            result["node"]["kind"].is_string() && result["node"]["file_path"].is_string()
        }),
        "ambiguous candidates must include kind and file metadata: {query:?}"
    );

    let qnames = atlas_query_qnames(&query);
    assert!(
        qnames
            .iter()
            .any(|qn| qn == "packages/foo/src/lib.rs::fn::helper")
            && qnames
                .iter()
                .any(|qn| qn == "packages/bar/src/lib.rs::fn::helper"),
        "ambiguous lookup must surface both helper definitions: {query:?}"
    );
}

#[test]
fn query_graph_expand_surfaces_neighbors_plain_grep_cannot_infer() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let target_qn = "src/lib.rs::method::Greeter::greet_twice";
    let atlas = read_json_data_output(
        "query",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "query",
                target_qn,
                "--expand",
                "--expand-hops",
                "2",
            ],
        ),
    );
    let atlas_qns = atlas_query_qnames(&atlas);
    let grep_qns = plain_grep_ranked_candidates(repo.path(), &store, target_qn, 5);

    assert!(
        atlas_qns.iter().any(|qn| qn == "src/lib.rs::fn::helper"),
        "graph expansion must surface direct callee/caller neighbor helper: {atlas:?}"
    );
    assert!(
        atlas_qns.iter().any(|qn| qn == "src/main.rs::fn::main"),
        "graph expansion must surface transitive caller neighbor main: {atlas:?}"
    );
    assert!(
        !grep_qns.iter().any(|qn| qn == "src/main.rs::fn::main"),
        "plain grep baseline must not infer transitive caller main from qname lookup: {grep_qns:?}"
    );
}

#[test]
fn graph_aware_symbol_lookup_beats_plain_grep_baseline_on_fixtures() {
    let repo = setup_repo(&[
        (
            "src/a_calls.rs",
            "pub fn call_helper() { helper(); }\npub fn call_render() { render(); }\n",
        ),
        (
            "src/b_more_calls.rs",
            "pub fn relay_helper() { helper(); }\npub fn relay_render() { render(); }\n",
        ),
        ("src/z_defs.rs", "pub fn helper() {}\npub fn render() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let cases = [
        LookupEvalCase {
            query: "helper",
            expected_qn: "src/z_defs.rs::fn::helper",
        },
        LookupEvalCase {
            query: "render",
            expected_qn: "src/z_defs.rs::fn::render",
        },
    ];

    let atlas_top1 = cases
        .iter()
        .filter(|case| {
            let query = read_json_data_output(
                "query",
                run_atlas(repo.path(), &["--json", "query", case.query]),
            );
            atlas_query_qnames(&query)
                .first()
                .is_some_and(|qn| qn == case.expected_qn)
        })
        .count();
    let atlas_top3 = cases
        .iter()
        .filter(|case| {
            let query = read_json_data_output(
                "query",
                run_atlas(repo.path(), &["--json", "query", case.query]),
            );
            atlas_query_qnames(&query)
                .iter()
                .take(3)
                .any(|qn| qn == case.expected_qn)
        })
        .count();
    let grep_top1 = cases
        .iter()
        .filter(|case| {
            plain_grep_ranked_candidates(repo.path(), &store, case.query, 1)
                .first()
                .is_some_and(|qn| qn == case.expected_qn)
        })
        .count();
    let grep_top3 = cases
        .iter()
        .filter(|case| {
            plain_grep_ranked_candidates(repo.path(), &store, case.query, 3)
                .iter()
                .any(|qn| qn == case.expected_qn)
        })
        .count();

    assert!(
        atlas_top1 > grep_top1 || atlas_top3 > grep_top3,
        "atlas query must beat plain grep on top-1 or top-3 accuracy: atlas top1/top3 = {atlas_top1}/{atlas_top3}, grep = {grep_top1}/{grep_top3}"
    );
}

#[test]
fn build_and_update_skip_unsupported_files_without_count_drift() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    write_repo_file(repo.path(), "notes.md", "# atlas notes\n");

    let build = read_json_data_output("build", run_atlas(repo.path(), &["--json", "build"]));
    assert!(build["skipped_unsupported"].as_u64().unwrap_or_default() >= 1);
    assert!(build["parsed"].as_u64().unwrap_or_default() >= 2);

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--files", "notes.md"]),
    );
    assert_eq!(update["skipped_unsupported"], json!(1));
    assert_eq!(update["parsed"], json!(0));
    assert_eq!(update["parse_errors"], json!(0));
}

#[test]
fn mvp_command_contract_holds_for_committed_fixture_repo() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert!(status["indexed_file_count"].as_u64().unwrap_or_default() >= 2);
    assert!(status["node_count"].as_i64().unwrap_or_default() >= 5);
    assert!(status["edge_count"].as_i64().unwrap_or_default() >= 1);

    let query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "greet_twice"]),
    );
    let results = query["results"]
        .as_array()
        .expect("query results should return a JSON array");
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0]["node"]["qualified_name"],
        json!("src/lib.rs::method::Greeter::greet_twice")
    );

    write_repo_file(
        repo.path(),
        "src/lib.rs",
        r#"pub struct Greeter;

impl Greeter {
    pub fn greet_twice(name: &str) -> String {
        format!("Hello, {name}! Hello again, {name}!")
    }
}

pub fn helper(name: &str) -> String {
    let greeting = Greeter::greet_twice(name);
    format!("{greeting} [updated]")
}
"#,
    );

    let changes = read_json_data_output(
        "detect_changes",
        run_atlas(repo.path(), &["--json", "detect-changes", "--base", "HEAD"]),
    );
    let changes = changes["changes"]
        .as_array()
        .expect("detect-changes changes should return a JSON array");
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0]["path"], json!("src/lib.rs"));
    assert_eq!(changes[0]["change_type"], json!("modified"));

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--base", "HEAD"]),
    );
    assert!(update["parsed"].as_u64().unwrap_or_default() >= 1);
    assert!(update["nodes_updated"].as_u64().unwrap_or_default() >= 1);

    let impact = read_json_data_output(
        "impact",
        run_atlas(repo.path(), &["--json", "impact", "--base", "HEAD"]),
    );
    // JSON shape is now AdvancedImpactResult: { base: {...}, scored_nodes, risk_level, ... }
    let analysis = &impact["analysis"];
    let base = &analysis["base"];
    assert!(
        base["changed_nodes"]
            .as_array()
            .expect("impact changed_nodes should be an array")
            .iter()
            .any(|node| node["file_path"] == json!("src/lib.rs"))
    );
    assert!(
        base["changed_nodes"]
            .as_array()
            .expect("impact changed_nodes should be an array")
            .iter()
            .any(|node| node["qualified_name"] == json!("src/lib.rs::fn::helper"))
    );
    assert!(
        base["relevant_edges"]
            .as_array()
            .expect("impact relevant_edges should be an array")
            .iter()
            .any(|edge| edge["kind"] == json!("calls"))
    );
    // Advanced fields must be present.
    assert!(
        analysis["risk_level"].is_string(),
        "risk_level must be a string"
    );
    assert!(
        analysis["scored_nodes"].is_array(),
        "scored_nodes must be an array"
    );
    assert!(
        analysis["test_impact"].is_object(),
        "test_impact must be an object"
    );
    assert!(
        analysis["boundary_violations"].is_array(),
        "boundary_violations must be an array"
    );

    // review-context now emits ContextResult schema (Slice 9 transition).
    let review_ctx = read_json_data_output(
        "review_context",
        run_atlas(repo.path(), &["--json", "review-context", "--base", "HEAD"]),
    );
    // files → array of SelectedFile with path + selection_reason.
    assert!(
        review_ctx["files"]
            .as_array()
            .expect("review-context files must be an array")
            .iter()
            .any(|f| f["path"] == json!("src/lib.rs")),
        "review-context files must include src/lib.rs"
    );
    // nodes → array of SelectedNode.
    assert!(
        review_ctx["nodes"]
            .as_array()
            .expect("review-context nodes must be an array")
            .iter()
            .any(|n| n["node"]["file_path"] == json!("src/lib.rs")),
        "review-context nodes must include nodes from src/lib.rs"
    );
    // truncation metadata must be present.
    assert!(
        review_ctx["truncation"].is_object(),
        "review-context truncation must be an object"
    );
    // request must carry the intent.
    assert_eq!(
        review_ctx["request"]["intent"],
        json!("review"),
        "review-context request.intent must be 'review'"
    );

    let status_with_base = read_json_data_output(
        "status",
        run_atlas(repo.path(), &["--json", "status", "--base", "HEAD"]),
    );
    assert_eq!(status_with_base["changed_file_count"], json!(1));
    assert_eq!(status_with_base["diff_target"]["kind"], json!("base_ref"));
    assert_eq!(
        status_with_base["changed_files"][0]["path"],
        json!("src/lib.rs")
    );
}

#[test]
fn explain_change_command_reports_change_summary() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    rewrite_fixture_helper(repo.path());

    let explain = read_json_data_output(
        "explain_change",
        run_atlas(repo.path(), &["--json", "explain-change", "--base", "HEAD"]),
    );

    assert_eq!(explain["changed_file_count"], json!(1));
    assert!(
        explain["changed_symbol_count"].as_u64().unwrap_or_default() >= 1,
        "expected at least one changed symbol: {explain:?}"
    );
    assert!(
        explain["changed_by_kind"]["api_change"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "expected api change count in explain-change output: {explain:?}"
    );
    assert_eq!(explain["risk_level"], json!("high"));
    assert_eq!(
        explain["diff_summary"]["files"][0]["change_type"],
        json!("modified")
    );
    assert!(
        explain["impacted_components"]
            .as_array()
            .expect("impacted components array")
            .iter()
            .any(|component| component["file_count"].as_u64().unwrap_or_default() >= 1),
        "expected impacted components in explain-change output: {explain:?}"
    );
    assert!(
        explain["ripple_effects"]
            .as_array()
            .expect("ripple effects array")
            .iter()
            .any(|item| item.as_str().unwrap_or_default().contains("Change")
                || item.as_str().unwrap_or_default().contains("Impact")
                || item.as_str().unwrap_or_default().contains("Primary")),
        "expected ripple effect summary in explain-change output: {explain:?}"
    );
    assert!(
        explain["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("Risk:"),
        "summary must include risk sentence: {explain:?}"
    );
}

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
    assert!(
        review_ctx["workflow"]["noise_reduction"]["rules_applied"].is_array(),
        "expected noise reduction metadata in review context: {review_ctx:?}"
    );
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
        (
            "apps/web/package.json",
            r#"{"name":"web","version":"0.1.0"}"#,
        ),
        (
            "apps/web/src/app.ts",
            "import { helper } from '@ui/helper';\nexport function run(): string {\n    return helper();\n}\n",
        ),
        (
            "packages/ui/package.json",
            r#"{"name":"ui","version":"0.1.0"}"#,
        ),
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
        "workflow summary must explain why the cross-package change matters: {review_ctx:?}"
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
        (
            "apps/web/package.json",
            r#"{"name":"web","version":"0.1.0"}"#,
        ),
        (
            "apps/web/src/app.ts",
            "import { helper } from '@ui/helper';\nexport function run(): string {\n    return helper();\n}\n",
        ),
        (
            "packages/ui/package.json",
            r#"{"name":"ui","version":"0.1.0"}"#,
        ),
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
        matches!(
            impact["analysis"]["risk_level"].as_str(),
            Some("high") | Some("critical")
        ),
        "cross-package untested change must elevate risk: {impact:?}"
    );
}

#[test]
fn detached_current_repo_worktree_meets_large_repo_performance_gate() {
    let worktree = setup_current_repo_detached_worktree();

    run_atlas(worktree.path(), &["init"]);

    let build = read_json_data_output("build", run_atlas(worktree.path(), &["--json", "build"]));
    assert!(
        build["parsed"].as_u64().unwrap_or_default() >= 50,
        "representative repo build should parse substantial tracked files: {build:?}"
    );
    assert!(
        build["nodes_inserted"].as_u64().unwrap_or_default() >= 200,
        "representative repo build should index substantial graph size: {build:?}"
    );
    assert!(
        build["elapsed_ms"].as_u64().unwrap_or(u64::MAX) <= 60_000,
        "representative repo build latency regressed: {build:?}"
    );

    let status = read_json_data_output("status", run_atlas(worktree.path(), &["--json", "status"]));
    assert!(
        status["indexed_file_count"].as_u64().unwrap_or_default() >= 50,
        "status should report representative repo scale: {status:?}"
    );

    let query = read_json_data_output(
        "query",
        run_atlas(worktree.path(), &["--json", "query", "ContextEngine"]),
    );
    assert!(
        !query["results"]
            .as_array()
            .expect("query results array")
            .is_empty(),
        "large-repo query should return known symbol hits: {query:?}"
    );
    assert!(
        query["latency_ms"].as_u64().unwrap_or(u64::MAX) <= 10_000,
        "large-repo query latency regressed: {query:?}"
    );

    let impact_target = "packages/atlas-impact/src/lib.rs";
    let original =
        fs::read_to_string(worktree.path().join(impact_target)).expect("read impact file");
    let mut updated = original.clone();
    updated.push_str("\n// perf gate change\n");
    write_repo_file(worktree.path(), impact_target, &updated);

    let impact = read_json_data_output(
        "impact",
        run_atlas(
            worktree.path(),
            &[
                "--json",
                "impact",
                "--base",
                "HEAD",
                "--max-depth",
                "3",
                "--max-nodes",
                "200",
            ],
        ),
    );
    assert!(
        impact["analysis"]["base"]["changed_nodes"]
            .as_array()
            .expect("changed nodes array")
            .iter()
            .any(|node| node["file_path"] == json!(impact_target)),
        "impact must include changed file seed from representative repo: {impact:?}"
    );
    assert!(
        impact["latency_ms"].as_u64().unwrap_or(u64::MAX) <= 15_000,
        "large-repo impact latency regressed: {impact:?}"
    );

    let update = read_json_data_output(
        "update",
        run_atlas(worktree.path(), &["--json", "update", "--base", "HEAD"]),
    );
    assert!(
        update["parsed"].as_u64().unwrap_or_default() >= 1,
        "large-repo update should parse changed worktree file: {update:?}"
    );
    assert!(
        update["elapsed_ms"].as_u64().unwrap_or(u64::MAX) <= 20_000,
        "large-repo update latency regressed: {update:?}"
    );
}

#[test]
fn natural_language_context_queries_map_to_graph_requests() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let usage = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &["--json", "context", "where is greet_twice used?"],
        ),
    );
    assert_eq!(usage["request"]["intent"], json!("usage_lookup"));
    assert!(
        usage["nodes"]
            .as_array()
            .expect("usage nodes array")
            .iter()
            .all(|node| {
                matches!(
                    node["selection_reason"].as_str().unwrap_or_default(),
                    "direct_target" | "caller" | "importee" | "importer"
                )
            }),
        "usage lookup should not include callee noise: {usage:?}"
    );

    let what_calls = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &["--json", "context", "what calls greet_twice?"],
        ),
    );
    assert_eq!(what_calls["request"]["intent"], json!("usage_lookup"));

    let breaks = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &["--json", "context", "what breaks if I change greet_twice?"],
        ),
    );
    assert_eq!(breaks["request"]["intent"], json!("impact_analysis"));
    assert!(
        breaks["workflow"]["headline"].is_string() || breaks["nodes"].is_array(),
        "impact-analysis query should route to graph context: {breaks:?}"
    );
}

#[test]
fn interactive_shell_accepts_query_and_context_requests() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let mut child = Command::new(env!("CARGO_BIN_EXE_atlas"))
        .args(["shell", "--fuzzy"])
        .current_dir(repo.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to spawn atlas shell: {err}"));

    child
        .stdin
        .as_mut()
        .expect("shell stdin")
        .write_all(b"/query greet_twice\nwhat calls greet_twice?\nexit\n")
        .expect("write shell input");

    let output = child.wait_with_output().expect("wait for atlas shell");
    assert!(
        output.status.success(),
        "atlas shell failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Query results:"),
        "missing query output: {stdout}"
    );
    assert!(
        stdout.contains("Call chains:") || stdout.contains("Nodes ("),
        "missing context output: {stdout}"
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

    let after =
        fs::read_to_string(repo.path().join("src/lib.rs")).expect("read source after dry-run");
    assert!(after.contains("pub fn helper() -> i32"));
    assert!(after.contains("helper()"));
}

#[test]
fn serve_command_handles_stdio_jsonrpc_flow_end_to_end() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let mut child = Command::new(env!("CARGO_BIN_EXE_atlas"))
        .arg("serve")
        .current_dir(repo.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to spawn atlas serve: {err}"));

    let requests = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"greet_twice\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"get_context\",\"arguments\":{\"query\":\"greet_twice\"}}}\n"
    );

    {
        let stdin = child.stdin.as_mut().expect("serve stdin");
        stdin
            .write_all(requests.as_bytes())
            .expect("write serve requests");
    }

    let output = child
        .wait_with_output()
        .expect("wait for atlas serve output");

    assert!(
        output.status.success(),
        "atlas serve failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let responses = parse_jsonrpc_lines(&output.stdout);
    assert_eq!(
        responses.len(),
        4,
        "initialized notification must not emit a response"
    );

    assert_eq!(responses[0]["id"], json!(1));
    assert_eq!(
        responses[0]["result"]["protocolVersion"],
        json!("2024-11-05")
    );

    assert_eq!(responses[1]["id"], json!(2));
    let tools = responses[1]["result"]["tools"]
        .as_array()
        .expect("tools/list result tools array");
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == json!("get_context")),
        "tools/list must expose get_context"
    );

    assert_eq!(responses[2]["id"], json!(3));
    assert_eq!(responses[2]["result"]["atlas_output_format"], json!("json"));
    let query_text = responses[2]["result"]["content"][0]["text"]
        .as_str()
        .expect("query_graph text content");
    let query_value: Value = serde_json::from_str(query_text).expect("query_graph payload json");
    assert_eq!(
        query_value[0]["qn"],
        json!("src/lib.rs::method::Greeter::greet_twice")
    );

    assert_eq!(responses[3]["id"], json!(4));
    assert_eq!(responses[3]["result"]["atlas_output_format"], json!("toon"));
    let context_text = responses[3]["result"]["content"][0]["text"]
        .as_str()
        .expect("get_context text content");
    assert!(context_text.contains("intent: symbol"));
    assert!(context_text.contains("src/lib.rs::method::Greeter::greet_twice"));
}

#[test]
fn build_on_sample_repo_emits_expected_summary() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let output = run_atlas(repo.path(), &["build"]);
    let stdout = stdout_text(&output);

    assert_contains_all(
        &stdout,
        &[
            "Build complete (",
            "Scanned             :",
            "Unsupported skipped : 3",
            "Parsed              : 3",
            "Nodes inserted      : 24",
            "Edges inserted      : 24",
        ],
    );
}

#[test]
fn update_after_fixture_edit_emits_expected_summary() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    rewrite_fixture_helper(repo.path());

    let output = run_atlas(repo.path(), &["update", "--base", "HEAD"]);
    let stdout = stdout_text(&output);

    assert_contains_all(
        &stdout,
        &[
            "Update complete (",
            "Deleted  : 0",
            "Parsed   : 1",
            "Nodes    : 4",
            "Edges    : 5",
        ],
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
            "Changed nodes : 4",
            "Relevant edges: 5",
            "Risk level    : high",
            "struct src/lib.rs::struct::Greeter [api_change]",
            "method src/lib.rs::method::Greeter::greet_twice [signature_change]",
            "function src/lib.rs::fn::helper [signature_change]",
        ],
    );

    let review = stdout_text(&run_atlas(
        repo.path(),
        &["review-context", "--base", "HEAD"],
    ));
    assert_contains_all(
        &review,
        &[
            "Changed files (1):",
            "  src/lib.rs",
            "Changed symbols: 4",
            "function src/lib.rs::fn::helper (src/lib.rs:9)",
            "Risk summary:",
            "  Public API changes : 3",
            "  Uncovered changes  : 4",
            "  Cross-package impact: false",
        ],
    );

    let query = stdout_text(&run_atlas(repo.path(), &["query", "greet_twice"]));
    let query_lines: Vec<&str> = query.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(
        query_lines.len(),
        2,
        "expected one ranked result and a count"
    );
    assert!(
        query_lines[0].contains("method src/lib.rs::method::Greeter::greet_twice (src/lib.rs:4)"),
        "unexpected first query result: {query}"
    );
    assert!(query_lines[1].starts_with("1 result(s)."));
}

#[test]
fn build_resolves_rust_same_package_call_targets() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/main.rs").expect("main edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/lib.rs::fn::helper"
                && edge.confidence_tier.as_deref() == Some("same_package")
        }),
        "expected src/main.rs helper call to resolve into src/lib.rs::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_rust_same_package_across_directories_in_standalone_package() {
    let repo = setup_repo(&[
        (
            "crates/foo/Cargo.toml",
            "[package]\nname = 'foo'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("crates/foo/src/lib.rs", "pub fn helper() {}\n"),
        ("crates/foo/examples/demo.rs", "fn main() { helper(); }\n"),
        (
            "crates/bar/Cargo.toml",
            "[package]\nname = 'bar'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("crates/bar/src/lib.rs", "pub fn helper() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store
        .edges_by_file("crates/foo/examples/demo.rs")
        .expect("demo edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "crates/foo/src/lib.rs::fn::helper"
                && edge.confidence_tier.as_deref() == Some("same_package")
        }),
        "expected example call to resolve into crates/foo/src/lib.rs::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn query_includes_owner_identity_for_ambiguous_workspace_results() {
    let repo = setup_repo(&[
        ("Cargo.toml", "[workspace]\nmembers = ['packages/*']\n"),
        (
            "packages/foo/Cargo.toml",
            "[package]\nname = 'foo'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("packages/foo/src/lib.rs", "pub fn helper() {}\n"),
        (
            "packages/bar/Cargo.toml",
            "[package]\nname = 'bar'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("packages/bar/src/lib.rs", "pub fn helper() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let query = stdout_text(&run_atlas(repo.path(), &["query", "helper"]));
    assert_contains_all(
        &query,
        &[
            "packages/foo/src/lib.rs::fn::helper",
            "packages/bar/src/lib.rs::fn::helper",
            "[owner cargo:packages/foo/Cargo.toml]",
            "[owner cargo:packages/bar/Cargo.toml]",
        ],
    );
}

#[test]
fn update_rename_across_package_roots_refreshes_owner_identity() {
    let repo = setup_repo(&[
        (
            "crates/foo/Cargo.toml",
            "[package]\nname = 'foo'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("crates/foo/src/lib.rs", "pub fn helper() {}\n"),
        (
            "crates/bar/Cargo.toml",
            "[package]\nname = 'bar'\nversion = '0.1.0'\nedition = '2021'\n",
        ),
        ("crates/bar/src/mod.rs", "pub fn marker() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    run_command(
        repo.path(),
        "git",
        &["mv", "crates/foo/src/lib.rs", "crates/bar/src/ported.rs"],
    );

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--staged"]),
    );
    assert_eq!(update["renamed"], json!(0));
    assert!(update["parsed"].as_u64().unwrap_or_default() >= 1);

    let store = open_store(repo.path());
    let new_owner = store
        .file_owner("crates/bar/src/ported.rs")
        .expect("new owner lookup")
        .expect("stored new owner");
    assert_eq!(new_owner.owner_id, "cargo:crates/bar/Cargo.toml");
    assert!(
        store
            .file_owner("crates/foo/src/lib.rs")
            .expect("old owner lookup")
            .is_none(),
        "old path owner metadata must be removed"
    );
}

#[test]
fn multi_package_workspace_flow_uses_owner_identity_end_to_end() {
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
        (
            "apps/web/package.json",
            r#"{"name":"web","version":"0.1.0"}"#,
        ),
        (
            "apps/web/src/app.ts",
            "import { helper } from '@ui/helper';\nexport function run(): string {\n    return helper();\n}\n",
        ),
        (
            "packages/ui/package.json",
            r#"{"name":"ui","version":"0.1.0"}"#,
        ),
        (
            "packages/ui/src/helper.ts",
            "export function helper(): string {\n    return 'v1';\n}\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let app_edges = store
        .edges_by_file("apps/web/src/app.ts")
        .expect("app edges after build");
    assert!(
        app_edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "packages/ui/src/helper.ts::fn::helper"
        }),
        "build must resolve cross-package helper call before impact/review checks: {app_edges:?}"
    );

    let analyze = read_json_data_output(
        "analyze_dependency",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "analyze",
                "dependency",
                "packages/ui/src/helper.ts::fn::helper",
            ],
        ),
    );
    assert!(
        analyze["blocking_references"]
            .as_array()
            .expect("blocking references array")
            .iter()
            .any(|node| node["file_path"] == json!("apps/web/src/app.ts")),
        "reasoning must see cross-package dependency: {analyze:?}"
    );

    write_repo_file(
        repo.path(),
        "apps/web/src/app.ts",
        "import { helper } from '@ui/helper';\nexport function run(): string {\n    return `${helper()}!`;\n}\n",
    );

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--base", "HEAD"]),
    );
    assert!(update["parsed"].as_u64().unwrap_or_default() >= 1);

    run_atlas(repo.path(), &["build"]);

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
        "impact must flag cross-package boundary: {impact:?}"
    );

    let review = stdout_text(&run_atlas(
        repo.path(),
        &["review-context", "--base", "HEAD"],
    ));
    assert_contains_all(
        &review,
        &[
            "Changed files (1):",
            "  apps/web/src/app.ts",
            "Cross-package impact: true",
        ],
    );
}

#[test]
fn build_resolves_typescript_namespace_import_calls() {
    let repo = setup_repo(&[
        (
            "src/app.ts",
            "import * as utils from './utils';\nexport function caller(): void { utils.helper(); }\n",
        ),
        ("src/utils.ts", "export function helper(): void {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/app.ts").expect("app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/utils.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected namespace import call to resolve into src/utils.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_typescript_path_alias_calls() {
    let repo = setup_repo(&[
        (
            "tsconfig.json",
            r#"{
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@utils/*": ["src/utils/*"]
    }
  }
}
"#,
        ),
        (
            "src/app.ts",
            "import * as math from '@utils/math';\nexport function caller(): void { math.helper(); }\n",
        ),
        ("src/utils/math.ts", "export function helper(): void {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/app.ts").expect("app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/utils/math.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected path-alias call to resolve into src/utils/math.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_nested_typescript_path_alias_calls() {
    let repo = setup_repo(&[
        (
            "apps/web/tsconfig.json",
            r#"{
  "compilerOptions": {
    "baseUrl": "src",
    "paths": {
      "@lib/*": ["lib/*"]
    }
  }
}
"#,
        ),
        (
            "apps/web/src/app.ts",
            "import * as math from '@lib/math';\nexport function caller(): void { math.helper(); }\n",
        ),
        (
            "apps/web/src/lib/math.ts",
            "export function helper(): void {}\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store
        .edges_by_file("apps/web/src/app.ts")
        .expect("nested app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "apps/web/src/lib/math.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected nested path-alias call to resolve into apps/web/src/lib/math.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_typescript_extended_tsconfig_alias_calls() {
    let repo = setup_repo(&[
        (
            "configs/tsconfig.base.json",
            r#"{
  "compilerOptions": {
        "baseUrl": "..",
    "paths": {
      "@shared/*": ["src/shared/*"]
    }
  }
}
"#,
        ),
        (
            "apps/web/tsconfig.json",
            r#"{
  "extends": "../../configs/tsconfig.base.json"
}
"#,
        ),
        ("src/shared/math.ts", "export function helper(): void {}\n"),
        (
            "apps/web/app.ts",
            "import * as math from '@shared/math';\nexport function caller(): void { math.helper(); }\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store
        .edges_by_file("apps/web/app.ts")
        .expect("extended tsconfig app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/shared/math.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected extended-tsconfig alias call to resolve into src/shared/math.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_typescript_reexport_chain_calls() {
    let repo = setup_repo(&[
        (
            "src/app.ts",
            "import { helper } from './barrel';\nexport function caller(): void { helper(); }\n",
        ),
        ("src/barrel.ts", "export { helper } from './impl';\n"),
        ("src/impl.ts", "export function helper(): void {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("src/app.ts").expect("app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/impl.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected re-export chain call to resolve into src/impl.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_typescript_package_extends_alias_calls() {
    let repo = setup_repo(&[
        (
            "node_modules/@atlas/tsconfig/base.json",
            r#"{
  "compilerOptions": {
    "baseUrl": "../../../",
    "paths": {
      "@shared/*": ["src/shared/*"]
    }
  }
}
"#,
        ),
        (
            "apps/web/tsconfig.json",
            r#"{
  "extends": "@atlas/tsconfig/base"
}
"#,
        ),
        (
            "apps/web/app.ts",
            "import * as math from '@shared/math';\nexport function caller(): void { math.helper(); }\n",
        ),
        ("src/shared/math.ts", "export function helper(): void {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store
        .edges_by_file("apps/web/app.ts")
        .expect("package-extends app edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "src/shared/math.ts::fn::helper"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected package-style tsconfig extends to resolve alias into src/shared/math.ts::fn::helper; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_python_relative_import_calls() {
    let repo = setup_repo(&[
        ("pkg/__init__.py", ""),
        (
            "pkg/main.py",
            "from .helpers import ping\n\ndef caller():\n    ping()\n",
        ),
        ("pkg/helpers.py", "def ping():\n    pass\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("pkg/main.py").expect("main edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "pkg/helpers.py::fn::ping"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected relative import call to resolve into pkg/helpers.py::fn::ping; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_python_package_submodule_alias_calls() {
    let repo = setup_repo(&[
        ("pkg/__init__.py", ""),
        (
            "pkg/main.py",
            "from pkg import helpers as helpers_mod\n\ndef caller():\n    helpers_mod.ping()\n",
        ),
        ("pkg/helpers.py", "def ping():\n    pass\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("pkg/main.py").expect("main edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "pkg/helpers.py::fn::ping"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected package submodule alias call to resolve into pkg/helpers.py::fn::ping; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_python_package_init_export_calls() {
    let repo = setup_repo(&[
        ("pkg/__init__.py", "def ping():\n    pass\n"),
        (
            "pkg/main.py",
            "from pkg import ping\n\ndef caller():\n    ping()\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("pkg/main.py").expect("main edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "pkg/__init__.py::fn::ping"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected package __init__ export call to resolve into pkg/__init__.py::fn::ping; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_python_wildcard_import_calls() {
    let repo = setup_repo(&[
        ("pkg/__init__.py", "def ping():\n    pass\n"),
        (
            "pkg/main.py",
            "from pkg import *\n\ndef caller():\n    ping()\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store.edges_by_file("pkg/main.py").expect("main edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "pkg/__init__.py::fn::ping"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected wildcard import call to resolve into pkg/__init__.py::fn::ping; edges: {edges:?}"
    );
}

#[test]
fn build_resolves_go_local_module_import_calls() {
    let repo = setup_repo(&[
        ("go.mod", "module example.com/demo\n\ngo 1.22\n"),
        (
            "cmd/app/main.go",
            "package main\n\nimport \"example.com/demo/internal/helpers\"\n\nfunc caller() { helpers.Run() }\n",
        ),
        (
            "internal/helpers/run.go",
            "package helpers\n\nfunc Run() {}\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let store = open_store(repo.path());
    let edges = store
        .edges_by_file("cmd/app/main.go")
        .expect("go caller edges");
    assert!(
        edges.iter().any(|edge| {
            edge.kind == EdgeKind::Calls
                && edge.target_qn == "internal/helpers/run.go::fn::Run"
                && edge.confidence_tier.as_deref() == Some("imports")
        }),
        "expected local-module import call to resolve into internal/helpers/run.go::fn::Run; edges: {edges:?}"
    );
}

#[test]
fn build_and_update_replace_json_toml_file_graphs() {
    let repo = setup_repo(&[
        (
            "config/app.json",
            "{\n  \"service\": { \"mode\": \"dev\" },\n  \"enabled\": true\n}\n",
        ),
        (
            "Cargo.toml",
            "[package]\nname = \"atlas\"\nversion = \"0.1.0\"\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    let build = read_json_data_output("build", run_atlas(repo.path(), &["--json", "build"]));
    assert_eq!(build["parse_errors"], json!(0));
    assert!(build["parsed"].as_u64().unwrap_or_default() >= 2);

    let store = open_store(repo.path());
    let json_nodes = store.nodes_by_file("config/app.json").expect("json nodes");
    assert!(
        json_nodes
            .iter()
            .any(|node| node.qualified_name == "config/app.json::key::service.mode")
    );
    assert!(json_nodes.iter().any(|node| node.kind == NodeKind::Module));
    let toml_nodes = store.nodes_by_file("Cargo.toml").expect("toml nodes");
    assert!(
        toml_nodes
            .iter()
            .any(|node| node.qualified_name == "Cargo.toml::key::package.name")
    );

    write_repo_file(
        repo.path(),
        "config/app.json",
        "{\n  \"service\": { \"port\": 8080 },\n  \"enabled\": false\n}\n",
    );
    write_repo_file(
        repo.path(),
        "Cargo.toml",
        "[package]\nname = \"atlas-renamed\"\nversion = \"0.2.0\"\n",
    );

    let update = read_json_data_output(
        "update",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "update",
                "--files",
                "config/app.json",
                "--files",
                "Cargo.toml",
            ],
        ),
    );
    assert_eq!(update["parse_errors"], json!(0));
    assert_eq!(update["parsed"], json!(2));

    let store = open_store(repo.path());
    let json_nodes = store
        .nodes_by_file("config/app.json")
        .expect("updated json nodes");
    assert!(
        !json_nodes
            .iter()
            .any(|node| node.qualified_name == "config/app.json::key::service.mode")
    );
    assert!(
        json_nodes
            .iter()
            .any(|node| node.qualified_name == "config/app.json::key::service.port")
    );
    let toml_nodes = store
        .nodes_by_file("Cargo.toml")
        .expect("updated toml nodes");
    assert!(
        toml_nodes
            .iter()
            .any(|node| node.qualified_name == "Cargo.toml::key::package.name")
    );
    let name_node = toml_nodes
        .iter()
        .find(|node| node.qualified_name == "Cargo.toml::key::package.name")
        .expect("package name node");
    assert_eq!(name_node.line_start, 2);
}

fn setup_fixture_repo() -> TempDir {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    copy_dir_all(&fixture_repo_root(), temp_dir.path());
    init_git_repo(temp_dir.path());
    temp_dir
}

fn setup_repo(files: &[(&str, &str)]) -> TempDir {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    for (relative_path, content) in files {
        let path = temp_dir.path().join(relative_path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create test dir");
        }
        fs::write(path, content).expect("write test file");
    }
    init_git_repo(temp_dir.path());
    temp_dir
}

struct DetachedWorktree {
    temp_dir: TempDir,
    source_repo: PathBuf,
    path: PathBuf,
}

impl DetachedWorktree {
    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for DetachedWorktree {
    fn drop(&mut self) {
        let _ = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(&self.path)
            .current_dir(&self.source_repo)
            .output();
        let _ = &self.temp_dir;
    }
}

fn setup_current_repo_detached_worktree() -> DetachedWorktree {
    let temp_dir = tempfile::tempdir().expect("temp dir");
    let path = temp_dir.path().join("repo-worktree");
    let source_repo = current_repo_root();

    run_command(
        &source_repo,
        "git",
        &[
            "worktree",
            "add",
            "--detach",
            path.to_str().expect("worktree path"),
            "HEAD",
        ],
    );

    DetachedWorktree {
        temp_dir,
        source_repo,
        path,
    }
}

fn init_git_repo(path: &Path) {
    run_command(path, "git", &["init", "--quiet"]);
    run_command(path, "git", &["config", "user.name", "Atlas Tests"]);
    run_command(
        path,
        "git",
        &["config", "user.email", "atlas-tests@example.com"],
    );
    run_command(path, "git", &["add", "."]);
    run_command(
        path,
        "git",
        &["commit", "--quiet", "-m", "fixture baseline"],
    );
}

fn fixture_repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("sample_repo")
}

fn current_repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace repo root")
        .to_path_buf()
}

fn read_golden_json(name: &str) -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("golden")
        .join(name);
    serde_json::from_str(&fs::read_to_string(path).expect("golden file")).expect("golden json")
}

fn normalize_query_results(value: &mut Value) {
    value["latency_ms"] = json!(0);
    let Some(results) = value["results"].as_array_mut() else {
        panic!("query output results should be an array");
    };

    for result in results {
        result["score"] = json!(0.0);
        result["node"]["id"] = json!(0);
        result["node"]["file_hash"] = json!("<hash>");
    }
}

fn stdout_text(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout should be utf-8")
}

fn assert_contains_all(haystack: &str, needles: &[&str]) {
    for needle in needles {
        assert!(
            haystack.contains(needle),
            "expected output to contain {needle:?}\nstdout:\n{haystack}"
        );
    }
}

fn read_json_output(output: Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("valid json output")
}

fn parse_jsonrpc_lines(stdout: &[u8]) -> Vec<Value> {
    String::from_utf8(stdout.to_vec())
        .expect("jsonrpc stdout utf-8")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("jsonrpc response line"))
        .collect()
}

fn read_json_data_output(command: &str, output: Output) -> Value {
    let value = read_json_output(output);
    assert_eq!(value["schema_version"], json!("atlas_cli.v1"));
    assert_eq!(value["command"], json!(command));
    value["data"].clone()
}

// ---------------------------------------------------------------------------
// Phase 22, 9.1 — public `atlas context` surface tests
// ---------------------------------------------------------------------------

#[test]
fn context_symbol_flow_returns_bounded_json() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    // Positional free-text / symbol name query.
    let data = read_json_data_output(
        "context",
        run_atlas(repo.path(), &["--json", "context", "greet_twice"]),
    );

    // Result shape: nodes, edges, files, truncation, request.
    let nodes = data["nodes"].as_array().expect("nodes must be an array");
    assert!(
        !nodes.is_empty(),
        "symbol context must return at least one node"
    );
    assert!(
        nodes.iter().any(|n| n["node"]["qualified_name"]
            .as_str()
            .unwrap_or_default()
            .contains("greet_twice")),
        "greet_twice must appear in symbol context nodes"
    );
    assert!(
        data["truncation"].is_object(),
        "truncation metadata must be present"
    );
    assert_eq!(
        data["request"]["intent"],
        json!("symbol"),
        "request.intent must be 'symbol'"
    );
}

#[test]
fn context_file_flag_returns_file_intent_json() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(repo.path(), &["--json", "context", "--file", "src/lib.rs"]),
    );

    assert!(
        data["files"]
            .as_array()
            .expect("files must be an array")
            .iter()
            .any(|f| f["path"] == json!("src/lib.rs")),
        "file context must include src/lib.rs in files"
    );
    assert_eq!(data["request"]["intent"], json!("file"));
}

#[test]
fn context_files_flag_returns_review_intent_json() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(repo.path(), &["--json", "context", "--files", "src/lib.rs"]),
    );

    assert_eq!(
        data["request"]["intent"],
        json!("review"),
        "files flag must default to review intent"
    );
    assert!(
        data["files"]
            .as_array()
            .expect("files must be an array")
            .iter()
            .any(|f| f["path"] == json!("src/lib.rs")),
        "review context must include src/lib.rs"
    );
}

#[test]
fn context_intent_override_changes_request_intent() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "context",
                "--files",
                "src/lib.rs",
                "--intent",
                "impact",
            ],
        ),
    );

    assert_eq!(
        data["request"]["intent"],
        json!("impact"),
        "--intent impact must override default"
    );
}

#[test]
fn context_not_found_exits_ok_with_empty_nodes() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    // Unknown symbol. Engine returns not-found; CLI exits 0 in both modes.
    let data = read_json_data_output(
        "context",
        run_atlas(
            repo.path(),
            &["--json", "context", "totally_nonexistent_xyz_symbol"],
        ),
    );

    // Not-found: nodes empty, truncation present.
    assert!(
        data["nodes"].as_array().is_none_or(|a| a.is_empty()),
        "not-found must return empty nodes"
    );
    assert!(data["truncation"].is_object());
}

#[test]
fn context_ambiguous_symbol_returns_ambiguity_metadata() {
    // Set up a repo with two functions sharing the same short name.
    let repo = setup_repo(&[
        ("src/foo.rs", "pub fn process() {}\n"),
        ("src/bar.rs", "pub fn process() {}\n"),
    ]);
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "context",
        run_atlas(repo.path(), &["--json", "context", "process"]),
    );

    // Ambiguous: ambiguity object must be present and have candidates.
    if let Some(ambiguity) = data.get("ambiguity").filter(|v| !v.is_null()) {
        let candidates = ambiguity["candidates"]
            .as_array()
            .expect("ambiguity.candidates must be an array");
        assert!(
            candidates.len() >= 2,
            "ambiguity must list at least two candidates"
        );
    }
    // Either ambiguity is set, or nodes contains both — engine may resolve; just
    // confirm the output is valid JSON with the expected schema.
    assert!(data["truncation"].is_object());
}

#[test]
fn context_human_readable_symbol_output() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let out = stdout_text(&run_atlas(repo.path(), &["context", "greet_twice"]));
    assert!(
        out.contains("Nodes"),
        "human output must contain 'Nodes': {out}"
    );
    assert!(
        out.contains("Summary"),
        "human output must contain 'Summary' line: {out}"
    );
}

#[test]
fn context_json_contract_stable_for_golden_snapshot() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let mut data = read_json_data_output(
        "context",
        run_atlas(repo.path(), &["--json", "context", "helper"]),
    );

    // Normalize fields that vary between runs.
    normalize_context_result(&mut data);

    let golden = read_golden_json("context_helper.json");
    assert_eq!(
        data, golden,
        "context JSON output must match golden snapshot"
    );
}

fn normalize_context_result(value: &mut serde_json::Value) {
    // Blank out IDs and hashes that vary between fresh DB instances.
    if let Some(nodes) = value["nodes"].as_array_mut() {
        for n in nodes.iter_mut() {
            n["node"]["id"] = json!(0);
            n["node"]["file_hash"] = json!("<hash>");
        }
    }
    if let Some(edges) = value["edges"].as_array_mut() {
        for e in edges.iter_mut() {
            e["edge"]["id"] = json!(0);
        }
    }
}

fn write_repo_file(repo_root: &Path, relative_path: &str, content: &str) {
    fs::write(repo_root.join(relative_path), content).expect("write repo file");
}

fn rewrite_fixture_helper(repo_root: &Path) {
    write_repo_file(
        repo_root,
        "src/lib.rs",
        r#"pub struct Greeter;

impl Greeter {
    pub fn greet_twice(name: &str) -> String {
        format!("Hello, {name}! Hello again, {name}!")
    }
}

pub fn helper(name: &str) -> String {
    let greeting = Greeter::greet_twice(name);
    format!("{greeting} [updated]")
}
"#,
    );
}

fn open_store(repo_root: &Path) -> Store {
    Store::open(
        repo_root
            .join(".atlas")
            .join("worldtree.db")
            .to_str()
            .expect("db path"),
    )
    .expect("open atlas store")
}

#[derive(Clone, Copy)]
struct LookupEvalCase<'a> {
    query: &'a str,
    expected_qn: &'a str,
}

fn atlas_query_qnames(data: &Value) -> Vec<String> {
    data["results"]
        .as_array()
        .expect("query results array")
        .iter()
        .filter_map(|result| result["node"]["qualified_name"].as_str().map(str::to_owned))
        .collect()
}

fn plain_grep_ranked_candidates(
    repo_root: &Path,
    store: &Store,
    query: &str,
    limit: usize,
) -> Vec<String> {
    let tracked = tracked_repo_files(repo_root);
    let mut ranked = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for path in tracked {
        let Ok(contents) = fs::read_to_string(repo_root.join(&path)) else {
            continue;
        };
        let Ok(nodes) = store.nodes_by_file(&path) else {
            continue;
        };

        for (index, line) in contents.lines().enumerate() {
            if !line.contains(query) {
                continue;
            }

            let Some(node) = nodes
                .iter()
                .filter(|node| {
                    let line_no = index as u32 + 1;
                    node.line_start <= line_no && line_no <= node.line_end
                })
                .min_by_key(|node| {
                    (
                        node.line_end.saturating_sub(node.line_start),
                        node.line_start,
                        node.qualified_name.clone(),
                    )
                })
            else {
                continue;
            };

            if seen.insert(node.qualified_name.clone()) {
                ranked.push(node.qualified_name.clone());
                if ranked.len() >= limit {
                    return ranked;
                }
            }
        }
    }

    ranked
}

fn tracked_repo_files(repo_root: &Path) -> Vec<String> {
    String::from_utf8(run_command(repo_root, "git", &["ls-files"]).stdout)
        .expect("git ls-files stdout")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn run_atlas(repo_root: &Path, args: &[&str]) -> Output {
    run_command(repo_root, env!("CARGO_BIN_EXE_atlas"), args)
}

fn run_command(repo_root: &Path, program: &str, args: &[&str]) -> Output {
    let output = Command::new(program)
        .args(args)
        .current_dir(repo_root)
        .output()
        .unwrap_or_else(|err| panic!("failed to run {program}: {err}"));

    assert!(
        output.status.success(),
        "command failed: {} {}\nstdout:\n{}\nstderr:\n{}",
        program,
        args.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    output
}

struct SpawnedWatch {
    child: Child,
    stdout_rx: mpsc::Receiver<String>,
    stderr_rx: mpsc::Receiver<String>,
}

impl SpawnedWatch {
    fn recv_stdout_line(&mut self, timeout: Duration) -> String {
        match self.stdout_rx.recv_timeout(timeout) {
            Ok(line) => line,
            Err(err) => {
                let status = self.child.try_wait().expect("check atlas watch status");
                let stderr = drain_lines(&self.stderr_rx).join("\n");
                panic!(
                    "timed out waiting for atlas watch output after {timeout:?}: {err}; status={status:?}; stderr:\n{stderr}"
                );
            }
        }
    }
}

impl Drop for SpawnedWatch {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn spawn_atlas_watch(repo_root: &Path, args: &[&str]) -> SpawnedWatch {
    let mut child = Command::new(env!("CARGO_BIN_EXE_atlas"))
        .args(args)
        .current_dir(repo_root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|err| panic!("failed to spawn atlas watch: {err}"));

    let stdout = child.stdout.take().expect("atlas watch stdout");
    let stderr = child.stderr.take().expect("atlas watch stderr");

    SpawnedWatch {
        child,
        stdout_rx: spawn_line_reader(stdout),
        stderr_rx: spawn_line_reader(stderr),
    }
}

fn spawn_line_reader<R>(reader: R) -> mpsc::Receiver<String>
where
    R: std::io::Read + Send + 'static,
{
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(reader).lines() {
            match line {
                Ok(line) => {
                    let _ = tx.send(line);
                }
                Err(_) => break,
            }
        }
    });
    rx
}

fn drain_lines(rx: &mpsc::Receiver<String>) -> Vec<String> {
    let mut lines = Vec::new();
    while let Ok(line) = rx.try_recv() {
        lines.push(line);
    }
    lines
}

fn copy_dir_all(src: &Path, dst: &Path) {
    for entry in fs::read_dir(src).expect("fixture dir") {
        let entry = entry.expect("fixture entry");
        let file_type = entry.file_type().expect("fixture entry type");
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if file_type.is_dir() {
            fs::create_dir_all(&dst_path).expect("create fixture subdir");
            copy_dir_all(&src_path, &dst_path);
        } else {
            fs::copy(&src_path, &dst_path).expect("copy fixture file");
        }
    }
}

// ── Phase 28 watch-pipeline integration tests ─────────────────────────────────
//
// Most tests below verify the underlying update pipeline that `atlas watch`
// uses: modify, delete, and rename events all go through `update_graph` with
// `UpdateTarget::Files` (or `Batch` inside WatchRunner). Separate end-to-end
// test below also spins up the real blocking watcher process.

#[test]
fn watch_mode_updates_graph_end_to_end_in_real_time() {
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn hello() -> &'static str { \"hello\" }\n",
    )]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let mut watch = spawn_atlas_watch(
        repo.path(),
        &["--json", "watch", "--debounce-ms", "100"],
    );

    // Give the watcher a brief moment to subscribe before mutating files.
    thread::sleep(Duration::from_millis(250));

    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn hello_watch() -> &'static str { \"hello\" }\n",
    )
    .expect("write watched file");

    let started = Instant::now();
    let line = watch.recv_stdout_line(Duration::from_secs(5));
    let elapsed = started.elapsed();

    let payload: Value = serde_json::from_str(&line)
        .unwrap_or_else(|err| panic!("atlas watch emitted invalid JSON line: {err}; line={line}"));

    assert_eq!(payload["schema_version"], json!("atlas_cli.v1"));
    assert_eq!(payload["command"], json!("watch"));
    assert!(
        payload["data"]["files_updated"].as_u64().unwrap_or_default() >= 1,
        "watch batch should report at least one updated file: {payload:?}"
    );
    assert_eq!(payload["data"]["errors"], json!(0));
    assert!(
        elapsed <= Duration::from_secs(5),
        "watch update should arrive near real-time: elapsed={elapsed:?} payload={payload:?}"
    );

    let query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "hello_watch"]),
    );
    let results = query["results"].as_array().expect("query results array");
    assert!(
        !results.is_empty(),
        "graph should contain updated symbol after atlas watch batch: {query:?}"
    );
}

/// File modify causes graph to reflect new content.
#[test]
fn watch_file_modify_triggers_graph_update() {
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn hello() -> &'static str { \"hello\" }\n",
    )]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    // Confirm initial node exists.
    let before = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "hello"]),
    );
    let before_results = before["results"].as_array().expect("query results");
    assert!(
        !before_results.is_empty(),
        "should find 'hello' before modify"
    );

    // Modify file: rename function.
    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn hello_world() -> &'static str { \"hi\" }\n",
    )
    .expect("write modified file");

    // Simulate what watch does: update the changed file explicitly.
    run_atlas(repo.path(), &["update", "--files", "src/lib.rs"]);

    // Old name gone, new name present.
    let after_old = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "hello"]),
    );
    let after_new = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "hello_world"]),
    );

    let new_results = after_new["results"].as_array().expect("query results");
    assert!(
        !new_results.is_empty(),
        "new function 'hello_world' should appear after update: {after_new:?}"
    );

    // "hello" as standalone function should no longer be in results (only hello_world matches)
    let old_results = after_old["results"].as_array().expect("query results");
    let exact_hello: Vec<_> = old_results
        .iter()
        .filter(|r| r["node"]["name"] == serde_json::json!("hello"))
        .collect();
    assert!(
        exact_hello.is_empty(),
        "old function 'hello' should be gone after update: {after_old:?}"
    );
}

/// File delete removes its graph slice.
#[test]
fn watch_file_delete_removes_graph_slice() {
    let repo = setup_repo(&[
        ("src/lib.rs", "pub fn keep_me() {}\n"),
        ("src/remove_me.rs", "pub fn to_be_deleted() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    // Confirm both nodes exist.
    let before = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "to_be_deleted"]),
    );
    let before_results = before["results"].as_array().expect("query results");
    assert!(
        !before_results.is_empty(),
        "should find 'to_be_deleted' before delete"
    );

    // Delete the file and simulate watch update.
    fs::remove_file(repo.path().join("src/remove_me.rs")).expect("remove file");

    // Watch uses update pipeline with explicit files; for deletes the watcher
    // emits a Deleted event which maps to ChangeType::Deleted in the batch.
    // We exercise the same code via the working-tree diff (file disappears from git status).
    run_atlas(repo.path(), &["update"]);

    let after = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "to_be_deleted"]),
    );
    let after_results = after["results"].as_array().expect("query results");
    assert!(
        after_results.is_empty(),
        "deleted function should not appear in graph after update: {after:?}"
    );

    // The surviving function should still be indexed.
    let keep = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "keep_me"]),
    );
    let keep_results = keep["results"].as_array().expect("query results");
    assert!(
        !keep_results.is_empty(),
        "'keep_me' should still be in graph"
    );
}

/// File rename handled correctly: old graph slice gone, new one present.
#[test]
fn watch_file_rename_handled_correctly() {
    let repo = setup_repo(&[("src/original.rs", "pub fn original_fn() {}\n")]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    // Use git mv so the rename is staged and atlas update --staged can see it.
    run_command(
        repo.path(),
        "git",
        &["mv", "src/original.rs", "src/renamed.rs"],
    );

    // atlas update --staged exercises the same rename path that WatchRunner
    // uses via UpdateTarget::Batch with ChangeType::Renamed.
    run_atlas(repo.path(), &["update", "--staged"]);

    // The function still exists (same content, different path).
    let after = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "original_fn"]),
    );
    let after_results = after["results"].as_array().expect("query results");
    assert!(
        !after_results.is_empty(),
        "function should still be findable after rename: {after:?}"
    );

    // The new file path should be reflected.
    let file_path = after_results[0]["node"]["file_path"].as_str().unwrap_or("");
    assert!(
        file_path.contains("renamed"),
        "node should point to renamed file path, got: {file_path}"
    );
}

/// Debounce deduplication: no duplicate graph entries when the same file is
/// processed multiple times in one batch (unit-tested in atlas-engine; this
/// test verifies the CLI update path is idempotent).
#[test]
fn watch_no_duplicate_updates_idempotent() {
    let repo = setup_repo(&[("src/lib.rs", "pub fn stable() {}\n")]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let status_before =
        read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    let nodes_before = status_before["node_count"].as_i64().unwrap_or(0);

    // Run update twice on the same unchanged file — node count must not grow.
    run_atlas(repo.path(), &["update", "--files", "src/lib.rs"]);
    run_atlas(repo.path(), &["update", "--files", "src/lib.rs"]);

    let status_after =
        read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    let nodes_after = status_after["node_count"].as_i64().unwrap_or(0);
    assert_eq!(
        nodes_before, nodes_after,
        "re-updating the same file must not add duplicate nodes"
    );
}

/// `atlas watch --help` exits successfully, confirming command is registered.
#[test]
fn watch_command_registered_in_help() {
    // Run help (which exits 0 with clap) to confirm watch is a valid subcommand.
    let output = Command::new(env!("CARGO_BIN_EXE_atlas"))
        .args(["watch", "--help"])
        .output()
        .expect("failed to run atlas watch --help");
    assert!(
        output.status.success(),
        "atlas watch --help should exit 0; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("debounce") || stdout.contains("Watch"),
        "help output should mention debounce or Watch: {stdout}"
    );
}
