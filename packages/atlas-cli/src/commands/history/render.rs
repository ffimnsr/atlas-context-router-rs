use std::io;
use std::io::IsTerminal;
use std::time::Duration;

use anyhow::Result;
use atlas_history::HistoryEstimateSummary;
use atlas_history::{BuildPersistProgressKind, BuildProgressEvent};
use dialoguer::{Confirm, theme::ColorfulTheme};
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryOutputMode {
    Summary,
    Full,
}

pub(crate) struct HistoryBuildProgress {
    state: ProgressState,
    completed_units: u64,
    total_units: u64,
}

enum ProgressState {
    Hidden,
    Plain,
    Bar(ProgressBar),
}

impl HistoryBuildProgress {
    pub(crate) fn new(enabled: bool) -> Self {
        let state = if !enabled {
            ProgressState::Hidden
        } else if io::stderr().is_terminal() {
            let bar = ProgressBar::new(0);
            bar.set_style(
                ProgressStyle::with_template(
                    "{spinner:.cyan} {pos}/{len} [{elapsed_precise}] {wide_bar:.cyan/blue} {msg}",
                )
                .expect("valid progress template")
                .progress_chars("=>-"),
            );
            bar.enable_steady_tick(Duration::from_millis(100));
            ProgressState::Bar(bar)
        } else {
            ProgressState::Plain
        };

        Self {
            state,
            completed_units: 0,
            total_units: 0,
        }
    }

    pub(crate) fn observe(&mut self, event: BuildProgressEvent) {
        match event {
            BuildProgressEvent::RunStarted { total_commits } => match &self.state {
                ProgressState::Plain => {
                    eprintln!("progress start: {total_commits} commit(s)");
                }
                ProgressState::Bar(bar) => {
                    bar.set_message(format!("resolving {total_commits} commit(s)"));
                }
                ProgressState::Hidden => {}
            },
            BuildProgressEvent::RunPhaseChanged { message } => match &self.state {
                ProgressState::Plain => {
                    let total = self.total_units.max(self.completed_units);
                    eprintln!("progress {}/{} {message}", self.completed_units, total);
                }
                ProgressState::Bar(bar) => {
                    bar.set_length(self.total_units.max(self.completed_units));
                    bar.set_position(self.completed_units);
                    bar.set_message(message);
                }
                ProgressState::Hidden => {}
            },
            BuildProgressEvent::CommitStarted {
                commit_index,
                total_commits,
                commit_sha,
                total_files,
            } => {
                self.total_units +=
                    total_files as u64 + BuildPersistProgressKind::total_steps() as u64;
                let message = format!(
                    "[{commit_index}/{total_commits}] {} enumerate {total_files} file(s)",
                    short_sha(&commit_sha)
                );
                match &self.state {
                    ProgressState::Plain => eprintln!("{message}"),
                    ProgressState::Bar(bar) => {
                        bar.set_length(self.total_units);
                        bar.set_message(message);
                    }
                    ProgressState::Hidden => {}
                }
            }
            BuildProgressEvent::CommitSkipped {
                commit_index,
                total_commits,
                commit_sha,
            } => {
                let message = format!(
                    "[{commit_index}/{total_commits}] {} already indexed",
                    short_sha(&commit_sha)
                );
                match &self.state {
                    ProgressState::Plain => eprintln!("{message}"),
                    ProgressState::Bar(bar) => {
                        bar.set_message(message);
                    }
                    ProgressState::Hidden => {}
                }
            }
            BuildProgressEvent::FileProcessed {
                commit_index,
                total_commits,
                commit_sha,
                outcome,
                file_path,
                ..
            } => {
                self.completed_units += 1;
                let total = self.total_units.max(self.completed_units);
                let message = format!(
                    "[{commit_index}/{total_commits}] {} {outcome}: {file_path}",
                    short_sha(&commit_sha)
                );
                match &self.state {
                    ProgressState::Plain => {
                        eprintln!("progress {}/{} {message}", self.completed_units, total);
                    }
                    ProgressState::Bar(bar) => {
                        bar.set_length(total);
                        bar.set_position(self.completed_units);
                        bar.set_message(message);
                    }
                    ProgressState::Hidden => {}
                }
            }
            BuildProgressEvent::CommitPersistStepStarted {
                commit_index,
                total_commits,
                commit_sha,
                step_index,
                total_steps,
                step,
            } => {
                self.completed_units += 1;
                let total = self.total_units.max(self.completed_units);
                let message = format!(
                    "[{commit_index}/{total_commits}] {} save {step_index}/{total_steps}: {}",
                    short_sha(&commit_sha),
                    step.label()
                );
                match &self.state {
                    ProgressState::Plain => {
                        eprintln!("progress {}/{} {message}", self.completed_units, total);
                    }
                    ProgressState::Bar(bar) => {
                        bar.set_length(total);
                        bar.set_position(self.completed_units);
                        bar.set_message(message);
                    }
                    ProgressState::Hidden => {}
                }
            }
        }
    }

    pub(crate) fn finish(&self) {
        if let ProgressState::Bar(bar) = &self.state {
            bar.finish_and_clear();
        }
    }
}

fn short_sha(sha: &str) -> &str {
    &sha[..sha.len().min(12)]
}

pub(crate) fn output_mode(_stat_only: bool, full: bool) -> HistoryOutputMode {
    if full {
        HistoryOutputMode::Full
    } else {
        HistoryOutputMode::Summary
    }
}

pub(crate) fn print_warnings(warnings: &[String]) {
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
}

pub(crate) fn should_print_history_estimate_preview(estimate: &HistoryEstimateSummary) -> bool {
    estimate.commits_to_process > 0 || !estimate.warnings.is_empty()
}

pub(crate) fn history_confirmation_needed(json: bool, assume_yes: bool) -> bool {
    !json && !assume_yes && io::stdin().is_terminal()
}

pub(crate) fn confirm_history_run(action: &str) -> Result<bool> {
    Confirm::with_theme(&ColorfulTheme::default())
        .with_prompt(format!("Continue with {action}?"))
        .default(true)
        .interact()
        .map_err(Into::into)
}

pub(crate) fn print_history_estimate_preview(action: &str, estimate: &HistoryEstimateSummary) {
    eprintln!("preflight {action} estimate:");
    if let Some(branch) = &estimate.branch {
        eprintln!("  branch            : {branch}");
    }
    if let Some(head_sha) = &estimate.head_sha {
        eprintln!("  head              : {head_sha}");
    }
    if let Some(indexed_base_sha) = &estimate.indexed_base_sha {
        eprintln!("  indexed base      : {indexed_base_sha}");
    }
    eprintln!("  commits selected  : {}", estimate.commits_selected);
    eprintln!("  commits skipped   : {}", estimate.commits_already_indexed);
    eprintln!("  commits to process: {}", estimate.commits_to_process);
    eprintln!("  files enumerated  : {}", estimate.estimated_total_files);
    eprintln!("  changed files     : {}", estimate.estimated_changed_files);
    eprintln!(
        "  parseable files   : {}",
        estimate.estimated_parseable_files
    );
    eprintln!("  reused blobs      : {}", estimate.estimated_reused_blobs);
    eprintln!("  new blobs         : {}", estimate.estimated_new_blobs);
    eprintln!(
        "  estimated time    : {}",
        format_eta_range(estimate.eta_low_secs, estimate.eta_high_secs)
    );
    eprintln!("  note              : estimate is approximate");
    print_warnings(&estimate.warnings);
}

fn format_eta_range(low_secs: f64, high_secs: f64) -> String {
    format!(
        "~{}-{}",
        format_eta_value(low_secs),
        format_eta_value(high_secs)
    )
}

fn format_eta_value(secs: f64) -> String {
    if secs < 1.0 {
        format!("{secs:.1}s")
    } else if secs < 60.0 {
        format!("{secs:.0}s")
    } else {
        format!("{:.1}m", secs / 60.0)
    }
}

pub(crate) fn print_diff_details(report: &atlas_history::GraphDiffReport) {
    if !report.modified_files.is_empty() {
        println!("modified file details:");
        for file in &report.modified_files {
            println!(
                "  {} old_hash={} new_hash={}",
                file.file_path,
                file.old_hash.as_deref().unwrap_or("-"),
                file.new_hash.as_deref().unwrap_or("-")
            );
        }
    }
    if !report.changed_nodes.is_empty() {
        println!("changed nodes:");
        for node in &report.changed_nodes {
            println!(
                "  {} [{}] {}",
                node.qualified_name,
                node.kind,
                node.changed_fields.join(", ")
            );
        }
    }
    if !report.changed_edges.is_empty() {
        println!("changed edges:");
        for edge in &report.changed_edges {
            println!(
                "  {} -> {} [{}] {}",
                edge.source_qn,
                edge.target_qn,
                edge.kind,
                edge.changed_fields.join(", ")
            );
        }
    }
}

pub(crate) fn print_symbol_details(report: &atlas_history::NodeHistoryReport) {
    if !report.findings.commits_where_changed.is_empty() {
        println!("change commits:");
        for change in &report.findings.commits_where_changed {
            println!("  {} {}", change.commit_sha, change.change_kinds.join(", "));
        }
    }
    if !report.findings.file_path_changes.is_empty() {
        println!("file path timeline:");
        for snapshot in &report.findings.file_path_changes {
            println!(
                "  {} {}",
                snapshot.commit_sha,
                snapshot.file_paths.join(", ")
            );
        }
    }
}

pub(crate) fn print_file_details(report: &atlas_history::FileHistoryReport) {
    if !report.findings.timeline.is_empty() {
        println!("timeline:");
        for point in &report.findings.timeline {
            println!(
                "  {} exists={} nodes={} edges={} added={} removed={}",
                point.commit_sha,
                point.exists,
                point.node_count,
                point.edge_count,
                point.symbol_additions.len(),
                point.symbol_removals.len()
            );
        }
    }
}

pub(crate) fn print_dependency_details(report: &atlas_history::EdgeHistoryReport) {
    if !report.findings.timeline.is_empty() {
        println!("timeline:");
        for point in &report.findings.timeline {
            println!(
                "  {} present={} edges={} added={} removed={}",
                point.commit_sha,
                point.present,
                point.edge_count,
                point.added_edges.len(),
                point.removed_edges.len()
            );
        }
    }
}

pub(crate) fn print_module_details(report: &atlas_history::ModuleHistoryReport) {
    if !report.findings.timeline.is_empty() {
        println!("timeline:");
        for point in &report.findings.timeline {
            println!(
                "  {} nodes={} deps={} coupling={} tests={}",
                point.commit_sha,
                point.node_count,
                point.dependency_count,
                point.coupling_count,
                point.test_adjacency_count
            );
        }
    }
}

pub(crate) fn print_churn_details(report: &atlas_history::ChurnReport) {
    if !report.symbol_churn.is_empty() {
        println!("top symbol churn:");
        for record in report.symbol_churn.iter().take(5) {
            println!(
                "  {} changes={} first={} last={}",
                record.qualified_name,
                record.change_count,
                record.first_commit_sha,
                record.last_commit_sha
            );
        }
    }
    if !report.trends.timeline.is_empty() {
        println!("trend timeline:");
        for point in &report.trends.timeline {
            println!(
                "  {} files={} nodes={} edges={} cycles={}",
                point.commit_sha,
                point.file_count,
                point.node_count,
                point.edge_count,
                point.cycle_count
            );
        }
    }
    println!(
        "storage summary: unique_hashes={} memberships={} snapshot_density={:.2}",
        report.storage_diagnostics.unique_file_hashes,
        report.storage_diagnostics.snapshot_file_memberships,
        report.storage_diagnostics.snapshot_density
    );
}

#[cfg(test)]
mod tests {
    use atlas_history::HistoryEstimateSummary;

    #[test]
    fn history_confirmation_needed_respects_json_and_yes() {
        assert!(!super::history_confirmation_needed(true, false));
        assert!(!super::history_confirmation_needed(false, true));
    }

    #[test]
    fn history_estimate_preview_hidden_when_no_work_and_no_warnings() {
        let estimate = HistoryEstimateSummary::default();
        assert!(!super::should_print_history_estimate_preview(&estimate));
    }

    #[test]
    fn history_estimate_preview_shown_for_work_or_warnings() {
        let mut estimate = HistoryEstimateSummary {
            commits_to_process: 1,
            ..HistoryEstimateSummary::default()
        };
        assert!(super::should_print_history_estimate_preview(&estimate));

        estimate.commits_to_process = 0;
        estimate.warnings.push("warning".to_string());
        assert!(super::should_print_history_estimate_preview(&estimate));
    }
}
