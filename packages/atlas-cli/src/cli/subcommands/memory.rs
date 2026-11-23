use super::*;

/// Sub-commands for `atlas memory`.
#[derive(Debug, Subcommand)]
pub enum MemoryCommand {
    /// Store a memory record for the current repo, or apply an EUA patch.
    Store {
        /// Memory body text; stored exactly as provided.
        text: Option<String>,

        /// Memory body text (flag form, same as the positional argument).
        #[arg(long = "text", conflicts_with = "patch_id")]
        text_flag: Option<String>,

        /// Topic label for the memory.
        #[arg(long)]
        topic: Option<String>,

        /// Optional title line.
        #[arg(long)]
        title: Option<String>,

        /// Importance: critical, high, normal, or low (default: normal).
        #[arg(long)]
        importance: Option<String>,

        /// Visibility scope: project, session, frontend, or global (default: project).
        #[arg(long)]
        scope: Option<String>,

        /// Frontend identifier; required when --scope is frontend.
        #[arg(long)]
        frontend: Option<String>,

        /// Source id linking this memory to a saved-context artifact.
        #[arg(long)]
        source_id: Option<String>,

        /// Searchable tag (repeatable; duplicates collapse).
        #[arg(long = "tag")]
        tags: Vec<String>,

        /// Exact memory id to patch with an EUA search/replace document.
        #[arg(long, conflicts_with = "text")]
        patch_id: Option<String>,

        /// File containing `<<<< SEARCH ... ==== ... >>>> REPLACE` blocks.
        #[arg(long, requires = "patch_id", conflicts_with = "patch_text")]
        patch_file: Option<String>,

        /// Inline `<<<< SEARCH ... ==== ... >>>> REPLACE` patch text.
        #[arg(long, requires = "patch_id", conflicts_with = "patch_file")]
        patch_text: Option<String>,
    },

    /// Recall memories matching a query; exact topic matches rank first.
    Recall {
        /// Search text matched against topic, title, and body.
        query: String,

        /// Restrict recall to one topic (case-insensitive exact match).
        #[arg(long)]
        topic: Option<String>,

        /// Restrict recall to one importance level.
        #[arg(long)]
        importance: Option<String>,

        /// Restrict recall to one scope.
        /// Restrict to one scope.
        #[arg(long)]
        scope: Option<String>,

        /// Require this tag (repeatable; every tag must match).
        #[arg(long = "tag")]
        tags: Vec<String>,

        /// Only return memories visible to every frontend (project + global).
        #[arg(long, conflicts_with = "scope")]
        shared: bool,

        /// Maximum memories to return.
        #[arg(long, default_value_t = 20)]
        limit: usize,

        /// Fuse optional memory embeddings with lexical recall (ICM-E2).
        #[arg(long)]
        hybrid: bool,
    },

    /// Link one memory to another; relations use the memoir vocabulary
    /// plus `supersedes`.
    Link {
        /// Exact source memory id.
        memory_id: String,

        /// Exact target memory id.
        target_id: String,

        /// Link relation (default: related_to).
        #[arg(long, default_value = "related_to")]
        relation: String,
    },

    /// Compute and store embeddings for memories that do not have one (ICM-E2).
    Embed {
        /// Restrict to one topic (case-insensitive exact match).
        #[arg(long)]
        topic: Option<String>,

        /// Restrict to one importance level.
        #[arg(long)]
        importance: Option<String>,

        /// Restrict to one scope.
        #[arg(long)]
        scope: Option<String>,

        /// Maximum memories to embed in this run.
        #[arg(long, default_value_t = 50)]
        limit: usize,

        /// List memories that would be embedded without calling the provider.
        #[arg(long)]
        dry_run: bool,
    },

    /// Browse memory, feedback, memoir, overview, health, and artifacts (read-only).
    Tui(MemoryTuiArgs),

    /// List stored memories with filters; sorted by updated_at DESC.
    List {
        /// Filter by topic (case-insensitive exact match).
        #[arg(long)]
        topic: Option<String>,

        /// Filter by importance level.
        #[arg(long)]
        importance: Option<String>,

        /// Filter by scope.
        #[arg(long)]
        scope: Option<String>,

        /// Only memories updated before this date (YYYY-MM-DD or RFC 3339).
        #[arg(long)]
        older_than: Option<String>,

        /// Only memories updated after this date (YYYY-MM-DD or RFC 3339).
        #[arg(long)]
        newer_than: Option<String>,
    },

    /// Delete one memory by its exact id; linked saved-context artifacts are kept.
    Delete {
        /// Exact memory id to delete.
        memory_id: String,

        /// Show what would be deleted without deleting.
        #[arg(long)]
        dry_run: bool,
    },

    /// Compute updated decay scores for stored memories without deleting rows.
    Decay {
        /// Restrict to one topic (case-insensitive exact match).
        #[arg(long)]
        topic: Option<String>,

        /// Report updated scores without writing them back.
        #[arg(long)]
        dry_run: bool,
    },

    /// List memories past their retention window; critical rows are never candidates.
    Stale {
        /// Restrict to one topic (case-insensitive exact match).
        #[arg(long)]
        topic: Option<String>,

        /// Restrict to one scope.
        #[arg(long)]
        scope: Option<String>,
    },

    /// Delete memories marked pruneable by the decay policy.
    Prune {
        /// Show what would be pruned without deleting.
        #[arg(long)]
        dry_run: bool,

        /// Restrict to one topic (case-insensitive exact match).
        #[arg(long)]
        topic: Option<String>,

        /// Restrict to one importance level.
        #[arg(long)]
        importance: Option<String>,

        /// Only memories updated before this date (YYYY-MM-DD or RFC 3339).
        #[arg(long)]
        older_than: Option<String>,

        /// Explicit override required before any critical memory can be pruned.
        #[arg(long)]
        allow_critical: bool,
    },

    /// Deterministic memory health report with actionable follow-up commands.
    Health {
        /// Restrict to one topic (case-insensitive exact match).
        #[arg(long)]
        topic: Option<String>,

        /// Restrict to one scope.
        #[arg(long)]
        scope: Option<String>,
    },

    /// Deterministically consolidate duplicate memories per topic.
    Consolidate {
        /// Restrict to one topic (case-insensitive exact match).
        #[arg(long)]
        topic: Option<String>,

        /// Restrict to one scope.
        #[arg(long)]
        scope: Option<String>,

        /// Report the consolidation plan without mutating storage.
        #[arg(long)]
        dry_run: bool,
    },

    /// Show the bounded typed-operator history for one memory row (VM1).
    Ops {
        /// Exact memory id to inspect.
        memory_id: String,
    },

    /// Show the cross-session global memory summary for this repo.
    Global(MemoryGlobalArgs),

    /// Search persisted decision memory for prior conclusions.
    Decisions(MemoryDecisionsArgs),
}
