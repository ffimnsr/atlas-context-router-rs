use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::Context;
use atlas_parser::ParserRegistry;
use atlas_store_sqlite::Store;
use serde::Serialize;

use crate::error::{HistoryError, Result};
use crate::git;
use crate::select::CommitSelector;

#[derive(Debug, Clone, Serialize, Default)]
pub struct HistoryEstimateSummary {
    pub branch: Option<String>,
    pub head_sha: Option<String>,
    pub indexed_base_sha: Option<String>,
    pub commits_selected: usize,
    pub commits_already_indexed: usize,
    pub commits_to_process: usize,
    pub estimated_total_files: usize,
    pub estimated_changed_files: usize,
    pub estimated_parseable_files: usize,
    pub estimated_reused_blobs: usize,
    pub estimated_new_blobs: usize,
    pub estimated_parseable_by_language: BTreeMap<String, usize>,
    pub warnings: Vec<String>,
    pub eta_low_secs: f64,
    pub eta_high_secs: f64,
}

pub fn estimate_historical_build(
    repo: &Path,
    canonical_root: &str,
    store: &Store,
    selector: &CommitSelector,
    registry: &ParserRegistry,
) -> Result<HistoryEstimateSummary> {
    let mut summary = HistoryEstimateSummary::default();
    if let Some(warning) = shallow_build_warning(repo, selector) {
        summary.warnings.push(warning);
    }

    let mut commits = selector.resolve(repo).context("resolve commit selector")?;
    if selector.prefers_oldest_first() && commits.len() > 1 {
        commits.reverse();
    }
    summary.commits_selected = commits.len();

    let repo_id = store.find_repo_id(canonical_root)?;
    estimate_selected_commits(repo, store, repo_id, registry, &commits, &mut summary)?;
    finalize_eta(&mut summary);
    Ok(summary)
}

pub fn estimate_historical_update(
    repo: &Path,
    canonical_root: &str,
    store: &Store,
    branch: Option<&str>,
    repair: bool,
    max_commits: Option<usize>,
    registry: &ParserRegistry,
) -> Result<HistoryEstimateSummary> {
    let branch = branch.unwrap_or("HEAD").to_owned();
    let mut summary = HistoryEstimateSummary {
        branch: Some(branch.clone()),
        ..HistoryEstimateSummary::default()
    };
    let repo_id = store.find_repo_id(canonical_root)?.ok_or_else(|| {
        anyhow::anyhow!("history not initialized; run `atlas history build` first")
    })?;

    if git::is_shallow(repo).unwrap_or(false) {
        summary.warnings.push(
            "shallow clone detected; update only sees fetched ancestry and may require `git fetch --unshallow` or deeper history.".to_owned(),
        );
    }

    let head_sha =
        git::rev_parse(repo, &branch).with_context(|| format!("resolve branch/ref {branch}"))?;
    summary.head_sha = Some(head_sha.clone());
    let latest_indexed_any_sha = store.latest_commit_sha(repo_id)?;

    let commits = git::log_commits(repo, &head_sha, max_commits, None, None)
        .with_context(|| format!("walk history from {head_sha}"))?;
    let ancestry_shas = commits
        .iter()
        .map(|commit| commit.sha.clone())
        .collect::<BTreeSet<_>>();
    let mut missing = Vec::new();
    let mut indexed_base_sha = None;
    for commit in commits {
        if store.find_snapshot(repo_id, &commit.sha)?.is_some() {
            indexed_base_sha = Some(commit.sha.clone());
            break;
        }
        missing.push(commit);
    }
    summary.indexed_base_sha = indexed_base_sha.clone();

    if summary
        .warnings
        .iter()
        .any(|warning| warning.contains("shallow clone detected"))
        && latest_indexed_any_sha.is_some()
        && indexed_base_sha.is_none()
    {
        return Err(HistoryError::Other(format!(
            "shallow clone missing indexed base commit {}; fetch more history or rerun after `git fetch --unshallow`",
            latest_indexed_any_sha.as_deref().unwrap_or("(unknown)")
        )));
    }

    let orphaned_indexed_snapshot = store
        .list_snapshots_ordered(repo_id)?
        .iter()
        .any(|snapshot| !ancestry_shas.contains(&snapshot.commit_sha));
    let divergence_detected = orphaned_indexed_snapshot
        || (latest_indexed_any_sha.is_some() && indexed_base_sha.is_none());
    if divergence_detected && !repair {
        return Err(HistoryError::Divergence(format!(
            "indexed history diverged from current {branch} ancestry; rerun with `atlas history update --repair` after verifying force-push or rewritten history"
        )));
    }

    missing.reverse();
    summary.commits_selected = missing.len();
    estimate_selected_commits(repo, store, Some(repo_id), registry, &missing, &mut summary)?;
    finalize_eta(&mut summary);
    Ok(summary)
}

fn estimate_selected_commits(
    repo: &Path,
    store: &Store,
    repo_id: Option<i64>,
    registry: &ParserRegistry,
    commits: &[git::GitCommitMeta],
    summary: &mut HistoryEstimateSummary,
) -> Result<()> {
    let mut known_snapshots = BTreeSet::new();
    let mut known_snapshot_file_counts = BTreeMap::new();
    let mut known_blob_graphs = BTreeSet::new();
    let mut checked_blob_graphs = BTreeMap::new();

    for meta in commits {
        let is_indexed = snapshot_metadata(store, repo_id, &meta.sha)?;
        if let Some(file_count) = is_indexed {
            summary.commits_already_indexed += 1;
            known_snapshots.insert(meta.sha.clone());
            known_snapshot_file_counts.insert(meta.sha.clone(), file_count);
            continue;
        }

        summary.commits_to_process += 1;
        let parent_sha = meta.parent_sha.as_deref();
        let parent_count = parent_sha
            .and_then(|sha| known_snapshot_file_counts.get(sha).copied())
            .or_else(|| {
                parent_sha.and_then(|sha| snapshot_metadata(store, repo_id, sha).ok().flatten())
            });

        let parent_known = parent_sha.is_some_and(|sha| {
            known_snapshots.contains(sha)
                || snapshot_metadata(store, repo_id, sha)
                    .ok()
                    .flatten()
                    .is_some()
        });

        let estimated_file_count = if parent_known {
            let changes = git::diff_tree_files(repo, &meta.sha, parent_sha)
                .with_context(|| format!("diff-tree for parent of {}", meta.sha))?;
            summary.estimated_changed_files += changes.len();
            let changed_paths = changes
                .iter()
                .filter_map(|(_, new_path, status)| match status {
                    'A' | 'M' | 'R' | 'C' => Some(new_path.clone()),
                    _ => None,
                })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let entry_map = git::ls_tree_paths(repo, &meta.sha, &changed_paths)?
                .into_iter()
                .map(|entry| (entry.file_path.clone(), entry))
                .collect::<BTreeMap<_, _>>();
            classify_entries(
                entry_map.values(),
                store,
                registry,
                &mut known_blob_graphs,
                &mut checked_blob_graphs,
                summary,
            )?;

            let mut estimated_file_count = parent_count.unwrap_or_default() as usize;
            for (old_path, _, status) in changes {
                match status {
                    'A' | 'C' => estimated_file_count += 1,
                    'D' => estimated_file_count = estimated_file_count.saturating_sub(1),
                    'R' if old_path.is_empty() => estimated_file_count += 1,
                    _ => {}
                }
            }
            estimated_file_count
        } else {
            let entries = git::ls_tree(repo, &meta.sha)
                .with_context(|| format!("ls-tree for {}", meta.sha))?;
            let blob_entries = entries
                .iter()
                .filter(|entry| entry.object_type == "blob")
                .collect::<Vec<_>>();
            let estimated_file_count = blob_entries.len();
            summary.estimated_changed_files += estimated_file_count;
            classify_entries(
                blob_entries.iter().copied(),
                store,
                registry,
                &mut known_blob_graphs,
                &mut checked_blob_graphs,
                summary,
            )?;
            blob_entries.len()
        };

        summary.estimated_total_files += estimated_file_count;
        known_snapshots.insert(meta.sha.clone());
        known_snapshot_file_counts.insert(meta.sha.clone(), estimated_file_count as i64);
    }

    Ok(())
}

fn classify_entries<'a, I>(
    entries: I,
    store: &Store,
    registry: &ParserRegistry,
    known_blob_graphs: &mut BTreeSet<String>,
    checked_blob_graphs: &mut BTreeMap<String, bool>,
    summary: &mut HistoryEstimateSummary,
) -> Result<()>
where
    I: IntoIterator<Item = &'a git::TreeEntry>,
{
    for entry in entries {
        if entry.object_type != "blob" || !registry.supports(&entry.file_path) {
            continue;
        }
        summary.estimated_parseable_files += 1;
        let bucket = language_bucket(&entry.file_path);
        *summary
            .estimated_parseable_by_language
            .entry(bucket)
            .or_insert(0) += 1;

        let has_graph = if known_blob_graphs.contains(&entry.object_hash) {
            true
        } else if let Some(has_graph) = checked_blob_graphs.get(&entry.object_hash) {
            *has_graph
        } else {
            let has_graph = store.has_historical_file_graph(&entry.object_hash)?;
            checked_blob_graphs.insert(entry.object_hash.clone(), has_graph);
            has_graph
        };

        if has_graph {
            summary.estimated_reused_blobs += 1;
            known_blob_graphs.insert(entry.object_hash.clone());
        } else {
            summary.estimated_new_blobs += 1;
            known_blob_graphs.insert(entry.object_hash.clone());
            checked_blob_graphs.insert(entry.object_hash.clone(), true);
        }
    }
    Ok(())
}

fn snapshot_metadata(store: &Store, repo_id: Option<i64>, commit_sha: &str) -> Result<Option<i64>> {
    let Some(repo_id) = repo_id else {
        return Ok(None);
    };
    Ok(store
        .find_snapshot(repo_id, commit_sha)?
        .map(|snapshot| snapshot.file_count))
}

fn shallow_build_warning(repo: &Path, selector: &CommitSelector) -> Option<String> {
    match git::is_shallow(repo) {
        Ok(true) => match selector {
            CommitSelector::Explicit { .. } => None,
            _ => Some(
                "shallow clone detected; build may omit older commits beyond fetched history. Fetch more history or use --commits with reachable SHAs.".to_owned(),
            ),
        },
        Ok(false) => None,
        Err(_) => None,
    }
}

fn finalize_eta(summary: &mut HistoryEstimateSummary) {
    let parse_secs = summary
        .estimated_parseable_by_language
        .iter()
        .map(|(language, count)| parse_cost_secs(language) * (*count as f64))
        .sum::<f64>();
    let write_secs = write_cost_secs(summary);
    let lifecycle_secs = lifecycle_cost_secs(summary);
    let base = 0.25
        + (summary.commits_to_process as f64 * 0.04)
        + (summary.estimated_total_files as f64 * 0.0003)
        + (summary.estimated_changed_files as f64 * 0.0015)
        + (summary.estimated_reused_blobs as f64 * 0.0012)
        + parse_secs
        + write_secs
        + lifecycle_secs;
    summary.eta_low_secs = (base * 0.7).max(0.1);
    summary.eta_high_secs = (base * 1.8).max(summary.eta_low_secs + 0.1);
}

fn write_cost_secs(summary: &HistoryEstimateSummary) -> f64 {
    if summary.commits_to_process == 0 {
        return 0.0;
    }

    (summary.commits_to_process as f64 * 0.02)
        + (summary.estimated_total_files as f64 * 0.0007)
        + (summary.estimated_reused_blobs as f64 * 0.0004)
        + (summary.estimated_new_blobs as f64 * 0.0009)
}

fn lifecycle_cost_secs(summary: &HistoryEstimateSummary) -> f64 {
    if summary.commits_to_process == 0 {
        return 0.0;
    }

    0.03 + (summary.commits_to_process as f64 * 0.015)
        + (summary.estimated_parseable_files as f64 * 0.0005)
}

fn parse_cost_secs(language: &str) -> f64 {
    match language {
        "rust" => 0.014,
        "typescript" | "javascript" => 0.012,
        "python" | "go" | "java" | "kotlin" | "cpp" | "c" => 0.01,
        "markdown" | "toml" | "yaml" | "sql" | "shell" => 0.003,
        _ => 0.008,
    }
}

fn language_bucket(path: &str) -> String {
    let ext = Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    match ext.as_str() {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" => "python",
        "go" => "go",
        "java" => "java",
        "kt" | "kts" => "kotlin",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" => "cpp",
        "md" | "markdown" => "markdown",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "sql" => "sql",
        "sh" | "bash" | "zsh" => "shell",
        "" => "unknown",
        other => other,
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use atlas_parser::ParserRegistry;
    use atlas_store_sqlite::Store;

    use super::*;
    use crate::select::CommitSelector;
    use crate::test_support::{commit_all, git, git_init, write_file};
    use crate::{build_historical_graph, rebuild_historical_snapshot};

    fn open_store(temp: &tempfile::TempDir) -> (String, Store) {
        let db_path = temp.path().join("history.sqlite");
        let db = db_path.to_string_lossy().into_owned();
        let store = Store::open(&db).expect("open store");
        (db, store)
    }

    #[test]
    fn estimate_build_counts_full_tree_for_unindexed_commit() {
        let repo = tempfile::tempdir().expect("tempdir");
        git_init(repo.path());
        write_file(repo.path(), "src/lib.rs", "pub fn alpha() -> i32 { 1 }\n");
        let first = commit_all(repo.path(), "first");

        let db_dir = tempfile::tempdir().expect("db tempdir");
        let (_db, store) = open_store(&db_dir);
        let registry = ParserRegistry::with_defaults();
        let canonical_root = std::fs::canonicalize(repo.path())
            .expect("canonical root")
            .to_string_lossy()
            .into_owned();

        let estimate = estimate_historical_build(
            repo.path(),
            &canonical_root,
            &store,
            &CommitSelector::Explicit { shas: vec![first] },
            &registry,
        )
        .expect("estimate build");

        assert_eq!(estimate.commits_selected, 1);
        assert_eq!(estimate.commits_to_process, 1);
        assert_eq!(estimate.estimated_total_files, 1);
        assert_eq!(estimate.estimated_changed_files, 1);
        assert_eq!(estimate.estimated_parseable_files, 1);
        assert_eq!(estimate.estimated_new_blobs, 1);
        assert_eq!(estimate.estimated_reused_blobs, 0);
        assert!(estimate.eta_high_secs >= estimate.eta_low_secs);
    }

    #[test]
    fn estimate_build_reuses_blob_hash_seen_earlier_in_same_run() {
        let repo = tempfile::tempdir().expect("tempdir");
        git_init(repo.path());
        write_file(repo.path(), "src/lib.rs", "pub fn alpha() -> i32 { 1 }\n");
        let first = commit_all(repo.path(), "first");
        git(repo.path(), &["mv", "src/lib.rs", "src/core.rs"]);
        let second = commit_all(repo.path(), "rename");

        let db_dir = tempfile::tempdir().expect("db tempdir");
        let (_db, store) = open_store(&db_dir);
        let registry = ParserRegistry::with_defaults();
        let canonical_root = std::fs::canonicalize(repo.path())
            .expect("canonical root")
            .to_string_lossy()
            .into_owned();

        let estimate = estimate_historical_build(
            repo.path(),
            &canonical_root,
            &store,
            &CommitSelector::Explicit {
                shas: vec![first, second],
            },
            &registry,
        )
        .expect("estimate build");

        assert_eq!(estimate.commits_selected, 2);
        assert_eq!(estimate.commits_to_process, 2);
        assert_eq!(estimate.estimated_new_blobs, 1);
        assert_eq!(estimate.estimated_reused_blobs, 1);
    }

    #[test]
    fn eta_adds_explicit_write_and_lifecycle_costs() {
        let summary = HistoryEstimateSummary {
            commits_to_process: 2,
            estimated_total_files: 10,
            estimated_changed_files: 4,
            estimated_parseable_files: 8,
            estimated_reused_blobs: 3,
            estimated_new_blobs: 5,
            ..HistoryEstimateSummary::default()
        };

        assert!(write_cost_secs(&summary) > 0.0);
        assert!(lifecycle_cost_secs(&summary) > 0.0);

        let idle = HistoryEstimateSummary::default();
        assert_eq!(write_cost_secs(&idle), 0.0);
        assert_eq!(lifecycle_cost_secs(&idle), 0.0);
    }

    #[test]
    fn estimate_update_only_counts_missing_commits() {
        let repo = tempfile::tempdir().expect("tempdir");
        git_init(repo.path());
        write_file(repo.path(), "src/lib.rs", "pub fn alpha() -> i32 { 1 }\n");
        let first = commit_all(repo.path(), "first");
        write_file(
            repo.path(),
            "src/lib.rs",
            "pub fn alpha() -> i32 { 1 }\npub fn beta() -> i32 { 2 }\n",
        );
        let second = commit_all(repo.path(), "second");

        let db_dir = tempfile::tempdir().expect("db tempdir");
        let (_db, store) = open_store(&db_dir);
        let registry = ParserRegistry::with_defaults();
        let canonical_root = std::fs::canonicalize(repo.path())
            .expect("canonical root")
            .to_string_lossy()
            .into_owned();

        build_historical_graph(
            repo.path(),
            &canonical_root,
            &store,
            &CommitSelector::Explicit {
                shas: vec![first.clone()],
            },
            &registry,
            None,
        )
        .expect("initial build");

        let estimate = estimate_historical_update(
            repo.path(),
            &canonical_root,
            &store,
            Some("HEAD"),
            false,
            None,
            &registry,
        )
        .expect("estimate update");

        assert_eq!(estimate.branch.as_deref(), Some("HEAD"));
        assert_eq!(estimate.head_sha.as_deref(), Some(second.as_str()));
        assert_eq!(estimate.indexed_base_sha.as_deref(), Some(first.as_str()));
        assert_eq!(estimate.commits_selected, 1);
        assert_eq!(estimate.commits_to_process, 1);
    }

    #[test]
    fn estimate_update_does_not_false_positive_on_rebuilt_older_snapshot() {
        let repo = tempfile::tempdir().expect("tempdir");
        git_init(repo.path());
        write_file(repo.path(), "src/lib.rs", "pub fn alpha() -> i32 { 1 }\n");
        let first = commit_all(repo.path(), "first");
        write_file(
            repo.path(),
            "src/lib.rs",
            "pub fn alpha() -> i32 { 1 }\npub fn beta() -> i32 { 2 }\n",
        );
        let second = commit_all(repo.path(), "second");

        let db_dir = tempfile::tempdir().expect("db tempdir");
        let (_db, store) = open_store(&db_dir);
        let registry = ParserRegistry::with_defaults();
        let canonical_root = std::fs::canonicalize(repo.path())
            .expect("canonical root")
            .to_string_lossy()
            .into_owned();

        build_historical_graph(
            repo.path(),
            &canonical_root,
            &store,
            &CommitSelector::Explicit {
                shas: vec![first.clone(), second.clone()],
            },
            &registry,
            None,
        )
        .expect("initial build");
        rebuild_historical_snapshot(
            repo.path(),
            &canonical_root,
            &store,
            &first,
            &registry,
            Some("HEAD"),
        )
        .expect("rebuild older snapshot");

        let estimate = estimate_historical_update(
            repo.path(),
            &canonical_root,
            &store,
            Some("HEAD"),
            false,
            None,
            &registry,
        )
        .expect("estimate update");

        assert_eq!(estimate.head_sha.as_deref(), Some(second.as_str()));
        assert_eq!(estimate.indexed_base_sha.as_deref(), Some(second.as_str()));
        assert_eq!(estimate.commits_selected, 0);
        assert_eq!(estimate.commits_to_process, 0);
    }

    #[test]
    fn estimate_update_requires_repair_for_rewritten_history() {
        let repo = tempfile::tempdir().expect("tempdir");
        git_init(repo.path());
        write_file(repo.path(), "src/lib.rs", "pub fn alpha() -> i32 { 1 }\n");
        let first = commit_all(repo.path(), "first");
        write_file(
            repo.path(),
            "src/lib.rs",
            "pub fn alpha() -> i32 { 1 }\npub fn beta() -> i32 { 2 }\n",
        );
        let second = commit_all(repo.path(), "second");

        let db_dir = tempfile::tempdir().expect("db tempdir");
        let (_db, store) = open_store(&db_dir);
        let registry = ParserRegistry::with_defaults();
        let canonical_root = std::fs::canonicalize(repo.path())
            .expect("canonical root")
            .to_string_lossy()
            .into_owned();

        build_historical_graph(
            repo.path(),
            &canonical_root,
            &store,
            &CommitSelector::Explicit {
                shas: vec![first.clone(), second.clone()],
            },
            &registry,
            None,
        )
        .expect("initial build");

        git(repo.path(), &["checkout", "--quiet", &first]);
        git(repo.path(), &["checkout", "--quiet", "-B", "main"]);
        write_file(repo.path(), "src/lib.rs", "pub fn gamma() -> i32 { 3 }\n");
        let rewritten = commit_all(repo.path(), "rewritten");

        let error = estimate_historical_update(
            repo.path(),
            &canonical_root,
            &store,
            Some("HEAD"),
            false,
            None,
            &registry,
        )
        .expect_err("must require repair");
        assert!(error.to_string().contains("--repair"));

        let repaired = estimate_historical_update(
            repo.path(),
            &canonical_root,
            &store,
            Some("HEAD"),
            true,
            None,
            &registry,
        )
        .expect("repair estimate");

        assert_eq!(repaired.head_sha.as_deref(), Some(rewritten.as_str()));
        assert_eq!(repaired.indexed_base_sha.as_deref(), Some(first.as_str()));
        assert_eq!(repaired.commits_selected, 1);
        assert_eq!(repaired.commits_to_process, 1);
    }
}
