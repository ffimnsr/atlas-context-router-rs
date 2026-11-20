use std::io;
use std::path::PathBuf;

use thiserror::Error;

/// Errors raised by the markdown store.
#[derive(Debug, Error)]
pub enum MarkdownError {
    /// File system failure tied to an explicit path.
    #[error("io error at {path}: {source}")]
    Io {
        /// Path the operation touched.
        path: PathBuf,
        /// Underlying IO error.
        #[source]
        source: io::Error,
    },
    /// Frontmatter start or end fence is missing.
    #[error("missing frontmatter fence at {path}")]
    MissingFence {
        /// Record path.
        path: PathBuf,
    },
    /// Malformed frontmatter line.
    #[error("invalid frontmatter at {path} line {line}: {message}")]
    InvalidFrontmatter {
        /// Record path.
        path: PathBuf,
        /// 1-based line number inside the frontmatter block.
        line: usize,
        /// Human-readable reason.
        message: String,
    },
    /// Frontmatter exceeded the bounded-read limit.
    #[error("frontmatter at {path} exceeds {limit} bytes")]
    FrontmatterTooLarge {
        /// Record path.
        path: PathBuf,
        /// Configured limit.
        limit: usize,
    },
    /// Record body was not valid UTF-8.
    #[error("record body at {path} is not valid utf-8")]
    InvalidUtf8 {
        /// Record path.
        path: PathBuf,
    },
    /// A frontmatter value could not be rendered or interpreted.
    #[error("invalid value for field `{field}`: {message}")]
    InvalidValue {
        /// Field name.
        field: String,
        /// Human-readable reason.
        message: String,
    },
    /// Writer lock failure.
    #[error("lock error at {path}: {message}")]
    Lock {
        /// Lock file path.
        path: PathBuf,
        /// Human-readable reason.
        message: String,
    },
    /// Path escaped the store root or was absolute.
    #[error("unsafe relative path: {path}")]
    UnsafePath {
        /// Rejected path.
        path: PathBuf,
    },
}

impl MarkdownError {
    /// Build an IO error bound to a path.
    pub fn io(path: impl Into<PathBuf>, source: io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}
