use std::collections::HashSet;

use atlas_core::{ConfidenceTier, DeadCodeCandidate, Node, NodeKind, Result};

use super::{
    ReasoningEngine,
    helpers::{ENTRYPOINT_NAMES, dead_code_reasons},
};

/// Blocking reason for a method declared directly on a trait.
pub(super) const TRAIT_DECLARATION_BLOCKER: &str = "trait method declaration: implementations are reached through trait dispatch, not inbound edges";

/// Blocking reason for a method whose parent `impl` block targets a trait.
///
/// Trait methods are reached through trait dispatch, which Rust call extraction
/// does not turn into inbound edges.
pub(super) const TRAIT_DISPATCH_BLOCKER: &str = "trait implementation method: call sites may dispatch through the trait and are not recorded as inbound edges";

/// Blocking reason for a method whose call sites use the parser's short target
/// form instead of the file-qualified qname.
pub(super) const SHORT_TARGET_BLOCKER: &str = "inbound call sites target the short `Type::method` form, so zero inbound edges does not prove zero callers";

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

        let mut candidates = raw
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
            .collect::<Vec<_>>();

        self.downgrade_unresolved_methods(&mut candidates)?;
        Ok(candidates)
    }

    /// Receiver method calls resolve to `Type::method` (or `Type.method()`)
    /// while method nodes are registered as `file::method::Type::method`, so the
    /// store's exact-qname anti-join cannot see them. Trait implementations are
    /// unreachable for the same reason. Downgrade those candidates instead of
    /// reporting them as definite dead code.
    fn downgrade_unresolved_methods(&self, candidates: &mut [DeadCodeCandidate]) -> Result<()> {
        let mut trait_declarations = HashSet::new();
        let method_indexes = candidates
            .iter()
            .enumerate()
            .filter(|(_, candidate)| {
                candidate.node.kind == NodeKind::Method
                    && candidate.certainty == ConfidenceTier::High
            })
            .filter_map(|(index, candidate)| {
                if is_trait_declaration(&candidate.node) {
                    trait_declarations.insert(index);
                    return None;
                }
                Some(index)
            })
            .collect::<Vec<_>>();
        for index in trait_declarations {
            let candidate = &mut candidates[index];
            candidate.certainty = ConfidenceTier::Medium;
            if !candidate
                .blockers
                .iter()
                .any(|existing| existing == TRAIT_DECLARATION_BLOCKER)
            {
                candidate
                    .blockers
                    .push(TRAIT_DECLARATION_BLOCKER.to_owned());
            }
        }
        if method_indexes.is_empty() {
            return Ok(());
        }

        let method_qnames = method_indexes
            .iter()
            .map(|index| candidates[*index].node.qualified_name.as_str())
            .collect::<Vec<_>>();
        let trait_impls = self.store.trait_impl_method_qnames(&method_qnames)?;

        let mut probe_targets = Vec::new();
        let mut probe_owner = Vec::new();
        for index in &method_indexes {
            for target in method_short_targets(&candidates[*index].node) {
                probe_targets.push(target);
                probe_owner.push(*index);
            }
        }
        let probe_refs = probe_targets.iter().map(String::as_str).collect::<Vec<_>>();
        let resolved = self.store.inbound_semantic_edge_targets(&probe_refs)?;
        let mut resolved_indexes = HashSet::new();
        for (position, target) in probe_targets.iter().enumerate() {
            if resolved.contains(target) {
                resolved_indexes.insert(probe_owner[position]);
            }
        }

        for index in method_indexes {
            let candidate = &mut candidates[index];
            let blocker = if trait_impls.contains(&candidate.node.qualified_name) {
                TRAIT_DISPATCH_BLOCKER
            } else if resolved_indexes.contains(&index) {
                SHORT_TARGET_BLOCKER
            } else {
                continue;
            };
            candidate.certainty = ConfidenceTier::Medium;
            if !candidate
                .blockers
                .iter()
                .any(|existing| existing == blocker)
            {
                candidate.blockers.push(blocker.to_owned());
            }
        }
        Ok(())
    }
}

/// Short call-target spellings the Rust extractor emits for receiver calls on
/// `<Type>`: `Type::method` and `Type.method()`.
pub(super) fn method_short_targets(node: &Node) -> Vec<String> {
    let Some(type_name) = owning_impl_type(node) else {
        return Vec::new();
    };
    vec![
        format!("{type_name}::{}", node.name),
        format!("{type_name}.{}()", node.name),
    ]
}

/// Extract `<Type>` from `parent_name` = `<file>::impl::<Type>`.
fn owning_impl_type(node: &Node) -> Option<&str> {
    let parent = node.parent_name.as_deref()?;
    let (_, type_name) = parent.rsplit_once("::impl::")?;
    if type_name.is_empty() {
        return None;
    }
    Some(type_name)
}

/// True when `node` is a method declared in a `trait` block (`parent_name` =
/// `<file>::trait::<Trait>`).
fn is_trait_declaration(node: &Node) -> bool {
    node.parent_name
        .as_deref()
        .is_some_and(|parent| parent.contains("::trait::"))
}
