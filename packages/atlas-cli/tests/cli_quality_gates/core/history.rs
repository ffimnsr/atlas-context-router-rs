use super::*;

fn head_sha(repo_root: &Path) -> String {
    String::from_utf8(run_command(repo_root, "git", &["rev-parse", "HEAD"]).stdout)
        .expect("git rev-parse output")
        .trim()
        .to_owned()
}

fn commit_repo_change(repo_root: &Path, message: &str) -> String {
    run_command(repo_root, "git", &["add", "."]);
    run_command(repo_root, "git", &["commit", "--quiet", "-m", message]);
    head_sha(repo_root)
}

fn shallow_clone_repo(source: &Path) -> tempfile::TempDir {
    let clone = tempfile::tempdir().expect("clone tempdir");
    let source_url = format!("file://{}", source.display());
    run_command(
        Path::new("/"),
        "git",
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            source_url.as_str(),
            clone.path().to_str().expect("clone path"),
        ],
    );
    clone
}

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
            && stderr.contains("src/lib.rs"),
        "expected progress output on stderr\nstderr:\n{stderr}"
    );
}

fn commit_with_time(repo_root: &Path, message: &str, epoch_secs: i64) -> String {
    let output = sanitized_command("git")
        .args(["add", "."])
        .current_dir(repo_root)
        .output()
        .expect("git add for timed commit");
    assert!(output.status.success(), "git add failed: {:?}", output);

    let timestamp = epoch_secs.to_string();
    let output = sanitized_command("git")
        .args(["commit", "--quiet", "-m", message])
        .current_dir(repo_root)
        .env("GIT_AUTHOR_DATE", &timestamp)
        .env("GIT_COMMITTER_DATE", &timestamp)
        .output()
        .expect("git commit for timed commit");
    assert!(
        output.status.success(),
        "git timed commit failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    head_sha(repo_root)
}

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

#[test]
fn history_dependency_json_reports_add_and_remove_commits() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history dependency add fixture");
    write_repo_file(
        repo.path(),
        "src/lib.rs",
        r#"pub struct Greeter;

impl Greeter {
    pub fn greet_twice(name: &str) -> String {
        format!(\"Hello, {name}! Hello again, {name}!\")
    }
}

pub fn helper(name: &str, suffix: &str) -> String {
    let greeting = Greeter::greet_twice(name);
    format!(\"{greeting} [{suffix}]\")
}
"#,
    );
    let third_sha = commit_repo_change(repo.path(), "history dependency remove fixture");
    let commits = format!("{first_sha},{second_sha},{third_sha}");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", commits.as_str()],
    );

    let report = read_json_data_output(
        "history_dependency",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "history",
                "dependency",
                "src/lib.rs::method::Greeter::greet_twice",
                "src/lib.rs::method::Greeter::new",
            ],
        ),
    );

    assert_eq!(report["summary"]["first_appearance_commit_sha"], json!(second_sha));
    assert_eq!(report["summary"]["disappearance_commit_sha"], json!(third_sha));
    assert!(
        report["findings"]["commits_where_changed"]
            .as_array()
            .expect("dependency change commits")
            .iter()
            .any(|entry| {
                entry["removed_edges"]
                    .as_array()
                    .expect("removed edges")
                    .iter()
                    .any(|edge| edge.as_str().unwrap_or_default().contains("Greeter::new"))
            }),
        "expected removed dependency edge in dependency history: {report:?}"
    );
}

#[test]
fn history_module_json_reports_growth_trend() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history module fixture");
    let commits = format!("{first_sha},{second_sha}");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", commits.as_str()],
    );

    let report = read_json_data_output(
        "history_module",
        run_atlas(repo.path(), &["--json", "history", "module", "src/lib.rs"]),
    );

    assert_eq!(report["module"], json!("src/lib.rs"));
    assert!(
        report["findings"]["timeline"]
            .as_array()
            .expect("module timeline")
            .len()
            >= 2,
        "expected at least two module history points: {report:?}"
    );
    assert!(
        report["summary"]["max_node_count"].as_u64().unwrap_or_default() >= 3,
        "expected node growth in module history: {report:?}"
    );
}

#[test]
fn history_module_fixture_reports_split_and_merge_lifecycle() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    write_repo_file(
        repo.path(),
        "src/lib.rs",
        "mod helpers;\npub use helpers::{helper, Greeter};\n",
    );
    write_repo_file(
        repo.path(),
        "src/helpers.rs",
        r#"pub struct Greeter;

impl Greeter {
    pub fn greet(name: &str) -> String {
        format!("Hello, {name}!")
    }
}

pub fn helper(name: &str) -> String {
    Greeter::greet(name)
}
"#,
    );
    let first_sha = commit_repo_change(repo.path(), "history module split baseline");

    write_repo_file(
        repo.path(),
        "src/lib.rs",
        "mod core;\nmod extra;\npub use core::Greeter;\npub use extra::helper;\n",
    );
    write_repo_file(
        repo.path(),
        "src/core.rs",
        r#"pub struct Greeter;

impl Greeter {
    pub fn greet(name: &str) -> String {
        format!("Hello, {name}!")
    }
}
"#,
    );
    write_repo_file(
        repo.path(),
        "src/extra.rs",
        r#"use crate::core::Greeter;

pub fn helper(name: &str) -> String {
    Greeter::greet(name)
}
"#,
    );
    fs::remove_file(repo.path().join("src/helpers.rs")).expect("remove split source module");
    let second_sha = commit_repo_change(repo.path(), "history module split fixture");

    run_atlas(
        repo.path(),
        &[
            "history",
            "build",
            "--commits",
            format!("{first_sha},{second_sha}").as_str(),
        ],
    );

    let old_module = read_json_data_output(
        "history_module",
        run_atlas(repo.path(), &["--json", "history", "module", "src/helpers.rs"]),
    );
    let core_module = read_json_data_output(
        "history_module",
        run_atlas(repo.path(), &["--json", "history", "module", "src/core.rs"]),
    );
    let extra_module = read_json_data_output(
        "history_module",
        run_atlas(repo.path(), &["--json", "history", "module", "src/extra.rs"]),
    );

    assert_eq!(old_module["summary"]["first_appearance_commit_sha"], json!(first_sha));
    assert_eq!(old_module["summary"]["removal_commit_sha"], json!(second_sha));
    assert_eq!(core_module["summary"]["first_appearance_commit_sha"], json!(second_sha));
    assert_eq!(extra_module["summary"]["first_appearance_commit_sha"], json!(second_sha));
}

#[test]
fn history_json_outputs_include_required_evidence_fields() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    let first_sha = head_sha(repo.path());
    rewrite_fixture_helper(repo.path());
    let second_sha = commit_repo_change(repo.path(), "history evidence fixture");
    let commits = format!("{first_sha},{second_sha}");

    run_atlas(
        repo.path(),
        &["history", "build", "--commits", commits.as_str()],
    );

    for (command, args) in [
        (
            "history_symbol",
            vec!["--json", "history", "symbol", "src/lib.rs::fn::helper"],
        ),
        (
            "history_file",
            vec!["--json", "history", "file", "src/lib.rs"],
        ),
        (
            "history_dependency",
            vec![
                "--json",
                "history",
                "dependency",
                "src/lib.rs::fn::helper",
                "src/lib.rs::method::Greeter::greet_twice",
            ],
        ),
        (
            "history_module",
            vec!["--json", "history", "module", "src/lib.rs"],
        ),
    ] {
        let report = read_json_data_output(command, run_atlas(repo.path(), &args));
        assert!(report["summary"].is_object(), "missing summary for {command}: {report:?}");
        assert!(
            report["findings"].is_object(),
            "missing findings for {command}: {report:?}"
        );
        assert!(report["evidence"].is_object(), "missing evidence for {command}: {report:?}");
        assert!(report["evidence"]["snapshot_ids"].is_array());
        assert!(report["evidence"]["commit_shas"].is_array());
        assert!(report["evidence"]["node_identifiers"].is_array());
        assert!(report["evidence"]["edge_identifiers"].is_array());
        assert!(report["evidence"]["canonical_file_paths"].is_array());
    }
}
