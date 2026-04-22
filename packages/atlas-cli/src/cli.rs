use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Debug, Parser)]
#[command(name = "atlas", about = "Atlas code graph CLI")]
pub struct Cli {
    /// Path to the repository root (default: auto-detect from cwd).
    #[arg(long, global = true)]
    pub repo: Option<String>,

    /// Path to the atlas database (default: <repo>/.atlas/worldtree.db).
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
    },

    /// Generate and store embeddings for all un-embedded chunks.
    Embed {
        /// Maximum number of chunks to embed in one run.
        #[arg(long, default_value_t = 1000)]
        limit: usize,
    },

    /// Start a JSON-RPC / MCP stdio server.
    Serve,

    /// Run an integrity check on the atlas database (SQLite + orphan/dangling checks).
    DbCheck,

    /// Run a health check on the atlas setup (repo, config, database).
    Doctor,

    /// Show detailed graph structure for debugging.
    #[command(name = "debug-graph")]
    DebugGraph {
        /// Maximum number of orphan nodes and dangling edges to display.
        #[arg(long, default_value_t = 20)]
        limit: usize,
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

        /// Show what would be done without writing any files.
        #[arg(long)]
        dry_run: bool,

        /// Skip installing git hooks.
        #[arg(long)]
        no_hooks: bool,

        /// Skip injecting platform-specific graph instructions.
        #[arg(long)]
        no_instructions: bool,
    },

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

    /// Print the CLI version and build commit hash.
    Version,
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).expect("parse should succeed")
    }

    // -------------------------------------------------------------------------
    // Global flags
    // -------------------------------------------------------------------------

    #[test]
    fn global_verbose_and_json_flags() {
        let cli = parse(&["atlas", "--verbose", "--json", "status"]);
        assert!(cli.verbose);
        assert!(cli.json);
        assert!(matches!(cli.command, Command::Status { .. }));
    }

    #[test]
    fn global_repo_and_db_flags() {
        let cli = parse(&[
            "atlas",
            "--repo",
            "/tmp/proj",
            "--db",
            "/tmp/w.sqlite",
            "status",
        ]);
        assert_eq!(cli.repo.as_deref(), Some("/tmp/proj"));
        assert_eq!(cli.db.as_deref(), Some("/tmp/w.sqlite"));
    }

    #[test]
    fn defaults_are_none_and_false() {
        let cli = parse(&["atlas", "init"]);
        assert!(cli.repo.is_none());
        assert!(cli.db.is_none());
        assert!(!cli.verbose);
        assert!(!cli.json);
    }

    // -------------------------------------------------------------------------
    // init / build / serve / db-check
    // -------------------------------------------------------------------------

    #[test]
    fn parse_init_command() {
        let cli = parse(&["atlas", "init"]);
        assert!(matches!(cli.command, Command::Init));
    }

    #[test]
    fn parse_build_command_no_flags() {
        let cli = parse(&["atlas", "build"]);
        assert!(matches!(cli.command, Command::Build { fail_fast: false }));
    }

    #[test]
    fn parse_build_fail_fast() {
        let cli = parse(&["atlas", "build", "--fail-fast"]);
        assert!(matches!(cli.command, Command::Build { fail_fast: true }));
    }

    #[test]
    fn parse_serve_command() {
        let cli = parse(&["atlas", "serve"]);
        assert!(matches!(cli.command, Command::Serve));
    }

    #[test]
    fn parse_db_check_command() {
        let cli = parse(&["atlas", "db-check"]);
        assert!(matches!(cli.command, Command::DbCheck));
    }

    #[test]
    fn parse_doctor_command() {
        let cli = parse(&["atlas", "doctor"]);
        assert!(matches!(cli.command, Command::Doctor));
    }

    // -------------------------------------------------------------------------
    // update
    // -------------------------------------------------------------------------

    #[test]
    fn parse_update_with_base_ref() {
        let cli = parse(&["atlas", "update", "--base", "origin/main"]);
        if let Command::Update {
            base,
            staged,
            working_tree,
            files,
            fail_fast,
        } = cli.command
        {
            assert_eq!(base.as_deref(), Some("origin/main"));
            assert!(!staged);
            assert!(!working_tree);
            assert!(files.is_empty());
            assert!(!fail_fast);
        } else {
            panic!("expected Update command");
        }
    }

    #[test]
    fn parse_update_staged() {
        let cli = parse(&["atlas", "update", "--staged"]);
        if let Command::Update { staged, .. } = cli.command {
            assert!(staged);
        } else {
            panic!("expected Update command");
        }
    }

    #[test]
    fn parse_update_explicit_files() {
        let cli = parse(&["atlas", "update", "--files", "src/a.rs", "src/b.rs"]);
        if let Command::Update { files, .. } = cli.command {
            assert_eq!(files, vec!["src/a.rs", "src/b.rs"]);
        } else {
            panic!("expected Update command");
        }
    }

    // -------------------------------------------------------------------------
    // query
    // -------------------------------------------------------------------------

    #[test]
    fn parse_query_text_only() {
        let cli = parse(&["atlas", "query", "ReplaceFileGraph"]);
        if let Command::Query {
            text,
            kind,
            language,
            include_files,
            limit,
            ..
        } = cli.command
        {
            assert_eq!(text, "ReplaceFileGraph");
            assert!(kind.is_none());
            assert!(language.is_none());
            assert!(!include_files);
            assert_eq!(limit, 20);
        } else {
            panic!("expected Query command");
        }
    }

    #[test]
    fn parse_query_with_kind_and_language_filters() {
        let cli = parse(&[
            "atlas",
            "query",
            "foo",
            "--kind",
            "function",
            "--language",
            "rust",
        ]);
        if let Command::Query {
            text,
            kind,
            language,
            ..
        } = cli.command
        {
            assert_eq!(text, "foo");
            assert_eq!(kind.as_deref(), Some("function"));
            assert_eq!(language.as_deref(), Some("rust"));
        } else {
            panic!("expected Query command");
        }
    }

    #[test]
    fn parse_query_expand_flag() {
        let cli = parse(&["atlas", "query", "foo", "--expand", "--expand-hops", "3"]);
        if let Command::Query {
            expand,
            expand_hops,
            fuzzy,
            ..
        } = cli.command
        {
            assert!(expand);
            assert_eq!(expand_hops, 3);
            assert!(!fuzzy);
        } else {
            panic!("expected Query command");
        }
    }

    #[test]
    fn parse_query_fuzzy_flag() {
        let cli = parse(&["atlas", "query", "greter", "--fuzzy"]);
        if let Command::Query { fuzzy, .. } = cli.command {
            assert!(fuzzy);
        } else {
            panic!("expected Query command");
        }
    }

    #[test]
    fn parse_query_include_files_flag() {
        let cli = parse(&["atlas", "query", "guide", "--include-files"]);
        if let Command::Query { include_files, .. } = cli.command {
            assert!(include_files);
        } else {
            panic!("expected Query command");
        }
    }

    // -------------------------------------------------------------------------
    // impact
    // -------------------------------------------------------------------------

    #[test]
    fn parse_impact_defaults() {
        let cli = parse(&["atlas", "impact"]);
        if let Command::Impact {
            max_depth,
            max_nodes,
            ..
        } = cli.command
        {
            assert_eq!(max_depth, 5);
            assert_eq!(max_nodes, 200);
        } else {
            panic!("expected Impact command");
        }
    }

    #[test]
    fn parse_impact_with_files() {
        let cli = parse(&["atlas", "impact", "--files", "a.rs", "b.rs"]);
        if let Command::Impact { files, .. } = cli.command {
            assert_eq!(files, vec!["a.rs", "b.rs"]);
        } else {
            panic!("expected Impact command");
        }
    }

    #[test]
    fn parse_impact_with_depth_and_nodes() {
        let cli = parse(&["atlas", "impact", "--max-depth", "3", "--max-nodes", "50"]);
        if let Command::Impact {
            max_depth,
            max_nodes,
            ..
        } = cli.command
        {
            assert_eq!(max_depth, 3);
            assert_eq!(max_nodes, 50);
        } else {
            panic!("expected Impact command");
        }
    }

    // -------------------------------------------------------------------------
    // review-context
    // -------------------------------------------------------------------------

    #[test]
    fn parse_review_context_defaults() {
        let cli = parse(&["atlas", "review-context"]);
        if let Command::ReviewContext {
            max_depth,
            max_nodes,
            base,
            files,
        } = cli.command
        {
            assert_eq!(max_depth, 3);
            assert_eq!(max_nodes, 200);
            assert!(base.is_none());
            assert!(files.is_empty());
        } else {
            panic!("expected ReviewContext command");
        }
    }

    // -------------------------------------------------------------------------
    // detect-changes
    // -------------------------------------------------------------------------

    #[test]
    fn parse_detect_changes_with_base() {
        let cli = parse(&["atlas", "detect-changes", "--base", "origin/main"]);
        if let Command::DetectChanges { base, staged } = cli.command {
            assert_eq!(base.as_deref(), Some("origin/main"));
            assert!(!staged);
        } else {
            panic!("expected DetectChanges command");
        }
    }

    #[test]
    fn parse_detect_changes_staged() {
        let cli = parse(&["atlas", "detect-changes", "--staged"]);
        if let Command::DetectChanges { staged, .. } = cli.command {
            assert!(staged);
        } else {
            panic!("expected DetectChanges command");
        }
    }

    #[test]
    fn parse_explain_change_with_base() {
        let cli = parse(&["atlas", "explain-change", "--base", "origin/main"]);
        if let Command::ExplainChange {
            base,
            staged,
            files,
            max_depth,
            max_nodes,
        } = cli.command
        {
            assert_eq!(base.as_deref(), Some("origin/main"));
            assert!(!staged);
            assert!(files.is_empty());
            assert_eq!(max_depth, 5);
            assert_eq!(max_nodes, 200);
        } else {
            panic!("expected ExplainChange command");
        }
    }

    // -------------------------------------------------------------------------
    // unknown / missing required args
    // -------------------------------------------------------------------------

    #[test]
    fn unknown_subcommand_fails() {
        assert!(Cli::try_parse_from(["atlas", "foobar"]).is_err());
    }

    #[test]
    fn query_missing_text_arg_fails() {
        assert!(Cli::try_parse_from(["atlas", "query"]).is_err());
    }

    // -------------------------------------------------------------------------
    // install
    // -------------------------------------------------------------------------

    #[test]
    fn parse_install_defaults() {
        let cli = parse(&["atlas", "install"]);
        if let Command::Install {
            platform,
            dry_run,
            no_hooks,
            no_instructions,
        } = cli.command
        {
            assert_eq!(platform, "all");
            assert!(!dry_run);
            assert!(!no_hooks);
            assert!(!no_instructions);
        } else {
            panic!("expected Install command");
        }
    }

    #[test]
    fn parse_install_platform_claude() {
        let cli = parse(&["atlas", "install", "--platform", "claude"]);
        if let Command::Install { platform, .. } = cli.command {
            assert_eq!(platform, "claude");
        } else {
            panic!("expected Install command");
        }
    }

    #[test]
    fn parse_install_dry_run() {
        let cli = parse(&["atlas", "install", "--dry-run"]);
        if let Command::Install { dry_run, .. } = cli.command {
            assert!(dry_run);
        } else {
            panic!("expected Install command");
        }
    }

    #[test]
    fn parse_install_no_hooks_and_no_instructions() {
        let cli = parse(&["atlas", "install", "--no-hooks", "--no-instructions"]);
        if let Command::Install {
            no_hooks,
            no_instructions,
            ..
        } = cli.command
        {
            assert!(no_hooks);
            assert!(no_instructions);
        } else {
            panic!("expected Install command");
        }
    }

    // -------------------------------------------------------------------------
    // completions
    // -------------------------------------------------------------------------

    #[test]
    fn parse_completions_bash() {
        let cli = parse(&["atlas", "completions", "bash"]);
        assert!(matches!(
            cli.command,
            Command::Completions {
                shell: clap_complete::Shell::Bash
            }
        ));
    }

    #[test]
    fn parse_completions_zsh() {
        let cli = parse(&["atlas", "completions", "zsh"]);
        assert!(matches!(
            cli.command,
            Command::Completions {
                shell: clap_complete::Shell::Zsh
            }
        ));
    }

    #[test]
    fn completions_missing_shell_fails() {
        assert!(Cli::try_parse_from(["atlas", "completions"]).is_err());
    }

    #[test]
    fn parse_shell_with_flags() {
        let cli = parse(&["atlas", "shell", "--fuzzy", "--paging"]);
        assert!(matches!(
            cli.command,
            Command::Shell {
                fuzzy: true,
                paging: true
            }
        ));
    }

    #[test]
    fn parse_analyze_dead_code_with_subpath() {
        let cli = parse(&["atlas", "analyze", "dead-code", "--subpath", "src"]);
        if let Command::Analyze { subcommand } = cli.command {
            match subcommand {
                AnalyzeCommand::DeadCode { subpath, limit, .. } => {
                    assert_eq!(subpath.as_deref(), Some("src"));
                    assert_eq!(limit, 100);
                }
                _ => panic!("expected analyze dead-code"),
            }
        } else {
            panic!("expected Analyze command");
        }
    }

    #[test]
    fn parse_refactor_rename_with_named_flags() {
        let cli = parse(&[
            "atlas",
            "refactor",
            "rename",
            "--symbol",
            "src/lib.rs::fn::helper",
            "--to",
            "helper_renamed",
            "--dry-run",
        ]);
        if let Command::Refactor { subcommand } = cli.command {
            match subcommand {
                RefactorCommand::Rename {
                    symbol,
                    to,
                    dry_run,
                    legacy_symbol,
                    legacy_to,
                } => {
                    assert_eq!(symbol.as_deref(), Some("src/lib.rs::fn::helper"));
                    assert_eq!(to.as_deref(), Some("helper_renamed"));
                    assert!(dry_run);
                    assert!(legacy_symbol.is_none());
                    assert!(legacy_to.is_none());
                }
                _ => panic!("expected refactor rename"),
            }
        } else {
            panic!("expected Refactor command");
        }
    }

    #[test]
    fn parse_refactor_rename_legacy_positionals() {
        let cli = parse(&[
            "atlas",
            "refactor",
            "rename",
            "src/lib.rs::fn::helper",
            "helper_renamed",
            "--dry-run",
        ]);
        if let Command::Refactor { subcommand } = cli.command {
            match subcommand {
                RefactorCommand::Rename {
                    symbol,
                    to,
                    dry_run,
                    legacy_symbol,
                    legacy_to,
                } => {
                    assert!(symbol.is_none());
                    assert!(to.is_none());
                    assert!(dry_run);
                    assert_eq!(legacy_symbol.as_deref(), Some("src/lib.rs::fn::helper"));
                    assert_eq!(legacy_to.as_deref(), Some("helper_renamed"));
                }
                _ => panic!("expected refactor rename"),
            }
        } else {
            panic!("expected Refactor command");
        }
    }
}
