use super::*;

#[test]
fn watch_mode_updates_graph_end_to_end_in_real_time() {
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn hello() -> &'static str { \"hello\" }\n",
    )]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let mut watch = spawn_atlas_watch(repo.path(), &["--json", "watch", "--debounce-ms", "100"]);
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
        payload["data"]["files_updated"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "watch batch should report at least one updated file: {payload:?}"
    );
    assert!(payload["data"]["observed_events"].as_u64().is_some());
    assert!(payload["data"]["coalesced_events"].as_u64().is_some());
    assert!(payload["data"]["dropped_events"].as_u64().is_some());
    assert!(payload["data"]["recovery_mode"].as_str().is_some());
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

#[test]
fn watch_file_modify_triggers_graph_update() {
    let repo = setup_repo(&[(
        "src/lib.rs",
        "pub fn hello() -> &'static str { \"hello\" }\n",
    )]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let before = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "hello"]),
    );
    let before_results = before["results"].as_array().expect("query results");
    assert!(
        !before_results.is_empty(),
        "should find 'hello' before modify"
    );

    fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn hello_world() -> &'static str { \"hi\" }\n",
    )
    .expect("write modified file");

    run_atlas(repo.path(), &["update", "--files", "src/lib.rs"]);

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

    let old_results = after_old["results"].as_array().expect("query results");
    let exact_hello: Vec<_> = old_results
        .iter()
        .filter(|result| result["node"]["name"] == json!("hello"))
        .collect();
    assert!(
        exact_hello.is_empty(),
        "old function 'hello' should be gone after update: {after_old:?}"
    );
}

#[test]
fn watch_file_delete_removes_graph_slice() {
    let repo = setup_repo(&[
        ("src/lib.rs", "pub fn keep_me() {}\n"),
        ("src/remove_me.rs", "pub fn to_be_deleted() {}\n"),
    ]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let before = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "to_be_deleted"]),
    );
    let before_results = before["results"].as_array().expect("query results");
    assert!(
        !before_results.is_empty(),
        "should find 'to_be_deleted' before delete"
    );

    fs::remove_file(repo.path().join("src/remove_me.rs")).expect("remove file");
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

#[test]
fn watch_file_rename_handled_correctly() {
    let repo = setup_repo(&[("src/original.rs", "pub fn original_fn() {}\n")]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    run_command(
        repo.path(),
        "git",
        &["mv", "src/original.rs", "src/renamed.rs"],
    );

    run_atlas(repo.path(), &["update", "--staged"]);

    let after = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "original_fn"]),
    );
    let after_results = after["results"].as_array().expect("query results");
    assert!(
        !after_results.is_empty(),
        "function should still be findable after rename: {after:?}"
    );

    let file_path = after_results[0]["node"]["file_path"].as_str().unwrap_or("");
    assert!(
        file_path.contains("renamed"),
        "node should point to renamed file path, got: {file_path}"
    );
}

#[test]
fn watch_no_duplicate_updates_idempotent() {
    let repo = setup_repo(&[("src/lib.rs", "pub fn stable() {}\n")]);

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let status_before =
        read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    let nodes_before = status_before["node_count"].as_i64().unwrap_or(0);

    run_atlas(repo.path(), &["update", "--files", "src/lib.rs"]);
    run_atlas(repo.path(), &["update", "--files", "src/lib.rs"]);

    let status_after =
        read_json_data_output("status", run_atlas(repo.path(), &["--json", "status"]));
    let nodes_after = status_after["node_count"].as_i64().unwrap_or(0);
    assert_eq!(
        nodes_before, nodes_after,
        "re-updating same file must not add duplicate nodes"
    );
}

#[test]
fn watch_command_registered_in_help() {
    let output = sanitized_command(env!("CARGO_BIN_EXE_atlas"))
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
