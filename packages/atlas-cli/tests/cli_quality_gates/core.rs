use super::*;
use atlas_adapters::normalize_event;
use atlas_session::{SessionEventType, SessionId, SessionStore};
use rusqlite::Connection;
use std::ffi::OsString;
use std::io::Write;
use std::process::Stdio;

fn run_installed_hook(repo_root: &Path, frontend: &str, event: &str, payload: &str) {
    let runner = repo_root.join(".atlas").join("hooks").join("atlas-hook");
    let atlas_bin = Path::new(env!("CARGO_BIN_EXE_atlas"));
    let mut path_value = OsString::from(atlas_bin.parent().expect("atlas binary dir"));
    if let Some(existing_path) = std::env::var_os("PATH") {
        path_value.push(":");
        path_value.push(existing_path);
    }

    let mut child = sanitized_command(runner.to_str().expect("runner path"))
        .args([frontend, event])
        .current_dir(repo_root)
        .env("PATH", path_value)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn installed hook runner");

    child
        .stdin
        .as_mut()
        .expect("runner stdin")
        .write_all(payload.as_bytes())
        .expect("write hook payload");

    let output = child.wait_with_output().expect("wait for installed hook runner");
    assert!(
        output.status.success(),
        "installed hook runner failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

#[test]
fn sqlite_fts5_smoke_round_trip() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(status["mcp"]["worker_threads"], json!(2));
    assert_eq!(status["mcp"]["tool_timeout_ms"], json!(300000));
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
fn doctor_reports_mcp_serve_config() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    let doctor = read_json_data_output("doctor", output);
    let checks = doctor["checks"].as_array().expect("doctor checks array");
    let mcp_check = checks
        .iter()
        .find(|item| item["check"] == json!("mcp_serve_config"))
        .expect("mcp serve config check present");

    assert_eq!(mcp_check["ok"], json!(true));
    assert_eq!(mcp_check["detail"], json!("workers=2 timeout_ms=300000"));
}

#[test]
fn doctor_reports_retrieval_index_unavailable_issue_code() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let content_db = repo.path().join(".atlas").join("context.db");
    fs::remove_file(&content_db).expect("remove content db");

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    assert!(
        !output.status.success(),
        "doctor should fail when retrieval index is unavailable"
    );

    let doctor = read_json_data_output("doctor", output);
    let retrieval = doctor["checks"]
        .as_array()
        .expect("doctor checks array")
        .iter()
        .find(|item| item["check"] == json!("retrieval_index"))
        .expect("retrieval index check present");

    assert_eq!(doctor["error_code"], json!("checks_failed"));
    assert_eq!(retrieval["ok"], json!(false));
    assert_eq!(
        retrieval["issue_code"],
        json!("retrieval_index_unavailable")
    );
}

#[test]
fn doctor_reports_noncanonical_content_path_identity() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let context_db = repo.path().join(".atlas").join("context.db");
    let conn = Connection::open(&context_db).expect("open context db");
    conn.execute(
        "INSERT INTO sources (
             id, session_id, source_type, label, repo_root, identity_kind, identity_value, created_at
         ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            "bad-source",
            "sess1",
            "review_context",
            "bad source",
            repo.path().to_string_lossy().to_string(),
            "repo_path",
            "./src/lib.rs",
            "2025-01-01T00:00:00Z"
        ],
    )
    .expect("seed noncanonical source row");

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    assert!(
        !output.status.success(),
        "doctor should fail when content path identity is noncanonical"
    );

    let doctor = read_json_data_output("doctor", output);
    let content_check = doctor["checks"]
        .as_array()
        .expect("doctor checks array")
        .iter()
        .find(|item| item["check"] == json!("content_path_identity"))
        .expect("content path identity check present");

    assert_eq!(content_check["ok"], json!(false));
    assert_eq!(content_check["issue_code"], json!("noncanonical_path_rows"));
    assert!(content_check["detail"]
        .as_str()
        .is_some_and(|text| text.contains("canonical=src/lib.rs")));
}

#[test]
fn purge_noncanonical_removes_context_and_session_but_keeps_graph_db() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let atlas_dir = repo.path().join(".atlas");
    let graph_db = atlas_dir.join("worldtree.db");
    let context_db = atlas_dir.join("context.db");
    let session_db = atlas_dir.join("session.db");

    assert!(graph_db.exists(), "graph db should exist after init");
    assert!(context_db.exists(), "context db should exist after init");
    assert!(session_db.exists(), "session db should exist after init");

    let purge = read_json_data_output(
        "purge_noncanonical",
        run_atlas(repo.path(), &["--json", "purge-noncanonical"]),
    );

    assert_eq!(purge["context_db"]["removed"], json!(true));
    assert_eq!(purge["session_db"]["removed"], json!(true));
    assert_eq!(purge["graph_db"]["preserved"], json!(true));
    assert_eq!(purge["next_steps"][0], json!("atlas build"));
    assert_eq!(purge["next_steps"][1], json!("atlas session start"));

    assert!(graph_db.exists(), "graph db should be preserved");
    assert!(!context_db.exists(), "context db should be removed");
    assert!(!session_db.exists(), "session db should be removed");
}

#[test]
fn installed_hook_runner_executes_lifecycle_restore_and_handoff_end_to_end() {
    let repo = setup_repo(&[
        (
            "Cargo.toml",
            "[package]\nname = \"runner-lifecycle\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
        ("src/lib.rs", "pub fn alpha() {}\n"),
    ]);

    run_atlas(repo.path(), &["install", "--platform", "codex"]);
    assert!(repo.path().join(".atlas").join("hooks").join("lib").is_dir());

    let repo_root = canonical_path(repo.path());
    let repo_root_str = repo_root.to_string_lossy().into_owned();
    let session_id = SessionId::derive(&repo_root_str, "", "codex");

    let mut session_store = SessionStore::open_in_repo(repo.path()).expect("open session store");
    session_store
        .upsert_session_meta(session_id.clone(), &repo_root_str, "codex", None)
        .expect("upsert session meta");
    session_store
        .append_event(
            normalize_event(
                SessionEventType::UserIntent,
                3,
                json!({ "prompt": "review src/lib.rs" }),
            )
            .bind(session_id.clone()),
        )
        .expect("append prompt event");
    session_store
        .build_resume(&session_id)
        .expect("build resume snapshot");

    run_installed_hook(repo.path(), "codex", "session-start", "{}");

    let session_store = SessionStore::open_in_repo(repo.path()).expect("reopen session store");
    let snapshot = session_store
        .get_resume_snapshot(&session_id)
        .expect("load resume snapshot")
        .expect("resume snapshot row");
    assert!(snapshot.consumed, "installed runner should consume pending snapshot");

    run_installed_hook(repo.path(), "codex", "stop", "{}");

    let context_db = repo.path().join(".atlas").join("context.db");
    let conn = Connection::open(&context_db).expect("open context db");
    let handoff_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sources WHERE source_type = 'hook_handoff' AND session_id = ?1",
            [session_id.as_str()],
            |row| row.get(0),
        )
        .expect("query hook handoff sources");
    assert!(
        handoff_count >= 1,
        "installed runner stop hook should persist handoff artifact"
    );
}

#[test]
fn installed_hook_runner_refreshes_graph_after_file_edit_end_to_end() {
    let repo = setup_repo(&[
        (
            "Cargo.toml",
            "[package]\nname = \"runner-refresh\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
        ("src/lib.rs", "pub fn alpha() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    run_atlas(repo.path(), &["install", "--platform", "codex"]);

    write_repo_file(repo.path(), "src/lib.rs", "pub fn alpha() {}\npub fn beta() {}\n");

    run_installed_hook(
        repo.path(),
        "codex",
        "post-tool-use",
        r#"{"tool_name":"Write","changed_files":["src/lib.rs"]}"#,
    );

    let nodes = open_store(repo.path())
        .nodes_by_file("src/lib.rs")
        .expect("nodes by file after installed hook refresh");
    assert!(
        nodes
            .iter()
            .any(|node| node.qualified_name.ends_with("::fn::beta")),
        "installed runner should refresh graph after file edit"
    );
}

#[test]
fn installed_hook_runner_clears_stale_status_after_file_edit_end_to_end() {
    let repo = setup_repo(&[
        (
            "Cargo.toml",
            "[package]\nname = \"runner-refresh-status\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
        ("src/lib.rs", "pub fn alpha() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    run_atlas(repo.path(), &["install", "--platform", "codex"]);

    write_repo_file(repo.path(), "src/lib.rs", "pub fn alpha() {}\npub fn beta() {}\n");

    let stale_before = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(stale_before["error_code"], json!("stale_index"), "status={stale_before:?}");
    assert_eq!(stale_before["stale_index"], json!(true), "status={stale_before:?}");

    run_installed_hook(
        repo.path(),
        "codex",
        "post-tool-use",
        r#"{"tool_name":"Write","changed_files":["src/lib.rs"]}"#,
    );

    let nodes = open_store(repo.path())
        .nodes_by_file("src/lib.rs")
        .expect("nodes by file after installed hook refresh");
    assert!(
        nodes
            .iter()
            .any(|node| node.qualified_name.ends_with("::fn::beta")),
        "installed runner should refresh graph after file edit"
    );

    let stale_after = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_ne!(stale_after["error_code"], json!("stale_index"), "status={stale_after:?}");
    assert_eq!(stale_after["stale_index"], json!(false), "status={stale_after:?}");
    assert_eq!(stale_after["pending_graph_change_count"], json!(0), "status={stale_after:?}");
}

#[test]
fn status_marks_stale_index_when_graph_relevant_file_changes() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    rewrite_fixture_helper(repo.path());

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(status["ok"], json!(false));
    assert_eq!(status["error_code"], json!("stale_index"));
    assert_eq!(status["stale_index"], json!(true));
    assert!(
        status["pending_graph_changes"]
            .as_array()
            .is_some_and(|files| files.iter().any(|path| path == "src/lib.rs")),
        "pending graph changes should mention modified source file: {status:?}"
    );
}

#[test]
fn status_clears_stale_index_after_update_indexes_dirty_worktree() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    rewrite_fixture_helper(repo.path());
    run_atlas(repo.path(), &["update"]);

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_ne!(status["error_code"], json!("stale_index"), "status={status:?}");
    assert_eq!(status["stale_index"], json!(false), "status={status:?}");
    assert_eq!(status["pending_graph_change_count"], json!(0), "status={status:?}");
}

#[test]
fn status_reports_schema_mismatch_for_malformed_build_state_table() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let db_path = repo.path().join(".atlas").join("worldtree.db");
    let conn = Connection::open(&db_path).expect("open atlas db");
    conn.execute_batch(
        "
        DROP TABLE graph_build_state;
        CREATE TABLE graph_build_state (
            repo_root TEXT PRIMARY KEY,
            state TEXT NOT NULL
        );
        ",
    )
    .expect("replace build state table with malformed schema");

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(status["ok"], json!(false));
    assert_eq!(status["error_code"], json!("schema_mismatch"));
    assert!(status["graph_query_error"]
        .as_str()
        .is_some_and(|text| text.contains("graph_build_state")));
}

#[test]
fn init_creates_graph_content_and_session_databases() {
    let repo = setup_fixture_repo();

    let data = read_json_data_output("init", run_atlas(repo.path(), &["--json", "init"]));

    let graph_db = repo.path().join(".atlas").join("worldtree.db");
    let content_db = repo.path().join(".atlas").join("context.db");
    let session_db = repo.path().join(".atlas").join("session.db");

    assert!(
        graph_db.is_file(),
        "graph db missing: {}",
        graph_db.display()
    );
    assert!(
        content_db.is_file(),
        "content db missing: {}",
        content_db.display()
    );
    assert!(
        session_db.is_file(),
        "session db missing: {}",
        session_db.display()
    );

    assert_eq!(json_path(&data["db_path"]), canonical_path(&graph_db));
    assert_eq!(
        json_path(&data["content_db_path"]),
        canonical_path(&content_db)
    );
    assert_eq!(
        json_path(&data["session_db_path"]),
        canonical_path(&session_db)
    );
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
        "fuzzy typo query should recover close match: {fuzzy:?}"
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
fn query_fuzzy_typo_prefers_code_symbol_over_markdown_noise() {
    let repo = setup_repo(&[
        ("go.mod", "module example.com/atlasfixture\n\ngo 1.22\n"),
        (
            "internal/requestctx/context.go",
            "package requestctx\n\nfunc LoadIdentityMessages() {}\n",
        ),
        (
            "docs/load_identity_messages.md",
            "# Load Identity Messages\n\nContext guide for identity message loading.\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let fuzzy = read_json_data_output(
        "query",
        run_atlas(
            repo.path(),
            &["--json", "query", "LoadIdentityMesages", "--fuzzy"],
        ),
    );
    let results = fuzzy["results"].as_array().expect("query results array");
    assert!(
        !results.is_empty(),
        "fuzzy typo query should return code-symbol result: {fuzzy:?}"
    );
    assert_eq!(results[0]["node"]["name"], json!("LoadIdentityMessages"));
    assert_eq!(results[0]["node"]["kind"], json!("function"));

    let with_files = read_json_data_output(
        "query",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "query",
                "LoadIdentityMesages",
                "--fuzzy",
                "--include-files",
            ],
        ),
    );
    let with_files_results = with_files["results"]
        .as_array()
        .expect("query results array with files");
    assert!(
        !with_files_results.is_empty(),
        "include-files fuzzy query should still return code-symbol result: {with_files:?}"
    );
    assert_eq!(
        with_files_results[0]["node"]["name"],
        json!("LoadIdentityMessages")
    );
    assert_eq!(with_files["query"]["include_files"], json!(true));
    assert!(
        with_files_results.iter().any(|result| {
            result["node"]["file_path"] == json!("docs/load_identity_messages.md")
                || result["node"]["language"] == json!("markdown")
        }),
        "include-files query should keep markdown/file noise visible for ranking comparison: {with_files:?}"
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
        "graph expansion must surface direct neighbor helper: {atlas:?}"
    );
    assert!(
        atlas_qns.iter().any(|qn| qn == "src/main.rs::fn::main"),
        "graph expansion must surface transitive caller main: {atlas:?}"
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
        atlas_top1 == cases.len() && atlas_top3 == cases.len(),
        "atlas query must rank expected definitions in top-1 and top-3: atlas top1/top3 = {atlas_top1}/{atlas_top3}"
    );
    assert!(
        atlas_top1 >= grep_top1 && atlas_top3 >= grep_top3,
        "atlas query must not underperform plain grep: atlas top1/top3 = {atlas_top1}/{atlas_top3}, grep = {grep_top1}/{grep_top3}"
    );
}

#[test]
fn build_and_update_skip_unsupported_files_without_count_drift() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    write_repo_file(repo.path(), "notes.unsupported", "atlas notes\n");

    let build = read_json_data_output("build", run_atlas(repo.path(), &["--json", "build"]));
    assert!(build["skipped_unsupported"].as_u64().unwrap_or_default() >= 1);
    assert!(build["parsed"].as_u64().unwrap_or_default() >= 2);

    let update = read_json_data_output(
        "update",
        run_atlas(
            repo.path(),
            &["--json", "update", "--files", "notes.unsupported"],
        ),
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
        "workflow summary must explain why cross-package change matters: {review_ctx:?}"
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

    assert_eq!(
        result["plan"]["operation"]["kind"],
        json!("rename_symbol")
    );
    assert_eq!(
        result["plan"]["operation"]["old_qname"],
        json!("src/lib.rs::fn::helper")
    );
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
    assert_eq!(
        result["plan"]["operation"]["kind"],
        json!("remove_dead_code")
    );
    assert_eq!(
        result["plan"]["operation"]["target_qname"],
        json!("src/lib.rs::fn::unused_helper")
    );
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
fn serve_command_handles_stdio_jsonrpc_flow_end_to_end() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let mut child = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
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
        "initialized notification must not emit response"
    );

    let by_id: std::collections::HashMap<_, _> = responses
        .into_iter()
        .map(|response| (response["id"].clone(), response))
        .collect();

    assert_eq!(
        by_id[&json!(1)]["result"]["protocolVersion"],
        json!("2024-11-05")
    );

    let tools = by_id[&json!(2)]["result"]["tools"]
        .as_array()
        .expect("tools/list result tools array");
    assert!(
        tools
            .iter()
            .any(|tool| tool["name"] == json!("get_context")),
        "tools/list must expose get_context"
    );

    let query_format = by_id[&json!(3)]["result"]["atlas_output_format"]
        .as_str()
        .expect("query_graph output format");
    assert!(query_format == "toon" || query_format == "json");
    let query_text = by_id[&json!(3)]["result"]["content"][0]["text"]
        .as_str()
        .expect("query_graph text content");
    if query_format == "json" {
        let query_value: Value =
            serde_json::from_str(query_text).expect("query_graph payload json");
        assert_eq!(
            query_value[0]["qn"],
            json!("src/lib.rs::method::Greeter::greet_twice")
        );
    } else {
        assert!(query_text.contains("src/lib.rs::method::Greeter::greet_twice"));
    }

    assert_eq!(
        by_id[&json!(4)]["result"]["atlas_output_format"],
        json!("toon")
    );
    let context_text = by_id[&json!(4)]["result"]["content"][0]["text"]
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
            "Unsupported skipped : 0",
            "Parsed              : 2",
            "Nodes inserted      : 6",
            "Edges inserted      : 7",
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
            "Parsed   :",
            "Nodes    :",
            "Edges    :",
        ],
    );
}

#[test]
fn staged_submodule_edit_flows_through_detect_changes_and_update() {
    let repo = setup_repo_with_submodule(
        &[("README.md", "# atlas fixture\n")],
        "docs/wiki",
        &[(
            "src/lib.rs",
            "pub fn nested() -> &'static str {\n    \"v1\"\n}\n",
        )],
    );

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    write_repo_file(
        &repo.path().join("docs/wiki"),
        "src/lib.rs",
        "pub fn nested_v2() -> &'static str {\n    \"v2\"\n}\n",
    );
    run_command(repo.path().join("docs/wiki").as_path(), "git", &["add", "src/lib.rs"]);

    let changes = read_json_data_output(
        "detect_changes",
        run_atlas(repo.path(), &["--json", "detect-changes", "--staged"]),
    );
    let changes = changes["changes"]
        .as_array()
        .expect("detect-changes changes array");
    assert!(
        changes.iter().any(|change| {
            change["path"] == json!("docs/wiki/src/lib.rs")
                && change["change_type"] == json!("modified")
        }),
        "staged submodule edit should surface child path; got {changes:?}"
    );

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--staged"]),
    );
    assert!(update["parsed"].as_u64().unwrap_or_default() >= 1);
    assert!(update["nodes_updated"].as_u64().unwrap_or_default() >= 1);

    let new_query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "nested_v2"]),
    );
    assert!(
        new_query["results"]
            .as_array()
            .expect("query results array")
            .iter()
            .any(|result| result["node"]["qualified_name"] == json!("docs/wiki/src/lib.rs::fn::nested_v2")),
        "updated query should include renamed submodule function: {new_query:?}"
    );

    let old_query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "nested"]),
    );
    assert!(
        old_query["results"]
            .as_array()
            .expect("query results array")
            .iter()
            .all(|result| result["node"]["qualified_name"] != json!("docs/wiki/src/lib.rs::fn::nested")),
        "old submodule symbol should be removed after update: {old_query:?}"
    );
}

#[test]
fn base_ref_dirty_submodule_edit_flows_through_detect_changes_and_update() {
    let repo = setup_repo_with_submodule(
        &[("README.md", "# atlas fixture\n")],
        "docs/wiki",
        &[(
            "src/lib.rs",
            "pub fn nested() -> &'static str {\n    \"v1\"\n}\n",
        )],
    );

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    write_repo_file(
        &repo.path().join("docs/wiki"),
        "src/lib.rs",
        "pub fn nested_v2() -> &'static str {\n    \"v2\"\n}\n",
    );

    let changes = read_json_data_output(
        "detect_changes",
        run_atlas(repo.path(), &["--json", "detect-changes", "--base", "HEAD"]),
    );
    let changes = changes["changes"]
        .as_array()
        .expect("detect-changes changes array");
    assert!(
        changes.iter().any(|change| {
            change["path"] == json!("docs/wiki/src/lib.rs")
                && change["change_type"] == json!("modified")
        }),
        "base-ref dirty submodule edit should surface child path; got {changes:?}"
    );

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--base", "HEAD"]),
    );
    assert!(update["parsed"].as_u64().unwrap_or_default() >= 1);
    assert!(update["nodes_updated"].as_u64().unwrap_or_default() >= 1);

    let new_query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "nested_v2"]),
    );
    assert!(
        new_query["results"]
            .as_array()
            .expect("query results array")
            .iter()
            .any(|result| result["node"]["qualified_name"] == json!("docs/wiki/src/lib.rs::fn::nested_v2")),
        "updated query should include dirty submodule function: {new_query:?}"
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

    let review = stdout_text(&run_atlas(
        repo.path(),
        &["review-context", "--base", "HEAD"],
    ));
    assert_contains_all(
        &review,
        &[
            "Changed files (1):",
            "  src/lib.rs",
            "Changed symbols:",
            "function src/lib.rs::fn::helper",
            "Risk summary:",
            "  Public API changes :",
            "  Uncovered changes  :",
            "  Cross-package impact: false",
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
