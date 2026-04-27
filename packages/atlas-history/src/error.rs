use thiserror::Error;

/// Typed error for all public `atlas-history` operations.
///
/// Convention: library crates expose typed errors; CLI / MCP entry points use
/// `anyhow::Result` at command boundaries and add context via `.context(...)`.
///
/// All internal helpers keep `anyhow::Result` for ergonomics.  `?` at public
/// function boundaries auto-converts via `From<anyhow::Error>`.
#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("history not initialized: run `atlas history build` first")]
    NotInitialized,

    #[error("divergence detected: {0}")]
    Divergence(String),

    #[error("invalid selector: {0}")]
    InvalidSelector(String),

    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for HistoryError {
    fn from(e: anyhow::Error) -> Self {
        Self::Other(format!("{e:#}"))
    }
}

/// Convenience alias used by all public `atlas-history` APIs.
pub type Result<T> = std::result::Result<T, HistoryError>;
