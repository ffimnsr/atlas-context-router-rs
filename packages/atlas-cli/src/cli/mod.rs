use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

mod subcommands;

#[cfg(test)]
mod tests;

pub use subcommands::{
    AnalyzeCommand, CommunitiesCommand, ConfigCommand, FlowsCommand, HistoryCommand,
    RefactorCommand, SessionCommand,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ReviewContextFormat {
    Text,
    Markdown,
}

#[derive(Debug, Parser)]
#[command(
    name = "atlas",
    about = "Atlas code graph CLI",
    version = env!("ATLAS_LONG_VERSION")
)]
pub struct Cli {
    /// Path to the repository root (default: auto-detect from cwd).
    #[arg(long, global = true)]
    pub repo: Option<String>,

    /// Path to the atlas database (default: `REPO_ROOT/.atlas/worldtree.db`).
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
    Init {
        /// Config template profile to write on first init.
        #[arg(long, default_value = "standard", value_parser = ["minimal", "standard", "full"])]
        profile: String,
    },

    /// Run repo-local database migrations without building graph state.
    Migrate,

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
    Status {
        /// Git ref or range to diff against for changed-file status.
        #[arg(long)]
        base: Option<String>,

        /// Diff staged changes only for changed-file status.
        #[arg(long)]
        staged: bool,
    },

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

        /// Include file nodes in the result set.
        #[arg(long)]
        include_files: bool,

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

        /// Enable fuzzy matching boost for near-miss symbol names.
        #[arg(long)]
        fuzzy: bool,

        /// Use hybrid FTS + vector search with RRF (requires ATLAS_EMBED_URL).
        #[arg(long)]
        hybrid: bool,

        /// Use semantic (graph-aware) retrieval: expands the result set via
        /// graph neighbours of the initial FTS hits before re-ranking.
        #[arg(long)]
        semantic: bool,

        /// Treat text as a regex pattern matched against name and qualified_name
        /// via SQL UDF. Bypasses FTS; uses structural scan with kind/language/subpath filters.
        /// Use `|` for alternation: `atlas query "handle|HANDLE" --regex`.
        #[arg(long)]
        regex: bool,
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

        /// Maximum traversal depth.
        #[arg(long, default_value_t = 3)]
        max_depth: u32,

        /// Maximum number of impacted nodes to consider.
        #[arg(long, default_value_t = 200)]
        max_nodes: u32,

        /// Output profile for human-readable review comments.
        #[arg(long, value_enum, default_value_t = ReviewContextFormat::Text)]
        format: ReviewContextFormat,
    },

    /// Generate and store embeddings for all un-embedded chunks.
    Embed {
        /// Maximum number of chunks to embed in one run.
        #[arg(long, default_value_t = 1000)]
        limit: usize,
    },

    /// Start a JSON-RPC / MCP stdio server.
    Serve,

    /// Internal repo-scoped MCP daemon entrypoint.
    #[command(name = "serve-daemon", hide = true)]
    ServeDaemon,

    /// Start an HTTP + Server-Sent Events MCP server.
    ///
    /// Binds to `ATLAS_HTTP_BIND` (default `127.0.0.1:7070`).
    /// Require a Bearer token with `ATLAS_HTTP_AUTH_TOKEN`.
    ///
    /// Requires the `http-transport` feature (build with
    /// `cargo build --features http-transport`).
    #[command(name = "serve-http")]
    ServeHttp,

    /// Run an integrity check on the atlas database (SQLite + orphan/dangling checks).
    DbCheck,

    /// Run a health check on the atlas setup (repo, config, database).
    Doctor,

    /// Purge repo-local context/session stores after noncanonical path identity drift.
    #[command(name = "purge-noncanonical")]
    PurgeNoncanonical,

    /// Show detailed graph structure for debugging.
    #[command(name = "debug-graph")]
    DebugGraph {
        /// Maximum number of orphan nodes and dangling edges to display.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Show resolved config values and their sources.
    #[command(name = "debug-config")]
    DebugConfig,

    /// Inspect Atlas config surfaces.
    Config {
        #[command(subcommand)]
        subcommand: ConfigCommand,
    },

    /// Show what a query would match, with timing and match details.
    #[command(name = "explain-query")]
    ExplainQuery {
        /// Search text.
        text: String,

        /// Filter by node kind.
        #[arg(long)]
        kind: Option<String>,

        /// Filter by language.
        #[arg(long)]
        language: Option<String>,

        /// Filter by file path prefix.
        #[arg(long)]
        subpath: Option<String>,

        /// Maximum results.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Summarize a change-set: changed symbols, impact, risk, and test gaps.
    #[command(name = "explain-change")]
    ExplainChange {
        /// Git ref or range to diff against.
        #[arg(long)]
        base: Option<String>,

        /// Diff staged changes only.
        #[arg(long)]
        staged: bool,

        /// Explicit list of files to explain.
        #[arg(long, num_args = 1..)]
        files: Vec<String>,

        /// Maximum traversal depth.
        #[arg(long, default_value_t = 5)]
        max_depth: u32,

        /// Maximum number of impacted nodes to consider.
        #[arg(long, default_value_t = 200)]
        max_nodes: u32,
    },

    /// Install MCP server configuration for AI coding platforms.
    Install {
        /// Target platform: copilot, claude, codex, or all (default: all detected).
        #[arg(long, default_value = "all")]
        platform: String,

        /// Install platform config and platform hook files into repo or user home.
        #[arg(long, default_value = "repo", value_parser = ["repo", "user"])]
        scope: String,

        /// Show what would be done without writing any files.
        #[arg(long)]
        dry_run: bool,

        /// Validate existing install targets without writing files.
        #[arg(long)]
        validate_only: bool,

        /// Overwrite existing non-Atlas git hooks.
        #[arg(long)]
        force: bool,

        /// Skip installing git hooks and platform agent hook configs.
        #[arg(long)]
        no_hooks: bool,

        /// Skip injecting platform-specific graph instructions.
        #[arg(long)]
        no_instructions: bool,
    },

    /// Explain supported update path for the installed atlas binary.
    Selfupdate,

    /// Print shell completion script to stdout.
    Completions {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Start interactive developer shell for natural-language graph queries.
    Shell {
        /// Enable fuzzy matching for `/query` lookups.
        #[arg(long)]
        fuzzy: bool,

        /// Pipe long results through pager when possible.
        #[arg(long)]
        paging: bool,
    },

    /// Watch the repository for file-system changes and update the graph in near real-time.
    ///
    /// Atlas watches the repo tree, debounces rapid edits, and incrementally
    /// updates the graph without running a full rebuild.
    Watch {
        /// Debounce window in milliseconds: collect events for this long
        /// after the first change before processing (default: 200).
        #[arg(long, default_value_t = 200)]
        debounce_ms: u64,

        /// Emit machine-readable JSON lines to stdout (one object per batch).
        /// Mirrors the global --json flag but specific to watch output.
        #[arg(long)]
        json: bool,
    },

    /// Build context around a symbol, file, or change-set using the context engine.
    ///
    /// TARGET accepts: a symbol name, qualified name, free-text query, or file
    /// path.  Alternatively, supply --file or --files for explicit targets.
    /// Examples:
    ///   atlas context "AuthService"
    ///   atlas context "who calls handle_request"
    ///   atlas context --file src/auth.rs
    ///   atlas context --files src/auth.rs src/session.rs --intent review
    #[command(name = "context")]
    Context {
        /// Symbol name, qualified name, or free-text query (auto-classified).
        /// Omit when using --file or --files.
        #[arg(value_name = "TARGET")]
        query: Option<String>,

        /// Explicit repo-relative file path target (file intent).
        #[arg(long)]
        file: Option<String>,

        /// Changed file paths for review/impact context.
        #[arg(long, num_args = 1..)]
        files: Vec<String>,

        /// Override intent: symbol, file, review, impact, usage_lookup,
        /// refactor_safety, dead_code_check, rename_preview, dependency_removal.
        /// Inferred automatically when omitted.
        #[arg(long)]
        intent: Option<String>,

        /// Maximum nodes to include (default: 100).
        #[arg(long)]
        max_nodes: Option<usize>,

        /// Maximum edges to include (default: 100).
        #[arg(long)]
        max_edges: Option<usize>,

        /// Maximum files to include (default: 20).
        #[arg(long)]
        max_files: Option<usize>,

        /// Traversal depth in graph hops (default: 2).
        #[arg(long)]
        depth: Option<u32>,

        /// Include source line ranges in result.
        #[arg(long)]
        code_spans: bool,

        /// Include test-adjacency nodes.
        #[arg(long)]
        tests: bool,

        /// Include import edges and nodes.
        #[arg(long)]
        imports: bool,

        /// Include containment-sibling nodes.
        #[arg(long)]
        neighbors: bool,

        /// Route the query through graph-aware semantic expansion before
        /// building context.  When a session is active, prior-context files
        /// and symbols from recent events are used to boost relevance.
        #[arg(long)]
        semantic: bool,

        /// Restrict session-aware context enrichment to one agent partition.
        #[arg(long)]
        agent_id: Option<String>,

        /// Intentionally merge context across all agent partitions.
        #[arg(long)]
        merge_agent_partitions: bool,

        /// Maximum tokens to include in the result.  Overrides the default
        /// policy limit (context_tokens_estimate) for this request only.
        /// The engine enforces this as a strict cap; it cannot exceed the
        /// policy ceiling set in atlas.toml.
        #[arg(long)]
        token_budget: Option<usize>,
    },

    /// Manage named ordered sequences of graph nodes (flows).
    Flows {
        #[command(subcommand)]
        subcommand: FlowsCommand,
    },

    /// Manage graph node communities (clusters / partitions).
    Communities {
        #[command(subcommand)]
        subcommand: CommunitiesCommand,
    },

    /// Refresh derived graph analytics without reparsing source files.
    #[command(name = "postprocess")]
    Postprocess {
        /// Restrict refresh to files currently changed in the working tree.
        #[arg(long)]
        changed_only: bool,

        /// Run only one postprocess stage.
        #[arg(long)]
        stage: Option<String>,

        /// Compute the summary without recording lifecycle state.
        #[arg(long)]
        dry_run: bool,
    },

    /// Analyse a symbol or the whole graph for removal impact, dead code, safety, or dependencies.
    Analyze {
        #[command(subcommand)]
        subcommand: AnalyzeCommand,
    },

    /// Plan or apply deterministic refactoring operations.
    Refactor {
        #[command(subcommand)]
        subcommand: RefactorCommand,
    },

    /// Manage Atlas context memory sessions (start, status, resume, clear, list).
    Session {
        #[command(subcommand)]
        subcommand: SessionCommand,
    },

    /// Internal hook entrypoint for generated agent hook runners.
    #[command(hide = true)]
    Hook {
        /// Normalized hook event name (for example `session-start`).
        event: String,
    },

    /// Query, build, and inspect historical graph snapshots per git commit.
    History {
        #[command(subcommand)]
        subcommand: HistoryCommand,
    },

    /// Print CLI version, build commit hash, and build date.
    Version,
}
