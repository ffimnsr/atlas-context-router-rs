//! Atlas engine — shared build/update pipeline usable by CLI and MCP.

mod call_resolution;
pub mod config;
pub mod paths;

mod build;
mod update;

pub use build::{BuildOptions, BuildSummary, build_graph};
pub use config::Config;
pub use update::{UpdateOptions, UpdateSummary, UpdateTarget, update_graph};
