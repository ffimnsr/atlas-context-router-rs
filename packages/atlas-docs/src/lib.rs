//! Markdown documentation generation and dependency diagram export for Atlas
//! code graphs.
//!
//! `atlas-docs` turns a persisted graph (plus derived insight data such as
//! inferred modules, component labels, and duplicate groups) into a
//! deterministic set of Markdown files (`atlas docs generate`) and dependency
//! diagrams in Mermaid or Graphviz DOT form (`atlas docs export`).
//!
//! Rendering is pure and deterministic: every output is derived from sorted
//! inputs and a caller-supplied timestamp, so snapshots are stable across
//! runs and test fixtures.

pub mod context;
pub mod export;
pub mod generate;
pub mod model;

pub use context::{load_docs_context, load_docs_context_with_timestamp};
pub use export::{DocsExportFormat, DocsExportScope, ExportRequest, ExportResult, export_diagram};
pub use generate::generate_docs;
pub use model::{DocsData, DocsView};
