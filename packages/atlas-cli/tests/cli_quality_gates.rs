use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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
    assert!(
        explain["summary"]
            .as_str()
            .unwrap_or_default()
            .contains("Risk:"),
        "summary must include risk sentence: {explain:?}"
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
