mod analysis;
mod context_ops;
mod dispatch;
mod graph;
pub mod health;
mod postprocess;
mod registry;
mod shared;

pub use dispatch::call;
pub(crate) use registry::tool_descriptors;
pub use registry::{tool_list, tool_list_markdown};
pub(crate) use shared::parse_mcp_intent;

#[cfg(test)]
mod tests;
