//! Typed errors for rank parsing, loading, download, and encoding.

use std::path::PathBuf;

use thiserror::Error;

use crate::Rank;

/// Crate result type.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors produced by `atlas-tokenizer`.
#[derive(Debug, Error)]
pub enum Error {
    /// An encoding name is not known.
    #[error("unknown encoding: {name}")]
    UnknownEncoding {
        /// Requested encoding name.
        name: String,
    },
    /// A model name does not map to a known encoding.
    #[error("unknown model: {model}")]
    UnknownModel {
        /// Requested model name.
        model: String,
    },
    /// The requested encoding was compiled out by feature flags.
    #[error("encoding `{name}` is unavailable: enable the `{feature}` feature")]
    EncodingUnavailable {
        /// Requested encoding name.
        name: String,
        /// Required Cargo feature.
        feature: &'static str,
    },
    /// Named encoding loading is disabled because the `download` feature is
    /// off.
    #[error("encoding `{name}` requires the `download` feature or a local rank file")]
    DownloadDisabled {
        /// Requested encoding name.
        name: String,
    },
    /// The rank file exceeds the configured byte cap.
    #[error("rank file is {actual} bytes, exceeding the {limit} byte cap")]
    RankFileTooLarge {
        /// Actual byte length.
        actual: usize,
        /// Configured cap.
        limit: usize,
    },
    /// A rank file line exceeds the configured line-length cap.
    #[error("rank file line {line} exceeds the {limit} byte line cap")]
    RankLineTooLong {
        /// 1-based line number.
        line: usize,
        /// Configured cap.
        limit: usize,
    },
    /// A rank file contains more entries than the configured cap.
    #[error("rank file has more than {limit} entries")]
    RankEntriesTooMany {
        /// Configured cap.
        limit: usize,
    },
    /// A rank file line is not valid UTF-8.
    #[error("rank file line {line} is not valid UTF-8")]
    RankLineNotUtf8 {
        /// 1-based line number.
        line: usize,
    },
    /// A rank file line does not have exactly two fields.
    #[error("malformed rank file line {line}: expected `base64(token) rank`")]
    MalformedRankLine {
        /// 1-based line number.
        line: usize,
    },
    /// A rank file token is not valid base64.
    #[error("rank file line {line} has invalid base64: {message}")]
    InvalidRankBase64 {
        /// 1-based line number.
        line: usize,
        /// Decoder message without input content.
        message: String,
    },

    /// A rank value is not a valid `u32`.
    #[error("rank file line {line} has invalid rank `{value}`")]
    InvalidRankValue {
        /// 1-based line number.
        line: usize,
        /// Rejected field value.
        value: String,
    },
    /// A rank exceeds the configured rank cap.
    #[error("rank file line {line} rank {rank} exceeds the cap {limit}")]
    RankValueTooHigh {
        /// 1-based line number.
        line: usize,
        /// Rejected rank.
        rank: u32,
        /// Configured inclusive cap.
        limit: u32,
    },
    /// A token appears more than once in a rank file.
    #[error("rank file line {line} repeats an existing token")]
    DuplicateRankToken {
        /// 1-based line number.
        line: usize,
    },
    /// A rank value appears more than once in a rank file.
    #[error("rank file line {line} repeats rank {rank}")]
    DuplicateRankValue {
        /// 1-based line number.
        line: usize,
        /// Repeated rank.
        rank: u32,
    },
    /// A rank file has no entries.
    #[error("rank file contains no entries")]
    EmptyRankFile,
    /// The configured rank cap cannot be indexed on this platform.
    #[error("rank cap {limit} cannot be indexed on this platform")]
    RankLimitOverflow {
        /// Rejected cap.
        limit: Rank,
    },
    /// A rank file does not contain a rank for a single byte.
    #[error("rank file has no rank for byte 0x{byte:02x}")]
    MissingByteRank {
        /// Missing byte value.
        byte: u8,
    },
    /// A token rank does not exist in the encoding.
    #[error("unknown token rank {rank}")]
    UnknownRank {
        /// Unknown rank.
        rank: u32,
    },
    /// Decoded bytes are not valid UTF-8.
    #[error("decoded token bytes are not valid UTF-8")]
    InvalidUtf8Output,
    /// An encoding regex failed to compile.
    #[error("invalid encoding regex: {message}")]
    InvalidRegex {
        /// Compiler message.
        message: String,
    },
    /// A download URL is not HTTPS.
    #[error("refusing non-HTTPS download URL: {url}")]
    InsecureDownloadUrl {
        /// Rejected URL.
        url: String,
    },
    /// A redirect target is not HTTPS.
    #[error("refusing redirect to non-HTTPS URL: {url}")]
    InsecureRedirect {
        /// Redirect target.
        url: String,
    },
    /// A download response exceeds the size cap.
    #[error("download exceeds the {limit} byte cap")]
    DownloadTooLarge {
        /// Configured cap.
        limit: usize,
    },
    /// A downloaded or cached asset has the wrong sha256 digest.
    #[error("sha256 mismatch: expected {expected}, got {actual}")]
    HashMismatch {
        /// Expected lowercase hex digest.
        expected: String,
        /// Actual lowercase hex digest.
        actual: String,
    },
    /// The HTTP request failed before a response could be checked.
    #[error("download request for {url} failed: {message}")]
    DownloadRequest {
        /// Requested URL.
        url: String,
        /// Transport error without payload content.
        message: String,
    },
    /// The server returned a non-success HTTP status.
    #[error("download for {url} returned HTTP {status}")]
    HttpStatus {
        /// Requested URL.
        url: String,
        /// HTTP status code.
        status: u16,
    },
    /// Reading or writing a cache entry failed.
    #[error("cache I/O failed at {path}: {source}")]
    CacheIo {
        /// Cache path involved in the failure.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// Input contains a special-token literal that is not allowed.
    #[error("disallowed special token `{token}`")]
    DisallowedSpecialToken {
        /// Special-token literal found in the input.
        token: String,
    },
    /// A special-token definition is empty.
    #[error("special token `{name}` must not be empty")]
    EmptySpecialToken {
        /// Special-token name.
        name: String,
    },
    /// A special-token definition repeats another special-token name.
    #[error("duplicate special token `{name}`")]
    DuplicateSpecialToken {
        /// Repeated special-token literal.
        name: String,
    },
    /// A special-token rank collides with an ordinary token rank.
    #[error("special token `{name}` rank {rank} collides with an ordinary token")]
    SpecialTokenRankCollision {
        /// Special-token literal.
        name: String,
        /// Colliding rank.
        rank: Rank,
    },
    /// Reading a rank file failed.
    #[error("cannot read rank file {path}: {source}")]
    ReadRankFile {
        /// Path that failed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
}

impl Error {
    /// Construct an [`Error::UnknownEncoding`].
    pub fn unknown_encoding(name: impl Into<String>) -> Self {
        Self::UnknownEncoding { name: name.into() }
    }

    /// Construct an [`Error::UnknownModel`].
    pub fn unknown_model(model: impl Into<String>) -> Self {
        Self::UnknownModel {
            model: model.into(),
        }
    }
}
