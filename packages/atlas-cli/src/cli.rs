use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "atlas", about = "Atlas code graph CLI")]
pub struct Cli {
    /// Path to the repository root (default: auto-detect from cwd).
    #[arg(long, global = true)]
    pub repo: Option<String>,

    /// Path to the atlas database (default: <repo>/.atlas/worldview.sqlite).
    #[arg(long, global = true)]
    pub db: Option<String>,

    /// Enable verbose diagnostic output.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Emit machine-readable JSON output where supported.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Initialize the atlas work directory and database in the current repo.
    Init,

    /// Scan all tracked files and build the code graph from scratch.
    Build {
        /// Stop immediately on the first parse error instead of continuing.
        #[arg(long)]
        fail_fast: bool,
    },

    /// Incrementally update the graph for files changed since a base ref.
    Update {
        /// Git ref or range to diff against (e.g. `origin/main`).
        #[arg(long)]
        base: Option<String>,

        /// Diff staged changes only.
        #[arg(long)]
        staged: bool,

        /// Diff working-tree (unstaged) changes only.
        #[arg(long)]
        working_tree: bool,

        /// Explicit list of files to update.
        #[arg(long, num_args = 1..)]
        files: Vec<String>,

        /// Stop immediately on the first parse error instead of continuing.
        #[arg(long)]
        fail_fast: bool,
    },

    /// Show database status and graph statistics.
    Status,

    /// List files changed since a base ref.
    DetectChanges {
        /// Git ref or range to diff against.
        #[arg(long)]
        base: Option<String>,

        /// Diff staged changes only.
        #[arg(long)]
        staged: bool,
    },

    /// Search the code graph by keyword.
    Query {
        /// Search text.
        text: String,

        /// Filter by node kind (e.g. `function`, `struct`).
        #[arg(long)]
        kind: Option<String>,

        /// Filter by language.
        #[arg(long)]
        language: Option<String>,

        /// Filter by a file path prefix (subpath within the repo).
        #[arg(long)]
        subpath: Option<String>,

        /// Maximum results to return.
        #[arg(long, default_value_t = 20)]
        limit: usize,

        /// Expand results through graph edges (graph-aware search).
        #[arg(long)]
        expand: bool,

        /// Maximum edge hops when --expand is active.
        #[arg(long, default_value_t = 1)]
        expand_hops: u32,
    },

    /// Compute the impact radius of changed files.
    Impact {
        /// Git ref or range to diff against.
        #[arg(long)]
        base: Option<String>,

        /// Explicit list of files.
        #[arg(long, num_args = 1..)]
        files: Vec<String>,

        /// Maximum traversal depth.
        #[arg(long, default_value_t = 5)]
        max_depth: u32,

        /// Maximum number of impacted nodes to return.
        #[arg(long, default_value_t = 200)]
        max_nodes: u32,
    },

    /// Assemble review context for changed files.
    ReviewContext {
        /// Git ref or range to diff against.
        #[arg(long)]
        base: Option<String>,

        /// Explicit list of files.
        #[arg(long, num_args = 1..)]
        files: Vec<String>,
    },

    /// Start a JSON-RPC / MCP stdio server.
    Serve,

    /// Run an integrity check on the atlas database.
    DbCheck,
}
