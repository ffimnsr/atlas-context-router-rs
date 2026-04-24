use std::collections::HashSet;

use atlas_core::{Node, NodeKind, ReferenceScope, RenamePreviewResult, RenameReference, Result};

use super::{
    ReasoningEngine,
    helpers::{EDGE_QUERY_LIMIT, normalize_qn_kind_tokens, rename_risk, same_module},
};

impl<'s> ReasoningEngine<'s> {
    /// Preview all references that would need updating when renaming `qname`
    /// to `new_name`.
    pub fn preview_rename_radius(
        &self,
        qname: &str,
        new_name: &str,
    ) -> Result<RenamePreviewResult> {
        let qname = normalize_qn_kind_tokens(qname);
        let target = match self.store.node_by_qname(&qname)? {
            Some(node) => node,
            None => {
                return Err(atlas_core::AtlasError::Other(format!(
                    "node not found: {qname}"
                )));
            }
        };

        let inbound = self.store.inbound_edges(&qname, EDGE_QUERY_LIMIT)?;

        let mut affected_references: Vec<RenameReference> = Vec::new();
        let mut affected_files: HashSet<String> = HashSet::new();
        let mut manual_review_flags: Vec<String> = Vec::new();

        for (ref_node, edge) in inbound {
            if ref_node.is_test || ref_node.kind == NodeKind::Test {
                affected_files.insert(ref_node.file_path.clone());
                affected_references.push(RenameReference {
                    node: ref_node,
                    edge,
                    scope: ReferenceScope::Test,
                });
                continue;
            }

            let scope = if ref_node.file_path == target.file_path {
                ReferenceScope::SameFile
            } else if same_module(&ref_node, &target) {
                ReferenceScope::SameModule
            } else {
                ReferenceScope::CrossModule
            };

            if edge.confidence < 0.5 {
                manual_review_flags.push(format!(
                    "unresolved reference in `{}` — verify manually",
                    ref_node.file_path
                ));
            }

            affected_files.insert(ref_node.file_path.clone());
            affected_references.push(RenameReference {
                node: ref_node,
                edge,
                scope,
            });
        }

        let collision_warnings = self.detect_name_collisions(new_name, &target)?;
        let risk_level = rename_risk(
            &target,
            affected_references.len(),
            manual_review_flags.len(),
            !collision_warnings.is_empty(),
        );

        let mut affected_file_list: Vec<String> = affected_files.into_iter().collect();
        affected_file_list.sort();

        Ok(RenamePreviewResult {
            target,
            new_name: new_name.to_owned(),
            affected_references,
            affected_files: affected_file_list,
            risk_level,
            collision_warnings,
            manual_review_flags,
        })
    }

    /// Check whether any node in the same file has `name == new_name` and
    /// could collide with the rename target.
    fn detect_name_collisions(&self, new_name: &str, target: &Node) -> Result<Vec<String>> {
        let file_nodes = self.store.nodes_by_file(&target.file_path)?;
        let warnings: Vec<String> = file_nodes
            .iter()
            .filter(|node| node.name == new_name && node.qualified_name != target.qualified_name)
            .map(|node| {
                format!(
                    "name `{new_name}` already exists in `{}` as `{}`",
                    target.file_path, node.qualified_name
                )
            })
            .collect();
        Ok(warnings)
    }
}
