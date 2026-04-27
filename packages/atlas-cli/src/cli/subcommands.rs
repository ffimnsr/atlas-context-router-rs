use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show resolved config values and where they came from.
    Show,
}

/// Sub-commands for `atlas session`.
#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Start (or re-register) the current session.
    Start,

    /// Show the current session status and recent event summary.
    Status {
        #[arg(long)]
        agent_id: Option<String>,

        #[arg(long)]
        merge_agent_partitions: bool,
    },

    /// Print the pending resume snapshot, then mark it consumed.
    Resume {
        #[arg(long)]
        agent_id: Option<String>,

        #[arg(long)]
        merge_agent_partitions: bool,
    },

    /// Delete the current session and all its stored events.
    Clear,

    /// List all known sessions for this repo.
    List,

    /// Search persisted decision memory for prior conclusions.
    Decisions {
        /// Search query text.
        query: String,

        /// Restrict lookup to current CLI session only.
        #[arg(long)]
        current_session: bool,

        /// Maximum decisions to return.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },

    /// Compact and curate the session event ledger.
    ///
    /// Removes stale low-value events, merges repeated actions, deduplicates
    /// reasoning outputs, and promotes high-value events to a higher priority
    /// so they survive future eviction cycles.
    Compact,
}

/// Sub-commands for `atlas flows`.
#[derive(Debug, Subcommand)]
pub enum FlowsCommand {
    /// List all flows.
    List,

    /// Create a new flow.
    Create {
        /// Name for the new flow.
        name: String,

        /// Optional kind label (e.g. `auth`, `data-pipeline`).
        #[arg(long)]
        kind: Option<String>,

        /// Optional human-readable description.
        #[arg(long)]
        description: Option<String>,
    },

    /// Delete a flow by name.
    Delete {
        /// Name of the flow to delete.
        name: String,
    },

    /// List members of a flow.
    Members {
        /// Name of the flow.
        name: String,
    },

    /// Add a node to a flow.
    #[command(name = "add-member")]
    AddMember {
        /// Name of the flow.
        flow: String,

        /// Qualified name of the node to add.
        node_qn: String,

        /// Ordering position in the flow (optional).
        #[arg(long)]
        position: Option<i64>,

        /// Optional role label for the node in this flow.
        #[arg(long)]
        role: Option<String>,
    },

    /// Remove a node from a flow.
    #[command(name = "remove-member")]
    RemoveMember {
        /// Name of the flow.
        flow: String,

        /// Qualified name of the node to remove.
        node_qn: String,
    },

    /// List flows that contain a given node.
    #[command(name = "for-node")]
    ForNode {
        /// Qualified name of the node.
        node_qn: String,
    },
}

/// Sub-commands for `atlas communities`.
#[derive(Debug, Subcommand)]
pub enum CommunitiesCommand {
    /// List all communities.
    List,

    /// Create a new community.
    Create {
        /// Name for the new community.
        name: String,

        /// Clustering algorithm used (e.g. `louvain`, `label-propagation`).
        #[arg(long)]
        algorithm: Option<String>,

        /// Nesting level (0 = top-level).
        #[arg(long)]
        level: Option<i64>,

        /// Id of the parent community for hierarchical clustering.
        #[arg(long)]
        parent: Option<i64>,
    },

    /// Delete a community by name.
    Delete {
        /// Name of the community to delete.
        name: String,
    },

    /// List node members of a community.
    Nodes {
        /// Name of the community.
        name: String,
    },

    /// Add a node to a community.
    #[command(name = "add-node")]
    AddNode {
        /// Name of the community.
        community: String,

        /// Qualified name of the node to add.
        node_qn: String,
    },

    /// Remove a node from a community.
    #[command(name = "remove-node")]
    RemoveNode {
        /// Name of the community.
        community: String,

        /// Qualified name of the node to remove.
        node_qn: String,
    },

    /// List communities that contain a given node.
    #[command(name = "for-node")]
    ForNode {
        /// Qualified name of the node.
        node_qn: String,
    },
}

/// Sub-commands for `atlas analyze`.
#[derive(Debug, Subcommand)]
pub enum AnalyzeCommand {
    /// Show the blast radius of removing a symbol from the codebase.
    Remove {
        /// Fully-qualified name of the symbol to analyse.
        symbol: String,

        /// Maximum BFS traversal depth.
        #[arg(long, default_value_t = 3)]
        max_depth: u32,

        /// Maximum number of impacted nodes.
        #[arg(long, default_value_t = 200)]
        max_nodes: usize,
    },

    /// List dead-code candidates in the graph.
    #[command(name = "dead-code")]
    DeadCode {
        /// Additional qualified names to suppress (treat as live).
        #[arg(long, num_args = 1..)]
        allowlist: Vec<String>,

        /// Restrict candidates to files under this repo-relative path prefix.
        #[arg(long)]
        subpath: Option<String>,

        /// Maximum number of candidates to return.
        #[arg(long, default_value_t = 100)]
        limit: usize,

        /// Output only counts, not the full candidate list.
        #[arg(long)]
        summary: bool,

        /// Exclude candidates of these kinds (e.g. `constant`, `variable`).
        #[arg(long = "exclude-kind", num_args = 1..)]
        exclude_kind: Vec<String>,

        /// Restrict output to code symbols only (functions, methods,
        /// structs/types, traits, enums, interfaces, constants, variables).
        /// This is the default; provided for explicitness.
        #[arg(long, default_value_t = true)]
        code_only: bool,

        /// Maximum number of impacted files to show per candidate.
        #[arg(long)]
        max_files: Option<usize>,

        /// Maximum number of edges to show per candidate.
        #[arg(long)]
        max_edges: Option<usize>,
    },

    /// Score refactor safety for a symbol.
    Safety {
        /// Fully-qualified name of the symbol to score.
        symbol: String,
    },

    /// Check whether a symbol or import dependency can be removed.
    Dependency {
        /// Fully-qualified name of the symbol or import to check.
        symbol: String,
    },
}

/// Sub-commands for `atlas refactor`.
#[derive(Debug, Subcommand)]
pub enum RefactorCommand {
    /// Rename a symbol across all reference sites.
    Rename {
        /// Fully-qualified name of the symbol to rename.
        #[arg(long)]
        symbol: Option<String>,

        /// New simple name for the symbol.
        #[arg(long = "to")]
        to: Option<String>,

        /// Legacy positional symbol argument.
        #[arg(hide = true)]
        legacy_symbol: Option<String>,

        /// Legacy positional new-name argument.
        #[arg(hide = true)]
        legacy_to: Option<String>,

        /// Preview edits without writing any files.
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove a dead-code symbol from the codebase.
    #[command(name = "remove-dead")]
    RemoveDead {
        /// Fully-qualified name of the dead-code symbol to remove.
        symbol: String,

        /// Preview edits without writing any files.
        #[arg(long)]
        dry_run: bool,
    },

    /// Remove unused imports from a source file.
    #[command(name = "clean-imports")]
    CleanImports {
        /// Repo-relative path to the source file.
        file: String,

        /// Preview edits without writing any files.
        #[arg(long)]
        dry_run: bool,
    },
}

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
