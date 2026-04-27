//! Commit metadata ingestion: read git metadata and persist to the store.

use std::path::Path;

use anyhow::Context;

use crate::error::Result;
use atlas_store_sqlite::{Store, StoredCommit};
use serde::Serialize;
use tracing::warn;

use crate::git::{self, GitCommitMeta};
use crate::select::CommitSelector;

/// Error wrapper for ingest operations that want to continue on partial
/// failures rather than aborting the whole run.
#[derive(Debug, Clone, Serialize)]
pub struct IngestError {
    pub commit_sha: Option<String>,
    pub message: String,
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(sha) = &self.commit_sha {
            write!(f, "ingest error for {sha}: {}", self.message)
        } else {
            write!(f, "ingest error: {}", self.message)
        }
    }
}

/// Summary of one `ingest_commits` run.
#[derive(Debug, Default)]
pub struct IngestSummary {
    pub commits_processed: usize,
    pub commits_already_indexed: usize,
    pub errors: Vec<IngestError>,
}

/// Resolve the selector and store commit metadata.
///
/// The caller provides the canonical `repo_root` (already canonicalized) and
/// an open `Store`.  Shallow-clone and detached-HEAD warnings are surfaced via
/// the returned summary rather than hard errors.
pub fn ingest_commits(
    repo: &Path,
    canonical_root: &str,
    store: &Store,
    selector: &CommitSelector,
    indexed_ref: Option<&str>,
) -> Result<IngestSummary> {
    let mut summary = IngestSummary::default();

    // Warn on shallow clone — missing history is a soft safeguard.
    match git::is_shallow(repo) {
        Ok(true) => {
            warn!("repo is a shallow clone; history may be incomplete");
        }
        Ok(false) => {}
        Err(e) => {
            warn!("could not determine shallow state: {e}");
        }
    }

    let commits = selector.resolve(repo).context("resolve commit selector")?;

    if commits.is_empty() {
        return Ok(summary);
    }

    let repo_id = store
        .upsert_repo(canonical_root)
        .context("upsert repo row")?;

    for meta in commits {
        summary.commits_processed += 1;
        if let Err(e) = ingest_one(store, repo_id, &meta, indexed_ref) {
            warn!("failed to ingest commit {}: {e}", meta.sha);
            summary.errors.push(IngestError {
                commit_sha: Some(meta.sha.clone()),
                message: format!("{e:#}"),
            });
        }
    }

    Ok(summary)
}

fn ingest_one(
    store: &Store,
    repo_id: i64,
    meta: &GitCommitMeta,
    indexed_ref: Option<&str>,
) -> Result<()> {
    let stored = StoredCommit {
        commit_sha: meta.sha.clone(),
        repo_id,
        parent_sha: meta.parent_sha.clone(),
        indexed_ref: indexed_ref.map(str::to_owned),
        author_name: Some(meta.author_name.clone()),
        author_email: Some(meta.author_email.clone()),
        author_time: meta.author_time,
        committer_time: meta.committer_time,
        subject: meta.subject.clone(),
        message: if meta.body.is_empty() {
            None
        } else {
            Some(format!("{}\n\n{}", meta.subject, meta.body))
        },
        indexed_at: String::new(), // filled by the store
    };
    store.upsert_commit(&stored).context("upsert commit")?;
    Ok(())
}
