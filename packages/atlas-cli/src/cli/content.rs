//! CLI argument families for companion content and relationship commands.
//!
//! Each struct mirrors the matching MCP tool input so the CLI can forward the
//! same arguments to the shared tool implementation and keep both surfaces in
//! lockstep.

use clap::{Args, Subcommand};

/// Companion content and relationship commands (`atlas content …`).
#[derive(Debug, Subcommand)]
pub enum ContentCommand {
    /// Discover files by glob pattern (MCP `search_files`).
    Files(FilesArgs),

    /// Search file contents by literal string or regex (MCP `search_content`).
    Grep(GrepArgs),

    /// Read repo-scoped file excerpts by line range or line-with-context.
    Excerpt(ExcerptArgs),

    /// Show grouped snippets around one file's literal or regex matches.
    #[command(name = "match-context")]
    MatchContext(MatchContextArgs),

    /// Discover template files by engine kind (MCP `search_templates`).
    Templates(AssetDiscoveryArgs),

    /// Discover SQL, config, env, and prompt assets (MCP `search_text_assets`).
    #[command(name = "text-assets")]
    TextAssets(AssetDiscoveryArgs),

    /// Show immediate graph neighbours of a symbol (MCP `symbol_neighbors`).
    Neighbors(NeighborsArgs),

    /// Show files coupled through shared symbols (MCP `cross_file_links`).
    Links(LinksArgs),

    /// Show related file groups around seed files (MCP `concept_clusters`).
    Clusters(ClustersArgs),
}

/// `atlas content files` — file-path discovery by glob pattern.
#[derive(Debug, Args)]
pub struct FilesArgs {
    /// Glob pattern matched against file names and repo-relative paths.
    pub pattern: String,

    /// Include-path filter glob (repeatable).
    #[arg(long = "glob", value_name = "GLOB")]
    pub globs: Vec<String>,

    /// Exclusion filter glob (repeatable).
    #[arg(long = "exclude-glob", value_name = "GLOB")]
    pub exclude_globs: Vec<String>,

    /// Match globs case-sensitively.
    #[arg(long)]
    pub case_sensitive: bool,

    /// Restrict the walk to a repo-relative subpath.
    #[arg(long)]
    pub subpath: Option<String>,
}

/// `atlas grep` — content search by literal string or regex.
#[derive(Debug, Args)]
pub struct GrepArgs {
    /// Text to search for (literal by default).
    pub query: String,

    /// Treat `query` as a regex pattern.
    #[arg(long)]
    pub regex: bool,

    /// Include-path filter glob (repeatable).
    #[arg(long = "glob", value_name = "GLOB")]
    pub globs: Vec<String>,

    /// Exclusion filter glob (repeatable).
    #[arg(long = "exclude-glob", value_name = "GLOB")]
    pub exclude_globs: Vec<String>,

    /// Also search generated and vendored files.
    #[arg(long)]
    pub include_generated: bool,

    /// Context lines around each match.
    #[arg(long)]
    pub context_lines: Option<u64>,

    /// Return grouped per-match snippets with before/after context.
    #[arg(long)]
    pub rich_snippets: bool,

    /// Context lines per grouped snippet (default: max(context_lines, 2)).
    #[arg(long)]
    pub snippet_context_lines: Option<u64>,

    /// Maximum match lines to return (default: 50).
    #[arg(long)]
    pub max_results: Option<u64>,

    /// Restrict the walk to a repo-relative subpath.
    #[arg(long)]
    pub subpath: Option<String>,

    /// Non-recursive scope: only files directly inside this directory.
    #[arg(long)]
    pub exact_dir: Option<String>,

    /// Exclude a subpath prefix (repeatable).
    #[arg(long = "exclude-subpath")]
    pub exclude_subpaths: Vec<String>,
}

/// `atlas excerpt` — bounded file reads by line range or line-with-context.
#[derive(Debug, Args)]
pub struct ExcerptArgs {
    /// Repo-relative file path.
    pub file: String,

    /// First line of a single range selector.
    #[arg(long, requires = "end_line")]
    pub start_line: Option<u64>,

    /// Last line of a single range selector.
    #[arg(long, requires = "start_line")]
    pub end_line: Option<u64>,

    /// Additional range selector as `START:END` (repeatable).
    #[arg(long = "range", value_name = "START:END")]
    pub ranges: Vec<String>,

    /// Center a context selector on this line.
    #[arg(long)]
    pub line: Option<u64>,

    /// Context lines before/after `--line` (default: 2).
    #[arg(long, default_value_t = 2, requires = "line")]
    pub context: u64,

    /// Maximum excerpt lines to emit (default: 200).
    #[arg(long)]
    pub max_lines: Option<u64>,
}

/// `atlas match-context` — grouped snippets around one file's matches.
#[derive(Debug, Args)]
pub struct MatchContextArgs {
    /// Repo-relative file path.
    pub file: String,

    /// Literal or regex pattern to match.
    pub query: String,

    /// Treat `query` as a regex pattern.
    #[arg(long)]
    pub regex: bool,

    /// Match case-sensitively (default: true for regex, false for literal).
    #[arg(long)]
    pub case_sensitive: bool,

    /// Context lines before each match (default: 2).
    #[arg(long, default_value_t = 2)]
    pub before: u64,

    /// Context lines after each match (default: 2).
    #[arg(long, default_value_t = 2)]
    pub after: u64,

    /// Maximum matched lines to consider (default: 20).
    #[arg(long)]
    pub max_matches: Option<u64>,

    /// Maximum snippet lines to emit (default: 200).
    #[arg(long)]
    pub max_lines: Option<u64>,
}

/// Shared filters for template and text-asset discovery.
#[derive(Debug, Args)]
pub struct AssetDiscoveryArgs {
    /// Asset kind filter (for example `html`, `jinja`, `sql`, `config`).
    #[arg(long)]
    pub kind: Option<String>,

    /// Restrict the walk to a repo-relative subpath.
    #[arg(long)]
    pub subpath: Option<String>,

    /// Include-path filter glob (repeatable).
    #[arg(long = "glob", value_name = "GLOB")]
    pub globs: Vec<String>,

    /// Exclusion filter glob (repeatable).
    #[arg(long = "exclude-glob", value_name = "GLOB")]
    pub exclude_globs: Vec<String>,

    /// Match globs case-sensitively.
    #[arg(long)]
    pub case_sensitive: bool,

    /// Maximum assets to return (default: 100).
    #[arg(long)]
    pub max_results: Option<u64>,
}

/// `atlas neighbors` — immediate graph neighbourhood of one symbol.
#[derive(Debug, Args)]
pub struct NeighborsArgs {
    /// Fully qualified symbol name.
    pub qname: String,

    /// Maximum nodes per relationship kind (default: 10).
    #[arg(long)]
    pub limit: Option<u64>,
}

/// `atlas links` — files coupled through shared symbol references.
#[derive(Debug, Args)]
pub struct LinksArgs {
    /// Repo-relative file path.
    pub file: String,

    /// Maximum linked files to return (default: 20).
    #[arg(long)]
    pub limit: Option<u64>,

    /// Include weak same-package fallback edges.
    #[arg(long)]
    pub include_weak: bool,
}

/// `atlas clusters` — related file groups around seed files.
#[derive(Debug, Args)]
pub struct ClustersArgs {
    /// Seed file paths (repo-relative).
    #[arg(value_name = "FILE", num_args = 1..)]
    pub files: Vec<String>,

    /// Maximum clusters to return (default: 10).
    #[arg(long)]
    pub limit: Option<u64>,
}
