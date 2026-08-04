mod build;
mod init;
mod status;

#[cfg(test)]
mod tests;

mod update;
mod watch;

use std::fmt::Display;

pub use build::run_build;
pub use init::run_init;
pub use status::run_status;
pub use update::run_update;
pub use watch::run_watch;

#[derive(Default)]
struct MultiRepoBudgetAggregate {
    selected_repo_count: usize,
    processed_repo_count: usize,
    failed_repo_count: usize,
    skipped_repo_count: usize,
    excluded_manual_repo_count: usize,
    budget_hit_repo_count: usize,
    files_accepted: usize,
    files_skipped_by_byte_budget: usize,
    bytes_accepted: u64,
    bytes_skipped: u64,
}
fn print_summary_value(label: &str, value: impl Display) {
    println!("  {label:<20}: {value}");
}
