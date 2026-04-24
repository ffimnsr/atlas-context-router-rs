use std::collections::HashSet;

use atlas_core::{DeadCodeCandidate, NodeKind, Result};

use super::{
    ReasoningEngine,
    helpers::{ENTRYPOINT_NAMES, dead_code_reasons},
};

impl<'s> ReasoningEngine<'s> {
    /// Detect dead-code candidates: nodes with no inbound semantic edges, not
    /// public/exported, not a test, not in the entrypoint allowlist.
    ///
    /// The store pre-filters on visibility and edge absence; this method
    /// applies the remaining suppression logic plus certainty assignment.
    ///
    /// By default only code symbols are returned (functions, methods,
    /// structs/types, traits, enums, interfaces, constants, variables) — this
    /// is enforced at the store query level and matches the `code_only`
    /// semantic. Pass `exclude_kinds` to further narrow the result set.
    pub fn detect_dead_code(
        &self,
        extra_allowlist: &[&str],
        subpath: Option<&str>,
        limit: Option<usize>,
        exclude_kinds: &[NodeKind],
    ) -> Result<Vec<DeadCodeCandidate>> {
        let cap = limit.unwrap_or(500);
        let raw = self.store.dead_code_candidates_filtered(subpath, cap)?;

        let allowlist_set: HashSet<&str> = extra_allowlist.iter().copied().collect();
        let exclude_set: HashSet<NodeKind> = exclude_kinds.iter().copied().collect();

        let candidates = raw
            .into_iter()
            .filter_map(|node| {
                if ENTRYPOINT_NAMES.contains(&node.name.as_str()) {
                    return None;
                }
                if allowlist_set.contains(node.qualified_name.as_str()) {
                    return None;
                }
                if exclude_set.contains(&node.kind) {
                    return None;
                }

                let (reasons, certainty, blockers) = dead_code_reasons(&node);
                Some(DeadCodeCandidate {
                    node,
                    reasons,
                    certainty,
                    blockers,
                })
            })
            .collect();

        Ok(candidates)
    }
}
