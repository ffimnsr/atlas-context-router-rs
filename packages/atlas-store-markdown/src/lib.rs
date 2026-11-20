//! Markdown-truth persistence for Atlas continuity records.
//!
//! Durable records live as Markdown files with YAML-shaped frontmatter under
//! `.atlas/`. This crate owns the shared mechanics:
//!
//! - canonical frontmatter parse/serialize with byte-stable output
//!   (fixed field order, stable quoting, verbatim bodies);
//! - bounded frontmatter reads for L0/tiered loading;
//! - atomic writes staged in `.atlas/tmp/` then renamed into place;
//! - advisory writer locks centralized in `.atlas/locks/`;
//! - lock-free reads (rename atomicity protects readers);
//! - a bounded path+mtime frontmatter cache for long-lived processes;
//! - deterministic slugs and content-derived ids.
//!
//! No mutable globals, no `unsafe`, and no lock or temp files are ever placed
//! inside committed record directories.

#![forbid(unsafe_code)]

pub mod atomic;
pub mod cache;
pub mod error;
pub mod frontmatter;
pub mod lock;
pub mod record;
pub mod slug;
pub mod store;
mod value;

pub use cache::FrontmatterCache;
pub use error::MarkdownError;
pub use frontmatter::Frontmatter;
pub use record::{DEFAULT_MAX_FRONTMATTER_BYTES, MarkdownRecord};
pub use store::MarkdownStore;
pub use value::Value;
