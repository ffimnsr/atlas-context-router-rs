use std::collections::BTreeSet;
use std::path::Path;
use std::time::Instant;

use anyhow::Context;

use crate::error::{HistoryError, Result};
use atlas_parser::ParserRegistry;
use atlas_store_sqlite::Store;
use serde::Serialize;

use crate::build::{BuildProgressEvent, BuildSummary, build_historical_graph_with_progress};
use crate::git;
use crate::lifecycle::{LifecycleSummary, load_existing_lifecycle_summary, recompute_lifecycle};
use crate::select::CommitSelector;

#[derive(Debug, Clone, Serialize, Default)]
pub struct HistoryUpdateSummary {
    pub branch: String,
    pub head_sha: String,
    pub indexed_base_sha: Option<String>,
    pub latest_indexed_sha: Option<String>,
    pub commits_found: usize,
    pub commits_processed: usize,
    pub divergence_detected: bool,
    pub repair_mode: bool,
    pub warnings: Vec<String>,
    pub lifecycle: LifecycleSummary,
    pub elapsed_secs: f64,
}

pub fn update_historical_graph(
    repo: &Path,
    canonical_root: &str,
    store: &Store,
    branch: Option<&str>,
    repair: bool,
    max_commits: Option<usize>,
    registry: &ParserRegistry,
) -> Result<HistoryUpdateSummary> {
    update_historical_graph_with_progress(
        repo,
        canonical_root,
        store,
        branch,
        repair,
        max_commits,
        registry,
        |_| {},
    )
}

#[allow(clippy::too_many_arguments)]
pub fn update_historical_graph_with_progress<P>(
    repo: &Path,
    canonical_root: &str,
    store: &Store,
    branch: Option<&str>,
    repair: bool,
    max_commits: Option<usize>,
    registry: &ParserRegistry,
    mut progress: P,
) -> Result<HistoryUpdateSummary>
where
    P: FnMut(BuildProgressEvent),
{
    let started = Instant::now();
    let branch = branch.unwrap_or("HEAD").to_owned();
    let mut warnings = Vec::new();
    let repo_id = store.find_repo_id(canonical_root)?.ok_or_else(|| {
        anyhow::anyhow!("history not initialized; run `atlas history build` first")
    })?;

    let is_shallow = git::is_shallow(repo).unwrap_or(false);
    if is_shallow {
        warnings.push(
            "shallow clone detected; update only sees fetched ancestry and may require `git fetch --unshallow` or deeper history.".to_owned(),
        );
    }

    let head_sha =
        git::rev_parse(repo, &branch).with_context(|| format!("resolve branch/ref {branch}"))?;
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

    if is_shallow && latest_indexed_any_sha.is_some() && indexed_base_sha.is_none() {
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

    let latest_indexed_sha = indexed_base_sha.clone().or(latest_indexed_any_sha);

    missing.reverse();
    let build_summary = if missing.is_empty() {
        BuildSummary::default()
    } else {
        let selector = CommitSelector::Explicit {
            shas: missing.into_iter().map(|meta| meta.sha).collect(),
        };
        build_historical_graph_with_progress(
            repo,
            canonical_root,
            store,
            &selector,
            registry,
            Some(&branch),
            &mut progress,
        )
        .context("build missing history commits")?
    };
    let lifecycle = if build_summary.commits_processed == 0 {
        load_existing_lifecycle_summary(canonical_root, store)
            .context("load existing lifecycle summary")?
    } else {
        progress(BuildProgressEvent::RunPhaseChanged {
            message: "recomputing lifecycle history".to_owned(),
        });
        recompute_lifecycle(canonical_root, store).context("recompute lifecycle")?
    };

    Ok(HistoryUpdateSummary {
        branch,
        head_sha,
        indexed_base_sha,
        latest_indexed_sha,
        commits_found: build_summary.commits_processed,
        commits_processed: build_summary.commits_processed,
        divergence_detected,
        repair_mode: repair,
        warnings,
        lifecycle,
        elapsed_secs: started.elapsed().as_secs_f64(),
    })
}

#[cfg(test)]
mod tests {
    use atlas_parser::ParserRegistry;
    use atlas_store_sqlite::Store;

    use super::*;
    use crate::select::CommitSelector;
    use crate::test_support::{commit_all, git, git_clone_shallow, git_init, write_file};
    use crate::{build_historical_graph, rebuild_historical_snapshot};

    fn open_store(temp: &tempfile::TempDir) -> (String, Store) {
        let db_path = temp.path().join("history.sqlite");
        let db = db_path.to_string_lossy().into_owned();
        let store = Store::open(&db).expect("open store");
        (db, store)
    }

    #[test]
    fn update_processes_only_missing_commits() {
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
        let summary = update_historical_graph(
            repo.path(),
            &canonical_root,
            &store,
            Some("HEAD"),
            false,
            None,
            &registry,
        )
        .expect("update");
        assert_eq!(summary.commits_processed, 1);
        assert_eq!(summary.indexed_base_sha.as_deref(), Some(first.as_str()));
        let repo_id = store
            .find_repo_id(&canonical_root)
            .expect("repo id")
            .expect("repo");
        assert!(
            store
                .find_snapshot(repo_id, &second)
                .expect("find snapshot")
                .is_some()
        );
    }

    #[test]
    fn update_requires_repair_for_rewritten_history() {
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
        commit_all(repo.path(), "rewritten");

        let error = update_historical_graph(
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

        let repaired = update_historical_graph(
            repo.path(),
            &canonical_root,
            &store,
            Some("HEAD"),
            true,
            None,
            &registry,
        )
        .expect("repair update");
        assert!(repaired.divergence_detected);
        assert!(repaired.repair_mode);
    }

    #[test]
    fn update_reports_missing_indexed_base_for_shallow_clone() {
        let source = tempfile::tempdir().expect("source tempdir");
        git_init(source.path());
        write_file(source.path(), "src/lib.rs", "pub fn alpha() -> i32 { 1 }\n");
        let first = commit_all(source.path(), "first");
        write_file(
            source.path(),
            "src/lib.rs",
            "pub fn alpha() -> i32 { 1 }\npub fn beta() -> i32 { 2 }\n",
        );
        commit_all(source.path(), "second");

        let clone = tempfile::tempdir().expect("clone tempdir");
        git_clone_shallow(source.path(), clone.path());

        let db_dir = tempfile::tempdir().expect("db tempdir");
        let (_db, store) = open_store(&db_dir);
        let registry = ParserRegistry::with_defaults();
        let canonical_root = std::fs::canonicalize(clone.path())
            .expect("canonical root")
            .to_string_lossy()
            .into_owned();
        let repo_id = store.upsert_repo(&canonical_root).expect("upsert repo");
        store
            .upsert_commit(&atlas_store_sqlite::StoredCommit {
                commit_sha: first.clone(),
                repo_id,
                parent_sha: None,
                indexed_ref: Some("HEAD".to_owned()),
                author_name: Some("Atlas Test".to_owned()),
                author_email: Some("test@atlas".to_owned()),
                author_time: 1,
                committer_time: 1,
                subject: "first".to_owned(),
                message: Some("first".to_owned()),
                indexed_at: String::new(),
            })
            .expect("upsert commit");
        store
            .insert_snapshot(repo_id, &first, None, 0, 0, 0, 1.0, 0)
            .expect("insert snapshot");

        let error = update_historical_graph(
            clone.path(),
            &canonical_root,
            &store,
            Some("HEAD"),
            false,
            None,
            &registry,
        )
        .expect_err("expected shallow clone diagnostic");
        let message = error.to_string();
        assert!(message.contains("shallow clone missing indexed base commit"));
        assert!(message.contains(&first));
    }

    #[test]
    fn update_does_not_false_positive_on_rebuilt_older_snapshot() {
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

        let summary = update_historical_graph(
            repo.path(),
            &canonical_root,
            &store,
            Some("HEAD"),
            false,
            None,
            &registry,
        )
        .expect("update without false divergence");

        assert_eq!(summary.indexed_base_sha.as_deref(), Some(second.as_str()));
        assert_eq!(summary.latest_indexed_sha.as_deref(), Some(second.as_str()));
        assert!(!summary.divergence_detected);
        assert_eq!(summary.commits_processed, 0);
    }

    #[test]
    fn update_noop_reuses_existing_lifecycle_summary() {
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
        let baseline = recompute_lifecycle(&canonical_root, &store).expect("baseline lifecycle");

        let summary = update_historical_graph(
            repo.path(),
            &canonical_root,
            &store,
            Some("HEAD"),
            false,
            None,
            &registry,
        )
        .expect("noop update");

        assert_eq!(summary.commits_processed, 0);
        assert_eq!(summary.lifecycle.snapshot_count, baseline.snapshot_count);
        assert_eq!(
            summary.lifecycle.node_history_rows,
            baseline.node_history_rows
        );
        assert_eq!(
            summary.lifecycle.edge_history_rows,
            baseline.edge_history_rows
        );
    }
}
