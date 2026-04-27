use super::*;

#[path = "determinism_support.rs"]
mod determinism_support;

fn assert_json_stdout_deterministic(repo_root: &Path, args: &[&str]) {
    determinism_support::assert_text_deterministic(&format!("CLI {:?}", args), || {
        let output = run_atlas(repo_root, args);
        String::from_utf8(output.stdout).expect("atlas json stdout utf-8")
    });
}

#[test]
fn stable_json_commands_are_byte_identical_for_same_repo_and_commit() {
    let repo = setup_fixture_repo();

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    assert_json_stdout_deterministic(repo.path(), &["--json", "query", "greet_twice"]);
    assert_json_stdout_deterministic(repo.path(), &["--json", "context", "greet_twice"]);
    assert_json_stdout_deterministic(
        repo.path(),
        &[
            "--json",
            "impact",
            "--files",
            "src/lib.rs",
            "--max-depth",
            "3",
            "--max-nodes",
            "20",
        ],
    );
    assert_json_stdout_deterministic(
        repo.path(),
        &[
            "--json",
            "review-context",
            "--files",
            "src/lib.rs",
            "--max-depth",
            "3",
            "--max-nodes",
            "20",
        ],
    );
    assert_json_stdout_deterministic(
        repo.path(),
        &[
            "--json",
            "explain-change",
            "--files",
            "src/lib.rs",
            "--max-depth",
            "3",
            "--max-nodes",
            "20",
        ],
    );
}
