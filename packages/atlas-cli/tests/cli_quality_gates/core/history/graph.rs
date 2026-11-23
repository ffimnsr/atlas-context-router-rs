use super::*;

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
