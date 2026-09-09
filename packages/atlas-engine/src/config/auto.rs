//! Auto profile: tune `.atlas/config.toml` from hardware + repo estimates.
//!
//! Used by `atlas init --profile auto` (the default). Everything is
//! dependency-injected through [`SystemSnapshot`] and [`RepoEstimate`] so the
//! tuning math is unit-testable without real hardware or git access.

use std::path::Path;

use anyhow::Result;
use atlas_repo::estimate_tracked_files;
use camino::Utf8Path;

use super::Config;
use super::template::ConfigTemplateProfile;

/// Conservative fallback used when a platform probe yields nothing.
const FALLBACK_LOGICAL_CORES: usize = 4;
const FALLBACK_RAM_BYTES: u64 = 8 * 1024 * 1024 * 1024;

/// One-shot hardware snapshot collected at `atlas init` time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemSnapshot {
    pub logical_cores: usize,
    pub physical_cores: usize,
    pub ram_total_bytes: u64,
    pub ram_available_bytes: u64,
}

impl Default for SystemSnapshot {
    fn default() -> Self {
        Self {
            logical_cores: FALLBACK_LOGICAL_CORES,
            physical_cores: FALLBACK_LOGICAL_CORES,
            ram_total_bytes: FALLBACK_RAM_BYTES,
            ram_available_bytes: FALLBACK_RAM_BYTES / 2,
        }
    }
}

/// Repo size estimate collected at `atlas init` time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RepoEstimate {
    pub files: usize,
    pub bytes: u64,
}

/// Probe hardware with `sysinfo`. Never fails; falls back to conservative
/// defaults when the platform cannot report a value.
pub fn probe_system() -> SystemSnapshot {
    let system = sysinfo::System::new_all();
    let logical_cores = system.cpus().len().max(1);
    let physical_cores = sysinfo::System::physical_core_count()
        .filter(|count| *count > 0)
        .unwrap_or(logical_cores);
    SystemSnapshot {
        logical_cores,
        physical_cores,
        ram_total_bytes: system.total_memory(),
        ram_available_bytes: system.available_memory(),
    }
}

/// Probe repo size via `git ls-files` + one stat per file (never reads
/// contents). Errors propagate so `init` can surface a clear message.
pub fn probe_repo(repo_root: &Path) -> Result<RepoEstimate> {
    let root = Utf8Path::from_path(repo_root)
        .ok_or_else(|| anyhow::anyhow!("repo root is not valid UTF-8: {}", repo_root.display()))?;
    let (files, bytes) = estimate_tracked_files(root)?;
    Ok(RepoEstimate { files, bytes })
}

/// Estimated full-build wall time in seconds for `files` files on
/// `eff_cores` effective cores.
///
/// Calibrated on real repos: ~20s per 1000 files at 8 effective cores, plus a
/// ~12s discovery/startup floor, scaled sub-linearly by core count.
pub fn estimate_build_wall_seconds(files: usize, eff_cores: usize) -> f64 {
    let cores = eff_cores.max(1) as f64;
    (files as f64 / 1000.0) * 20.0 * (8.0 / cores).sqrt() + 12.0
}

fn ram_gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

/// Derive the tuned active config from hardware + repo estimates.
///
/// Every value is clamped to the same policy maxima enforced by config
/// validation (`run_budget` / `budget_policy`).
pub fn tuned_config(system: &SystemSnapshot, repo: &RepoEstimate) -> Config {
    let mut config = Config::default();

    let eff_cores = system.physical_cores.clamp(1, 8);
    let wall_est_s = estimate_build_wall_seconds(repo.files, eff_cores);

    // [build]
    config.build.max_wall_time_ms = ((wall_est_s * 2000.0) as u64).clamp(60_000, 300_000);
    config.build.max_files_per_run = (repo.files.saturating_mul(11) / 10).div_ceil(1000) * 1000;
    config.build.max_files_per_run = config.build.max_files_per_run.clamp(1_000, 50_000);
    config.build.max_total_bytes_per_run = repo
        .bytes
        .saturating_mul(2)
        .clamp(64 * 1024 * 1024, 512 * 1024 * 1024);
    config.build.parse_batch_size = (system.physical_cores.saturating_mul(16)).clamp(16, 256);

    // [search]
    config.search.max_query_candidates = if ram_gib(system.ram_total_bytes) >= 16.0 {
        60
    } else if ram_gib(system.ram_total_bytes) >= 4.0 {
        40
    } else {
        20
    };

    // [mcp]
    config.mcp.worker_threads = system.logical_cores.clamp(2, 16);
    if wall_est_s > 120.0 {
        config
            .mcp
            .tool_timeout_ms_by_tool
            .insert("build_graph".to_owned(), 600_000);
        config
            .mcp
            .tool_timeout_ms_by_tool
            .insert("update_graph".to_owned(), 600_000);
    }

    // [context] RAM guards
    let ram_gib = ram_gib(system.ram_total_bytes);
    if ram_gib < 4.0 {
        config.context.max_context_nodes = 50;
        config.context.max_context_payload_bytes = 16 * 1024;
    } else if ram_gib >= 16.0 {
        config.context.max_context_nodes = 150;
    }

    // [insights] docs and data fixtures are not code
    config.insights.ignore_files = vec!["*.md".to_owned(), "*.json".to_owned()];

    config
}

/// Render the auto-profile template for the given probes.
pub fn render_auto_template(system: &SystemSnapshot, repo: &RepoEstimate) -> Result<String> {
    let active = tuned_config(system, repo);
    let eff_cores = system.physical_cores.clamp(1, 8);
    let wall_est_s = estimate_build_wall_seconds(repo.files, eff_cores);
    let banner = vec![
        "# Auto profile: values tuned from hardware and repo estimates.".to_owned(),
        format!(
            "#   cores={} ({} logical)  ram={:.0}GiB  files={}  est_build={:.0}s",
            system.physical_cores,
            system.logical_cores,
            ram_gib(system.ram_total_bytes),
            repo.files,
            wall_est_s,
        ),
        "# Tuned values are active. Edit freely; re-tune by removing this file".to_owned(),
        "# and running `atlas init --profile auto` again.".to_owned(),
    ];
    Config::render_template_with(ConfigTemplateProfile::Auto, active, banner)
}
