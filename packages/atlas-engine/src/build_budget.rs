use std::time::Instant;

use atlas_core::{BudgetManager, BudgetPolicy, BudgetReport, BuildUpdateBudgetCounters};

use crate::config::BuildRunBudget;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildBudgetDecision {
    Continue,
    Degraded,
    Blocked,
}

pub(crate) struct BuildBudgetTracker {
    policy: BudgetPolicy,
    manager: BudgetManager,
    limits: BuildRunBudget,
    counters: BuildUpdateBudgetCounters,
}

impl BuildBudgetTracker {
    pub(crate) fn new(limits: BuildRunBudget) -> Self {
        Self {
            policy: BudgetPolicy::default(),
            manager: BudgetManager::new(),
            limits,
            counters: BuildUpdateBudgetCounters::default(),
        }
    }

    pub(crate) fn set_files_discovered(&mut self, files_discovered: usize) {
        self.counters.files_discovered = files_discovered;
    }

    pub(crate) fn note_skipped_by_file_bytes(&mut self, file_bytes: u64) {
        self.counters.files_skipped_by_byte_budget += 1;
        self.counters.bytes_skipped = self.counters.bytes_skipped.saturating_add(file_bytes);
        self.counters
            .budget_stop_reason
            .get_or_insert_with(|| "max_file_bytes".to_owned());
        self.manager.record_usage(
            self.policy.build_update.file_bytes,
            "build_update.max_file_bytes",
            self.limits.max_file_bytes as usize,
            file_bytes as usize,
            true,
        );
    }

    pub(crate) fn note_scanned_file_byte_skips(&mut self, count: usize) {
        if count == 0 {
            return;
        }
        self.counters.files_skipped_by_byte_budget += count;
        self.counters
            .budget_stop_reason
            .get_or_insert_with(|| "max_file_bytes".to_owned());
        self.manager.record_usage(
            self.policy.build_update.file_bytes,
            "build_update.max_file_bytes",
            self.limits.max_file_bytes as usize,
            self.limits.max_file_bytes as usize + 1,
            true,
        );
    }

    pub(crate) fn maybe_stop_for_time(&mut self, started: Instant) -> BuildBudgetDecision {
        let elapsed_ms = started.elapsed().as_millis() as u64;
        if elapsed_ms <= self.limits.max_wall_time_ms {
            return BuildBudgetDecision::Continue;
        }

        self.counters.budget_stop_reason = Some("max_wall_time_ms".to_owned());
        self.manager.record_usage(
            self.policy.build_update.wall_time_ms,
            "build_update.max_wall_time_ms",
            self.limits.max_wall_time_ms as usize,
            elapsed_ms as usize,
            true,
        );
        BuildBudgetDecision::Degraded
    }

    pub(crate) fn try_accept_file(&mut self, file_bytes: u64) -> BuildBudgetDecision {
        if self.counters.files_accepted >= self.limits.max_files_per_run {
            self.counters.budget_stop_reason = Some("max_files_per_run".to_owned());
            self.manager.record_usage(
                self.policy.build_update.files_per_run,
                "build_update.max_files_per_run",
                self.limits.max_files_per_run,
                self.counters.files_accepted.saturating_add(1),
                true,
            );
            return BuildBudgetDecision::Degraded;
        }

        let next_bytes = self.counters.bytes_accepted.saturating_add(file_bytes);
        if next_bytes > self.limits.max_total_bytes_per_run {
            self.counters.files_skipped_by_byte_budget += 1;
            self.counters.bytes_skipped = self.counters.bytes_skipped.saturating_add(file_bytes);
            self.counters.budget_stop_reason = Some("max_total_bytes_per_run".to_owned());
            self.manager.record_usage(
                self.policy.build_update.total_bytes_per_run,
                "build_update.max_total_bytes_per_run",
                self.limits.max_total_bytes_per_run as usize,
                next_bytes as usize,
                true,
            );
            return BuildBudgetDecision::Degraded;
        }

        self.counters.files_accepted += 1;
        self.counters.bytes_accepted = next_bytes;
        BuildBudgetDecision::Continue
    }

    pub(crate) fn note_parse_failure(&mut self, attempted_files: usize) -> BuildBudgetDecision {
        self.counters.parse_failures += 1;

        if self.counters.parse_failures > self.limits.max_parse_failures {
            self.counters.budget_stop_reason = Some("max_parse_failures".to_owned());
            self.manager.record_usage(
                self.policy.build_update.parse_failures,
                "build_update.max_parse_failures",
                self.limits.max_parse_failures,
                self.counters.parse_failures,
                false,
            );
            return BuildBudgetDecision::Blocked;
        }

        let failure_ratio_bps = if attempted_files == 0 {
            0
        } else {
            ((self.counters.parse_failures as u128) * 10_000 / attempted_files as u128) as usize
        };
        if failure_ratio_bps > self.limits.max_parse_failure_ratio_bps {
            self.counters.budget_stop_reason = Some("max_parse_failure_ratio".to_owned());
            self.manager.record_usage(
                self.policy.build_update.parse_failure_ratio_bps,
                "build_update.max_parse_failure_ratio",
                self.limits.max_parse_failure_ratio_bps,
                failure_ratio_bps,
                false,
            );
            return BuildBudgetDecision::Blocked;
        }

        BuildBudgetDecision::Continue
    }

    pub(crate) fn finish(self) -> (BuildUpdateBudgetCounters, BudgetReport) {
        let observed = self
            .counters
            .parse_failures
            .max(self.counters.files_skipped_by_byte_budget)
            .max(self.counters.files_accepted);
        let budget = self.manager.summary(
            "build_update",
            self.counters.files_accepted.max(1),
            observed,
        );
        (self.counters, budget)
    }
}
