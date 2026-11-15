//! `atlas skills` — agent skill guides served by this Atlas version.

use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum SkillsCommand {
    /// List the skill guides and references this Atlas version serves.
    List,

    /// Print one skill guide, one reference, or every reference.
    Get {
        /// Skill name, for example `core`.
        name: String,

        /// Print one reference of the skill instead of its guide.
        #[arg(long = "ref", value_name = "NAME", conflicts_with = "full")]
        reference: Option<String>,

        /// Print the guide plus every reference in one output.
        #[arg(long, conflicts_with = "reference")]
        full: bool,
    },
}
