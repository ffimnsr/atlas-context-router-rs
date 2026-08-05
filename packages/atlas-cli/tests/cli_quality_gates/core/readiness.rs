use super::*;

// ---------------------------------------------------------------------------
// status: execution_state field
// ---------------------------------------------------------------------------

#[test]
fn status_emits_execution_state_fresh_after_build() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let status = read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    assert_eq!(
        status["execution_state"],
        json!("fresh"),
        "built graph should report execution_state=fresh: {status:?}"
    );
}

#[test]
fn status_emits_execution_state_missing_before_build() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    // No build.

    let output = run_atlas_capture(repo.path(), &["--json", "status"]);
    let status = read_json_data_output("status", output);
    assert_eq!(
        status["execution_state"],
        json!("missing"),
        "uninitialised graph should report execution_state=missing: {status:?}"
    );
}

// ---------------------------------------------------------------------------
// doctor: execution_state field
// ---------------------------------------------------------------------------

#[test]
fn doctor_emits_execution_state_fresh_after_build() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    // doctor may fail due to retrieval_index_unavailable; we still read JSON.
    let doctor = read_json_data_output("doctor", output);
    let state = doctor["execution_state"].as_str().unwrap_or("");
    assert!(
        matches!(state, "fresh" | "stale"),
        "doctor after build should report fresh or stale, got: {state}"
    );
}

#[test]
fn doctor_emits_execution_state_missing_before_build() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
        .args(["--json", "doctor"])
        .current_dir(repo.path())
        .output()
        .expect("run atlas doctor");
    let doctor = read_json_data_output("doctor", output);
    assert_eq!(
        doctor["execution_state"],
        json!("missing"),
        "doctor before build should report execution_state=missing: {doctor:?}"
    );
}

// ---------------------------------------------------------------------------
// query: blocked when graph is missing
// ---------------------------------------------------------------------------

#[test]
fn query_blocked_on_missing_graph() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    // No build — graph is missing.

    let output = run_atlas_capture(repo.path(), &["--json", "query", "greet"]);
    assert!(
        !output.status.success(),
        "query should fail when graph is missing"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Either JSON error on stdout or text error on stderr — both acceptable.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let combined = format!("{stdout}{stderr}");
    assert!(
        combined.contains("missing")
            || combined.contains("not been built")
            || combined.contains("build"),
        "error message should mention graph state: {combined}"
    );
}

#[test]
fn query_blocked_on_missing_graph_json_execution_state() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let output = run_atlas_capture(repo.path(), &["--json", "query", "greet"]);
    assert!(
        !output.status.success(),
        "query must fail with missing graph"
    );

    let value = read_json_output(output);
    let data = value["data"].clone();
    assert_eq!(data["ok"], json!(false));
    assert_eq!(data["blocked"], json!(true));
    assert_eq!(data["execution_state"], json!("missing"));
    assert_eq!(data["error_code"], json!("missing_graph_db"));
    assert_eq!(
        data["message"],
        json!("Graph database not found. Run `atlas build` to create it.")
    );
}

// ---------------------------------------------------------------------------
// query: allowed (with warning) on stale graph
// ---------------------------------------------------------------------------

#[test]
fn query_blocked_on_sqlite_corrupt_graph_includes_rebuild_metadata() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let db_path = repo.path().join(".atlas").join("worldtree.db");
    std::fs::write(&db_path, "not a sqlite database").expect("overwrite graph db");

    let output = run_atlas_capture(repo.path(), &["--json", "query", "greet"]);
    assert!(
        !output.status.success(),
        "query must fail closed on corrupt graph"
    );

    let value = read_json_output(output);
    let data = value["data"].clone();
    assert_eq!(data["ok"], json!(false));
    assert_eq!(data["blocked"], json!(true));
    assert_eq!(data["error_code"], json!("sqlite_corrupt"));
    assert_eq!(data["health_class"], json!("sqlite_corrupt"));
    assert_eq!(data["execution_state"], json!("corrupt"));
    assert_eq!(data["quarantine_path"], json!(null));
    assert_eq!(data["recommended_rebuild_command"], json!("atlas build"));
}

#[test]
fn query_allowed_on_stale_graph_with_warning() {
    let repo = setup_repo(&[
        ("src/lib.rs", "pub fn hello() {}"),
        (
            "Cargo.toml",
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    // Modify a file to make the graph stale.
    std::fs::write(
        repo.path().join("src").join("lib.rs"),
        "pub fn hello() {}\npub fn world() {}",
    )
    .expect("write new file");

    // Query should still succeed (stale is allowed by default for SymbolLookup).
    let output = run_atlas_capture(repo.path(), &["--json", "query", "hello"]);
    assert!(
        output.status.success(),
        "query on stale graph should succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

// ---------------------------------------------------------------------------
// impact/review-context/analyze/refactor: blocked when graph is missing
// ---------------------------------------------------------------------------

#[test]
fn impact_blocked_on_missing_graph() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let output = run_atlas_capture(repo.path(), &["--json", "impact", "--files", "src/lib.rs"]);
    assert!(
        !output.status.success(),
        "impact should fail when graph is missing"
    );
}

#[test]
fn review_context_blocked_on_missing_graph() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let output = run_atlas_capture(
        repo.path(),
        &["--json", "review-context", "--files", "src/lib.rs"],
    );
    assert!(
        !output.status.success(),
        "review-context should fail when graph is missing"
    );
}

#[test]
fn analyze_blocked_on_missing_graph() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let output = run_atlas_capture(repo.path(), &["--json", "analyze", "dead-code"]);
    assert!(
        !output.status.success(),
        "analyze should fail when graph is missing"
    );
}

#[test]
fn explain_change_blocked_on_missing_graph() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let output = run_atlas_capture(
        repo.path(),
        &["--json", "explain-change", "--files", "src/lib.rs"],
    );
    assert!(
        !output.status.success(),
        "explain-change should fail when graph is missing"
    );
}
