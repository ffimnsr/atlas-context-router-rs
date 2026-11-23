use super::*;

#[test]
fn history_symbol_json_reports_evolution() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history symbol fixture");
    let commits = format!("{first_sha},{second_sha}");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", commits.as_str()],
    );

    let report = read_json_data_output(
        "history_symbol",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "history",
                "symbol",
                "src/lib.rs::fn::helper",
            ],
        ),
    );

    assert_eq!(report["qualified_name"], json!("src/lib.rs::fn::helper"));
    assert_eq!(report["summary"]["first_appearance_commit_sha"], json!(first_sha));
    assert_eq!(report["summary"]["last_appearance_commit_sha"], json!(second_sha));
    assert!(
        report["findings"]["commits_where_changed"]
            .as_array()
            .expect("change commits array")
            .iter()
            .any(|entry| {
                entry["change_kinds"]
                    .as_array()
                    .expect("change kinds")
                    .iter()
                    .any(|kind| kind == "signature_evolution")
            }),
        "expected signature evolution in symbol history: {report:?}"
    );
    assert!(report["findings"]["signature_evolution"].is_array());
}

#[test]
fn history_symbol_fixture_reports_introduced_modified_and_removed() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history symbol modify fixture");
    write_repo_file(
        repo.path(),
        "src/lib.rs",
        r#"pub struct Greeter;

impl Greeter {
    pub fn greet_twice(name: &str) -> String {
        format!("Hello, {name}! Hello again, {name}!")
    }
}
"#,
    );
    let third_sha = commit_repo_change(repo.path(), "history symbol remove fixture");
    let commits = format!("{first_sha},{second_sha},{third_sha}");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", commits.as_str()],
    );

    let report = read_json_data_output(
        "history_symbol",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "history",
                "symbol",
                "src/lib.rs::fn::helper",
            ],
        ),
    );

    assert_eq!(report["summary"]["first_appearance_commit_sha"], json!(first_sha));
    assert_eq!(report["summary"]["last_appearance_commit_sha"], json!(second_sha));
    assert_eq!(report["summary"]["removal_commit_sha"], json!(third_sha));
    assert!(
        report["findings"]["commits_where_changed"]
            .as_array()
            .expect("symbol change commits")
            .iter()
            .any(|entry| {
                entry["change_kinds"]
                    .as_array()
                    .expect("change kinds")
                    .iter()
                    .any(|kind| kind == "introduced")
            }),
        "expected introduced symbol fixture change: {report:?}"
    );
    assert!(
        report["findings"]["commits_where_changed"]
            .as_array()
            .expect("symbol change commits")
            .iter()
            .any(|entry| {
                entry["change_kinds"]
                    .as_array()
                    .expect("change kinds")
                    .iter()
                    .any(|kind| kind == "signature_evolution")
            }),
        "expected modified symbol fixture change: {report:?}"
    );
    assert!(
        report["findings"]["commits_where_changed"]
            .as_array()
            .expect("symbol change commits")
            .iter()
            .any(|entry| {
                entry["commit_sha"] == json!(third_sha)
                    && entry["change_kinds"]
                        .as_array()
                        .expect("change kinds")
                        .iter()
                        .any(|kind| kind == "removed")
            }),
        "expected removed symbol fixture change: {report:?}"
    );
}

#[test]
fn history_status_json_reports_partial_and_parse_error_counts() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let status = read_json_data_output(
        "history_status",
        run_atlas(repo.path(), &["--json", "history", "status"]),
    );

    assert_eq!(status["partial_snapshot_count"], json!(0));
    assert_eq!(status["parse_error_snapshot_count"], json!(0));
}

#[test]
fn history_file_json_reports_timeline_and_symbol_deltas() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history file fixture");
    let commits = format!("{first_sha},{second_sha}");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", commits.as_str()],
    );

    let report = read_json_data_output(
        "history_file",
        run_atlas(repo.path(), &["--json", "history", "file", "src/lib.rs"]),
    );

    assert_eq!(report["file_path"], json!("src/lib.rs"));
    assert_eq!(report["summary"]["first_appearance_commit_sha"], json!(first_sha));
    assert_eq!(report["summary"]["last_appearance_commit_sha"], json!(second_sha));
    assert!(
        report["findings"]["timeline"]
            .as_array()
            .expect("timeline array")
            .iter()
            .any(|entry| {
                entry["symbol_additions"]
                    .as_array()
                    .expect("symbol additions")
                    .iter()
                    .any(|symbol| symbol == "src/lib.rs::method::Greeter::new")
            }),
        "expected added constructor symbol in file history: {report:?}"
    );
}

#[test]
fn history_file_follow_renames_json_reports_prior_path_lineage() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    fs::rename(repo.path().join("src/lib.rs"), repo.path().join("src/history_lib.rs"))
        .expect("rename fixture file");
    run_command(repo.path(), "git", &["add", "-A"]);
    let second_sha = commit_repo_change(repo.path(), "history file rename fixture");
    let commits = format!("{first_sha},{second_sha}");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", commits.as_str()],
    );

    let without_follow = read_json_data_output(
        "history_file",
        run_atlas(
            repo.path(),
            &["--json", "history", "file", "src/history_lib.rs"],
        ),
    );
    assert_eq!(
        without_follow["summary"]["first_appearance_commit_sha"],
        json!(second_sha)
    );

    let with_follow = read_json_data_output(
        "history_file",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "history",
                "file",
                "src/history_lib.rs",
                "--follow-renames",
            ],
        ),
    );

    assert_eq!(with_follow["summary"]["first_appearance_commit_sha"], json!(first_sha));
    assert!(
        with_follow["evidence"]["canonical_file_paths"]
            .as_array()
            .expect("canonical file paths")
            .iter()
            .any(|path| path == "src/lib.rs"),
        "expected prior file path in rename-follow evidence: {with_follow:?}"
    );
}
