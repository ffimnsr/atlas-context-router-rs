use std::collections::HashMap;
use tree_sitter::Node as TsNode;

use crate::ast_helpers::node_text;
use crate::query_helpers::{QueryCaptureGroup, compile_query, run_query};

use super::RUST_DEFINITION_QUERY;

// ---------------------------------------------------------------------------
// Query-backed definition extraction
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RustItemKind {
    Function,
    FunctionSignature,
    Module,
    Struct,
    Enum,
    Trait,
    Const,
    Static,
    Impl,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RustImpl<'tree> {
    pub(super) node: TsNode<'tree>,
    pub(super) type_node: TsNode<'tree>,
    pub(super) trait_node: Option<TsNode<'tree>>,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RustItem<'tree> {
    pub(super) kind: RustItemKind,
    pub(super) node: TsNode<'tree>,
    pub(super) name_node: Option<TsNode<'tree>>,
    pub(super) rust_impl: Option<RustImpl<'tree>>,
}

#[derive(Debug)]
pub(super) struct RustSyntaxFacts<'tree> {
    pub(super) items: Vec<RustItem<'tree>>,
    pub(super) _impls: Vec<RustImpl<'tree>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(super) struct RustNodeKey {
    start_byte: usize,
    end_byte: usize,
}

impl RustItemKind {
    fn from_definition_capture(name: &str) -> Option<Self> {
        match name {
            "atlas.definition.function" => Some(Self::Function),
            "atlas.definition.function_signature" => Some(Self::FunctionSignature),
            "atlas.definition.module" => Some(Self::Module),
            "atlas.definition.struct" => Some(Self::Struct),
            "atlas.definition.enum" => Some(Self::Enum),
            "atlas.definition.trait" => Some(Self::Trait),
            "atlas.definition.const" => Some(Self::Const),
            "atlas.definition.static" => Some(Self::Static),
            "atlas.definition.impl" => Some(Self::Impl),
            _ => None,
        }
    }
}

impl<'tree> RustItem<'tree> {
    fn from_capture_group(group: &QueryCaptureGroup<'tree>) -> Result<Option<Self>, String> {
        let _ = group.pattern_index;
        let mut kind = None;
        let mut definition_node = None;
        let mut name_node = None;
        let mut impl_type_node = None;
        let mut impl_trait_node = None;

        for capture in &group.captures {
            if let Some(capture_kind) = RustItemKind::from_definition_capture(&capture.name) {
                kind = Some(capture_kind);
                definition_node = Some(capture.node);
                continue;
            }

            match capture.name.as_str() {
                "atlas.name" => name_node = Some(capture.node),
                "atlas.impl.type" => impl_type_node = Some(capture.node),
                "atlas.impl.trait" => impl_trait_node = Some(capture.node),
                _ => {}
            }
        }

        let Some(kind) = kind else {
            return Ok(None);
        };
        let definition_node = definition_node
            .ok_or_else(|| "rust query match missing definition capture".to_owned())?;
        let rust_impl = if kind == RustItemKind::Impl {
            Some(RustImpl {
                node: definition_node,
                type_node: impl_type_node
                    .ok_or_else(|| "rust impl query match missing @atlas.impl.type".to_owned())?,
                trait_node: impl_trait_node
                    .or_else(|| definition_node.child_by_field_name("trait")),
            })
        } else {
            None
        };

        Ok(Some(Self {
            kind,
            node: definition_node,
            name_node,
            rust_impl,
        }))
    }

    pub(super) fn name_text<'s>(&self, source: &'s [u8]) -> Option<&'s str> {
        self.name_node.map(|node| node_text(node, source))
    }
}

impl<'tree> RustSyntaxFacts<'tree> {
    pub(super) fn extract(root: TsNode<'tree>, source: &'tree [u8]) -> Result<Self, String> {
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let query = compile_query(language, RUST_DEFINITION_QUERY)?;
        let matches = run_query(&query, root, source);
        let impl_trait_captures = collect_impl_trait_captures(&matches);
        let mut items = Vec::new();
        let mut impls = Vec::new();

        for group in matches {
            let Some(mut item) = RustItem::from_capture_group(&group)? else {
                continue;
            };
            if let Some(rust_impl) = &mut item.rust_impl
                && let Some(trait_node) = impl_trait_captures.get(&node_key(rust_impl.node))
            {
                rust_impl.trait_node = Some(*trait_node);
            }
            if let Some(rust_impl) = item.rust_impl {
                impls.push(rust_impl);
            }
            items.push(item);
        }

        if items.is_empty() {
            items = collect_fallback_rust_items(root);
            impls = items.iter().filter_map(|item| item.rust_impl).collect();
        }

        items.sort_by_key(|item| (item.node.start_byte(), item.node.end_byte()));

        Ok(Self {
            items,
            _impls: impls,
        })
    }
}

pub(super) fn node_key(node: TsNode<'_>) -> RustNodeKey {
    RustNodeKey {
        start_byte: node.start_byte(),
        end_byte: node.end_byte(),
    }
}

fn collect_impl_trait_captures<'tree>(
    matches: &[QueryCaptureGroup<'tree>],
) -> HashMap<RustNodeKey, TsNode<'tree>> {
    let mut trait_captures = HashMap::new();

    for group in matches {
        let mut impl_node = None;
        let mut trait_node = None;

        for capture in &group.captures {
            match capture.name.as_str() {
                "atlas.impl.item" => impl_node = Some(capture.node),
                "atlas.impl.trait" => trait_node = Some(capture.node),
                _ => {}
            }
        }

        if let (Some(impl_node), Some(trait_node)) = (impl_node, trait_node) {
            trait_captures.insert(node_key(impl_node), trait_node);
        }
    }

    trait_captures
}

fn collect_fallback_rust_items(root: TsNode<'_>) -> Vec<RustItem<'_>> {
    let mut items = Vec::new();
    collect_fallback_rust_items_inner(root, &mut items);
    items
}

fn collect_fallback_rust_items_inner<'tree>(node: TsNode<'tree>, items: &mut Vec<RustItem<'tree>>) {
    let fallback_item = match node.kind() {
        "function_item" => Some(RustItem {
            kind: RustItemKind::Function,
            node,
            name_node: node.child_by_field_name("name"),
            rust_impl: None,
        }),
        "function_signature_item" => Some(RustItem {
            kind: RustItemKind::FunctionSignature,
            node,
            name_node: node.child_by_field_name("name"),
            rust_impl: None,
        }),
        "mod_item" => Some(RustItem {
            kind: RustItemKind::Module,
            node,
            name_node: node.child_by_field_name("name"),
            rust_impl: None,
        }),
        "struct_item" => Some(RustItem {
            kind: RustItemKind::Struct,
            node,
            name_node: node.child_by_field_name("name"),
            rust_impl: None,
        }),
        "enum_item" => Some(RustItem {
            kind: RustItemKind::Enum,
            node,
            name_node: node.child_by_field_name("name"),
            rust_impl: None,
        }),
        "trait_item" => Some(RustItem {
            kind: RustItemKind::Trait,
            node,
            name_node: node.child_by_field_name("name"),
            rust_impl: None,
        }),
        "const_item" => Some(RustItem {
            kind: RustItemKind::Const,
            node,
            name_node: node.child_by_field_name("name"),
            rust_impl: None,
        }),
        "static_item" => Some(RustItem {
            kind: RustItemKind::Static,
            node,
            name_node: node.child_by_field_name("name"),
            rust_impl: None,
        }),
        "impl_item" => node.child_by_field_name("type").map(|type_node| RustItem {
            kind: RustItemKind::Impl,
            node,
            name_node: None,
            rust_impl: Some(RustImpl {
                node,
                type_node,
                trait_node: node.child_by_field_name("trait"),
            }),
        }),
        _ => None,
    };

    if let Some(item) = fallback_item {
        items.push(item);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_fallback_rust_items_inner(child, items);
    }
}
pub(super) fn rust_query_matches<'tree>(
    root: TsNode<'tree>,
    source: &'tree [u8],
) -> Result<Vec<QueryCaptureGroup<'tree>>, String> {
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let query = compile_query(language, RUST_DEFINITION_QUERY)?;
    Ok(run_query(&query, root, source))
}
