use super::*;

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
        .expect("query results should return JSON array");
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
        .expect("detect-changes changes should return JSON array");
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
    let analysis = &impact["analysis"];
    let base = &analysis["base"];
    assert!(
        base["changed_nodes"]
            .as_array()
            .expect("impact changed_nodes should be array")
            .iter()
            .any(|node| node["file_path"] == json!("src/lib.rs"))
    );
    assert!(
        base["changed_nodes"]
            .as_array()
            .expect("impact changed_nodes should be array")
            .iter()
            .any(|node| node["qualified_name"] == json!("src/lib.rs::fn::helper"))
    );
    assert!(
        base["relevant_edges"]
            .as_array()
            .expect("impact relevant_edges should be array")
            .iter()
            .any(|edge| edge["kind"] == json!("calls"))
    );
    assert!(analysis["risk_level"].is_string());
    assert!(analysis["scored_nodes"].is_array());
    assert!(analysis["test_impact"].is_object());
    assert!(analysis["boundary_violations"].is_array());

    let review_ctx = read_json_data_output(
        "review_context",
        run_atlas(repo.path(), &["--json", "review-context", "--base", "HEAD"]),
    );
    assert!(
        review_ctx["files"]
            .as_array()
            .expect("review-context files must be array")
            .iter()
            .any(|file| file["path"] == json!("src/lib.rs")),
        "review-context files must include src/lib.rs"
    );
    assert!(
        review_ctx["nodes"]
            .as_array()
            .expect("review-context nodes must be array")
            .iter()
            .any(|node| node["node"]["file_path"] == json!("src/lib.rs")),
        "review-context nodes must include nodes from src/lib.rs"
    );
    assert!(review_ctx["truncation"].is_object());
    assert_eq!(review_ctx["request"]["intent"], json!("review"));

    let status_with_base = read_json_data_output(
        "status",
        run_atlas(repo.path(), &["--json", "status", "--base", "HEAD"]),
    );
    assert_eq!(status_with_base["changed_file_count"], json!(1));
    assert_eq!(status_with_base["diff_target"]["kind"], json!("base_ref"));
    assert_eq!(status_with_base["changed_files"][0]["path"], json!("src/lib.rs"));
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
    assert_eq!(explain["diff_summary"]["files"][0]["change_type"], json!("modified"));
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
            .any(|item| {
                item.as_str().unwrap_or_default().contains("Change")
                    || item.as_str().unwrap_or_default().contains("Impact")
                    || item.as_str().unwrap_or_default().contains("Primary")
            }),
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
fn explain_query_cli_and_mcp_share_execution_explanation() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let cli_explain = read_json_data_output(
        "explain_query",
        run_atlas(repo.path(), &["--json", "explain-query", "greet_twice"]),
    );

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"explain_query\",\"arguments\":{\"text\":\"greet_twice\",\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(
        output.status.success(),
        "atlas serve explain_query failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let mcp_explain = read_json_tool_result(&output, 2);
    assert_eq!(cli_explain["active_query_mode"], mcp_explain["active_query_mode"]);
    assert_eq!(cli_explain["search_path"], mcp_explain["search_path"]);
    assert_eq!(cli_explain["result_count"], mcp_explain["result_count"]);
    assert_eq!(cli_explain["ranking_factors"], mcp_explain["ranking_factors"]);
    assert_eq!(
        cli_explain["matches"][0]["qualified_name"],
        mcp_explain["matches"][0]["qualified_name"]
    );

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn query_cli_and_mcp_share_ranked_results() {
    let repo = setup_repo(&[("src/foo.rs", "pub fn helper() {}\n"), ("src/bar.rs", "pub fn helper() {}\n")]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let cli_query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "helper"]),
    );
    let cli_qnames = atlas_query_qnames(&cli_query);

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"helper\",\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(
        output.status.success(),
        "atlas serve query_graph failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let mcp_query = read_json_tool_result(&output, 2);
    let mcp_qnames: Vec<String> = mcp_query
        .as_array()
        .expect("mcp query results array")
        .iter()
        .filter_map(|result| result["qn"].as_str().map(str::to_owned))
        .collect();

    assert_eq!(cli_qnames, mcp_qnames);

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn mcp_query_and_explain_query_share_match_order() {
    let repo = setup_repo(&[("src/foo.rs", "pub fn helper() {}\n"), ("src/bar.rs", "pub fn helper() {}\n")]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"helper\",\"output_format\":\"json\"}}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"explain_query\",\"arguments\":{\"text\":\"helper\",\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(
        output.status.success(),
        "atlas serve query/explain_query failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let query = read_json_tool_result(&output, 2);
    let explain = read_json_tool_result(&output, 3);

    let query_qnames: Vec<String> = query
        .as_array()
        .expect("query_graph results array")
        .iter()
        .filter_map(|result| result["qn"].as_str().map(str::to_owned))
        .collect();
    let explain_qnames: Vec<String> = explain["matches"]
        .as_array()
        .expect("explain_query matches array")
        .iter()
        .filter_map(|result| result["qualified_name"].as_str().map(str::to_owned))
        .collect();

    assert_eq!(query_qnames, explain_qnames);

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn explain_change_cli_and_mcp_share_summary_builder() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    rewrite_fixture_helper(repo.path());

    let cli_explain = read_json_data_output(
        "explain_change",
        run_atlas(repo.path(), &["--json", "explain-change", "--base", "HEAD"]),
    );

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"explain_change\",\"arguments\":{\"base\":\"HEAD\",\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(
        output.status.success(),
        "atlas serve explain_change failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let mcp_explain = read_json_tool_result(&output, 2);
    assert_eq!(cli_explain["risk_level"], mcp_explain["risk_level"]);
    assert_eq!(cli_explain["changed_file_count"], mcp_explain["changed_file_count"]);
    assert_eq!(cli_explain["changed_symbol_count"], mcp_explain["changed_symbol_count"]);
    assert_eq!(cli_explain["changed_by_kind"], mcp_explain["changed_by_kind"]);
    assert_eq!(cli_explain["diff_summary"], mcp_explain["diff_summary"]);
    assert_eq!(cli_explain["summary"], mcp_explain["summary"]);

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn review_context_cli_and_get_context_share_review_seed_results() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let cli_review = read_json_data_output(
        "review_context",
        run_atlas(repo.path(), &["--json", "review-context", "--files", "src/lib.rs"]),
    );

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"get_context\",\"arguments\":{\"files\":[\"src/lib.rs\"],\"intent\":\"review\",\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(
        output.status.success(),
        "atlas serve get_context failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let mcp_context = read_json_tool_result(&output, 2);

    let cli_file_paths: Vec<String> = cli_review["files"]
        .as_array()
        .expect("cli review files array")
        .iter()
        .filter_map(|file| file["path"].as_str().map(str::to_owned))
        .collect();
    let mcp_file_paths: Vec<String> = mcp_context["files"]
        .as_array()
        .expect("mcp get_context files array")
        .iter()
        .filter_map(|file| file["path"].as_str().map(str::to_owned))
        .collect();
    let cli_qnames: Vec<String> = cli_review["nodes"]
        .as_array()
        .expect("cli review nodes array")
        .iter()
        .filter_map(|node| node["node"]["qualified_name"].as_str().map(str::to_owned))
        .collect();
    let mcp_qnames: Vec<String> = mcp_context["nodes"]
        .as_array()
        .expect("mcp get_context nodes array")
        .iter()
        .filter_map(|node| node["qn"].as_str().map(str::to_owned))
        .collect();
    let cli_edges: Vec<(String, String, String)> = cli_review["edges"]
        .as_array()
        .expect("cli review edges array")
        .iter()
        .map(|edge| {
            (
                edge["edge"]["source_qn"].as_str().unwrap_or_default().to_string(),
                edge["edge"]["target_qn"].as_str().unwrap_or_default().to_string(),
                edge["selection_reason"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    let mcp_edges: Vec<(String, String, String)> = mcp_context["edges"]
        .as_array()
        .expect("mcp get_context edges array")
        .iter()
        .map(|edge| {
            (
                edge["from"].as_str().unwrap_or_default().to_string(),
                edge["to"].as_str().unwrap_or_default().to_string(),
                edge["reason"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();

    assert_eq!(cli_review["request"]["intent"], json!("review"));
    assert_eq!(mcp_context["intent"], json!("review"));
    assert_eq!(cli_file_paths, mcp_file_paths);
    assert_eq!(cli_qnames, mcp_qnames);
    assert_eq!(cli_edges, mcp_edges);
    assert_eq!(cli_review["truncation"]["truncated"], mcp_context["truncated"]);
    assert_eq!(cli_review["truncation"]["nodes_dropped"], mcp_context["nodes_dropped"]);
    assert_eq!(cli_review["truncation"]["edges_dropped"], mcp_context["edges_dropped"]);

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn review_context_and_get_context_share_node_trimming_semantics() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let cli_review = read_json_data_output(
        "review_context",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "review-context",
                "--files",
                "src/lib.rs",
                "--max-nodes",
                "1",
            ],
        ),
    );

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"get_context\",\"arguments\":{\"files\":[\"src/lib.rs\"],\"intent\":\"review\",\"max_nodes\":1,\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(
        output.status.success(),
        "atlas serve get_context truncation case failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let mcp_context = read_json_tool_result(&output, 2);

    assert_eq!(cli_review["truncation"]["truncated"], json!(true));
    assert_eq!(cli_review["truncation"]["truncated"], mcp_context["truncated"]);
    assert_eq!(cli_review["truncation"]["nodes_dropped"], mcp_context["nodes_dropped"]);
    assert_eq!(cli_review["nodes"].as_array().expect("cli nodes").len(), 1);
    assert_eq!(mcp_context["nodes"].as_array().expect("mcp nodes").len(), 1);

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn impact_and_explain_change_share_changed_file_seed_summary() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    rewrite_fixture_helper(repo.path());

    let impact = read_json_data_output(
        "impact",
        run_atlas(repo.path(), &["--json", "impact", "--base", "HEAD"]),
    );
    let explain = read_json_data_output(
        "explain_change",
        run_atlas(repo.path(), &["--json", "explain-change", "--base", "HEAD"]),
    );

    let impact_changed_paths: std::collections::BTreeSet<String> = impact["analysis"]["base"]["changed_nodes"]
        .as_array()
        .expect("impact changed_nodes array")
        .iter()
        .filter_map(|node| node["file_path"].as_str().map(str::to_owned))
        .collect();
    let explain_paths: std::collections::BTreeSet<String> = explain["diff_summary"]["files"]
        .as_array()
        .expect("explain-change diff_summary files array")
        .iter()
        .filter_map(|file| file["path"].as_str().map(str::to_owned))
        .collect();

    assert_eq!(impact["analysis"]["risk_level"], explain["risk_level"]);
    assert_eq!(explain["changed_file_count"], json!(1));
    assert_eq!(explain_paths.len(), 1);
    assert!(impact_changed_paths.contains("src/lib.rs"));
    assert_eq!(explain_paths, std::collections::BTreeSet::from(["src/lib.rs".to_string()]));

    let impact_changed_qnames: std::collections::BTreeSet<String> = impact["analysis"]["base"]["changed_nodes"]
        .as_array()
        .expect("impact changed_nodes array")
        .iter()
        .filter_map(|node| node["qualified_name"].as_str().map(str::to_owned))
        .collect();
    let explain_changed_qnames: std::collections::BTreeSet<String> = explain["changed_symbols"]
        .as_array()
        .expect("explain-change changed_symbols array")
        .iter()
        .filter_map(|node| node["qn"].as_str().map(str::to_owned))
        .collect();

    assert_eq!(impact_changed_qnames, explain_changed_qnames);
}

#[test]
fn impact_and_explain_change_share_max_node_cap() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    rewrite_fixture_helper(repo.path());

    let impact = read_json_data_output(
        "impact",
        run_atlas(
            repo.path(),
            &["--json", "impact", "--base", "HEAD", "--max-nodes", "1"],
        ),
    );
    let explain = read_json_data_output(
        "explain_change",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "explain-change",
                "--base",
                "HEAD",
                "--max-nodes",
                "1",
            ],
        ),
    );

    let impact_changed_nodes = impact["analysis"]["base"]["changed_nodes"]
        .as_array()
        .expect("impact changed_nodes array");
    let impact_impacted_nodes = impact["analysis"]["base"]["impacted_nodes"]
        .as_array()
        .expect("impact impacted_nodes array");

    assert_eq!(impact["analysis"]["risk_level"], explain["risk_level"]);
    assert_eq!(impact_changed_nodes.len(), explain["changed_symbol_count"].as_u64().unwrap_or_default() as usize);
    assert_eq!(impact_impacted_nodes.len(), explain["impacted_node_count"].as_u64().unwrap_or_default() as usize);
    assert!(impact_impacted_nodes.len() <= 1, "impact max_nodes cap must apply: {impact:?}");
    assert!(
        explain["high_impact_nodes"].as_array().expect("explain high_impact_nodes").len() <= 1,
        "explain-change workflow should respect same cap: {explain:?}"
    );
}

#[test]
fn analyze_remove_cli_and_mcp_share_ordering_primitives() {
    let repo = setup_repo(&[
        (
            "src/lib.rs",
            "mod a;\nmod b;\nmod c;\n\npub fn root() {\n    b::caller_b();\n    c::caller_c();\n}\n",
        ),
        ("src/a.rs", "pub fn target() {}\n"),
        ("src/b.rs", "pub fn caller_b() {\n    crate::a::target();\n}\n"),
        ("src/c.rs", "pub fn caller_c() {\n    crate::a::target();\n}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let cli_remove = read_json_data_output(
        "analyze_remove",
        run_atlas(repo.path(), &["--json", "analyze", "remove", "src/a.rs::fn::target"]),
    );
    let cli_impacted = cli_remove["impacted_symbols"].as_array().expect("cli impacted symbols array");

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"analyze_remove\",\"arguments\":{\"symbols\":[\"src/a.rs::fn::target\"],\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(
        output.status.success(),
        "atlas serve analyze_remove failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let mcp_remove = read_json_tool_result(&output, 2);
    let mcp_impacted = mcp_remove["impacted_symbols"].as_array().expect("mcp impacted symbols array");

    assert_eq!(cli_impacted[0]["node"]["qualified_name"], mcp_impacted[0]["qn"]);
    assert_eq!(cli_impacted[1]["node"]["qualified_name"], mcp_impacted[1]["qn"]);

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn analyze_dead_code_cli_and_mcp_share_ordering_primitives() {
    let repo = setup_repo(&[
        (
            "src/lib.rs",
            "mod alpha;\nmod beta;\n\npub fn live() {\n    alpha::live();\n}\n",
        ),
        ("src/alpha.rs", "pub fn live() {}\n\nfn unused_alpha() {}\n"),
        ("src/beta.rs", "fn unused_beta() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let cli_dead_code = read_json_data_output(
        "analyze_dead_code",
        run_atlas(repo.path(), &["--json", "analyze", "dead-code"]),
    );
    let cli_candidates = cli_dead_code.as_array().expect("cli dead-code candidates array");

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"analyze_dead_code\",\"arguments\":{\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(
        output.status.success(),
        "atlas serve analyze_dead_code failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let mcp_dead_code = read_json_tool_result(&output, 2);
    let mcp_candidates = mcp_dead_code["candidates"].as_array().expect("mcp dead-code candidates array");

    assert_eq!(cli_candidates[0]["node"]["qualified_name"], mcp_candidates[0]["qn"]);
    assert_eq!(cli_candidates[1]["node"]["qualified_name"], mcp_candidates[1]["qn"]);

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn analyze_dependency_cli_and_mcp_share_ordering_primitives() {
    let repo = setup_repo(&[
        (
            "src/lib.rs",
            "mod a;\nmod b;\nmod c;\n\npub fn root() {\n    b::caller_b();\n    c::caller_c();\n}\n",
        ),
        ("src/a.rs", "pub fn target() {}\n"),
        ("src/b.rs", "pub fn caller_b() {\n    crate::a::target();\n}\n"),
        ("src/c.rs", "pub fn caller_c() {\n    crate::a::target();\n}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let cli_dependency = read_json_data_output(
        "analyze_dependency",
        run_atlas(repo.path(), &["--json", "analyze", "dependency", "src/a.rs::fn::target"]),
    );
    let cli_blockers = cli_dependency["blocking_references"].as_array().expect("cli blocking refs array");

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"analyze_dependency\",\"arguments\":{\"symbol\":\"src/a.rs::fn::target\",\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(
        output.status.success(),
        "atlas serve analyze_dependency failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let mcp_dependency = read_json_tool_result(&output, 2);
    let mcp_blockers = mcp_dependency["blocking_references"].as_array().expect("mcp blocking refs array");

    assert_eq!(cli_blockers[0]["qualified_name"], mcp_blockers[0]["qn"]);
    assert_eq!(cli_blockers[1]["qualified_name"], mcp_blockers[1]["qn"]);

    cleanup_mcp_daemons(repo.path());
}
