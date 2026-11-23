use super::*;

/// Which diagram format `atlas docs export` renders.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum DocsFormat {
    /// Mermaid `flowchart LR` diagram.
    Mermaid,
    /// Graphviz DOT source.
    Dot,
}

impl From<DocsFormat> for atlas_docs::DocsExportFormat {
    fn from(value: DocsFormat) -> Self {
        match value {
            DocsFormat::Mermaid => Self::Mermaid,
            DocsFormat::Dot => Self::Dot,
        }
    }
}

/// Which part of the repository `atlas docs export` describes.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub enum DocsScope {
    /// Whole-repo module dependency graph (file graph when no modules).
    Repo,
    /// One inferred module's internal symbol graph.
    Module,
    /// One component label's file graph.
    Component,
    /// One file's internal symbol graph.
    File,
    /// One symbol and its direct neighbors.
    Symbol,
}

impl From<DocsScope> for atlas_docs::DocsExportScope {
    fn from(value: DocsScope) -> Self {
        match value {
            DocsScope::Repo => Self::Repo,
            DocsScope::Module => Self::Module,
            DocsScope::Component => Self::Component,
            DocsScope::File => Self::File,
            DocsScope::Symbol => Self::Symbol,
        }
    }
}

/// Sub-commands for `atlas docs`.
#[derive(Debug, Subcommand)]
pub enum DocsCommand {
    /// Generate deterministic Markdown documentation from the graph.
    Generate {
        /// Output directory for the generated Markdown files (default: `.atlas/docs`).
        #[arg(long)]
        output: Option<String>,

        /// Override the generated timestamp (RFC 3339) for reproducible output.
        #[arg(long)]
        timestamp: Option<String>,

        /// Embed Mermaid dependency diagrams into the generated files.
        #[arg(long)]
        include_diagrams: bool,
    },

    /// Export a dependency diagram (Mermaid or Graphviz DOT) to stdout or a file.
    Export {
        /// Diagram format.
        #[arg(long, value_enum, default_value_t = DocsFormat::Mermaid)]
        format: DocsFormat,

        /// Export scope.
        #[arg(long, value_enum, default_value_t = DocsScope::Repo)]
        scope: DocsScope,

        /// Target name for module, component, file, and symbol scopes.
        #[arg(long)]
        name: Option<String>,

        /// Cap rendered nodes; extra nodes are omitted and reported.
        #[arg(long, default_value_t = 200)]
        max_nodes: usize,

        /// Cap rendered edges; extra edges are omitted and reported.
        #[arg(long, default_value_t = 400)]
        max_edges: usize,

        /// Write the diagram to this file instead of stdout.
        #[arg(long)]
        output: Option<String>,
    },
}
