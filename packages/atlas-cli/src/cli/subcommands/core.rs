use super::*;

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Show resolved config values and where they came from.
    Show,
}

/// Sub-commands for `atlas repo`.
#[derive(Debug, Subcommand)]
pub enum RepoCommand {
    /// List registered repos.
    List,

    /// Register a sibling or external git repo in this registry.
    Add {
        /// Path inside target git repo.
        path: String,
    },

    /// Remove one repo registration by stable repo id.
    Remove {
        /// Stable repo id to remove.
        repo_id: String,
    },

    /// Refresh refs, remotes, enabled state, and submodule warnings.
    Sync,
}
