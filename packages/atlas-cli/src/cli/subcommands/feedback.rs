use super::*;

/// Sub-commands for `atlas feedback` (ICM-C).
#[derive(Debug, Subcommand)]
pub enum FeedbackCommand {
    /// Record a correction when an analysis prediction was wrong.
    Record {
        /// What the analysis predicted; required.
        #[arg(long)]
        predicted: String,

        /// What actually happened; required. Differs from --predicted to mark
        /// this record as false-positive evidence for confidence adjustment.
        #[arg(long)]
        actual: String,

        /// Optional correction text explaining the right answer.
        #[arg(long)]
        correction: Option<String>,

        /// Tool that produced the prediction (e.g. cli, mcp).
        #[arg(long)]
        tool: Option<String>,

        /// Analysis kind: dead_code, remove, safety, or remove_dead.
        #[arg(long)]
        analysis_kind: Option<String>,

        /// Exact qualified name of the symbol the prediction was about.
        #[arg(long)]
        symbol: Option<String>,

        /// Repo-relative path of the file the prediction was about.
        #[arg(long)]
        file: Option<String>,

        /// Source id linking this record to a saved-context artifact.
        #[arg(long)]
        source_id: Option<String>,
    },

    /// Search past corrections to inform future predictions.
    Search {
        /// Text matched against predicted, actual, correction, symbol, and file.
        query: String,

        /// Restrict to one tool.
        #[arg(long)]
        tool: Option<String>,

        /// Restrict to one analysis kind.
        #[arg(long)]
        analysis_kind: Option<String>,

        /// Restrict to one exact related symbol.
        #[arg(long)]
        symbol: Option<String>,

        /// Restrict to one exact related file.
        #[arg(long)]
        file: Option<String>,

        /// Maximum records to return.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Deterministic feedback statistics.
    Stats,

    /// Report feedback-driven confidence adjustment weights (idempotent).
    Adjust,
}

/// Sub-commands for `atlas flows`.
#[derive(Debug, Subcommand)]
pub enum FlowsCommand {
    /// List all flows.
    List,

    /// Create a new flow.
    Create {
        /// Name for the new flow (positional or `--name`).
        #[arg(value_name = "NAME")]
        name: Option<String>,

        /// Name for the new flow (flag form).
        #[arg(long = "name")]
        name_flag: Option<String>,

        /// Optional kind label (e.g. `auth`, `data-pipeline`).
        #[arg(long)]
        kind: Option<String>,

        /// Optional human-readable description.
        #[arg(long)]
        description: Option<String>,
    },

    /// Trace a call path through a flow or list its ordered members.
    Trace {
        /// Name of the flow.
        name: String,

        /// Optional start symbol qualified name.
        #[arg(long)]
        from: Option<String>,

        /// Optional target symbol qualified name.
        #[arg(long)]
        to: Option<String>,

        /// Maximum traversal depth.
        #[arg(long, default_value_t = 5)]
        depth: u32,
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

    /// Detect communities with deterministic Louvain modularity clustering.
    Detect {
        /// Minimum members required to persist a detected community.
        #[arg(long, default_value_t = 2)]
        min_members: usize,

        /// Compute the partition without writing communities.
        #[arg(long)]
        dry_run: bool,
    },

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
