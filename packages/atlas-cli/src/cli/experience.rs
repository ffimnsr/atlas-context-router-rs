//! `atlas experience` — query-conditioned experience edge inspection.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum ExperienceCommand {
    /// Inspect experience edges touching one atlas:// URI.
    Inspect {
        /// Canonical source or target URI.
        uri: String,

        /// Edge direction: both, outgoing, or incoming.
        #[arg(long, default_value = "both")]
        direction: String,

        /// Maximum edges to return.
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },

    /// Prune old experience edges.
    Prune {
        /// Keep edges validated within this many days.
        #[arg(long, default_value_t = 30)]
        older_than: u64,

        /// Report what would be removed without deleting.
        #[arg(long)]
        dry_run: bool,
    },
}
