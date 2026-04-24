//! Commit selection strategies.
//!
//! A `CommitSelector` describes which commits to index.  Call `.resolve()`
//! with a repo root to expand it into an ordered list of
//! [`crate::git::GitCommitMeta`] records.

use std::path::Path;

use anyhow::{Result, bail};

use crate::git::{self, GitCommitMeta};

/// How to select commits for historical indexing.
#[derive(Debug, Clone)]
pub enum CommitSelector {
    /// Only the single most-recent commit on `start_ref`.
    Latest { start_ref: String },

    /// Up to `max_commits` commits reachable from `start_ref`,
    /// with optional ISO-date `since` / `until` filters.
    Bounded {
        start_ref: String,
        max_commits: Option<usize>,
        since: Option<String>,
        until: Option<String>,
    },

    /// An explicit ordered list of 40-char SHA strings.
    Explicit { shas: Vec<String> },

    /// A git revision range like `"abc123..HEAD"` or `"v1.0..v2.0"`.
    Range { range: String },
}

impl CommitSelector {
    /// Resolve the selector to an ordered list of commit metadata.
    ///
    /// "Ordered" is most-recent-first, matching `git log` defaults, except
    /// for [`CommitSelector::Explicit`] which preserves caller order.
    pub fn resolve(&self, repo: &Path) -> Result<Vec<GitCommitMeta>> {
        match self {
            CommitSelector::Latest { start_ref } => {
                let sha = git::rev_parse(repo, start_ref)
                    .map_err(|e| anyhow::anyhow!("ref not found {:?}: {e}", start_ref))?;
                git::log_commits(repo, &sha, Some(1), None, None)
            }

            CommitSelector::Bounded {
                start_ref,
                max_commits,
                since,
                until,
            } => {
                let sha = git::rev_parse(repo, start_ref)
                    .map_err(|e| anyhow::anyhow!("ref not found {:?}: {e}", start_ref))?;
                git::log_commits(repo, &sha, *max_commits, since.as_deref(), until.as_deref())
            }

            CommitSelector::Explicit { shas } => {
                if shas.is_empty() {
                    return Ok(vec![]);
                }
                for sha in shas {
                    git::validate_sha(sha)
                        .map_err(|e| anyhow::anyhow!("invalid SHA in explicit list: {e}"))?;
                }
                git::log_commits_explicit(repo, shas)
            }

            CommitSelector::Range { range } => {
                // Reject ranges that look like shell injection attempts.
                if range
                    .chars()
                    .any(|c| matches!(c, '&' | '|' | ';' | '$' | '`' | '\n' | '\r'))
                {
                    bail!("unsafe characters in commit range: {:?}", range);
                }
                git::log_commits(repo, range, None, None, None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn range_rejects_shell_injection() {
        let sel = CommitSelector::Range {
            range: "main && rm -rf /".into(),
        };
        let tmp = std::env::temp_dir();
        // Even if the dir exists the validation fires before the git call.
        let err = sel.resolve(&tmp).unwrap_err();
        assert!(err.to_string().contains("unsafe characters"));
    }

    #[test]
    fn explicit_rejects_invalid_sha() {
        let sel = CommitSelector::Explicit {
            shas: vec!["not-a-sha".into()],
        };
        let tmp = std::env::temp_dir();
        let err = sel.resolve(&tmp).unwrap_err();
        assert!(err.to_string().contains("invalid SHA"));
    }
}
