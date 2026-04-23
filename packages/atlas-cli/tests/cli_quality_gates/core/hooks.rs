use super::*;

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
