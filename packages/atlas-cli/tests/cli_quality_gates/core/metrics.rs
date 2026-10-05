use super::*;

/// Contract test for the additive `data.metrics` block: real build/update runs
/// must expose counters and histograms, while dry-run output stays free of
/// nondeterministic timing data so golden snapshots remain stable.
#[test]
fn build_and_update_json_expose_runtime_metrics() {
    let repo = setup_repo(&[("src/lib.rs", "pub fn live() -> u32 { 1 }\n")]);
    run_atlas(repo.path(), &["init"]);

    let build = read_json_data_output("build", run_atlas(repo.path(), &["--json", "build"]));
    let build_metrics = &build["metrics"];
    assert_eq!(build_metrics["build_runs"]["build"], json!(1));
    assert!(
        build_metrics["build_duration_ms"]["build"]["count"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "build duration histogram must observe the run: {build_metrics}"
    );
    assert!(
        build_metrics["build_parsed_files"]["build"]["count"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "parsed-files histogram must observe the run: {build_metrics}"
    );
    assert!(
        build_metrics["parser_parses_total"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "parser parse counter must record build parses: {build_metrics}"
    );
    assert_eq!(build_metrics["parser_cache_reuse_ratio"], json!(0.0));

    write_repo_file(repo.path(), "src/lib.rs", "pub fn live() -> u32 { 2 }\n");
    let update = read_json_data_output("update", run_atlas(repo.path(), &["--json", "update"]));
    let update_metrics = &update["metrics"];
    assert_eq!(update_metrics["build_runs"]["update"], json!(1));
    assert!(
        update_metrics["parser_parses_total"]
            .as_u64()
            .unwrap_or_default()
            >= 1,
        "parser parse counter must record update parses: {update_metrics}"
    );
    assert!(update_metrics["parser_tree_reuses_total"].is_u64());

    let dry_run = read_json_data_output(
        "build",
        run_atlas(repo.path(), &["--json", "build", "--dry-run"]),
    );
    assert!(dry_run["dry_run"].as_bool().unwrap_or(false));
    assert!(
        dry_run.get("metrics").is_none(),
        "dry-run JSON must omit timing metrics for deterministic goldens: {dry_run}"
    );
}
