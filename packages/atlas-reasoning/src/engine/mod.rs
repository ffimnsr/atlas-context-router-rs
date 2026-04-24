//! `ReasoningEngine` — Phase 23 implementation.
//!
//! All methods operate on graph data from the Store; no external inference.
//! Results are deterministic given the same graph state.

mod dead_code;
mod helpers;
mod removal;
mod rename;
mod risk;

#[cfg(test)]
mod tests;

use atlas_store_sqlite::Store;

/// Provides autonomous code-reasoning queries backed by the Atlas graph store.
///
/// Wrap a `Store` reference; all methods borrow it immutably.
pub struct ReasoningEngine<'s> {
    store: &'s Store,
}

impl<'s> ReasoningEngine<'s> {
    /// Create a new engine from a shared store reference.
    pub fn new(store: &'s Store) -> Self {
        Self { store }
    }
}
