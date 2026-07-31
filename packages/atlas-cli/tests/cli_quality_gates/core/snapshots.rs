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
        run_atlas(
            repo.path(),
            &["--json", "update", "--base", "HEAD", "--dry-run"],
        ),
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
        format!(
            "{}{}",
            initialized_session_prelude(1),
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
            if let Some(matches_value) = value.get_mut("matches") {
                let results = matches_value
                    .as_array_mut()
                    .expect("query_graph matches array");
                for item in results.iter_mut() {
                    item["score"] = json!(0.0);
                    item["ranking_evidence"]["raw_score"] = json!(0.0);
                    item["ranking_evidence"]["final_score"] = json!(0.0);
                    if let Some(object) = item.as_object_mut() {
                        object.remove("repo");
                    }
                }
                let simplified = Value::Array(results.clone());
                *value = simplified;
                return;
            }

            let results = value.as_array_mut().expect("query_graph array");
            for item in results {
                item["score"] = json!(0.0);
                item["ranking_evidence"]["raw_score"] = json!(0.0);
                item["ranking_evidence"]["final_score"] = json!(0.0);
                if let Some(object) = item.as_object_mut() {
                    object.remove("repo");
                }
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
        format!(
            "{}{}",
            initialized_session_prelude(1),
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"get_context\",\"arguments\":{\"target\":{\"kind\":\"files\",\"files\":[\"src/lib.rs\"]},\"max_nodes\":1,\"max_edges\":0,\"max_files\":1,\"output_format\":\"json\"}}}\n"
        ),
    );
    assert!(output.status.success(), "serve get_context failed");

    assert_mcp_json_snapshot(
        repo.path(),
        &output,
        2,
        "mcp_get_context_greet_twice.json",
        |value| {
            if let Some(ambiguity) = value["ambiguity"].as_object_mut() {
                ambiguity.remove("candidates_detailed");
            }
            if let Some(object) = value.as_object_mut() {
                object.remove("cross_repo_context_hops");
                object.remove("target");
            }
            if let Some(detail_controls) = value["detail_controls"].as_object_mut() {
                detail_controls.remove("allow_cross_repo_edges");
            }
        },
    );

    cleanup_mcp_daemons(repo.path());
}
