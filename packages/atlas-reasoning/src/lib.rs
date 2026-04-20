//! Atlas reasoning engine — Phase 23.
//!
//! Answers structural questions from graph + parser + store facts only.
//! No unsupported claims. All results carry structured `ReasoningEvidence`
//! and `ConfidenceTier` so callers can explain decisions.
//!
//! # Entry point
//! ```ignore
//! let engine = ReasoningEngine::new(store);
//! let result = engine.analyze_removal(&["my_crate::foo::bar"], 3, 200)?;
//! ```

mod engine;

pub use engine::ReasoningEngine;
