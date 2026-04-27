use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn sqlite_fts5_smoke_round_trip() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(status["mcp"]["worker_threads"], json!(2));
    assert_eq!(status["mcp"]["tool_timeout_ms"], json!(300000));
    assert_eq!(status["mcp"]["tool_timeout_ms_by_tool"], json!({}));
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
    assert_eq!(
        mcp_check["detail"],
        json!("workers=2 timeout_ms=300000")
    );
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
    assert_eq!(retrieval["issue_code"], json!("retrieval_index_unavailable"));
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
fn update_redacts_internal_sql_errors_from_stderr() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    rewrite_fixture_helper(repo.path());

    let db_path = repo.path().join(".atlas").join("worldtree.db");
    let conn = Connection::open(&db_path).expect("open atlas db");
    conn.execute_batch("DROP TABLE files;")
        .expect("drop files table to force schema mismatch");
    drop(conn);

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["update"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas update");
    assert!(!output.status.success(), "update should fail on broken schema");

    let stderr = String::from_utf8(output.stderr).expect("stderr utf-8");
    assert!(
        stderr.contains("Graph database schema does not match this Atlas build."),
        "stderr should contain friendly graph message\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.to_ascii_lowercase().contains("sqlite"),
        "stderr must not leak sqlite internals\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.to_ascii_lowercase().contains("sql"),
        "stderr must not leak sql internals\nstderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("no such table"),
        "stderr must not leak raw schema failure\nstderr:\n{stderr}"
    );
}

#[test]
fn init_creates_graph_content_and_session_databases() {
    let repo = setup_fixture_repo();

    let data = read_json_data_output("init", run_atlas(repo.path(), &["--json", "init"]));

    let graph_db = repo.path().join(".atlas").join("worldtree.db");
    let content_db = repo.path().join(".atlas").join("context.db");
    let session_db = repo.path().join(".atlas").join("session.db");

    assert!(graph_db.is_file(), "graph db missing: {}", graph_db.display());
    assert!(content_db.is_file(), "content db missing: {}", content_db.display());
    assert!(session_db.is_file(), "session db missing: {}", session_db.display());

    assert_eq!(json_path(&data["db_path"]), canonical_path(&graph_db));
    assert_eq!(json_path(&data["content_db_path"]), canonical_path(&content_db));
    assert_eq!(json_path(&data["session_db_path"]), canonical_path(&session_db));
}

#[test]
fn init_full_profile_writes_active_config_template() {
    let repo = setup_fixture_repo();

    let data = read_json_data_output(
        "init",
        run_atlas(repo.path(), &["--json", "init", "--profile", "full"]),
    );
    let config_path = repo.path().join(".atlas").join("config.toml");
    let config_text = fs::read_to_string(&config_path).expect("read generated config");

    assert_eq!(data["config_profile"], json!("full"));
    assert!(config_text.contains("# profile = \"full\""));
    assert!(config_text.contains("hybrid_enabled = true"));
    assert!(config_text.contains("tool_timeout_ms_by_tool = { build_or_update_graph = 900000, get_review_context = 120000 }"));
}

#[test]
fn migrate_reports_all_repo_local_databases() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let payload = read_json_data_output("migrate", run_atlas(repo.path(), &["--json", "migrate"]));
    let databases = payload["databases"]
        .as_array()
        .expect("migrate databases array");

    assert_eq!(databases.len(), 3);
    assert!(databases.iter().any(|db| db["label"] == json!("graph_db")));
    assert!(databases.iter().all(|db| db["schema_version"] == db["latest_version"]));
}

#[test]
fn debug_config_reports_file_cli_and_env_sources() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    fs::write(
        repo.path().join(".atlas").join("config.toml"),
        "[mcp]\nworker_threads = 9\n",
    )
    .expect("write config override");

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "--db", "custom.db", "debug-config"])
        .env("ATLAS_EMBED_URL", "http://embed.test")
        .current_dir(repo.path())
        .output()
        .expect("run atlas debug-config");
    assert!(output.status.success(), "debug-config failed: {output:?}");

    let payload = read_json_data_output("debug_config", output);
    assert_eq!(
        payload["resolved"]["mcp.worker_threads"]["source"],
        json!("file")
    );
    assert_eq!(
        payload["resolved"]["runtime.db_path"]["source"],
        json!("cli")
    );
    assert_eq!(
        payload["resolved"]["env.ATLAS_EMBED_URL"]["source"],
        json!("env")
    );
    assert_eq!(
        payload["resolved"]["env.ATLAS_EMBED_URL"]["value"],
        json!("http://embed.test")
    );
}

#[test]
fn config_show_alias_matches_debug_config_json_shape() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let payload = read_json_data_output(
        "debug_config",
        run_atlas(repo.path(), &["--json", "config", "show"]),
    );

    assert_eq!(payload["config_exists"], json!(true));
    assert!(payload["resolved"].get("runtime.repo_root").is_some());
}

#[test]
fn selfupdate_returns_explicit_refusal_with_next_steps() {
    let repo = setup_fixture_repo();

    let payload = read_json_data_output(
        "selfupdate",
        run_atlas(repo.path(), &["--json", "selfupdate"]),
    );

    assert_eq!(payload["ok"], json!(false));
    assert_eq!(payload["error_code"], json!("selfupdate_not_supported"));
    assert_eq!(payload["next_steps"][0], json!("./install.sh"));
    assert_eq!(
        payload["next_steps"][1],
        json!("cargo install --path packages/atlas-cli --force")
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
fn build_budget_degraded_case_surfaces_in_cli_json_and_status() {
    let repo = setup_repo(&[("a.rs", "pub fn a() {}\n"), ("b.rs", "pub fn b() {}\n")]);

    run_atlas(repo.path(), &["init"]);
    fs::write(
        repo.path().join(".atlas").join("config.toml"),
        r#"[build]
parse_batch_size = 16
max_files_per_run = 1
max_total_bytes_per_run = 1048576
max_file_bytes = 1048576
max_parse_failures = 10
max_parse_failure_ratio = 1.0
max_wall_time_ms = 30000
"#,
    )
    .expect("write config");

    let build = read_json_data_output("build", run_atlas(repo.path(), &["--json", "build"]));
    assert_eq!(build["budget"]["budget_status"], json!("partial_result"));
    assert_eq!(
        build["budget_counters"]["budget_stop_reason"],
        json!("max_files_per_run")
    );
    assert_eq!(build["budget_counters"]["files_accepted"], json!(1));
    assert_eq!(build["parsed"], json!(1));

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(status["error_code"], json!("degraded_build"));
    assert_eq!(status["build_state"], json!("degraded"));
    assert_eq!(status["build_status"]["budget_stop_reason"], json!("max_files_per_run"));
    assert_eq!(status["build_status"]["files_accepted"], json!(1));
}

#[test]
fn build_budget_fail_closed_case_surfaces_build_failed_state() {
    let repo = setup_repo(&[("lib.rs", "pub fn ok() {}\n")]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    fs::write(
        repo.path().join(".atlas").join("config.toml"),
        r#"[build]
parse_batch_size = 16
max_files_per_run = 10
max_total_bytes_per_run = 1048576
max_file_bytes = 1048576
max_parse_failures = 0
max_parse_failure_ratio = 1.0
max_wall_time_ms = 30000
"#,
    )
    .expect("write config");

    let lib_path = repo.path().join("lib.rs");
    let original_mode = fs::metadata(&lib_path)
        .expect("metadata")
        .permissions()
        .mode();

    let mut perms = fs::metadata(&lib_path).expect("metadata").permissions();
    perms.set_mode(0o000);
    fs::set_permissions(&lib_path, perms).expect("set unreadable permissions");

    let update = read_json_data_output("update", run_atlas(repo.path(), &["--json", "update"]));

    let mut restore = fs::metadata(&lib_path).expect("metadata").permissions();
    restore.set_mode(original_mode);
    fs::set_permissions(&lib_path, restore).expect("restore file permissions");

    assert_eq!(update["budget"]["budget_status"], json!("blocked"));
    assert_eq!(
        update["budget_counters"]["budget_stop_reason"],
        json!("max_parse_failures")
    );
    assert_eq!(update["parse_errors"], json!(1));

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(status["error_code"], json!("failed_build"));
    assert_eq!(status["build_state"], json!("build_failed"));
    assert_eq!(status["build_status"]["budget_stop_reason"], json!("max_parse_failures"));
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
            "Nodes inserted      : 7",
            "Edges inserted      : 8",
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
