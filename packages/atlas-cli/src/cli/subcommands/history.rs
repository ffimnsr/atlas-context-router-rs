use super::*;

/// Sub-commands for `atlas history`.
#[derive(Debug, Subcommand)]
pub enum HistoryCommand {
    /// Show historical indexing status: commit count, snapshot count, latest
    /// indexed commit, and any shallow-clone or missing-ref warnings.
    Status,

    /// Build historical graph snapshots for a bounded commit range.
    ///
    /// Parses each commit's tracked files checkout-free (via `git show`),
    /// reuses parsed graphs for unchanged blobs, and stores snapshot
    /// membership.  Prints a summary of commits, files, and nodes processed.
    Build {
        /// Only include commits after this date or commit SHA (e.g.
        /// `2024-01-01` or a 40-char SHA).
        #[arg(long)]
        since: Option<String>,

        /// Only include commits before this date or commit SHA.
        #[arg(long)]
        until: Option<String>,

        /// Maximum number of commits to index in one run.
        #[arg(long)]
        max_commits: Option<usize>,

        /// Branch or ref to walk (defaults to `HEAD`).
        #[arg(long)]
        branch: Option<String>,

        /// Explicit comma-separated list of full 40-char commit SHAs to
        /// index instead of walking the branch history.
        #[arg(long, value_delimiter = ',')]
        commits: Vec<String>,

        /// Skip interactive confirmation after preflight estimate.
        #[arg(long)]
        yes: bool,
    },

    /// Append missing commits from current branch ancestry without rebuilding existing snapshots.
    Update {
        /// Branch or ref to walk (defaults to `HEAD`).
        #[arg(long)]
        branch: Option<String>,

        /// Allow update after detecting rewritten history / force-push divergence.
        #[arg(long)]
        repair: bool,

        /// Maximum number of commits to inspect while searching for missing history.
        #[arg(long)]
        max_commits: Option<usize>,

        /// Skip interactive confirmation after preflight estimate.
        #[arg(long)]
        yes: bool,
    },

    /// Rebuild one indexed historical snapshot for an exact commit SHA.
    #[command(visible_alias = "reindex")]
    Rebuild {
        /// Exact 40-char commit SHA to rebuild.
        commit_sha: String,
    },

    /// Diff two indexed historical snapshots.
    Diff {
        /// Older or left-side commit SHA.
        commit_a: String,

        /// Newer or right-side commit SHA.
        commit_b: String,

        /// Emit only compact summary counts.
        #[arg(long, conflicts_with = "full")]
        stat_only: bool,

        /// Emit expanded human-readable details.
        #[arg(long, conflicts_with = "stat_only")]
        full: bool,
    },

    /// Query history for one fully-qualified symbol.
    ///
    /// Qualified-name changes are reported as remove + add. Atlas does not
    /// infer rename continuity for symbols in the first pass.
    Symbol {
        /// Fully-qualified symbol name.
        qualified_name: String,

        /// Emit only compact summary fields.
        #[arg(long, conflicts_with = "full")]
        stat_only: bool,

        /// Emit expanded human-readable details.
        #[arg(long, conflicts_with = "stat_only")]
        full: bool,
    },

    /// Query history for one canonical repo-relative file path.
    File {
        /// Repo-relative file path.
        path: String,

        /// Follow git-detected renames across indexed snapshots.
        #[arg(long)]
        follow_renames: bool,

        /// Emit only compact summary fields.
        #[arg(long, conflicts_with = "full")]
        stat_only: bool,

        /// Emit expanded human-readable details.
        #[arg(long, conflicts_with = "stat_only")]
        full: bool,
    },

    /// Query history for one dependency edge.
    Dependency {
        /// Fully-qualified source symbol name.
        source: String,

        /// Fully-qualified target symbol name.
        target: String,

        /// Emit only compact summary fields.
        #[arg(long, conflicts_with = "full")]
        stat_only: bool,

        /// Emit expanded human-readable details.
        #[arg(long, conflicts_with = "stat_only")]
        full: bool,
    },

    /// Query growth and coupling trends for one module bucket.
    Module {
        /// Module bucket, for example `packages/atlas-cli` or `src/lib.rs`.
        module: String,

        /// Emit only compact summary fields.
        #[arg(long, conflicts_with = "full")]
        stat_only: bool,

        /// Emit expanded human-readable details.
        #[arg(long, conflicts_with = "stat_only")]
        full: bool,
    },

    /// Compute churn, stability, trend, and storage diagnostics across indexed history.
    Churn {
        /// Emit only compact summary fields.
        #[arg(long, conflicts_with = "full")]
        stat_only: bool,

        /// Emit expanded human-readable details.
        #[arg(long, conflicts_with = "stat_only")]
        full: bool,
    },

    /// Prune indexed history snapshots using one or more retention policies.
    Prune {
        /// Keep all indexed commits and snapshots.
        #[arg(long)]
        keep_all: bool,

        /// Keep latest N indexed commits.
        #[arg(long)]
        keep_latest: Option<usize>,

        /// Keep commits newer than N days from now.
        #[arg(long)]
        older_than_days: Option<u64>,

        /// Keep commits that still have a reachable git tag.
        #[arg(long)]
        keep_tagged_only: bool,

        /// Keep latest indexed snapshot from each calendar week bucket.
        #[arg(long)]
        keep_weekly: bool,
    },
}
