use super::*;

/// Sub-commands for `atlas session`.
#[derive(Debug, Subcommand)]
pub enum SessionCommand {
    /// Start (or re-register) the current session.
    Start,

    /// Show the current session status and recent event summary.
    Status {
        #[arg(long)]
        agent_id: Option<String>,

        #[arg(long)]
        merge_agent_partitions: bool,

        /// Explicit session id; defaults to the derived cli session.
        #[arg(long)]
        session: Option<String>,
    },

    /// Print the pending resume snapshot, then mark it consumed.
    Resume {
        #[arg(long)]
        agent_id: Option<String>,

        #[arg(long)]
        merge_agent_partitions: bool,

        /// Explicit session id; defaults to the derived cli session.
        #[arg(long)]
        session: Option<String>,
    },

    /// Delete the current session and all its stored events.
    Clear,

    /// List all known sessions for this repo.
    List,

    /// Search persisted decision memory for prior conclusions.
    Decisions {
        /// Search query text.
        query: String,

        /// Restrict lookup to current CLI session only.
        #[arg(long)]
        current_session: bool,

        /// Maximum decisions to return.
        #[arg(long, default_value_t = 10)]
        limit: usize,
    },

    /// Compact and curate the session event ledger.
    ///
    /// Removes stale low-value events, merges repeated actions, deduplicates
    /// reasoning outputs, and promotes high-value events to a higher priority
    /// so they survive future eviction cycles.
    Compact {
        /// Explicit session id; defaults to the derived cli session.
        #[arg(long)]
        session: Option<String>,
    },

    /// Persist a large payload as a saved-context artifact.
    SaveArtifact(SessionSaveArtifactArgs),

    /// Read a saved-context artifact and print its content verbatim.
    ReadArtifact(SessionReadArtifactArgs),

    /// Search saved-context artifacts.
    Search(SessionSearchArgs),

    /// Show saved-context storage statistics.
    Stats(SessionStatsArgs),
}
