use super::*;

#[test]
fn history_update_human_output_includes_summary_fields() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    run_atlas(
        repo.path(),
        &["history", "build", "--commits", first_sha.as_str()],
    );

    rewrite_fixture_helper(repo.path());
    commit_repo_change(repo.path(), "history human update fixture");

    let output = run_atlas(repo.path(), &["history", "update"]);
    let stdout = stdout_text(&output);

    assert_contains_all(
        &stdout,
        &[
            "branch            : HEAD",
            "head              : ",
            "indexed base      : ",
            "latest indexed    : ",
            "commits processed : 1",
            "divergence        : no",
            "repair mode       : no",
            "node history rows : ",
            "edge history rows : ",
            "elapsed           : ",
        ],
    );
}

#[test]
fn history_update_human_output_emits_progress_updates() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    run_atlas(
        repo.path(),
        &["history", "build", "--commits", first_sha.as_str()],
    );

    rewrite_fixture_helper(repo.path());
    commit_repo_change(repo.path(), "history update progress fixture");

    let output = run_atlas(repo.path(), &["history", "update"]);
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");

    assert!(
        stderr.contains("progress start:")
            && stderr.contains("progress 1/")
            && stderr.contains("save 1/4")
            && stderr.contains("saving commit metadata")
            && stderr.contains("recomputing lifecycle history")
            && stderr.contains("src/lib.rs"),
        "expected update progress output on stderr\nstderr:\n{stderr}"
    );
}

#[test]
fn history_rebuild_json_replaces_single_snapshot() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history rebuild fixture");

    run_atlas(
        repo.path(),
        &[
            "history",
            "build",
            "--commits",
            format!("{first_sha},{second_sha}").as_str(),
        ],
    );

    let report = read_json_data_output(
        "history_rebuild",
        run_atlas(
            repo.path(),
            &["--json", "history", "rebuild", second_sha.as_str()],
        ),
    );

    assert_eq!(report["commit_sha"], json!(second_sha));
    assert!(report["replaced_snapshot_id"].as_i64().unwrap_or_default() > 0);
    assert!(report["rebuilt_snapshot_id"].as_i64().unwrap_or_default() > 0);
    assert_eq!(report["build"]["commits_processed"], json!(1));
    assert_eq!(report["lifecycle"]["snapshot_count"], json!(2));
}

#[test]
fn history_rebuild_human_output_emits_progress_updates() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history rebuild progress fixture");

    run_atlas(
        repo.path(),
        &[
            "history",
            "build",
            "--commits",
            format!("{first_sha},{second_sha}").as_str(),
        ],
    );

    let output = run_atlas(repo.path(), &["history", "rebuild", second_sha.as_str()]);
    let stderr = String::from_utf8(output.stderr).expect("stderr should be utf-8");

    assert!(
        stderr.contains("progress start:")
            && stderr.contains("progress 1/")
            && stderr.contains("save 1/4")
            && stderr.contains("saving commit metadata")
            && stderr.contains("recomputing lifecycle history")
            && stderr.contains("src/lib.rs"),
        "expected rebuild progress output on stderr\nstderr:\n{stderr}"
    );
}

#[test]
fn history_diff_json_reports_structural_changes() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history diff fixture");
    let commits = format!("{first_sha},{second_sha}");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", commits.as_str()],
    );

    let report = read_json_data_output(
        "history_diff",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "history",
                "diff",
                first_sha.as_str(),
                second_sha.as_str(),
            ],
        ),
    );

    assert_eq!(report["commit_a"], json!(first_sha));
    assert_eq!(report["commit_b"], json!(second_sha));
    assert!(
        report["modified_files"]
            .as_array()
            .expect("modified files array")
            .iter()
            .any(|file| file["file_path"] == json!("src/lib.rs")),
        "expected src/lib.rs in modified files output: {report:?}"
    );
    assert!(
        report["added_nodes"]
            .as_array()
            .expect("added nodes array")
            .iter()
            .any(|node| node["qualified_name"] == json!("src/lib.rs::method::Greeter::new")),
        "expected added constructor node in history diff output: {report:?}"
    );
    assert!(
        report["changed_nodes"]
            .as_array()
            .expect("changed nodes array")
            .iter()
            .any(|node| {
                node["qualified_name"] == json!("src/lib.rs::fn::helper")
                    && node["changed_fields"]
                        .as_array()
                        .expect("changed_fields array")
                        .iter()
                        .any(|field| field == "signature")
            }),
        "expected helper signature change in history diff output: {report:?}"
    );
    assert!(
        report["added_edges"].as_array().map_or(0, Vec::len) >= 1,
        "expected added edges in history diff output: {report:?}"
    );
    assert!(report["snapshot_a"]["nodes"].is_array());
    assert!(report["snapshot_b"]["edges"].is_array());
    assert!(report["architecture"]["changed_coupling"].is_array());
}

#[test]
fn history_diff_human_output_reports_summary_counts() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history diff human fixture");
    let commits = format!("{first_sha},{second_sha}");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", commits.as_str()],
    );

    let output = run_atlas(
        repo.path(),
        &["history", "diff", first_sha.as_str(), second_sha.as_str()],
    );
    let stdout = stdout_text(&output);

    assert_contains_all(
        &stdout,
        &[
            "commit a          : ",
            "commit b          : ",
            "files modified    : ",
            "nodes added       : ",
            "nodes changed     : ",
            "edges added       : ",
            "module changes    : ",
            "new cycles        : ",
            "broken cycles     : ",
        ],
    );
}

#[test]
fn history_diff_stat_only_json_returns_summary_contract() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history diff stat-only fixture");
    let commits = format!("{first_sha},{second_sha}");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", commits.as_str()],
    );

    let summary = read_json_data_output(
        "history_diff",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "history",
                "diff",
                first_sha.as_str(),
                second_sha.as_str(),
                "--stat-only",
            ],
        ),
    );

    assert!(summary.get("added_node_count").is_some(), "expected summary payload: {summary:?}");
    assert!(summary.get("commit_a").is_none(), "stat-only JSON should omit full diff payload: {summary:?}");
}

#[test]
fn history_diff_full_human_output_includes_detail_sections() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history diff full fixture");
    let commits = format!("{first_sha},{second_sha}");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", commits.as_str()],
    );

    let output = run_atlas(
        repo.path(),
        &[
            "history",
            "diff",
            first_sha.as_str(),
            second_sha.as_str(),
            "--full",
        ],
    );
    let stdout = stdout_text(&output);

    assert_contains_all(
        &stdout,
        &[
            "snapshot a        : nodes=",
            "snapshot b        : nodes=",
            "modified file details:",
            "changed nodes:",
        ],
    );
}
