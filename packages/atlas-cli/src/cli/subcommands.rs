use clap::Subcommand;

/// Sub-commands for `atlas session`.
#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Start (or re-register) the current session.
    Start,

    /// Show the current session status and recent event summary.
    Status,

    /// Print the pending resume snapshot, then mark it consumed.
    Resume,

    /// Delete the current session and all its stored events.
    Clear,

    /// List all known sessions for this repo.
    List,

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
}
