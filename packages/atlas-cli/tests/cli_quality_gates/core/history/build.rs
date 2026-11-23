use super::*;

#[test]
fn history_update_json_reports_incremental_summary() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", first_sha.as_str()],
    );

    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history update fixture");

    let summary = read_json_data_output(
        "history_update",
        run_atlas(repo.path(), &["--json", "history", "update"]),
    );

    assert_eq!(summary["branch"], json!("HEAD"));
    assert_eq!(summary["head_sha"], json!(second_sha));
    assert_eq!(summary["indexed_base_sha"], json!(first_sha));
    assert_eq!(summary["latest_indexed_sha"], json!(first_sha));
    assert_eq!(summary["commits_processed"], json!(1));
    assert_eq!(summary["divergence_detected"], json!(false));
    assert_eq!(summary["repair_mode"], json!(false));
    assert_eq!(summary["lifecycle"]["snapshot_count"], json!(2));
    assert!(
        summary["lifecycle"]["node_history_rows"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "expected node history rows in history update output: {summary:?}"
    );
    assert!(
        summary["elapsed_secs"].as_f64().unwrap_or_default() >= 0.0,
        "expected elapsed_secs in history update output: {summary:?}"
    );
}

#[test]
fn history_build_json_reports_shallow_clone_warning() {
    let source = setup_fixture_repo();
    rewrite_fixture_helper(source.path());
    commit_repo_change(source.path(), "history shallow source fixture");

    let clone = shallow_clone_repo(source.path());
    run_atlas(clone.path(), &["init"]);

    let summary = read_json_data_output(
        "history_build",
        run_atlas(clone.path(), &["--json", "history", "build"]),
    );

    assert!(
        summary["warnings"]
            .as_array()
            .expect("warnings array")
            .iter()
            .any(|warning| warning.as_str().unwrap_or_default().contains("shallow clone detected")),
        "expected shallow clone warning in history build output: {summary:?}"
    );
    assert!(
        summary["elapsed_secs"].as_f64().unwrap_or_default() >= 0.0,
        "expected elapsed_secs in history build output: {summary:?}"
    );
}

#[test]
fn history_build_human_output_emits_progress_updates() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);

    let output = run_atlas(repo.path(), &["history", "build"]);
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");

    assert!(
        stderr.contains("progress start:")
            && stderr.contains("progress 1/")
            && stderr.contains("save 1/4")
            && stderr.contains("saving commit metadata")
            && stderr.contains("recomputing lifecycle history")
            && stderr.contains("src/lib.rs"),
        "expected progress output on stderr\nstderr:\n{stderr}"
    );
}
