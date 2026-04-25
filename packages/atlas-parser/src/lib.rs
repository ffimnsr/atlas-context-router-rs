//! Parser registry and tree-cache crate.
//!
//! This crate does not open SQLite connections and has no database ownership
//! role. It only parses source bytes and manages parser/tree-cache state, so it
//! is outside Atlas SQLite sharing and thread-confinement risk.

pub mod ast_helpers;
pub mod lang;
pub mod parse_runtime;
pub mod registry;
pub mod traits;
pub mod tree_cache;

pub use registry::ParserRegistry;
pub use traits::{LangParser, ParseContext};
pub use tree_cache::TreeCache;
