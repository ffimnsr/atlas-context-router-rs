mod analysis;
mod analysis_bounds;
mod analysis_payloads;
mod config_defaults;
mod context_ops;
mod dispatch;
mod graph;
mod graph_debug;
mod graph_stats;
pub mod health;
mod inventory;
mod manual;
mod manual_bounds;
mod metrics;
mod postprocess;
mod registry;
mod resolve;
pub(crate) mod shared;
mod symbols;
mod trace;
mod traversal_wire;

pub use dispatch::call;
pub(crate) use dispatch::{call_with_worker_threads, is_known_tool_name};
pub(crate) use graph::{tool_concept_clusters, tool_cross_file_links, tool_symbol_neighbors};
pub use manual::{ToolManualDocument, render_tool_manual_text, tool_manual};
pub(crate) use registry::tool_descriptors;
pub use registry::{tool_list, tool_list_markdown};
pub(crate) use shared::parse_mcp_intent;

#[cfg(test)]
mod tests;
