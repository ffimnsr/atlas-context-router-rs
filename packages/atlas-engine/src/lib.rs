#![doc = include_str!("../README.md")]

mod build_budget;
mod call_resolution;
pub mod config;
pub mod lang_policy;
mod owner_graph;
pub mod paths;

mod build;
mod postprocess;
mod update;
pub mod watch;

pub use build::{BuildOptions, BuildSummary, build_graph};
pub use config::{BuildRunBudget, Config, ConfigTemplateProfile};
pub use lang_policy::{Feature, LangEntry, LanguagePolicy, Maturity};
pub use postprocess::{
    POSTPROCESS_STAGE_ARCHITECTURE_METRICS, POSTPROCESS_STAGE_COMMUNITIES, POSTPROCESS_STAGE_FLOWS,
    POSTPROCESS_STAGE_LARGE_FUNCTION_SUMMARIES, POSTPROCESS_STAGE_QUERY_HINTS, PostprocessOptions,
    postprocess_graph, supported_postprocess_stages,
};
pub use update::{UpdateOptions, UpdateSummary, UpdateTarget, update_graph};
pub use watch::{FileWatcher, WatchBatchResult, WatchEvent, WatchRunner, WatchState};
