use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result, bail};
use atlas_parser::ParserRegistry;
use atlas_store_sqlite::Store;
use serde::Serialize;

use crate::build::{BuildSummary, build_historical_graph};
use crate::git;
use crate::lifecycle::{LifecycleSummary, recompute_lifecycle};
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
    let latest_indexed_sha = store.latest_commit_sha(repo_id)?;

    let commits = git::log_commits(repo, &head_sha, max_commits, None, None)
        .with_context(|| format!("walk history from {head_sha}"))?;

    let mut missing = Vec::new();
    let mut indexed_base_sha = None;
    for commit in commits {
        if store.find_snapshot(repo_id, &commit.sha)?.is_some() {
            indexed_base_sha = Some(commit.sha.clone());
            break;
        }
        missing.push(commit);
    }

    if is_shallow && latest_indexed_sha.is_some() && indexed_base_sha.is_none() {
        bail!(
            "shallow clone missing indexed base commit {}; fetch more history or rerun after `git fetch --unshallow`",
            latest_indexed_sha.as_deref().unwrap_or("(unknown)")
        );
    }

    let divergence_detected = match latest_indexed_sha.as_ref() {
        Some(latest) => indexed_base_sha.as_ref() != Some(latest),
        None => false,
    };
    if divergence_detected && !repair {
        bail!(
            "indexed history diverged from current {branch} ancestry; rerun with `atlas history update --repair` after verifying force-push or rewritten history"
        );
    }

    missing.reverse();
    let build_summary = if missing.is_empty() {
        BuildSummary::default()
    } else {
        let selector = CommitSelector::Explicit {
            shas: missing.into_iter().map(|meta| meta.sha).collect(),
        };
        build_historical_graph(
            repo,
            canonical_root,
            store,
            &selector,
            registry,
            Some(&branch),
        )
        .context("build missing history commits")?
    };
    let lifecycle = recompute_lifecycle(canonical_root, store).context("recompute lifecycle")?;

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
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use atlas_parser::ParserRegistry;
    use atlas_store_sqlite::Store;

    use super::*;
    use crate::build_historical_graph;
    use crate::select::CommitSelector;

    const GIT_TEST_NAME: &str = "Atlas Test";
    const GIT_TEST_EMAIL: &str = "test@atlas";
    const GIT_LOCAL_ENV_VARS: &[&str] = &[
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
        "GIT_COMMON_DIR",
        "GIT_CONFIG",
        "GIT_CONFIG_COUNT",
        "GIT_CONFIG_KEY_0",
        "GIT_CONFIG_VALUE_0",
        "GIT_DIR",
        "GIT_GRAFT_FILE",
        "GIT_IMPLICIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_INTERNAL_SUPER_PREFIX",
        "GIT_NAMESPACE",
        "GIT_NO_REPLACE_OBJECTS",
        "GIT_OBJECT_DIRECTORY",
        "GIT_PREFIX",
        "GIT_REPLACE_REF_BASE",
        "GIT_SHALLOW_FILE",
        "GIT_WORK_TREE",
    ];

    fn sanitized_git(dir: &Path) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(dir);
        for var in GIT_LOCAL_ENV_VARS {
            cmd.env_remove(var);
        }
        cmd.env("GIT_AUTHOR_NAME", GIT_TEST_NAME);
        cmd.env("GIT_AUTHOR_EMAIL", GIT_TEST_EMAIL);
        cmd.env("GIT_COMMITTER_NAME", GIT_TEST_NAME);
        cmd.env("GIT_COMMITTER_EMAIL", GIT_TEST_EMAIL);
        cmd
    }

    fn git(dir: &Path, args: &[&str]) {
        let status = sanitized_git(dir).args(args).status().expect("git command");
        assert!(status.success(), "git {args:?} failed");
    }

    fn git_output(dir: &Path, args: &[&str]) -> String {
        let output = sanitized_git(dir).args(args).output().expect("git output");
        assert!(output.status.success(), "git {args:?} failed");
        String::from_utf8(output.stdout).expect("utf8")
    }

    fn git_init(dir: &Path) {
        git(dir, &["init", "--quiet"]);
        git(dir, &["config", "user.email", GIT_TEST_EMAIL]);
        git(dir, &["config", "user.name", GIT_TEST_NAME]);
    }

    fn write_file(root: &Path, rel: &str, content: &str) {
        let path = root.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("mkdirs");
        }
        fs::write(path, content).expect("write file");
    }

    fn commit_all(root: &Path, message: &str) -> String {
        git(root, &["add", "-A"]);
        git(root, &["commit", "--quiet", "-m", message]);
        git_output(root, &["rev-parse", "HEAD"]).trim().to_owned()
    }

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
        let source_url = format!("file://{}", source.path().display());
        let output = Command::new("git")
            .args(["clone", "--quiet", "--depth", "1", &source_url])
            .arg(clone.path())
            .output()
            .expect("git clone --depth 1");
        assert!(output.status.success(), "git clone failed: {output:?}");

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
}
