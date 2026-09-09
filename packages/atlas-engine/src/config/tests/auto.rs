//! Auto-profile tuning tests: pure `tuned_config` math with injected probes,
//! plus template rendering round-trips through `Config::load`.

use std::fs;

use tempfile::tempdir;

use super::super::{
    Config, ConfigTemplateProfile, RepoEstimate, SystemSnapshot, estimate_build_wall_seconds,
    render_auto_template, tuned_config,
};

fn system(physical: usize, logical: usize, ram_gib: f64) -> SystemSnapshot {
    SystemSnapshot {
        logical_cores: logical,
        physical_cores: physical,
        ram_total_bytes: (ram_gib * 1024.0 * 1024.0 * 1024.0) as u64,
        ram_available_bytes: (ram_gib * 512.0 * 1024.0 * 1024.0) as u64,
    }
}

fn repo(files: usize, bytes: u64) -> RepoEstimate {
    RepoEstimate { files, bytes }
}

#[test]
fn wall_time_is_clamped_to_policy_bounds() {
    // Formula: (1000/1000)*20*sqrt(8/8)+12 = 32s -> times 2 = 64s.
    let config = tuned_config(&system(8, 16, 16.0), &repo(1_000, 64 * 1024 * 1024));
    assert_eq!(config.build.max_wall_time_ms, 64_000);

    let config = tuned_config(&system(4, 4, 8.0), &repo(50_000, 1_000 * 1024 * 1024));
    assert_eq!(config.build.max_wall_time_ms, 300_000, "cap at policy max");

    let config = tuned_config(&system(8, 16, 16.0), &repo(0, 0));
    assert_eq!(config.build.max_wall_time_ms, 60_000, "small-repo floor");
}

#[test]
fn files_and_bytes_are_clamped_to_policy_bounds() {
    let config = tuned_config(&system(8, 16, 16.0), &repo(3_200, 40 * 1024 * 1024));
    assert_eq!(config.build.max_files_per_run, 4_000);
    assert_eq!(config.build.max_total_bytes_per_run, 80 * 1024 * 1024);

    let config = tuned_config(&system(8, 16, 16.0), &repo(100, 20 * 1024 * 1024));
    assert_eq!(
        config.build.max_total_bytes_per_run,
        64 * 1024 * 1024,
        "floor"
    );

    let config = tuned_config(&system(8, 16, 16.0), &repo(200_000, 1_000 * 1024 * 1024));
    assert_eq!(config.build.max_files_per_run, 50_000, "cap at policy max");
    assert_eq!(
        config.build.max_total_bytes_per_run,
        512 * 1024 * 1024,
        "cap at policy max"
    );
}

#[test]
fn batch_size_tracks_physical_cores_within_bounds() {
    let config = tuned_config(&system(2, 4, 8.0), &repo(100, 1));
    assert_eq!(config.build.parse_batch_size, 32);

    let config = tuned_config(&system(32, 64, 32.0), &repo(100, 1));
    assert_eq!(config.build.parse_batch_size, 256, "capped");
}

#[test]
fn low_ram_tightens_context_and_search_candidates() {
    let config = tuned_config(&system(4, 4, 2.0), &repo(100, 1));
    assert_eq!(config.context.max_context_nodes, 50);
    assert_eq!(config.context.max_context_payload_bytes, 16 * 1024);
    assert_eq!(config.search.max_query_candidates, 20);

    let config = tuned_config(&system(4, 4, 32.0), &repo(100, 1));
    assert_eq!(config.context.max_context_nodes, 150);
    assert_eq!(config.search.max_query_candidates, 60);

    let config = tuned_config(&system(4, 4, 8.0), &repo(100, 1));
    assert_eq!(config.context.max_context_nodes, 100);
    assert_eq!(config.search.max_query_candidates, 40);
}

#[test]
fn mcp_workers_scale_with_logical_cores_and_long_repos_get_tool_timeouts() {
    let config = tuned_config(&system(4, 2, 8.0), &repo(100, 1));
    assert_eq!(config.mcp.worker_threads, 2, "floor");

    let config = tuned_config(&system(4, 64, 8.0), &repo(100, 1));
    assert_eq!(config.mcp.worker_threads, 16, "cap");

    let config = tuned_config(&system(4, 4, 8.0), &repo(50_000, 1));
    assert_eq!(
        config.mcp.tool_timeout_ms_by_tool.get("build_graph"),
        Some(&600_000)
    );
    assert_eq!(
        config.mcp.tool_timeout_ms_by_tool.get("update_graph"),
        Some(&600_000)
    );
}

#[test]
fn auto_profile_excludes_markdown_and_json_from_insights() {
    let config = tuned_config(&system(8, 16, 16.0), &repo(1_000, 1));
    assert_eq!(
        config.insights.ignore_files,
        vec!["*.md".to_owned(), "*.json".to_owned()]
    );
    config
        .insights
        .validate()
        .expect("tuned insights config validates");
}

#[test]
fn tuned_config_survives_budget_and_policy_validation() {
    for (system, repo) in [
        (system(8, 16, 16.0), repo(1_000, 64 * 1024 * 1024)),
        (system(2, 4, 2.0), repo(200_000, 1_000 * 1024 * 1024)),
        (system(32, 64, 64.0), repo(0, 0)),
    ] {
        let config = tuned_config(&system, &repo);
        config.build_run_budget().expect("build budget validates");
        config.budget_policy().expect("budget policy validates");
    }
}

#[test]
fn estimate_scales_with_files_and_cores() {
    assert!(estimate_build_wall_seconds(1_000, 8) > estimate_build_wall_seconds(500, 8));
    assert!(estimate_build_wall_seconds(1_000, 2) > estimate_build_wall_seconds(1_000, 8));
    assert_eq!(estimate_build_wall_seconds(0, 8), 12.0, "startup floor");
}

#[test]
fn rendered_auto_template_loads_back_as_valid_config() {
    let dir = tempdir().unwrap();
    fs::create_dir_all(dir.path().join(".atlas")).unwrap();
    let atlas_dir = dir.path().join(".atlas");
    let content =
        render_auto_template(&system(8, 16, 16.0), &repo(1_000, 64 * 1024 * 1024)).unwrap();
    fs::write(atlas_dir.join("config.toml"), &content).unwrap();

    let loaded = Config::load(&atlas_dir).expect("rendered auto template loads");
    assert_eq!(loaded.build.max_wall_time_ms, 64_000);
    assert_eq!(
        loaded.insights.ignore_files,
        vec!["*.md".to_owned(), "*.json".to_owned()]
    );
    assert_eq!(loaded.mcp.worker_threads, 16);
    assert!(content.contains("# profile = \"auto\""));
    assert!(content.contains("est_build="));

    // Tuned values are active, not commented.
    assert!(content.contains("max_wall_time_ms = 64000"));
    assert!(content.contains("max_query_candidates = 60"));

    // The static render path refuses the auto profile without probes.
    let err = Config::render_template(ConfigTemplateProfile::Auto).unwrap_err();
    assert!(
        err.to_string()
            .contains("requires hardware and repo probes")
    );
}
