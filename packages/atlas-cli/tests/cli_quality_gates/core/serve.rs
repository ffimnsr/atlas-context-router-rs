use super::*;

#[test]
fn serve_command_handles_stdio_jsonrpc_flow_end_to_end() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let output = run_serve_jsonrpc_session(repo.path(), &["serve"], serve_requests());

    assert!(
        output.status.success(),
        "atlas serve failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let responses = parse_jsonrpc_lines(&output.stdout);
    assert_eq!(responses.len(), 4, "initialized notification must not emit response");

    let by_id: std::collections::HashMap<_, _> = responses
        .into_iter()
        .map(|response| (response["id"].clone(), response))
        .collect();

    assert_eq!(by_id[&json!(1)]["result"]["protocolVersion"], json!("2024-11-05"));

    let tools = by_id[&json!(2)]["result"]["tools"]
        .as_array()
        .expect("tools/list result tools array");
    assert!(
        tools.iter().any(|tool| tool["name"] == json!("get_context")),
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
        let query_value: Value = serde_json::from_str(query_text).expect("query_graph payload json");
        assert_eq!(query_value[0]["qn"], json!("src/lib.rs::method::Greeter::greet_twice"));
    } else {
        assert!(query_text.contains("src/lib.rs::method::Greeter::greet_twice"));
    }

    assert_eq!(by_id[&json!(4)]["result"]["atlas_output_format"], json!("toon"));
    let context_text = by_id[&json!(4)]["result"]["content"][0]["text"]
        .as_str()
        .expect("get_context text content");
    assert!(context_text.contains("intent: symbol"));
    assert!(context_text.contains("src/lib.rs::method::Greeter::greet_twice"));

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn serve_broker_preserves_saved_context_and_session_tools() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let output = run_serve_jsonrpc_session(repo.path(), &["serve"], serve_requests_with_session_tools());
    assert!(
        output.status.success(),
        "atlas serve failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    let responses = parse_jsonrpc_lines(&output.stdout);
    assert_eq!(responses.len(), 4);
    let by_id: std::collections::HashMap<_, _> = responses
        .into_iter()
        .map(|response| (response["id"].clone(), response))
        .collect();

    let artifact_text = by_id[&json!(3)]["result"]["content"][0]["text"]
        .as_str()
        .expect("save_context_artifact text");
    let artifact_value: Value = serde_json::from_str(artifact_text).expect("artifact payload json");
    assert!(
        matches!(artifact_value["routing"].as_str(), Some("preview") | Some("pointer")),
        "artifact should route to preview or pointer: {artifact_value:?}"
    );
    assert!(artifact_value["source_id"].as_str().is_some());

    let session_text = by_id[&json!(4)]["result"]["content"][0]["text"]
        .as_str()
        .expect("get_session_status text");
    let session_value: Value = serde_json::from_str(session_text).expect("session status json");
    assert_eq!(session_value["status"], json!("active"));
    assert_eq!(session_value["frontend"], json!("mcp"));
    assert!(session_value["event_count"].as_u64().unwrap_or_default() >= 1);

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn concurrent_brokers_for_same_repo_and_db_share_one_daemon() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let repo_a = repo.path().to_path_buf();
    let repo_b = repo.path().to_path_buf();
    let first = std::thread::spawn(move || run_serve_jsonrpc_session(&repo_a, &["serve"], serve_requests()));
    let second = std::thread::spawn(move || run_serve_jsonrpc_session(&repo_b, &["serve"], serve_requests()));

    let first_output = first.join().expect("first broker join");
    let second_output = second.join().expect("second broker join");
    assert!(first_output.status.success(), "first broker failed: {}", String::from_utf8_lossy(&first_output.stderr));
    assert!(second_output.status.success(), "second broker failed: {}", String::from_utf8_lossy(&second_output.stderr));
    let first_stderr = String::from_utf8_lossy(&first_output.stderr);
    let second_stderr = String::from_utf8_lossy(&second_output.stderr);
    assert!(
        first_stderr.contains("atlas-mcp: broker spawn") || second_stderr.contains("atlas-mcp: broker spawn"),
        "one broker should spawn daemon\nfirst:\n{first_stderr}\nsecond:\n{second_stderr}"
    );
    assert!(
        first_stderr.contains("atlas-mcp: broker attach") || second_stderr.contains("atlas-mcp: broker attach"),
        "one broker should attach to existing daemon\nfirst:\n{first_stderr}\nsecond:\n{second_stderr}"
    );

    let instances = list_mcp_instance_metadata(repo.path());
    assert_eq!(instances.len(), 1, "same repo+db must create one daemon instance");
    let pid = instances[0]["pid"].as_u64().expect("daemon pid") as u32;
    assert!(pid_exists(pid), "shared daemon pid must be alive");

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn brokers_for_same_repo_and_different_dbs_start_separate_daemons() {
    let repo = setup_fixture_repo();
    let db_one = repo.path().join(".atlas").join("alternate-one.db");
    let db_two = repo.path().join(".atlas").join("alternate-two.db");
    let db_one_str = db_one.to_string_lossy().into_owned();
    let db_two_str = db_two.to_string_lossy().into_owned();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    run_atlas(repo.path(), &["--db", &db_one_str, "init"]);
    run_atlas(repo.path(), &["--db", &db_one_str, "build"]);
    run_atlas(repo.path(), &["--db", &db_two_str, "init"]);
    run_atlas(repo.path(), &["--db", &db_two_str, "build"]);

    let output_one = run_serve_jsonrpc_session(repo.path(), &["--db", &db_one_str, "serve"], serve_requests());
    let output_two = run_serve_jsonrpc_session(repo.path(), &["--db", &db_two_str, "serve"], serve_requests());
    assert!(output_one.status.success(), "first db broker failed: {}", String::from_utf8_lossy(&output_one.stderr));
    assert!(output_two.status.success(), "second db broker failed: {}", String::from_utf8_lossy(&output_two.stderr));

    let instances = list_mcp_instance_metadata(repo.path());
    assert_eq!(instances.len(), 2, "different db paths must create separate daemon instances");
    let db_paths = instances
        .iter()
        .map(|metadata| metadata["db_path"].as_str().expect("db path").to_owned())
        .collect::<Vec<_>>();
    assert!(db_paths.contains(&canonical_path(&db_one).to_string_lossy().into_owned()));
    assert!(db_paths.contains(&canonical_path(&db_two).to_string_lossy().into_owned()));

    let pids = instances
        .iter()
        .map(|metadata| metadata["pid"].as_u64().expect("pid") as u32)
        .collect::<Vec<_>>();
    assert_ne!(pids[0], pids[1], "separate dbs must run separate daemon pids");

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn broker_recovers_after_dead_daemon_pid() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let first_output = run_serve_jsonrpc_session(repo.path(), &["serve"], serve_requests());
    assert!(first_output.status.success(), "initial broker failed: {}", String::from_utf8_lossy(&first_output.stderr));

    let initial_instances = list_mcp_instance_metadata(repo.path());
    assert_eq!(initial_instances.len(), 1);
    let old_pid = initial_instances[0]["pid"].as_u64().expect("old pid") as u32;
    let socket_path = PathBuf::from(initial_instances[0]["socket_path"].as_str().expect("socket path"));
    assert!(pid_exists(old_pid));

    kill_pid(old_pid);
    wait_until(Duration::from_secs(2), || !pid_exists(old_pid));
    assert!(socket_path.exists(), "killed daemon should leave stale socket path for recovery test");

    let second_output = run_serve_jsonrpc_session(repo.path(), &["serve"], serve_requests());
    assert!(second_output.status.success(), "recovery broker failed: {}", String::from_utf8_lossy(&second_output.stderr));
    let recovery_stderr = String::from_utf8_lossy(&second_output.stderr);
    assert!(recovery_stderr.contains("atlas-mcp: broker cleanup") || recovery_stderr.contains("cleaning stale daemon state"));
    assert!(recovery_stderr.contains("atlas-mcp: broker spawn"));

    let recovered_instances = list_mcp_instance_metadata(repo.path());
    assert_eq!(recovered_instances.len(), 1);
    let new_pid = recovered_instances[0]["pid"].as_u64().expect("new pid") as u32;
    assert_ne!(old_pid, new_pid, "dead daemon must be replaced");
    assert!(pid_exists(new_pid), "replacement daemon pid must be alive");

    cleanup_mcp_daemons(repo.path());
}
