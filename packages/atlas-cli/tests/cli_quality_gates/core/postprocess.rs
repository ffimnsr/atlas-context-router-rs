use super::*;

fn repo_with_large_function() -> TempDir {
    setup_repo(&[(
        "src/lib.rs",
        &("pub fn helper() -> i32 {\n".to_string()
            + &"    let value = 1;\n".repeat(45)
            + "    value\n}\n"),
    )])
}

#[test]
fn postprocess_no_graph_returns_noop_json() {
    let repo = setup_repo(&[("src/lib.rs", "pub fn helper() {}\n")]);
    run_atlas(repo.path(), &["init"]);

    let data = read_json_data_output(
        "postprocess",
        run_atlas(repo.path(), &["--json", "postprocess"]),
    );
    assert_eq!(data["ok"], json!(true));
    assert_eq!(data["noop"], json!(true));
    assert_eq!(data["graph_built"], json!(false));
    assert_eq!(data["error_code"], json!("none"));
}

#[test]
fn postprocess_full_after_build_reports_all_stages() {
    let repo = repo_with_large_function();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "postprocess",
        run_atlas(repo.path(), &["--json", "postprocess"]),
    );
    assert_eq!(data["ok"], json!(true));
    assert_eq!(data["noop"], json!(false));
    assert_eq!(data["stages"].as_array().map(Vec::len), Some(5));
    assert!(
        data["stages"]
            .as_array()
            .expect("stages")
            .iter()
            .any(|stage| stage["stage"] == json!("large_function_summaries"))
    );
}

#[test]
fn postprocess_changed_only_after_update_keeps_changed_file_scope() {
    let repo = repo_with_large_function();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);
    write_repo_file(
        repo.path(),
        "src/lib.rs",
        &("pub fn helper() -> i32 {\n".to_string()
            + &"    let value = 2;\n".repeat(45)
            + "    value\n}\n"),
    );
    run_atlas(repo.path(), &["--json", "update", "--base", "HEAD"]);

    let data = read_json_data_output(
        "postprocess",
        run_atlas(repo.path(), &["--json", "postprocess", "--changed-only"]),
    );
    assert_eq!(data["requested_mode"], json!("changed_only"));
    assert!(
        data["changed_files"]
            .as_array()
            .expect("changed files")
            .iter()
            .any(|value| value == "src/lib.rs")
    );
}

#[test]
fn postprocess_single_stage_runs_requested_stage_only() {
    let repo = repo_with_large_function();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "postprocess",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "postprocess",
                "--stage",
                "large_function_summaries",
            ],
        ),
    );
    assert_eq!(data["stage_filter"], json!("large_function_summaries"));
    assert_eq!(data["stages"].as_array().map(Vec::len), Some(1));
    assert_eq!(data["stages"][0]["stage"], json!("large_function_summaries"));
}

#[test]
fn postprocess_unknown_stage_uses_stable_json_error_code() {
    let repo = repo_with_large_function();
    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    let data = read_json_data_output(
        "postprocess",
        run_atlas(
            repo.path(),
            &["--json", "postprocess", "--stage", "not_real"],
        ),
    );
    assert_eq!(data["ok"], json!(false));
    assert_eq!(data["error_code"], json!("unknown_stage"));
}
