use super::*;

#[test]
fn detached_current_repo_worktree_meets_large_repo_performance_gate() {
    let worktree = setup_current_repo_detached_worktree();

    run_atlas(worktree.path(), &["init"]);

    let build = read_json_data_output("build", run_atlas(worktree.path(), &["--json", "build"]));
    assert!(
        build["parsed"].as_u64().unwrap_or_default() >= 50,
        "representative repo build should parse substantial tracked files: {build:?}"
    );
    assert!(
        build["nodes_inserted"].as_u64().unwrap_or_default() >= 200,
        "representative repo build should index substantial graph size: {build:?}"
    );
    assert!(
        build["elapsed_ms"].as_u64().unwrap_or(u64::MAX) <= 60_000,
        "representative repo build latency regressed: {build:?}"
    );

    let status = read_json_data_output("status", run_atlas(worktree.path(), &["--json", "status"]));
    assert!(
        status["indexed_file_count"].as_u64().unwrap_or_default() >= 50,
        "status should report representative repo scale: {status:?}"
    );

    let query_started = Instant::now();
    let query = read_json_data_output(
        "query",
        run_atlas(worktree.path(), &["--json", "query", "ContextEngine"]),
    );
    let query_elapsed_ms = query_started.elapsed().as_millis();
    assert!(
        !query["results"].as_array().expect("query results array").is_empty(),
        "large-repo query should return known symbol hits: {query:?}"
    );
    assert!(query_elapsed_ms <= 10_000, "large-repo query latency regressed: {query:?}");
    let impact_target = "packages/atlas-impact/src/lib.rs";
    let original = fs::read_to_string(worktree.path().join(impact_target)).expect("read impact file");
    let mut updated = original.clone();
    updated.push_str("\n// perf gate change\n");
    write_repo_file(worktree.path(), impact_target, &updated);

    let impact_started = Instant::now();
    let impact = read_json_data_output(
        "impact",
        run_atlas(
            worktree.path(),
            &[
                "--json",
                "impact",
                "--base",
                "HEAD",
                "--max-depth",
                "3",
                "--max-nodes",
                "200",
            ],
        ),
    );
    let impact_elapsed_ms = impact_started.elapsed().as_millis();
    assert!(
        impact["analysis"]["base"]["changed_nodes"]
            .as_array()
            .expect("changed nodes array")
            .iter()
            .any(|node| node["file_path"] == json!(impact_target)),
        "impact must include changed file seed from representative repo: {impact:?}"
    );
    assert!(impact_elapsed_ms <= 15_000, "large-repo impact latency regressed: {impact:?}");

    let update = read_json_data_output(
        "update",
        run_atlas(worktree.path(), &["--json", "update", "--base", "HEAD"]),
    );
    assert!(
        update["parsed"].as_u64().unwrap_or_default() >= 1,
        "large-repo update should parse changed worktree file: {update:?}"
    );
    assert!(
        update["elapsed_ms"].as_u64().unwrap_or(u64::MAX) <= 20_000,
        "large-repo update latency regressed: {update:?}"
    );
}

#[test]
fn staged_submodule_edit_flows_through_detect_changes_and_update() {
    let repo = setup_repo_with_submodule(
        &[("README.md", "# atlas fixture\n")],
        "docs/wiki",
        &[(
            "src/lib.rs",
            "pub fn nested() -> &'static str {\n    \"v1\"\n}\n",
        )],
    );

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    write_repo_file(
        &repo.path().join("docs/wiki"),
        "src/lib.rs",
        "pub fn nested_v2() -> &'static str {\n    \"v2\"\n}\n",
    );
    run_command(repo.path().join("docs/wiki").as_path(), "git", &["add", "src/lib.rs"]);

    let changes = read_json_data_output(
        "detect_changes",
        run_atlas(repo.path(), &["--json", "detect-changes", "--staged"]),
    );
    let changes = changes["changes"].as_array().expect("detect-changes changes array");
    assert!(
        changes.iter().any(|change| {
            change["path"] == json!("docs/wiki/src/lib.rs")
                && change["change_type"] == json!("modified")
        }),
        "staged submodule edit should surface child path; got {changes:?}"
    );

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--staged"]),
    );
    assert!(update["parsed"].as_u64().unwrap_or_default() >= 1);
    assert!(update["nodes_updated"].as_u64().unwrap_or_default() >= 1);

    let new_query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "nested_v2"]),
    );
    assert!(
        new_query["results"]
            .as_array()
            .expect("query results array")
            .iter()
            .any(|result| result["node"]["qualified_name"] == json!("docs/wiki/src/lib.rs::fn::nested_v2")),
        "updated query should include renamed submodule function: {new_query:?}"
    );

    let old_query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "nested"]),
    );
    assert!(
        old_query["results"]
            .as_array()
            .expect("query results array")
            .iter()
            .all(|result| result["node"]["qualified_name"] != json!("docs/wiki/src/lib.rs::fn::nested")),
        "old submodule symbol should be removed after update: {old_query:?}"
    );
}

#[test]
fn base_ref_dirty_submodule_edit_flows_through_detect_changes_and_update() {
    let repo = setup_repo_with_submodule(
        &[("README.md", "# atlas fixture\n")],
        "docs/wiki",
        &[(
            "src/lib.rs",
            "pub fn nested() -> &'static str {\n    \"v1\"\n}\n",
        )],
    );

    run_atlas(repo.path(), &["init"]);
    run_atlas(repo.path(), &["build"]);

    write_repo_file(
        &repo.path().join("docs/wiki"),
        "src/lib.rs",
        "pub fn nested_v2() -> &'static str {\n    \"v2\"\n}\n",
    );

    let changes = read_json_data_output(
        "detect_changes",
        run_atlas(repo.path(), &["--json", "detect-changes", "--base", "HEAD"]),
    );
    let changes = changes["changes"].as_array().expect("detect-changes changes array");
    assert!(
        changes.iter().any(|change| {
            change["path"] == json!("docs/wiki/src/lib.rs")
                && change["change_type"] == json!("modified")
        }),
        "base-ref dirty submodule edit should surface child path; got {changes:?}"
    );

    let update = read_json_data_output(
        "update",
        run_atlas(repo.path(), &["--json", "update", "--base", "HEAD"]),
    );
    assert!(update["parsed"].as_u64().unwrap_or_default() >= 1);
    assert!(update["nodes_updated"].as_u64().unwrap_or_default() >= 1);

    let new_query = read_json_data_output(
        "query",
        run_atlas(repo.path(), &["--json", "query", "nested_v2"]),
    );
    assert!(
        new_query["results"]
            .as_array()
            .expect("query results array")
            .iter()
            .any(|result| result["node"]["qualified_name"] == json!("docs/wiki/src/lib.rs::fn::nested_v2")),
        "updated query should include dirty submodule function: {new_query:?}"
    );
}
