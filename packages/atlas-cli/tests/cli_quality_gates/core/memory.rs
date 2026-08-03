use super::*;

/// Stores a memory via the CLI and returns the JSON `data` payload.
fn store_memory(repo_root: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json", "memory", "store"];
    full.extend_from_slice(args);
    read_json_data_output("memory.store", run_atlas(repo_root, &full))
}

fn memory_id(payload: &Value) -> String {
    payload["memory"]["id"]
        .as_str()
        .expect("memory id string")
        .to_owned()
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
        r#"{"text":"parity body text","topic":"parity","title":"Parity note","importance":"critical","source_id":"artifact-parity","output_format":"json"}"#,
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
        r#"{"text":"defaults body","output_format":"json"}"#,
    );
    assert_eq!(cli_defaults["memory"]["importance"], json!("normal"));
    assert_eq!(cli_defaults["memory"]["scope"], json!("project"));
    assert_eq!(mcp_defaults["memory"]["importance"], json!("normal"));
    assert_eq!(mcp_defaults["memory"]["scope"], json!("project"));

    // MCP recall sees the memories stored through both surfaces (project scope
    // is visible to the mcp viewer) and returns retrieval hints.
    let recall = mcp_call(
        repo.path(),
        4,
        "memory_recall",
        r#"{"query":"parity","output_format":"json"}"#,
    );
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
                "importance": "urgent",
                "output_format": "json"
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

#[test]
fn memory_store_creates_record_with_defaults() {
    let repo = setup_fixture_repo();

    let payload = store_memory(repo.path(), &["remember to run the hook tests"]);

    let memory = &payload["memory"];
    let id = memory_id(&payload);
    assert_eq!(id.len(), 64, "id must be a stable sha256 hex id");
    assert_eq!(memory["body"], json!("remember to run the hook tests"));
    assert_eq!(memory["importance"], json!("normal"));
    assert_eq!(memory["scope"], json!("project"));
    assert_eq!(memory["topic"], json!(""));
    assert_eq!(memory["decay_score"], json!(0.0));
    assert_eq!(memory["session_id"], json!(null));
    assert_eq!(memory["created_at"], memory["updated_at"]);
    assert_eq!(memory["updated_at"], memory["last_accessed_at"]);

    // Row must exist in the continuity-side session database.
    let session_db = repo.path().join(".atlas").join("session.db");
    let conn = Connection::open(&session_db).expect("open session db");
    let (body, importance, scope): (String, String, String) = conn
        .query_row(
            "SELECT body, importance, scope FROM memories WHERE id = ?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("stored row");
    assert_eq!(body, "remember to run the hook tests");
    assert_eq!(importance, "normal");
    assert_eq!(scope, "project");
}

#[test]
fn memory_store_persists_all_flags() {
    let repo = setup_fixture_repo();

    let payload = store_memory(
        repo.path(),
        &[
            "frontend-private deploy note",
            "--topic",
            "deploy",
            "--title",
            "Deploy note",
            "--importance",
            "critical",
            "--scope",
            "frontend",
            "--frontend",
            "codex",
            "--source-id",
            "artifact-9",
        ],
    );

    let memory = &payload["memory"];
    assert_eq!(memory["importance"], json!("critical"));
    assert_eq!(memory["scope"], json!("frontend"));
    assert_eq!(memory["frontend"], json!("codex"));
    assert_eq!(memory["topic"], json!("deploy"));
    assert_eq!(memory["title"], json!("Deploy note"));
    assert_eq!(memory["source_id"], json!("artifact-9"));
    assert_eq!(memory["body"], json!("frontend-private deploy note"));

    // Global scope needs no active session either.
    let global = store_memory(
        repo.path(),
        &["global note", "--scope", "global", "--importance", "high"],
    );
    assert_eq!(global["memory"]["scope"], json!("global"));
    assert_eq!(global["memory"]["session_id"], json!(null));
}

#[test]
fn memory_store_rejects_invalid_values() {
    let repo = setup_fixture_repo();

    let cases: &[(&[&str], &str)] = &[
        (
            &["x", "--importance", "urgent"],
            "unknown memory importance",
        ),
        (&["x", "--scope", "org"], "unknown memory scope"),
        (
            &["x", "--scope", "frontend"],
            "requires a frontend identifier",
        ),
        (&["   "], "memory body must not be empty"),
    ];

    for (args, expected) in cases {
        let mut full = vec!["memory", "store"];
        full.extend_from_slice(args);
        let output = run_atlas_capture(repo.path(), &full);
        assert!(!output.status.success(), "store {:?} must fail", args);
        let stderr = String::from_utf8(output.stderr).expect("stderr utf-8");
        assert!(
            stderr.contains(expected),
            "stderr must mention {expected:?}\nstderr:\n{stderr}"
        );
    }
}

#[test]
fn memory_recall_ranks_exact_topic_first_and_enforces_visibility() {
    let repo = setup_fixture_repo();

    store_memory(
        repo.path(),
        &[
            "deploy pipeline runs weekly",
            "--topic",
            "hooks",
            "--importance",
            "critical",
        ],
    );
    let exact = store_memory(
        repo.path(),
        &[
            "deploy notes",
            "--topic",
            "deploy",
            "--importance",
            "critical",
        ],
    );
    let frontend_private = store_memory(
        repo.path(),
        &[
            "deploy secrets",
            "--topic",
            "deploy",
            "--scope",
            "frontend",
            "--frontend",
            "codex",
        ],
    );
    let session_scoped = store_memory(
        repo.path(),
        &[
            "deploy session note",
            "--topic",
            "deploy",
            "--scope",
            "session",
        ],
    );

    let recall = read_json_data_output(
        "memory.recall",
        run_atlas(repo.path(), &["--json", "memory", "recall", "deploy"]),
    );
    assert_eq!(recall["query"], json!("deploy"));
    let results = recall["results"].as_array().expect("recall results");
    let result_ids = results
        .iter()
        .map(|hit| hit["memory"]["id"].as_str().expect("id"))
        .collect::<Vec<_>>();

    // The cli viewer sees its own session memory but NOT codex-frontend ones.
    assert_eq!(results.len(), 3);
    assert!(!result_ids.contains(&memory_id(&frontend_private).as_str()));
    assert!(result_ids.contains(&memory_id(&session_scoped).as_str()));

    // Exact topic matches rank above broad body matches.
    assert_eq!(results[0]["memory"]["id"], json!(memory_id(&exact)));
    assert_eq!(results[0]["relevance_score"], json!(0));
    assert_eq!(
        results[1]["memory"]["id"],
        json!(memory_id(&session_scoped))
    );
    assert_eq!(results[2]["relevance_score"], json!(2));

    // --shared excludes session and frontend scoped memories entirely.
    let shared = read_json_data_output(
        "memory.recall",
        run_atlas(
            repo.path(),
            &["--json", "memory", "recall", "deploy", "--shared"],
        ),
    );
    let shared_ids = shared["results"]
        .as_array()
        .expect("shared results")
        .iter()
        .map(|hit| hit["memory"]["id"].as_str().expect("id"))
        .collect::<Vec<_>>();
    assert!(!shared_ids.contains(&memory_id(&frontend_private).as_str()));
    assert!(!shared_ids.contains(&memory_id(&session_scoped).as_str()));
    assert_eq!(shared_ids.len(), 2);

    // --scope frontend from the cli viewer shows only cli-frontend memories.
    let before = read_json_data_output(
        "memory.recall",
        run_atlas(
            repo.path(),
            &[
                "--json", "memory", "recall", "deploy", "--scope", "frontend",
            ],
        ),
    );
    assert_eq!(before["count"], json!(0));

    let cli_private = store_memory(
        repo.path(),
        &[
            "cli deploy note",
            "--topic",
            "deploy",
            "--scope",
            "frontend",
            "--frontend",
            "cli",
        ],
    );
    let after = read_json_data_output(
        "memory.recall",
        run_atlas(
            repo.path(),
            &[
                "--json", "memory", "recall", "deploy", "--scope", "frontend",
            ],
        ),
    );
    let frontend_ids = after["results"]
        .as_array()
        .expect("frontend results")
        .iter()
        .map(|hit| hit["memory"]["id"].as_str().expect("id"))
        .collect::<Vec<_>>();
    assert_eq!(frontend_ids, vec![memory_id(&cli_private).as_str()]);
    assert!(!frontend_ids.contains(&memory_id(&frontend_private).as_str()));
}

#[test]
fn memory_frontend_identities_are_normalized_and_unknown_ones_rejected() {
    let repo = setup_fixture_repo();

    // Known aliases normalize to canonical identities.
    let stored = store_memory(
        repo.path(),
        &[
            "claude note",
            "--scope",
            "frontend",
            "--frontend",
            "Claude Code",
        ],
    );
    assert_eq!(stored["memory"]["frontend"], json!("claude"));

    // Unknown frontends fail with a clear validation error.
    let unknown = run_atlas_capture(
        repo.path(),
        &[
            "memory",
            "store",
            "zed note",
            "--scope",
            "frontend",
            "--frontend",
            "zed",
        ],
    );
    assert!(!unknown.status.success());
    let stderr = String::from_utf8(unknown.stderr).expect("stderr utf-8");
    assert!(stderr.contains("unknown frontend: zed"), "got: {stderr}");
    assert!(
        stderr.contains("codex"),
        "error must list known frontends: {stderr}"
    );

    // Config explicitly allowing custom frontends accepts them verbatim.
    fs::write(
        repo.path().join(".atlas").join("config.toml"),
        "[memory]\nallow_custom_frontends = true\n",
    )
    .expect("write config");
    let custom = store_memory(
        repo.path(),
        &["zed note", "--scope", "frontend", "--frontend", "Zed Agent"],
    );
    assert_eq!(custom["memory"]["frontend"], json!("zed agent"));

    // The cli viewer never sees claude- or zed-frontend memories.
    let recall = read_json_data_output(
        "memory.recall",
        run_atlas(
            repo.path(),
            &["--json", "memory", "recall", "note", "--scope", "frontend"],
        ),
    );
    assert_eq!(recall["count"], json!(0));
}

#[test]
fn memory_list_filters_and_sorts_by_updated_at_desc() {
    let repo = setup_fixture_repo();

    let critical = store_memory(
        repo.path(),
        &[
            "critical hooks note",
            "--topic",
            "hooks",
            "--importance",
            "critical",
        ],
    );
    let low = store_memory(
        repo.path(),
        &["low hooks note", "--topic", "hooks", "--importance", "low"],
    );
    let global = store_memory(
        repo.path(),
        &["global note", "--topic", "other", "--scope", "global"],
    );

    // Backdate rows so updated_at ordering is observable.
    let session_db = repo.path().join(".atlas").join("session.db");
    let conn = Connection::open(&session_db).expect("open session db");
    conn.execute(
        "UPDATE memories SET updated_at = '2026-01-01T00:00:00Z' WHERE id = ?1",
        [&memory_id(&critical)],
    )
    .expect("backdate critical");
    drop(conn);

    let all = read_json_data_output(
        "memory.list",
        run_atlas(repo.path(), &["--json", "memory", "list"]),
    );
    assert_eq!(all["count"], json!(3));
    let ids = all["memories"]
        .as_array()
        .expect("memories array")
        .iter()
        .map(|memory| memory["id"].as_str().expect("id"))
        .collect::<Vec<_>>();
    // newest first; the backdated critical memory must sort last.
    assert_eq!(ids[2], memory_id(&critical).as_str());
    assert!(ids.contains(&memory_id(&low).as_str()));
    assert!(ids.contains(&memory_id(&global).as_str()));

    let critical_only = read_json_data_output(
        "memory.list",
        run_atlas(
            repo.path(),
            &["--json", "memory", "list", "--importance", "critical"],
        ),
    );
    assert_eq!(critical_only["count"], json!(1));
    assert_eq!(
        critical_only["memories"][0]["id"],
        json!(memory_id(&critical))
    );

    let old_only = read_json_data_output(
        "memory.list",
        run_atlas(
            repo.path(),
            &["--json", "memory", "list", "--older-than", "2026-02-01"],
        ),
    );
    assert_eq!(old_only["count"], json!(1));
    assert_eq!(old_only["memories"][0]["id"], json!(memory_id(&critical)));

    let invalid_date = run_atlas_capture(
        repo.path(),
        &["--json", "memory", "list", "--older-than", "not-a-date"],
    );
    assert!(!invalid_date.status.success());
    let stderr = String::from_utf8(invalid_date.stderr).expect("stderr utf-8");
    assert!(stderr.contains("expected YYYY-MM-DD"), "got: {stderr}");
}

#[test]
fn memory_delete_requires_exact_id_and_respects_dry_run() {
    let repo = setup_fixture_repo();

    let stored = store_memory(repo.path(), &["delete me", "--topic", "hooks"]);
    let id = memory_id(&stored);

    let dry = read_json_data_output(
        "memory.delete",
        run_atlas(
            repo.path(),
            &["--json", "memory", "delete", &id, "--dry-run"],
        ),
    );
    assert_eq!(dry["memory_id"], json!(id));
    assert_eq!(dry["deleted"], json!(false));
    assert_eq!(dry["dry_run"], json!(true));

    let conn = Connection::open(repo.path().join(".atlas").join("session.db")).expect("open db");
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 1, "dry-run must not mutate storage");

    let removed = read_json_data_output(
        "memory.delete",
        run_atlas(repo.path(), &["--json", "memory", "delete", &id]),
    );
    assert_eq!(removed["deleted"], json!(true));

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
        .expect("count");
    assert_eq!(count, 0);

    // Missing ids fail with a clear error (exact id required).
    let missing = run_atlas_capture(
        repo.path(),
        &["--json", "memory", "delete", "does-not-exist"],
    );
    assert!(!missing.status.success());
    let stderr = String::from_utf8(missing.stderr).expect("stderr utf-8");
    assert!(
        stderr.contains("no memory with id does-not-exist"),
        "got: {stderr}"
    );
}
