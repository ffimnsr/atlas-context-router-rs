//! ICM-G6 — `atlas overview` CLI surface.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum OverviewCommand {
    /// Export bounded overview analysis packets for external analysis.
    Export {
        /// Export scope: project, packages, modules, files, symbols, functions,
        /// methods, or changed.
        #[arg(long, default_value = "project")]
        scope: String,

        /// Specific subject (qualified name, file path, package, or module).
        #[arg(long = "subject")]
        subjects: Vec<String>,

        /// Export subjects affected by changed files only.
        #[arg(long)]
        changed: bool,

        /// Git ref to diff against when exporting changed subjects.
        #[arg(long)]
        since: Option<String>,

        /// Maximum packets to export (default: 50, max: 200).
        #[arg(long)]
        limit: Option<usize>,

        /// Include bounded source excerpts.
        #[arg(long)]
        include_code_spans: bool,

        /// Include caller qualified names.
        #[arg(long)]
        include_callers: bool,

        /// Include callee qualified names.
        #[arg(long)]
        include_callees: bool,

        /// Include test-adjacency qualified names.
        #[arg(long)]
        include_tests: bool,

        /// Approximate token budget per packet.
        #[arg(long)]
        max_tokens: Option<usize>,
    },

    /// Ingest externally produced analysis JSON.
    Ingest {
        /// Path to an analysis JSON file, or `-` for stdin.
        input: String,
    },

    /// Show one overview record by id, qualified name, path, package, or module.
    Show {
        /// Subject id, qualified name, canonical file path, package, or module.
        subject: String,

        /// Optional progressive disclosure level: l0, l1, or l2.
        #[arg(long)]
        level: Option<String>,
    },

    /// Search overview records with FTS and explicit ranking.
    Search {
        /// Search query.
        query: String,

        /// Restrict to one subject kind.
        #[arg(long)]
        subject_kind: Option<String>,

        /// Restrict to one freshness state.
        #[arg(long)]
        freshness: Option<String>,

        /// Restrict to one package name.
        #[arg(long)]
        package: Option<String>,

        /// Restrict to one module path.
        #[arg(long)]
        module: Option<String>,

        /// Restrict to one canonical file path.
        #[arg(long)]
        file: Option<String>,

        /// Restrict to one source kind: external_llm, manual, or imported.
        #[arg(long)]
        source_kind: Option<String>,

        /// Maximum results (default: 20).
        #[arg(long)]
        limit: Option<usize>,

        /// Optional progressive disclosure level: l0, l1, or l2.
        #[arg(long)]
        level: Option<String>,
    },

    /// List active overview records.
    List {
        /// Restrict to one subject kind.
        #[arg(long)]
        subject_kind: Option<String>,

        /// Restrict to one freshness state.
        #[arg(long)]
        freshness: Option<String>,

        /// Restrict to one source kind.
        #[arg(long)]
        source_kind: Option<String>,

        /// Only records updated before this RFC3339 timestamp.
        #[arg(long)]
        older_than: Option<String>,

        /// Only records updated at or after this RFC3339 timestamp.
        #[arg(long)]
        newer_than: Option<String>,

        /// Maximum results (default: 20).
        #[arg(long)]
        limit: Option<usize>,
    },

    /// List stale, possibly stale, or orphaned overview records.
    Stale {
        /// Restrict to one subject kind.
        #[arg(long)]
        subject_kind: Option<String>,

        /// Restrict to one canonical file path.
        #[arg(long)]
        file: Option<String>,

        /// Restrict to one package name.
        #[arg(long)]
        package: Option<String>,

        /// Restrict to one module path.
        #[arg(long)]
        module: Option<String>,

        /// Only records updated at or after this RFC3339 timestamp.
        #[arg(long)]
        since: Option<String>,

        /// Maximum results (default: 20).
        #[arg(long)]
        limit: Option<usize>,
    },

    /// Plan (or apply) overview freshness refresh after changes.
    #[command(name = "refresh-plan")]
    RefreshPlan {
        /// Only plan for changed files.
        #[arg(long)]
        changed: bool,

        /// Git ref to diff against.
        #[arg(long)]
        since: Option<String>,

        /// Apply freshness marking instead of read-only planning.
        #[arg(long)]
        apply: bool,
    },

    /// Delete one overview record by exact id.
    Delete {
        /// Exact overview id.
        overview_id: String,

        /// Report what would be removed without deleting.
        #[arg(long)]
        dry_run: bool,
    },

    /// Export readable Markdown specs grouped by project/package/module/file/symbol.
    #[command(name = "export-docs")]
    ExportDocs {
        /// Output format (only markdown is supported).
        #[arg(long, default_value = "markdown")]
        format: String,

        /// Output path; `-` writes to stdout.
        #[arg(long)]
        output: String,
    },

    /// Run the configured external analyzer command over exported packets.
    #[command(name = "run-external")]
    RunExternal {
        /// Print exact command invocations without executing them.
        #[arg(long)]
        dry_run: bool,

        /// Apply mode: execute, validate JSON output, and ingest results.
        #[arg(long)]
        apply: bool,

        /// Export scope used to build packets.
        #[arg(long, default_value = "changed")]
        scope: String,

        /// Git ref to diff against for changed subjects.
        #[arg(long)]
        since: Option<String>,

        /// Maximum subjects to process.
        #[arg(long)]
        limit: Option<usize>,
    },
}
