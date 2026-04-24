use thiserror::Error;

#[derive(Debug, Error)]
pub enum AtlasError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("database error: {0}")]
    Db(String),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("repo root not found starting from {0}")]
    RepoRootNotFound(String),

    #[error("unsupported language for file: {0}")]
    UnsupportedLanguage(String),

    #[error("parse error in {file}: {message}")]
    ParseError { file: String, message: String },

    #[error("database not initialized; run `atlas init` first")]
    DbNotInitialized,

    #[error("chunk cap exceeded: {0}")]
    ChunkCapExceeded(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, AtlasError>;
