use super::*;

fn normalize_build_like_snapshot(value: &mut Value) {
    value["elapsed_ms"] = json!(0);
    value["nodes_per_sec"] = json!(0);
}

fn normalize_doctor_snapshot(value: &mut Value) {
    let checks = value["checks"].as_array_mut().expect("doctor checks array");
    for item in checks {
        if let Some(detail) = item["detail"].as_str()
            && detail.contains(" tracked files")
        {
            item["detail"] = json!("2 tracked files");
        }
    }
}

#[test]
fn build_dry_run_output_matches_golden() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);

    assert_cli_json_snapshot(
        repo.path(),
        "build",
        run_atlas(repo.path(), &["--json", "build", "--dry-run"]),
        "build_dry_run.json",
        normalize_build_like_snapshot,
    );
}

#[test]
fn update_dry_run_output_matches_golden() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    rewrite_fixture_helper(repo.path());

    assert_cli_json_snapshot(
        repo.path(),
        "update",
        run_atlas(repo.path(), &["--json", "update", "--base", "HEAD", "--dry-run"]),
        "update_dry_run.json",
        normalize_build_like_snapshot,
    );
}

#[test]
fn doctor_output_matches_golden() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    assert_cli_json_snapshot(
        repo.path(),
        "doctor",
        run_atlas_capture(repo.path(), &["--json", "doctor"]),
        "doctor.json",
        normalize_doctor_snapshot,
    );
}

#[test]
fn mcp_query_graph_output_matches_golden() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"query_graph\",\"arguments\":{\"text\":\"greet_twice\",\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(output.status.success(), "serve query_graph failed");

    assert_mcp_json_snapshot(
        repo.path(),
        &output,
        2,
        "mcp_query_graph_greet_twice.json",
        |value| {
            let results = value.as_array_mut().expect("query_graph array");
            for item in results {
                item["score"] = json!(0.0);
                item["ranking_evidence"]["raw_score"] = json!(0.0);
                item["ranking_evidence"]["final_score"] = json!(0.0);
            }
        },
    );

    cleanup_mcp_daemons(repo.path());
}

#[test]
fn mcp_get_context_output_matches_golden() {
    let repo = setup_fixture_repo();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let output = run_serve_jsonrpc_session(
        repo.path(),
        &["serve"],
        concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"initialized\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"get_context\",\"arguments\":{\"files\":[\"src/lib.rs\"],\"intent\":\"review\",\"max_nodes\":1,\"max_edges\":0,\"max_files\":1,\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(output.status.success(), "serve get_context failed");

    assert_mcp_json_snapshot(
        repo.path(),
        &output,
        2,
        "mcp_get_context_greet_twice.json",
        |_| {},
    );

    cleanup_mcp_daemons(repo.path());
}
