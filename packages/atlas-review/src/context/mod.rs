// Phase 22 — Context Engine: Slices 3, 4, 5, 6, 8
// Phase CM6 — Retrieval-backed restoration
//
// Slice 3: resolve_target
// Slice 4: build_symbol_context
// Slice 5: rank_context / trim_context
// Slice 6: build_review_context / build_impact_context
// Slice 8: apply_code_spans
// CM6: retrieve_saved_context

pub(super) use std::collections::{HashMap, HashSet};

pub(super) use atlas_contentstore::{
    ContentStore,
    store::{SearchFilters, SourceRow},
};
pub(super) use atlas_core::{
    Result,
    model::{
        AmbiguityMeta, ContextRequest, ContextResult, ContextTarget, NoiseReductionSummary,
        SavedContextSource, SelectedEdge, SelectedFile, SelectedNode, SelectionReason,
        TruncationMeta, WorkflowCallChain, WorkflowComponent, WorkflowFocusNode, WorkflowSummary,
    },
};
pub(super) use atlas_store_sqlite::Store;

mod build;
mod rank;
mod resolve;
mod saved;
mod spans;
mod symbol;
#[cfg(test)]
mod tests;
mod workflow;

pub use self::build::build_context;
pub use self::resolve::{ResolvedTarget, normalize_qn_kind_tokens, resolve_target};

use self::rank::{rank_context, trim_context};
use self::saved::retrieve_saved_context;
use self::spans::apply_code_spans;
use self::symbol::{
    build_ambiguous_result, build_not_found_result, build_symbol_context, collect_files,
    update_file_node_counts,
};
use self::workflow::build_workflow_summary;

/// Default caps when the request does not specify limits.
/// Shared trimming primitive lives in `crate::ranking::TrimmingPrimitives`.
pub(super) const DEFAULT_MAX_NODES: usize = 50;
pub(super) const DEFAULT_MAX_EDGES: usize = 100;
pub(super) const DEFAULT_MAX_FILES: usize = 20;

/// Per-bucket limits fed to store helpers.
pub(super) const BUCKET_CALLERS: usize = 15;
pub(super) const BUCKET_CALLEES: usize = 15;
pub(super) const BUCKET_IMPORTS: usize = 10;
pub(super) const BUCKET_SIBLINGS: usize = 10;
pub(super) const BUCKET_TESTS: usize = 10;

/// Stateless context engine facade.
///
/// Wraps the free-function pipeline into a struct so callers inject one
/// [`Store`] reference and call engine operations as methods.  An optional
/// [`ContentStore`] enables CM6 saved-context retrieval.
pub struct ContextEngine<'a> {
    store: &'a Store,
    /// Optional content store for CM6 retrieval-backed restoration.
    content_store: Option<&'a ContentStore>,
}

impl<'a> ContextEngine<'a> {
    /// Create a new engine backed by `store`.
    pub fn new(store: &'a Store) -> Self {
        Self {
            store,
            content_store: None,
        }
    }

    /// Attach a content store to enable saved-context retrieval (CM6).
    ///
    /// When attached and `request.include_saved_context` is `true`, the engine
    /// queries the content store for relevant saved artifacts after graph
    /// retrieval and merges them into `ContextResult::saved_context_sources`.
    pub fn with_content_store(mut self, cs: &'a ContentStore) -> Self {
        self.content_store = Some(cs);
        self
    }

    /// Resolve a [`ContextTarget`] to a concrete node, file, or ambiguity result.
    pub fn resolve(&self, target: &ContextTarget) -> Result<ResolvedTarget> {
        resolve_target(self.store, target)
    }

    /// Build a bounded [`ContextResult`] for the given request.
    ///
    /// Routes by intent (Review / Impact / Symbol / File), resolves target,
    /// retrieves neighbors, ranks, trims, and optionally applies code spans.
    /// When `request.include_saved_context` is `true` and a content store is
    /// attached, also populates `saved_context_sources` (CM6).
    pub fn build(&self, request: &ContextRequest) -> Result<ContextResult> {
        let mut result = build_context(self.store, request)?;
        if request.include_saved_context
            && let Some(cs) = self.content_store
        {
            result.saved_context_sources = retrieve_saved_context(cs, request, &result);
        }
        Ok(result)
    }
}
