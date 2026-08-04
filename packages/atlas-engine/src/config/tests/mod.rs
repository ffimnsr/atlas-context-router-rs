//! Config unit tests, grouped by surface: transport (mcp/http-auth/embedding),
//! loading, template rendering, and insights/layer-rules validation.

use tempfile::tempdir;

mod insights;
mod load;
mod mcp;
mod template;
