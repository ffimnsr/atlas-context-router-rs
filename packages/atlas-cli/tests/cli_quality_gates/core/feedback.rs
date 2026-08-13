use super::*;

/// Stores a feedback record via the CLI and returns the JSON `data` payload.
fn record_feedback(repo_root: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json", "feedback", "record"];
    full.extend_from_slice(args);
    read_json_data_output("feedback.record", run_atlas(repo_root, &full))
}

/// Calls an MCP tool over `atlas serve` and returns the parsed tool body.
fn mcp_call(repo_root: &Path, id: u64, name: &str, arguments: &str) -> Value {
    let request = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": serde_json::from_str::<Value>(arguments).expect("arguments json"),
        },
    }))
    .expect("serialize request");
    let requests = format!("{}{}\n", initialized_session_prelude(1), request);
    let output = run_serve_jsonrpc_session(repo_root, &["serve"], requests);
    read_json_tool_result(&output, id)
}

#[test]
fn feedback_record_requires_predicted_and_actual() {
    let repo = setup_fixture_repo();

    // Clap-level: both flags are required.
    let missing_predicted =
        run_atlas_capture(repo.path(), &["feedback", "record", "--actual", "x"]);
    assert!(!missing_predicted.status.success());
    let stderr = String::from_utf8(missing_predicted.stderr).expect("stderr utf-8");
    assert!(stderr.contains("--predicted"), "got: {stderr}");

    let missing_actual =
        run_atlas_capture(repo.path(), &["feedback", "record", "--predicted", "x"]);
    assert!(!missing_actual.status.success());
    let stderr = String::from_utf8(missing_actual.stderr).expect("stderr utf-8");
    assert!(stderr.contains("--actual"), "got: {stderr}");

    // Service-level: whitespace-only values fail validation too.
    let blank = run_atlas_capture(
        repo.path(),
        &[
            "feedback",
            "record",
            "--predicted",
            "   ",
            "--actual",
            "alive",
        ],
    );
    assert!(!blank.status.success());
    let stderr = String::from_utf8(blank.stderr).expect("stderr utf-8");
    assert!(
        stderr.contains("predicted must not be empty"),
        "got: {stderr}"
    );
}

#[test]
fn feedback_record_search_and_stats_roundtrip() {
    let repo = setup_fixture_repo();

    let first = record_feedback(
        repo.path(),
        &[
            "--predicted",
            "dead code",
            "--actual",
            "used by macro registry",
            "--correction",
            "keep the symbol; the macro registry loads it by name",
            "--analysis-kind",
            "dead_code",
            "--symbol",
            "src/lib.rs::fn::legacy_parse",
            "--file",
            "src/lib.rs",
            "--tool",
            "cli",
        ],
    );
    assert_eq!(first["record"]["analysis_kind"], json!("dead_code"));
    assert_eq!(
        first["record"]["related_symbol"],
        json!("src/lib.rs::fn::legacy_parse")
    );
    assert_eq!(first["summary"]["is_false_positive_evidence"], json!(true));

    let second = record_feedback(
        repo.path(),
        &[
            "--predicted",
            "removable",
            "--actual",
            "blocked by trait object",
            "--correction",
            "callers reach it through a trait object",
            "--analysis-kind",
            "remove",
            "--file",
            "src/auth.rs",
        ],
    );

    // Search by correction text with analysis-kind filter.
    let search = read_json_data_output(
        "feedback.search",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "feedback",
                "search",
                "trait object",
                "--analysis-kind",
                "remove",
            ],
        ),
    );
    assert_eq!(search["count"], json!(1));
    assert_eq!(
        search["results"][0]["feedback"]["id"],
        json!(second["record"]["id"])
    );
    assert!(
        search["results"][0]["relevance_score"]
            .as_f64()
            .unwrap_or(0.0)
            > 0.0,
        "FTS hits must carry a positive relevance score"
    );

    // Search by symbol.
    let by_symbol = read_json_data_output(
        "feedback.search",
        run_atlas(
            repo.path(),
            &["--json", "feedback", "search", "legacy_parse"],
        ),
    );
    assert_eq!(by_symbol["count"], json!(1));

    // Search by file filter.
    let by_file = read_json_data_output(
        "feedback.search",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "feedback",
                "search",
                "blocked",
                "--file",
                "src/auth.rs",
            ],
        ),
    );
    assert_eq!(by_file["count"], json!(1));

    // Stats: deterministic breakdown.
    let stats = read_json_data_output(
        "feedback.stats",
        run_atlas(repo.path(), &["--json", "feedback", "stats"]),
    );
    assert_eq!(stats["total_count"], json!(2));
    assert_eq!(stats["correction_count"], json!(2));
    assert_eq!(stats["false_positive_count"], json!(2));
    assert_eq!(stats["by_analysis_kind"]["dead_code"], json!(1));
    assert_eq!(stats["by_analysis_kind"]["remove"], json!(1));
    assert_eq!(stats["by_tool"]["cli"], json!(2));
}

#[test]
fn feedback_stats_are_stable_zero_counts_on_empty_db() {
    let repo = setup_fixture_repo();

    let stats = read_json_data_output(
        "feedback.stats",
        run_atlas(repo.path(), &["--json", "feedback", "stats"]),
    );
    assert_eq!(stats["total_count"], json!(0));
    assert_eq!(stats["correction_count"], json!(0));
    assert_eq!(stats["false_positive_count"], json!(0));
    assert_eq!(stats["by_analysis_kind"], json!({}));
    assert_eq!(stats["by_tool"], json!({}));
}

#[test]
fn mcp_feedback_record_matches_cli_record_shape() {
    let repo = setup_fixture_repo();

    let cli = record_feedback(
        repo.path(),
        &[
            "--predicted",
            "parity dead",
            "--actual",
            "parity alive",
            "--correction",
            "parity fix",
            "--tool",
            "cli",
            "--analysis-kind",
            "dead_code",
            "--symbol",
            "src/lib.rs::fn::parity",
            "--file",
            "src/lib.rs",
            "--source-id",
            "artifact-parity",
        ],
    );
    let mcp = mcp_call(
        repo.path(),
        2,
        "feedback_record",
        r#"{"predicted":"parity dead","actual":"parity alive","correction":"parity fix","tool":"cli","analysis_kind":"dead_code","symbol":"src/lib.rs::fn::parity","file":"src/lib.rs","source_id":"artifact-parity"}"#,
    );

    for field in [
        "repo_root",
        "session_id",
        "tool_name",
        "analysis_kind",
        "predicted",
        "actual",
        "correction",
        "related_symbol",
        "related_file",
        "source_id",
    ] {
        assert_eq!(
            cli["record"][field], mcp["record"][field],
            "CLI and MCP feedback records must agree on {field}"
        );
    }

    // MCP records with the same validation contract: blank actual fails.
    let request = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "feedback_record",
            "arguments": { "predicted": "x", "actual": "   " },
        },
    }))
    .expect("serialize request");
    let requests = format!("{}{}\n", initialized_session_prelude(1), request);
    let output = run_serve_jsonrpc_session(repo.path(), &["serve"], requests);
    let response = parse_jsonrpc_lines(&output.stdout)
        .into_iter()
        .find(|response| response["id"] == json!(3))
        .expect("feedback_record error response");
    assert_eq!(response["result"]["isError"], json!(true));
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("error text");
    assert!(
        text.contains("actual must not be empty"),
        "MCP error must match CLI validation: {text}"
    );
}

#[test]
fn feedback_adjustment_lowers_safety_confidence_only_on_matching_evidence() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let symbol = "src/lib.rs::fn::helper";

    let baseline = read_json_data_output(
        "analyze_safety",
        run_atlas(repo.path(), &["--json", "analyze", "safety", symbol]),
    );
    let baseline_score = baseline["safety"]["score"].as_f64().expect("score");
    assert!(
        baseline.get("feedback_evidence").is_none(),
        "no feedback records yet means no adjustment"
    );

    // Record a false positive for THIS symbol and kind.
    record_feedback(
        repo.path(),
        &[
            "--predicted",
            "safe to refactor",
            "--actual",
            "broke the build",
            "--correction",
            "helper is reached through reflection",
            "--analysis-kind",
            "safety",
            "--symbol",
            symbol,
        ],
    );

    let adjusted = read_json_data_output(
        "analyze_safety",
        run_atlas(repo.path(), &["--json", "analyze", "safety", symbol]),
    );
    let adjusted_score = adjusted["safety"]["score"].as_f64().expect("score");
    assert!(
        (baseline_score - adjusted_score - 0.1).abs() < 1e-9,
        "matching feedback must lower the safety score by exactly 0.1: {baseline_score} -> {adjusted_score}"
    );
    let evidence = adjusted["feedback_evidence"].as_array().expect("evidence");
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0]["matched_on"], json!("symbol"));
    assert_eq!(evidence[0]["analysis_kind"], json!("safety"));

    // A record for a DIFFERENT symbol must not change anything.
    record_feedback(
        repo.path(),
        &[
            "--predicted",
            "safe to refactor",
            "--actual",
            "broke the build",
            "--analysis-kind",
            "safety",
            "--symbol",
            "src/lib.rs::method::Greeter::greet_twice",
        ],
    );
    let untouched = read_json_data_output(
        "analyze_safety",
        run_atlas(repo.path(), &["--json", "analyze", "safety", symbol]),
    );
    assert_eq!(untouched["safety"]["score"], adjusted["safety"]["score"]);

    // Disabling the flag restores the deterministic score.
    fs::write(
        repo.path().join(".atlas").join("config.toml"),
        "[analysis.feedback_adjustment]\nenabled = false\n",
    )
    .expect("write config");
    let disabled = read_json_data_output(
        "analyze_safety",
        run_atlas(repo.path(), &["--json", "analyze", "safety", symbol]),
    );
    assert!(
        (baseline_score - disabled["safety"]["score"].as_f64().unwrap()).abs() < 1e-9,
        "disabled config flag must restore the original score"
    );
}

#[test]
fn feedback_adjustment_lowers_dead_code_certainty_and_remove_impact() {
    // Dedicated repo with a genuinely dead private fn; the shared fixture has
    // no dead-code candidates by design.
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn live() -> u32 {\n    1\n}\n\nfn dead() -> u32 {\n    42\n}\n",
    )]);
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    // Dead-code: record a correction for one candidate symbol with kind
    // dead_code, then re-run the scan and confirm the matching candidate's
    // certainty dropped one tier and feedback_evidence is exposed.
    let before = read_json_data_output(
        "analyze_dead_code",
        run_atlas(repo.path(), &["--json", "analyze", "dead-code"]),
    );
    let candidates = before.as_array().expect("candidates array");
    assert!(
        !candidates.is_empty(),
        "repo must produce dead-code candidates"
    );
    let target = candidates
        .iter()
        .find(|candidate| candidate["node"]["qualified_name"] == json!("src/lib.rs::fn::dead"))
        .cloned()
        .unwrap_or_else(|| candidates[0].clone());
    let symbol = target["node"]["qualified_name"].as_str().expect("qname");
    let before_tier = target["certainty"].as_str().expect("certainty");

    record_feedback(
        repo.path(),
        &[
            "--predicted",
            "dead code",
            "--actual",
            "loaded by the macro registry",
            "--correction",
            "registered externally",
            "--analysis-kind",
            "dead_code",
            "--symbol",
            symbol,
        ],
    );

    let after = read_json_data_output(
        "analyze_dead_code",
        run_atlas(repo.path(), &["--json", "analyze", "dead-code"]),
    );
    let after_candidates = after["candidates"].as_array().expect("wrapped candidates");
    let after_target = after_candidates
        .iter()
        .find(|candidate| candidate["node"]["qualified_name"].as_str() == Some(symbol))
        .expect("target candidate still present");
    let after_tier = after_target["certainty"].as_str().expect("certainty");
    let tier_rank = |tier: &str| match tier {
        "high" => 0,
        "medium" => 1,
        "low" => 2,
        _ => 3,
    };
    assert!(
        tier_rank(after_tier) > tier_rank(before_tier),
        "certainty must drop one tier: {before_tier} -> {after_tier}"
    );
    assert!(
        after["feedback_evidence"]
            .as_array()
            .expect("evidence")
            .iter()
            .any(|entry| entry["related_symbol"].as_str() == Some(symbol)),
        "evidence must reference the matched symbol"
    );

    // Remove impact: matching evidence lowers Definite -> Probable.
    let remove_before = read_json_data_output(
        "analyze_remove",
        run_atlas(repo.path(), &["--json", "analyze", "remove", symbol]),
    );
    let definite = remove_before["impacted_symbols"]
        .as_array()
        .expect("impacted symbols")
        .iter()
        .find(|impact| impact["impact_class"] == json!("definite"))
        .cloned();
    if let Some(definite_impact) = definite {
        let impacted_qname = definite_impact["node"]["qualified_name"]
            .as_str()
            .expect("qname");
        record_feedback(
            repo.path(),
            &[
                "--predicted",
                "definitely impacted",
                "--actual",
                "not impacted",
                "--correction",
                "reached through dynamic dispatch",
                "--analysis-kind",
                "remove",
                "--symbol",
                impacted_qname,
            ],
        );
        let remove_after = read_json_data_output(
            "analyze_remove",
            run_atlas(repo.path(), &["--json", "analyze", "remove", symbol]),
        );
        let after_impact = remove_after["impacted_symbols"]
            .as_array()
            .expect("impacted symbols")
            .iter()
            .find(|impact| impact["node"]["qualified_name"] == json!(impacted_qname))
            .expect("impacted symbol still present");
        assert_eq!(after_impact["impact_class"], json!("probable"));
        assert!(
            remove_after["feedback_evidence"]
                .as_array()
                .expect("evidence")
                .iter()
                .any(|entry| entry["matched_on"] == json!("symbol")),
            "removal evidence must be exposed"
        );
    }
}
