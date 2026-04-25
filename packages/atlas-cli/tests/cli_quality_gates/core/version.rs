use super::*;

#[test]
fn version_flag_includes_build_metadata() {
    let output = run_command(&current_repo_root(), env!("CARGO_BIN_EXE_atlas"), &["--version"]);
    let stdout = stdout_text(&output);

    assert_contains_all(
        &stdout,
        &[
            "atlas ",
            env!("CARGO_PKG_VERSION"),
            env!("GIT_HASH"),
            env!("GIT_COMMIT_DATE"),
            env!("CARGO_PROFILE"),
            env!("RUSTC_VERSION"),
            env!("BUILD_DATE"),
        ],
    );
}

#[test]
fn version_json_includes_build_metadata() {
    let output = run_command(
        &current_repo_root(),
        env!("CARGO_BIN_EXE_atlas"),
        &["--json", "version"],
    );
    let data = read_json_data_output("version", output);

    assert_eq!(data["version"], json!(env!("CARGO_PKG_VERSION")));
    assert_eq!(data["git_hash"], json!(env!("GIT_HASH")));
    assert_eq!(data["git_commit_date"], json!(env!("GIT_COMMIT_DATE")));
    assert_eq!(data["git_dirty"], json!(env!("GIT_DIRTY") == "true"));
    assert_eq!(data["cargo_profile"], json!(env!("CARGO_PROFILE")));
    assert_eq!(data["rustc_version"], json!(env!("RUSTC_VERSION")));
    assert_eq!(data["build_date"], json!(env!("BUILD_DATE")));
    assert_eq!(data["long_version"], json!(env!("ATLAS_LONG_VERSION")));
}
