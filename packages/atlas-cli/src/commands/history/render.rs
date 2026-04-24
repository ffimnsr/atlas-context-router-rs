use std::io;
use std::io::IsTerminal;
use std::time::Duration;

use atlas_history::BuildProgressEvent;
use indicatif::{ProgressBar, ProgressStyle};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum HistoryOutputMode {
    Summary,
    Full,
}

pub(crate) struct HistoryBuildProgress {
    state: ProgressState,
    processed_files: u64,
    discovered_files: u64,
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
            processed_files: 0,
            discovered_files: 0,
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
            BuildProgressEvent::CommitStarted {
                commit_index,
                total_commits,
                commit_sha,
                total_files,
            } => {
                self.discovered_files += total_files as u64;
                let message = format!(
                    "[{commit_index}/{total_commits}] {} enumerate {total_files} file(s)",
                    short_sha(&commit_sha)
                );
                match &self.state {
                    ProgressState::Plain => eprintln!("{message}"),
                    ProgressState::Bar(bar) => {
                        bar.set_length(self.discovered_files);
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
                self.processed_files += 1;
                let total = self.discovered_files.max(self.processed_files);
                let message = format!(
                    "[{commit_index}/{total_commits}] {} {outcome}: {file_path}",
                    short_sha(&commit_sha)
                );
                match &self.state {
                    ProgressState::Plain => {
                        eprintln!("progress {}/{} {message}", self.processed_files, total);
                    }
                    ProgressState::Bar(bar) => {
                        bar.set_length(total);
                        bar.set_position(self.processed_files);
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
