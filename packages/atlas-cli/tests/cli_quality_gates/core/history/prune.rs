use super::*;

#[test]
fn history_churn_json_reports_dedup_ratio_and_growth_metrics() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history churn fixture");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", format!("{first_sha},{second_sha}").as_str()],
    );

    let report = read_json_data_output(
        "history_churn",
        run_atlas(repo.path(), &["--json", "history", "churn"]),
    );

    assert_eq!(report["summary"]["snapshot_count"], json!(2));
    assert!(report["symbol_churn"].is_array());
    assert!(report["stability"]["stable_symbols"].is_array());
    assert!(report["trends"]["timeline"].is_array());
    assert!(
        report["storage_diagnostics"]["deduplication_ratio"]
            .as_f64()
            .unwrap_or_default()
            >= 1.0,
        "expected deduplication ratio in churn report: {report:?}"
    );
}

#[test]
fn history_prune_latest_n_keeps_most_recent_commit() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history prune latest fixture");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", format!("{first_sha},{second_sha}").as_str()],
    );

    let report = read_json_data_output(
        "history_prune",
        run_atlas(repo.path(), &["--json", "history", "prune", "--keep-latest", "1"]),
    );

    assert_eq!(report["commits_before"], json!(2));
    assert_eq!(report["commits_after"], json!(1));
    assert_eq!(report["snapshots_after"], json!(1));
    assert_eq!(
        report["deleted_commit_shas"].as_array().map_or(0, Vec::len),
        1
    );

    let status = read_json_data_output(
        "history_status",
        run_atlas(repo.path(), &["--json", "history", "status"]),
    );
    assert_eq!(status["indexed_commit_count"], json!(1));
    assert_eq!(status["latest_commit_sha"], json!(second_sha));
}

#[test]
fn history_prune_by_age_removes_old_commits() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    rewrite_fixture_helper(repo.path());
    let old_sha = commit_with_time(repo.path(), "history prune old fixture", 1_577_836_800);
    write_repo_file(repo.path(), "src/lib.rs", "pub fn fresh() -> &'static str { \"fresh\" }\n");
    let new_sha = commit_repo_change(repo.path(), "history prune fresh fixture");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", format!("{old_sha},{new_sha}").as_str()],
    );

    let report = read_json_data_output(
        "history_prune",
        run_atlas(
            repo.path(),
            &["--json", "history", "prune", "--older-than-days", "30"],
        ),
    );

    assert_eq!(report["commits_after"], json!(1));
    assert!(
        report["deleted_commit_shas"]
            .as_array()
            .expect("deleted commits array")
            .iter()
            .any(|sha| sha == &json!(old_sha)),
        "expected old commit to be pruned: {report:?}"
    );
}

#[test]
fn history_prune_tagged_policy_keeps_tagged_release() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    run_command(
        repo.path(),
        "git",
        &["tag", "-a", "v1.0.0", "-m", "release v1.0.0", first_sha.as_str()],
    );
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history prune tag fixture");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", format!("{first_sha},{second_sha}").as_str()],
    );

    let report = read_json_data_output(
        "history_prune",
        run_atlas(
            repo.path(),
            &["--json", "history", "prune", "--keep-tagged-only"],
        ),
    );

    assert_eq!(report["commits_after"], json!(1));
    let status = read_json_data_output(
        "history_status",
        run_atlas(repo.path(), &["--json", "history", "status"]),
    );
    assert_eq!(status["latest_commit_sha"], json!(first_sha));
}
