use super::*;

#[test]
fn mcp_memory_store_and_recall_match_cli_record_shape() {
    let repo = setup_fixture_repo();

    // Same inputs through the CLI and through MCP must produce the same record
    // shape (id and timestamps are per-write values).
    let cli = store_memory(
        repo.path(),
        &[
            "parity body text",
            "--topic",
            "parity",
            "--title",
            "Parity note",
            "--importance",
            "critical",
            "--source-id",
            "artifact-parity",
        ],
    );
    let mcp = mcp_call(
        repo.path(),
        2,
        "memory_store",
        r#"{"text":"parity body text","topic":"parity","title":"Parity note","importance":"critical","source_id":"artifact-parity"}"#,
    );

    for field in [
        "repo_root",
        "session_id",
        "frontend",
        "scope",
        "topic",
        "title",
        "body",
        "importance",
        "decay_score",
        "source_id",
        "metadata",
    ] {
        assert_eq!(
            cli["memory"][field], mcp["memory"][field],
            "CLI and MCP memory records must agree on {field}"
        );
    }

    // Defaults agree too: no importance/scope means normal + project on both.
    let cli_defaults = store_memory(repo.path(), &["defaults body"]);
    let mcp_defaults = mcp_call(
        repo.path(),
        3,
        "memory_store",
        r#"{"text":"defaults body"}"#,
    );
    assert_eq!(cli_defaults["memory"]["importance"], json!("normal"));
    assert_eq!(cli_defaults["memory"]["scope"], json!("project"));
    assert_eq!(mcp_defaults["memory"]["importance"], json!("normal"));
    assert_eq!(mcp_defaults["memory"]["scope"], json!("project"));

    // MCP recall sees the memories stored through both surfaces (project scope
    // is visible to the mcp viewer) and returns retrieval hints.
    let recall = mcp_call(repo.path(), 4, "memory_recall", r#"{"query":"parity"}"#);
    assert!(recall["summary"]["match_count"].as_u64().unwrap() >= 1);
    assert!(
        recall["results"]
            .as_array()
            .expect("recall results")
            .iter()
            .any(|hit| hit["memory"]["source_id"] == json!("artifact-parity")),
        "source ids must be available in compact MCP output"
    );
    assert!(
        recall["retrieval_hints"]
            .as_array()
            .expect("retrieval hints")
            .iter()
            .any(|hint| hint["kind"] == json!("source_id")
                && hint["value"] == json!("artifact-parity")),
        "retrieval hints must surface source ids"
    );
}

#[test]
fn mcp_memory_store_errors_match_cli_validation() {
    let repo = setup_fixture_repo();

    // CLI rejection message.
    let cli_fail = run_atlas_capture(
        repo.path(),
        &["memory", "store", "x", "--importance", "urgent"],
    );
    assert!(!cli_fail.status.success());
    let cli_stderr = String::from_utf8(cli_fail.stderr).expect("stderr utf-8");

    // MCP rejection must carry the same validation message.
    let request = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "memory_store",
            "arguments": {
                "text": "x",
                "importance": "urgent"
            },
        },
    }))
    .expect("serialize request");
    let requests = format!("{}{}\n", initialized_session_prelude(1), request);
    let output = run_serve_jsonrpc_session(repo.path(), &["serve"], requests);
    let response = parse_jsonrpc_lines(&output.stdout)
        .into_iter()
        .find(|response| response["id"] == json!(2))
        .expect("memory_store error response");
    assert_eq!(response["result"]["isError"], json!(true));
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .expect("error text");
    assert!(
        text.contains("unknown memory importance: urgent"),
        "MCP error must match CLI validation: {text}"
    );
    assert!(
        cli_stderr.contains("unknown memory importance: urgent"),
        "CLI stderr must carry the same validation: {cli_stderr}"
    );
}
