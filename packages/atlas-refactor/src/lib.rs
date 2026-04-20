//! Atlas refactoring engine — Phase 24.
//!
//! Deterministic, syntax-aware transforms backed by graph validation.
//! Supports rename, dead-code removal, import cleanup, and
//! extract-function candidate detection.
//!
//! # Entry point
//! ```ignore
//! let engine = RefactorEngine::new(store, repo_root);
//! let plan = engine.plan_rename("my_crate::foo::bar", "baz")?;
//! let result = engine.apply_rename(&plan, /*dry_run=*/true)?;
//! println!("{}", result.patches[0].unified_diff);
//! ```

mod edits;
mod engine;
mod extract;
mod patch;

pub use engine::RefactorEngine;
