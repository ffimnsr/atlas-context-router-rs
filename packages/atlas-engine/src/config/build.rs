//! Build-phase configuration and run budgets.

use anyhow::Result;
use atlas_core::BudgetPolicy;
use atlas_repo::DEFAULT_MAX_FILE_BYTES;
use serde::{Deserialize, Serialize};

use super::{validate_u64_limit, validate_usize_limit};

/// Default parse-worker batch size.  Can be overridden in `.atlas/config.toml`.
pub const DEFAULT_PARSE_BATCH_SIZE: usize = 64;

/// Build-phase configuration.
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct BuildConfig {
    /// Number of files parsed in parallel per batch (clamped to 1–4096).
    pub parse_batch_size: usize,
    /// Maximum accepted files in one build/update run.
    pub max_files_per_run: usize,
    /// Maximum accepted total bytes in one build/update run.
    pub max_total_bytes_per_run: u64,
    /// Maximum accepted bytes for a single file.
    pub max_file_bytes: u64,
    /// Maximum parse failures tolerated before the run becomes build_failed.
    pub max_parse_failures: usize,
    /// Maximum tolerated parse failure ratio in the range [0.0, 1.0].
    pub max_parse_failure_ratio: f64,
    /// Maximum wall-clock time in milliseconds before the run becomes degraded.
    pub max_wall_time_ms: u64,
}

impl Default for BuildConfig {
    fn default() -> Self {
        let policy = BudgetPolicy::default();
        Self {
            parse_batch_size: DEFAULT_PARSE_BATCH_SIZE,
            max_files_per_run: policy.build_update.files_per_run.default_limit,
            max_total_bytes_per_run: policy.build_update.total_bytes_per_run.default_limit as u64,
            max_file_bytes: DEFAULT_MAX_FILE_BYTES,
            max_parse_failures: policy.build_update.parse_failures.default_limit,
            max_parse_failure_ratio: policy.build_update.parse_failure_ratio_bps.default_limit
                as f64
                / 10_000.0,
            max_wall_time_ms: policy.build_update.wall_time_ms.default_limit as u64,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BuildRunBudget {
    pub max_files_per_run: usize,
    pub max_total_bytes_per_run: u64,
    pub max_file_bytes: u64,
    pub max_parse_failures: usize,
    pub max_parse_failure_ratio_bps: usize,
    pub max_wall_time_ms: u64,
}

impl Default for BuildRunBudget {
    fn default() -> Self {
        let policy = BudgetPolicy::default();
        Self {
            max_files_per_run: policy.build_update.files_per_run.default_limit,
            max_total_bytes_per_run: policy.build_update.total_bytes_per_run.default_limit as u64,
            max_file_bytes: policy.build_update.file_bytes.default_limit as u64,
            max_parse_failures: policy.build_update.parse_failures.default_limit,
            max_parse_failure_ratio_bps: policy.build_update.parse_failure_ratio_bps.default_limit,
            max_wall_time_ms: policy.build_update.wall_time_ms.default_limit as u64,
        }
    }
}

impl BuildConfig {
    pub fn run_budget(&self) -> Result<BuildRunBudget> {
        let policy = BudgetPolicy::default();
        if !(0.0..=1.0).contains(&self.max_parse_failure_ratio) {
            anyhow::bail!(
                "invalid config: build.max_parse_failure_ratio={} must be within [0.0, 1.0]",
                self.max_parse_failure_ratio
            );
        }

        if self.max_parse_failures > policy.build_update.parse_failures.max_limit {
            anyhow::bail!(
                "invalid config: build.max_parse_failures={} exceeds safe maximum {}",
                self.max_parse_failures,
                policy.build_update.parse_failures.max_limit
            );
        }

        let ratio_bps = (self.max_parse_failure_ratio * 10_000.0).round() as usize;
        if ratio_bps > policy.build_update.parse_failure_ratio_bps.max_limit {
            anyhow::bail!(
                "invalid config: build.max_parse_failure_ratio={} exceeds safe maximum {}",
                self.max_parse_failure_ratio,
                policy.build_update.parse_failure_ratio_bps.max_limit as f64 / 10_000.0
            );
        }

        Ok(BuildRunBudget {
            max_files_per_run: validate_usize_limit(
                "build.max_files_per_run",
                self.max_files_per_run,
                policy.build_update.files_per_run.max_limit,
            )?,
            max_total_bytes_per_run: validate_u64_limit(
                "build.max_total_bytes_per_run",
                self.max_total_bytes_per_run,
                policy.build_update.total_bytes_per_run.max_limit,
            )?,
            max_file_bytes: validate_u64_limit(
                "build.max_file_bytes",
                self.max_file_bytes,
                policy.build_update.file_bytes.max_limit,
            )?,
            max_parse_failures: self.max_parse_failures,
            max_parse_failure_ratio_bps: ratio_bps,
            max_wall_time_ms: validate_u64_limit(
                "build.max_wall_time_ms",
                self.max_wall_time_ms,
                policy.build_update.wall_time_ms.max_limit,
            )?,
        })
    }
}
