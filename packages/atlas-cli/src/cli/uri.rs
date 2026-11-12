//! `atlas uri` — scoped `atlas://` access primitives.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum UriCommand {
    /// Read one atlas:// URI at an optional disclosure level.
    Read {
        /// Canonical atlas:// URI.
        uri: String,

        /// Disclosure level: l0, l1, or l2. Defaults to the URI form default.
        #[arg(long)]
        level: Option<String>,
    },

    /// List immediate children of an atlas:// repo tree URI.
    List {
        /// Canonical atlas:// tree URI.
        uri: String,
    },

    /// Grep content scoped to an atlas:// subtree.
    Grep {
        /// Canonical atlas:// tree, file, or symbol URI.
        uri: String,

        /// Literal or regex pattern.
        pattern: String,

        /// Treat `pattern` as a regex.
        #[arg(long)]
        regex: bool,
    },

    /// Search the graph, memories, or memoir scoped to an atlas:// URI.
    Search {
        /// Canonical atlas:// URI.
        uri: String,

        /// Search query.
        query: String,
    },
}
