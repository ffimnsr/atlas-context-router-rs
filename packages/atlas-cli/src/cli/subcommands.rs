use clap::Subcommand;

use super::session_args::{
    MemoryDecisionsArgs, MemoryGlobalArgs, MemoryTuiArgs, SessionReadArtifactArgs,
    SessionSaveArtifactArgs, SessionSearchArgs, SessionStatsArgs,
};

mod analyze;
mod core;
mod docs;
mod feedback;
mod history;
mod insights;
mod install;
mod memory;
mod session;

pub use self::analyze::AnalyzeCommand;
pub use self::analyze::RefactorCommand;
pub use self::core::ConfigCommand;
pub use self::core::RepoCommand;
pub use self::docs::DocsCommand;
pub use self::feedback::CommunitiesCommand;
pub use self::feedback::FeedbackCommand;
pub use self::feedback::FlowsCommand;
pub use self::history::HistoryCommand;
pub use self::insights::InsightsCommand;
pub use self::insights::InsightsLargeFunctionMode;
pub use self::install::InitMode;
pub use self::install::InstallMode;
pub use self::memory::MemoryCommand;
pub use self::session::SessionCommand;
