//! OpenAI `tiktoken`-compatible byte-pair tokenizer.
//!
//! This crate is a standalone port of the upstream `tiktoken` core. It has no
//! `atlas-*` dependencies, forbids unsafe code, and never panics on untrusted
//! rank files or input text.
//!
//! Vocabulary assets are fetched from pinned upstream HTTPS URLs, verified
//! with sha256, and cached using the upstream `sha1(url)` cache key. Assets are
//! never committed to this repository. See the crate `README.md` for the
//! pinned upstream commit, feature flags, and offline behavior.
//!
//! # Features
//!
//! - `encoding-r50k`, `encoding-p50k`, `encoding-cl100k`, and
//!   `encoding-o200k` enable the corresponding encoding families. They are on
//!   by default. `encoding-o200k-harmony` adds the harmony special tokens and
//!   is off by default.
//! - `download` (default) enables HTTPS asset download and cache population.
//!   With this feature off, only rank files and byte inputs can be loaded.
//! - `file-assets` (default) enables loading custom `.tiktoken` rank files.
//! - `batch` (off) enables optional rayon batch helpers.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

pub mod assets;
#[cfg(feature = "batch")]
pub mod batch;
pub mod bpe;
pub mod cache;
#[cfg(feature = "download")]
pub mod download;
pub mod encoding;
pub mod error;
pub mod model;
pub mod rank_file;
pub mod regex_split;
pub mod registry;

pub use encoding::{Encoding, EncodingName, EncodingSpec};
pub use error::{Error, Result};
pub use model::encoding_name_for_model;
pub use registry::{EncodingLoader, default_loader};

/// Token rank stored in a `.tiktoken` vocabulary.
pub type Rank = u32;
