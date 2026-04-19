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

        /// Maximum traversal depth.
        #[arg(long, default_value_t = 3)]
        max_depth: u32,

        /// Maximum number of impacted nodes to consider.
        #[arg(long, default_value_t = 200)]
        max_nodes: u32,
    },

    /// Start a JSON-RPC / MCP stdio server.
    Serve,

    /// Run an integrity check on the atlas database.
    DbCheck,

    /// Run a health check on the atlas setup (repo, config, database).
    Doctor,
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
        let cli = parse(&["atlas", "--repo", "/tmp/proj", "--db", "/tmp/w.sqlite", "status"]);
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
        if let Command::Update { base, staged, working_tree, files, fail_fast } = cli.command {
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
        if let Command::Query { text, kind, language, limit, .. } = cli.command {
            assert_eq!(text, "ReplaceFileGraph");
            assert!(kind.is_none());
            assert!(language.is_none());
            assert_eq!(limit, 20);
        } else {
            panic!("expected Query command");
        }
    }

    #[test]
    fn parse_query_with_kind_and_language_filters() {
        let cli = parse(&["atlas", "query", "foo", "--kind", "function", "--language", "rust"]);
        if let Command::Query { text, kind, language, .. } = cli.command {
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
        if let Command::Query { expand, expand_hops, .. } = cli.command {
            assert!(expand);
            assert_eq!(expand_hops, 3);
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
        if let Command::Impact { max_depth, max_nodes, .. } = cli.command {
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
        if let Command::Impact { max_depth, max_nodes, .. } = cli.command {
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
        if let Command::ReviewContext { max_depth, max_nodes, base, files } = cli.command {
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
}
