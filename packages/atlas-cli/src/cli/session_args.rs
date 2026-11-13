//! Argument structs for the session/memory CLI surface added by the MCP sweep
//! fixes.
//!
//! Kept out of `cli/subcommands.rs` (already above the repository 1K-LOC
//! threshold) and out of the inline enum variants so clap's generated builder
//! stays a bounded function in debug builds.

use clap::Args;

/// Arguments for `atlas session save-artifact`.
#[derive(Args, Debug)]
pub struct SessionSaveArtifactArgs {
    /// Artifact content.
    pub content: String,

    /// Human-readable label.
    #[arg(long)]
    pub label: String,

    /// Category tag (default: cli_artifact).
    #[arg(long)]
    pub source_type: Option<String>,

    /// MIME type (default: text/plain).
    #[arg(long)]
    pub content_type: Option<String>,

    /// Explicit session id; defaults to the derived cli session.
    #[arg(long)]
    pub session: Option<String>,
}

/// Arguments for `atlas session read-artifact`.
#[derive(Args, Debug)]
pub struct SessionReadArtifactArgs {
    /// Artifact source id returned by `session save-artifact`.
    pub source_id: String,

    /// Maximum bytes to return (default: policy cap).
    #[arg(long)]
    pub max_bytes: Option<u64>,

    /// Start offset; byte offset for verbatim artifacts, chunk index otherwise.
    #[arg(long, default_value_t = 0)]
    pub chunk_offset: u64,
}

/// Arguments for `atlas session search`.
#[derive(Args, Debug)]
pub struct SessionSearchArgs {
    /// Search query text.
    pub query: String,

    /// Maximum matches to return.
    #[arg(long, default_value_t = 10)]
    pub limit: usize,

    /// Restrict to one source_type.
    #[arg(long)]
    pub source_type: Option<String>,

    /// Search across every session in this repo.
    #[arg(long)]
    pub cross_session: bool,
}

/// Arguments for `atlas session stats`.
#[derive(Args, Debug)]
pub struct SessionStatsArgs {
    /// Explicit session id; defaults to the derived cli session.
    #[arg(long)]
    pub session: Option<String>,

    #[arg(long)]
    pub agent_id: Option<String>,
}

/// Arguments for `atlas memory global`.
#[derive(Args, Debug)]
pub struct MemoryGlobalArgs {
    /// Maximum entries per category.
    #[arg(long, default_value_t = 10)]
    pub limit: usize,

    /// Focus symbol qualified name used to match related past sessions (repeatable).
    #[arg(long = "focus-symbol")]
    pub focus_symbols: Vec<String>,

    /// Focus file path used to match related past sessions (repeatable).
    #[arg(long = "focus-file")]
    pub focus_files: Vec<String>,
}

/// Arguments for `atlas memory decisions`.
#[derive(Args, Debug)]
pub struct MemoryDecisionsArgs {
    /// Search query text.
    pub query: String,

    /// Maximum decisions to return.
    #[arg(long, default_value_t = 10)]
    pub limit: usize,

    /// Restrict lookup to the current cli session only.
    #[arg(long)]
    pub current_session: bool,
}

/// Arguments for `atlas memory tui`.
#[derive(Args, Debug)]
pub struct MemoryTuiArgs {
    /// Filter memories by topic (case-insensitive exact match).
    #[arg(long)]
    pub topic: Option<String>,

    /// Filter memories by scope.
    #[arg(long)]
    pub scope: Option<String>,

    /// Filter memories by importance level.
    #[arg(long)]
    pub importance: Option<String>,

    /// Filter memories by frontend identity.
    #[arg(long)]
    pub frontend: Option<String>,

    /// Filter overview records by subject kind.
    #[arg(long)]
    pub subject_kind: Option<String>,

    /// Filter overview records by freshness.
    #[arg(long)]
    pub freshness: Option<String>,

    /// Filter overview records by package name.
    #[arg(long)]
    pub package: Option<String>,

    /// Filter overview records by module path.
    #[arg(long)]
    pub module: Option<String>,

    /// Filter overview records by canonical file path.
    #[arg(long)]
    pub file: Option<String>,

    /// Filter overview records by source kind.
    #[arg(long)]
    pub source_kind: Option<String>,

    /// Only overview records updated after this date (YYYY-MM-DD or RFC 3339).
    #[arg(long)]
    pub newer_than: Option<String>,

    /// Only overview records updated before this date (YYYY-MM-DD or RFC 3339).
    #[arg(long)]
    pub older_than: Option<String>,

    /// Maximum rows rendered per section.
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
}
