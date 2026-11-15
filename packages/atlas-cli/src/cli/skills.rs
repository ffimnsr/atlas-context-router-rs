//! `atlas skills` — agent skill guides served by this Atlas version.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum SkillsCommand {
    /// List the skill guides this Atlas version serves.
    List,

    /// Print one skill guide to stdout.
    Get {
        /// Skill name, for example `core`.
        name: String,
    },
}
