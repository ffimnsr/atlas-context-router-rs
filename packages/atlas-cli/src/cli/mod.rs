use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::Shell;

mod content;
mod experience;
mod memoir;
mod overview;
mod session_args;
mod skills;
mod subcommands;
mod uri;

#[cfg(test)]
mod tests;

pub use content::{
    AssetDiscoveryArgs, ClustersArgs, ContentCommand, ExcerptArgs, FilesArgs, GrepArgs, LinksArgs,
    MatchContextArgs, NeighborsArgs,
};
pub use experience::ExperienceCommand;
pub use memoir::MemoirCommand;
pub use overview::OverviewCommand;
pub use skills::SkillsCommand;
pub use subcommands::{
    AnalyzeCommand, CommunitiesCommand, ConfigCommand, DocsCommand, FeedbackCommand, FlowsCommand,
    HistoryCommand, InitMode, InsightsCommand, InsightsLargeFunctionMode, InstallMode,
    MemoryCommand, RefactorCommand, RepoCommand, SessionCommand,
};
pub use uri::UriCommand;

use crate::install::InstructionsMode;

/// `atlas index` subcommands.
#[derive(Debug, Subcommand)]
pub enum IndexCommand {
    /// Rebuild context.db chunk/FTS indexes from `.atlas/artifacts/**`.
    Rebuild {
        /// Clear derived rows first and regenerate every chunk.
        #[arg(long)]
        full: bool,

        /// Also attempt provider-backed embedding regeneration.
        #[arg(long)]
        embeddings: bool,
    },
}

/// `atlas migrate` subcommands.
#[derive(Debug, Subcommand)]
pub enum MigrateCommand {
    /// Export durable continuity records to `.atlas/**` markdown truth files.
    Markdown {
        /// Report pending files without writing or backing up.
        #[arg(long)]
        dry_run: bool,

        /// Remove the pre-migration backup and legacy staging table.
        #[arg(long)]
        finalize: bool,

        /// Rebuild memory embeddings from markdown bodies after migration.
        #[arg(long)]
        embeddings: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ReviewContextFormat {
    Text,
    Markdown,
}

#[derive(Debug, Parser)]
#[command(
    name = "atlas",
    about = "Atlas code graph CLI",
    version = env!("ATLAS_LONG_VERSION")
)]
pub struct Cli {
    /// Repository root. Supports absolute paths, cwd-relative paths, ~, $HOME, and ${HOME}.
    /// When omitted, Atlas auto-detects the Git root from the startup working directory.
    #[arg(long, global = true)]
    pub repo: Option<String>,

    /// Path to the atlas database (default: `REPO_ROOT/.atlas/worldtree.db`).
    #[arg(long, global = true)]
    pub db: Option<String>,

    /// Enable verbose diagnostic output.
    #[arg(short, long, global = true)]
    pub verbose: bool,

    /// Emit machine-readable JSON output where supported.
    #[arg(long, global = true)]
    pub json: bool,

    #[command(subcommand)]
    pub command: Command,
}

mod command;

pub use self::command::Command;
