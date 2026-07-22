mod analysis;
mod context_ops;
mod dispatch;
mod graph;
pub mod health;
mod inventory;
mod manual;
mod postprocess;
mod registry;
mod shared;

pub use dispatch::call;
pub(crate) use dispatch::is_known_tool_name;
pub use manual::{ToolManualDocument, render_tool_manual_text, tool_manual};
pub(crate) use registry::tool_descriptors;
pub use registry::{tool_list, tool_list_markdown};
pub(crate) use shared::parse_mcp_intent;

#[cfg(test)]
mod tests;
