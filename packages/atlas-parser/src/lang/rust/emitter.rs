use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};

use super::facts::{RustItem, RustItemKind, RustSyntaxFacts};
use super::references::last_path_segment;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RustScopeKind {
    Module,
    Impl,
    Trait,
}

#[derive(Clone, Debug)]
pub(super) struct RustScope {
    kind: RustScopeKind,
    qualified_name: String,
    end_byte: usize,
    in_test_mod: bool,
}

pub(super) struct RustDefinitionEmitter<'s, 'o> {
    pub(super) source: &'s [u8],
    pub(super) rel_path: &'s str,
    pub(super) file_hash: &'s str,
    pub(super) nodes: &'o mut Vec<Node>,
    pub(super) edges: &'o mut Vec<Edge>,
    pub(super) scope_stack: Vec<RustScope>,
}

impl<'s, 'o> RustDefinitionEmitter<'s, 'o> {
    pub(super) fn emit(&mut self, facts: &RustSyntaxFacts<'_>) {
        for item in &facts.items {
            self.advance_to(item.node.start_byte());
            match item.kind {
                RustItemKind::Function => self.emit_fn(item),
                RustItemKind::FunctionSignature => self.emit_trait_method_signature(item),
                RustItemKind::Module => self.emit_mod(item),
                RustItemKind::Struct => self.emit_named_item(item, NodeKind::Struct, "struct"),
                RustItemKind::Enum => self.emit_named_item(item, NodeKind::Enum, "enum"),
                RustItemKind::Trait => self.emit_named_item(item, NodeKind::Trait, "trait"),
                RustItemKind::Const | RustItemKind::Static => {
                    self.emit_named_item(item, NodeKind::Constant, "const")
                }
                RustItemKind::Impl => self.emit_impl(item),
            }
        }
    }

    fn advance_to(&mut self, start_byte: usize) {
        while self
            .scope_stack
            .last()
            .is_some_and(|scope| start_byte >= scope.end_byte)
        {
            self.scope_stack.pop();
        }
    }

    fn current_parent_qn(&self) -> &str {
        self.scope_stack
            .last()
            .map(|scope| scope.qualified_name.as_str())
            .unwrap_or(self.rel_path)
    }

    fn current_in_test_mod(&self) -> bool {
        self.scope_stack
            .last()
            .is_some_and(|scope| scope.in_test_mod)
    }

    fn inside_impl(&self) -> bool {
        self.scope_stack
            .iter()
            .rev()
            .any(|scope| scope.kind == RustScopeKind::Impl)
    }

    fn inside_trait(&self) -> bool {
        self.scope_stack
            .iter()
            .rev()
            .any(|scope| scope.kind == RustScopeKind::Trait)
    }

    fn unique_local_target_qn<F>(&self, name: &str, predicate: F) -> Option<String>
    where
        F: Fn(NodeKind) -> bool,
    {
        let mut matches = self
            .nodes
            .iter()
            .filter(|node| predicate(node.kind) && node.name == name)
            .map(|node| node.qualified_name.as_str());
        let first = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(first.to_owned())
    }

    fn local_type_qn(&self, name: &str) -> Option<String> {
        self.unique_local_target_qn(name, |kind| {
            matches!(kind, NodeKind::Struct | NodeKind::Enum | NodeKind::Trait)
        })
    }

    fn local_trait_qn(&self, name: &str) -> Option<String> {
        self.unique_local_target_qn(name, |kind| kind == NodeKind::Trait)
    }

    fn emit_fn(&mut self, item: &RustItem<'_>) {
        let Some(name) = item.name_text(self.source) else {
            return;
        };
        let is_test = self.current_in_test_mod() || has_test_attr(item.node, self.source);
        let kind = if is_test {
            NodeKind::Test
        } else if self.inside_impl() || self.inside_trait() {
            NodeKind::Method
        } else {
            NodeKind::Function
        };

        let type_prefix = match kind {
            NodeKind::Test => "test",
            NodeKind::Method => "method",
            _ => "fn",
        };
        let parent_qn = self.current_parent_qn().to_owned();
        let qn = format!(
            "{}::{}::{}",
            self.rel_path,
            type_prefix,
            qualified_suffix(&parent_qn, self.rel_path, name)
        );
        let params = item
            .node
            .child_by_field_name("parameters")
            .map(|node| node_text(node, self.source).to_owned());
        let ret = item
            .node
            .child_by_field_name("return_type")
            .map(|node| node_text(node, self.source).to_owned());

        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_owned(),
            qualified_name: qn.clone(),
            file_path: self.rel_path.to_owned(),
            line_start: start_line(item.node),
            line_end: end_line(item.node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params,
            return_type: ret,
            modifiers: visibility_modifier(item.node, self.source),
            is_test: is_test || self.current_in_test_mod(),
            file_hash: self.file_hash.to_owned(),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        });
        self.edges.push(contains_edge(
            &parent_qn,
            &qn,
            self.rel_path,
            start_line(item.node),
        ));
    }

    fn emit_trait_method_signature(&mut self, item: &RustItem<'_>) {
        if !self.inside_trait() {
            return;
        }
        let Some(name) = item.name_text(self.source) else {
            return;
        };
        let parent_qn = self.current_parent_qn().to_owned();
        let qn = format!(
            "{}::method::{}",
            self.rel_path,
            qualified_suffix(&parent_qn, self.rel_path, name)
        );

        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Method,
            name: name.to_owned(),
            qualified_name: qn.clone(),
            file_path: self.rel_path.to_owned(),
            line_start: start_line(item.node),
            line_end: end_line(item.node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params: item
                .node
                .child_by_field_name("parameters")
                .map(|node| node_text(node, self.source).to_owned()),
            return_type: item
                .node
                .child_by_field_name("return_type")
                .map(|node| node_text(node, self.source).to_owned()),
            modifiers: visibility_modifier(item.node, self.source),
            is_test: self.current_in_test_mod(),
            file_hash: self.file_hash.to_owned(),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        });
        self.edges.push(contains_edge(
            &parent_qn,
            &qn,
            self.rel_path,
            start_line(item.node),
        ));
    }

    fn emit_mod(&mut self, item: &RustItem<'_>) {
        let Some(name) = item.name_text(self.source) else {
            return;
        };
        let parent_qn = self.current_parent_qn().to_owned();
        let suffix = qualified_suffix(&parent_qn, self.rel_path, name);
        let qn = format!("{}::module::{}", self.rel_path, suffix);

        let is_test_mod = self.current_in_test_mod() || has_cfg_test(item.node, self.source);

        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Module,
            name: name.to_owned(),
            qualified_name: qn.clone(),
            file_path: self.rel_path.to_owned(),
            line_start: start_line(item.node),
            line_end: end_line(item.node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params: None,
            return_type: None,
            modifiers: visibility_modifier(item.node, self.source),
            is_test: is_test_mod,
            file_hash: self.file_hash.to_owned(),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        });
        self.edges.push(contains_edge(
            &parent_qn,
            &qn,
            self.rel_path,
            start_line(item.node),
        ));

        if let Some(body) = item.node.child_by_field_name("body") {
            self.scope_stack.push(RustScope {
                kind: RustScopeKind::Module,
                qualified_name: qn,
                end_byte: body.end_byte(),
                in_test_mod: is_test_mod,
            });
        }
    }

    fn emit_named_item(&mut self, item: &RustItem<'_>, kind: NodeKind, type_prefix: &str) {
        let Some(name) = item.name_text(self.source) else {
            return;
        };
        let parent_qn = self.current_parent_qn().to_owned();
        let suffix = qualified_suffix(&parent_qn, self.rel_path, name);
        let qn = format!("{}::{}::{}", self.rel_path, type_prefix, suffix);

        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_owned(),
            qualified_name: qn.clone(),
            file_path: self.rel_path.to_owned(),
            line_start: start_line(item.node),
            line_end: end_line(item.node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params: None,
            return_type: None,
            modifiers: visibility_modifier(item.node, self.source),
            is_test: self.current_in_test_mod(),
            file_hash: self.file_hash.to_owned(),
            extra_json: serde_json::Value::Null,
            repo_provenance: None,
        });
        self.edges.push(contains_edge(
            &parent_qn,
            &qn,
            self.rel_path,
            start_line(item.node),
        ));

        if kind == NodeKind::Trait
            && let Some(body) = item.node.child_by_field_name("body")
        {
            self.scope_stack.push(RustScope {
                kind: RustScopeKind::Trait,
                qualified_name: qn,
                end_byte: body.end_byte(),
                in_test_mod: self.current_in_test_mod(),
            });
        }
    }

    fn emit_impl(&mut self, item: &RustItem<'_>) {
        let Some(rust_impl) = item.rust_impl else {
            return;
        };
        let type_name = node_text(rust_impl.type_node, self.source);
        let local_type_name = normalized_local_type_name(rust_impl.type_node, self.source);
        let trait_name = rust_impl
            .trait_node
            .and_then(|node| normalized_local_type_name(node, self.source));
        let parent_qn = self.current_parent_qn().to_owned();
        let suffix = qualified_suffix(&parent_qn, self.rel_path, type_name);
        let impl_scope = format!("{}::impl::{}", self.rel_path, suffix);

        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Module,
            name: format!("impl {type_name}"),
            qualified_name: impl_scope.clone(),
            file_path: self.rel_path.to_owned(),
            line_start: start_line(rust_impl.node),
            line_end: end_line(rust_impl.node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: self.current_in_test_mod(),
            file_hash: self.file_hash.to_owned(),
            extra_json: serde_json::json!({
                "scope_kind": "impl",
                "type_name": type_name,
                "trait_name": trait_name,
            }),
            repo_provenance: None,
        });
        self.edges.push(contains_edge(
            &parent_qn,
            &impl_scope,
            self.rel_path,
            start_line(rust_impl.node),
        ));

        if let (Some(type_name), Some(trait_name)) = (local_type_name.as_deref(), trait_name)
            && let (Some(type_qn), Some(trait_qn)) = (
                self.local_type_qn(type_name),
                self.local_trait_qn(&trait_name),
            )
        {
            self.edges.push(Edge {
                id: 0,
                kind: EdgeKind::Implements,
                source_qn: type_qn,
                target_qn: trait_qn,
                file_path: self.rel_path.to_owned(),
                line: Some(start_line(rust_impl.node)),
                confidence: 0.9,
                confidence_tier: Some("same_file".to_owned()),
                extra_json: serde_json::Value::Null,
                repo_provenance: None,
            });
        }

        if let Some(body) = rust_impl.node.child_by_field_name("body") {
            self.scope_stack.push(RustScope {
                kind: RustScopeKind::Impl,
                qualified_name: impl_scope,
                end_byte: body.end_byte(),
                in_test_mod: self.current_in_test_mod(),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

pub(super) fn file_node(rel_path: &str, file_hash: &str, line_end: u32) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::File,
        name: rel_path.rsplit('/').next().unwrap_or(rel_path).to_owned(),
        qualified_name: rel_path.to_owned(),
        file_path: rel_path.to_owned(),
        line_start: 1,
        line_end,
        language: "rust".to_owned(),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        file_hash: file_hash.to_owned(),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

fn contains_edge(parent_qn: &str, child_qn: &str, file_path: &str, line: u32) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Contains,
        source_qn: parent_qn.to_owned(),
        target_qn: child_qn.to_owned(),
        file_path: file_path.to_owned(),
        line: Some(line),
        confidence: 1.0,
        confidence_tier: Some("definite".to_owned()),
        extra_json: serde_json::Value::Null,
        repo_provenance: None,
    }
}

/// For nested scopes the method QN includes a disambiguating suffix from the
/// parent beyond the file root.  E.g. for an impl inside `mod foo`, the method
/// QN becomes `file::method::foo::Type.name`.
fn qualified_suffix(parent_qn: &str, rel_path: &str, name: &str) -> String {
    // Trim the leading `<rel_path>::<kind>::` prefix of the parent, if any.
    let parent_tail = parent_qn
        .strip_prefix(rel_path)
        .and_then(|s| s.strip_prefix("::"))
        .and_then(|s| s.split_once("::").map(|x| x.1))
        .unwrap_or("");
    if parent_tail.is_empty() {
        name.to_owned()
    } else {
        format!("{}::{}", parent_tail, name)
    }
}

fn visibility_modifier(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            return Some(node_text(child, source).to_owned());
        }
    }
    None
}

fn attribute_signature(node: TsNode<'_>, source: &[u8]) -> Option<(String, Option<String>)> {
    if node.kind() != "attribute_item" {
        return None;
    }

    let attribute = node.named_child(0)?;
    let path = attribute.named_child(0)?;
    let name = last_path_segment(node_text(path, source)).to_owned();
    let arguments = attribute
        .child_by_field_name("arguments")
        .map(|args| normalize_attribute_arguments(node_text(args, source)));
    Some((name, arguments))
}

fn normalize_attribute_arguments(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

fn preceding_attributes(node: TsNode<'_>) -> Vec<TsNode<'_>> {
    let mut attrs = Vec::new();
    let mut sib = node.prev_named_sibling();
    while let Some(s) = sib {
        if s.kind() != "attribute_item" {
            break;
        }
        attrs.push(s);
        sib = s.prev_named_sibling();
    }
    attrs.reverse();
    attrs
}

/// Returns true if the node has a preceding exact `#[test]` attribute sibling.
fn has_test_attr(node: TsNode<'_>, source: &[u8]) -> bool {
    preceding_attributes(node).into_iter().any(|attr| {
        matches!(
            attribute_signature(attr, source),
            Some((name, None)) if name == "test"
        )
    })
}

/// Returns true if the node has a preceding exact `#[cfg(test)]` attribute sibling.
fn has_cfg_test(node: TsNode<'_>, source: &[u8]) -> bool {
    preceding_attributes(node).into_iter().any(|attr| {
        matches!(
            attribute_signature(attr, source),
            Some((name, Some(arguments))) if name == "cfg" && arguments == "(test)"
        )
    })
}

fn normalized_local_type_name(node: TsNode<'_>, source: &[u8]) -> Option<String> {
    match node.kind() {
        "generic_type" => node
            .child_by_field_name("type")
            .and_then(|inner| normalized_local_type_name(inner, source)),
        "scoped_identifier" | "scoped_type_identifier" => node
            .child_by_field_name("name")
            .map(|name| node_text(name, source).to_owned()),
        "type_identifier" | "identifier" => Some(node_text(node, source).to_owned()),
        _ => {
            let text = last_path_segment(node_text(node, source)).trim();
            (!text.is_empty()).then(|| text.to_owned())
        }
    }
}
