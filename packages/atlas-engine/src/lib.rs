#![doc = include_str!("../README.md")]

mod annotate;
mod build_budget;
mod call_resolution;
pub mod config;
mod embedding;
pub mod lang_policy;
mod owner_graph;
pub mod paths;

mod build;
mod package_resolution;
mod postprocess;
mod reachability;
mod reachability_measure;
mod repo_graph;
mod run_metrics;
mod update;
mod update_files;
pub mod watch;

pub use build::{BuildOptions, BuildSummary};
pub use config::{BuildRunBudget, Config, ConfigTemplateProfile, EmbeddingBackendConfig};
pub use embedding::{
    DEFAULT_EMBED_CHUNK_LIMIT, EmbedChunkOptions, EmbedChunkReport, embed_pending_chunks,
};
pub use lang_policy::{Feature, LangEntry, LanguagePolicy, Maturity};
pub use postprocess::{
    POSTPROCESS_STAGE_ARCHITECTURE_METRICS, POSTPROCESS_STAGE_COMMUNITIES, POSTPROCESS_STAGE_FLOWS,
    POSTPROCESS_STAGE_LARGE_FUNCTION_SUMMARIES, POSTPROCESS_STAGE_QUERY_HINTS, PostprocessOptions,
    postprocess_graph, supported_postprocess_stages,
};
pub use reachability::{
    CrateRoot, CrateRootKind, ReachabilityGuard, ReachabilityResult, ReachabilityStatus,
    ReachabilitySummary, RustReachabilityIndex, crate_root_kind, diagnose_reachability,
};
pub use reachability_measure::{
    DEFAULT_MEASUREMENT_SAMPLE_LIMIT, MEASUREMENT_SOURCE, ReachabilityMeasurement, RustFileKind,
    SAME_PACKAGE_TIER, classify_rust_file_path, measure_same_package_edges,
};
pub use repo_graph::refresh_repo_registry_graph;
pub use run_metrics::{build_graph, update_graph};
pub use update::{UpdateOptions, UpdateSummary, UpdateTarget};
pub use watch::{FileWatcher, WatchBatchResult, WatchEvent, WatchRunner, WatchState};
