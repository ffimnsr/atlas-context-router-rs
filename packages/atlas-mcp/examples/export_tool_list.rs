//! Regenerates the checked-in `MCP_TOOLS.md` inventory.
//!
//! Usage: `cargo run -p atlas-mcp --example export_tool_list > MCP_TOOLS.md`

fn main() {
    print!("{}", atlas_mcp::tool_list_markdown());
}
