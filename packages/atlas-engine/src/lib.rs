//! Atlas engine — shared build/update pipeline usable by CLI and MCP.

mod call_resolution;
pub mod config;
pub mod lang_policy;
mod owner_graph;
pub mod paths;

mod build;
mod update;
pub mod watch;

pub use build::{BuildOptions, BuildSummary, build_graph};
pub use config::Config;
pub use lang_policy::{Feature, LangEntry, LanguagePolicy, Maturity};
pub use update::{UpdateOptions, UpdateSummary, UpdateTarget, update_graph};
pub use watch::{FileWatcher, WatchBatchResult, WatchEvent, WatchRunner, WatchState};
