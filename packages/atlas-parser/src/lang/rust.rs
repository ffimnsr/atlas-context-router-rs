use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use std::collections::{HashMap, HashSet};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, node_text, start_line};
use crate::query_helpers::{QueryCaptureGroup, compile_query, run_query};
use crate::traits::{LangParser, ParseContext};

pub struct RustParser;

const RUST_DEFINITION_QUERY: &str = include_str!("../../queries/rust.scm");

impl LangParser for RustParser {
    fn language_name(&self) -> &'static str {
        "rust"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".rs")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("tree-sitter-rust grammar failed to load");

        let tree = crate::parse_runtime::parse_tree(&mut parser, ctx.source, ctx.old_tree);
        let mut nodes: Vec<Node> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();

        // Always emit a File node.
        let (file_lines, _) = ctx.source.iter().fold((1u32, false), |(ln, _), &b| {
            if b == b'\n' {
                (ln + 1, true)
            } else {
                (ln, false)
            }
        });
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, file_lines));

        if let Some(ref tree) = tree {
            let syntax_facts = RustSyntaxFacts::extract(tree.root_node(), ctx.source)
                .unwrap_or_else(|err| panic!("rust definition query failed: {err}"));
            let mut emitter = RustDefinitionEmitter {
                source: ctx.source,
                rel_path: ctx.rel_path,
                file_hash: ctx.file_hash,
                nodes: &mut nodes,
                edges: &mut edges,
                scope_stack: Vec::new(),
            };
            emitter.emit(&syntax_facts);

            // Second pass: same-file call resolution.
            let mut call_edges =
                resolve_same_file_calls(tree.root_node(), ctx.source, ctx.rel_path, &nodes);
            edges.append(&mut call_edges);

            let mut reference_edges =
                resolve_same_file_references(tree.root_node(), ctx.source, ctx.rel_path, &nodes);
            edges.append(&mut reference_edges);
        }

        let pf = ParsedFile {
            path: ctx.rel_path.to_owned(),
            language: Some("rust".to_owned()),
            hash: ctx.file_hash.to_owned(),
            size: Some(ctx.source.len() as i64),
            nodes,
            edges,
        };
        (pf, tree)
    }
}

// ---------------------------------------------------------------------------
// Query-backed definition extraction
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RustItemKind {
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
struct RustImpl<'tree> {
    node: TsNode<'tree>,
    type_node: TsNode<'tree>,
    trait_node: Option<TsNode<'tree>>,
}

#[derive(Clone, Copy, Debug)]
struct RustItem<'tree> {
    kind: RustItemKind,
    node: TsNode<'tree>,
    name_node: Option<TsNode<'tree>>,
    rust_impl: Option<RustImpl<'tree>>,
}

#[derive(Debug)]
struct RustSyntaxFacts<'tree> {
    items: Vec<RustItem<'tree>>,
    _impls: Vec<RustImpl<'tree>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct RustNodeKey {
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

    fn name_text<'s>(&self, source: &'s [u8]) -> Option<&'s str> {
        self.name_node.map(|node| node_text(node, source))
    }
}

impl<'tree> RustSyntaxFacts<'tree> {
    fn extract(root: TsNode<'tree>, source: &'tree [u8]) -> Result<Self, String> {
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

fn node_key(node: TsNode<'_>) -> RustNodeKey {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RustScopeKind {
    Module,
    Impl,
    Trait,
}

#[derive(Clone, Debug)]
struct RustScope {
    kind: RustScopeKind,
    qualified_name: String,
    end_byte: usize,
    in_test_mod: bool,
}

struct RustDefinitionEmitter<'s, 'o> {
    source: &'s [u8],
    rel_path: &'s str,
    file_hash: &'s str,
    nodes: &'o mut Vec<Node>,
    edges: &'o mut Vec<Edge>,
    scope_stack: Vec<RustScope>,
}

impl<'s, 'o> RustDefinitionEmitter<'s, 'o> {
    fn emit(&mut self, facts: &RustSyntaxFacts<'_>) {
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

fn file_node(rel_path: &str, file_hash: &str, line_end: u32) -> Node {
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

// ---------------------------------------------------------------------------
// Same-file call resolution
// ---------------------------------------------------------------------------

/// Walk `root` looking for call and method-call expressions.
/// Emits `Calls` edges (confidence=0.8, tier="same_file") for any call whose
/// callee name matches a function or method defined in the same file.
fn resolve_same_file_calls(
    root: TsNode<'_>,
    source: &[u8],
    rel_path: &str,
    nodes: &[Node],
) -> Vec<Edge> {
    let callables = collect_callables(nodes);
    let call_sites = extract_rust_call_sites(root, source)
        .unwrap_or_else(|err| panic!("rust call query failed: {err}"));

    let mut edges = Vec::new();

    for site in call_sites {
        let Some(caller_qn) = caller_qn_for_line(&callables, start_line(site.node)) else {
            continue;
        };
        let called = match (site.receiver_node, site.method_node) {
            (Some(receiver_node), Some(method_node)) => Some((
                node_text(site.node, source).to_owned(),
                node_text(method_node, source).to_owned(),
                Some(node_text(receiver_node, source).to_owned()),
            )),
            _ => rust_call_target(site.target_node, source),
        };
        let Some((text, name, receiver)) = called else {
            continue;
        };
        if !should_emit_rust_call(&name) {
            continue;
        }
        if is_self_call(caller_qn, &name, receiver.as_deref()) {
            continue;
        }
        if let Some(callee_qn) = resolve_local_callee(caller_qn, &name, &callables)
            && callee_qn != caller_qn
        {
            edges.push(call_edge(
                caller_qn,
                &callee_qn,
                rel_path,
                start_line(site.node),
                &text,
                receiver.as_deref(),
                true,
            ));
        } else {
            edges.push(call_edge(
                caller_qn,
                &text,
                rel_path,
                start_line(site.node),
                &text,
                receiver.as_deref(),
                false,
            ));
        }
    }

    edges
}

#[derive(Clone)]
struct CallableNode {
    qn: String,
    name: String,
    parent_qn: String,
    line_start: u32,
    line_end: u32,
}

#[derive(Clone, Copy, Debug)]
struct RustCallSite<'tree> {
    node: TsNode<'tree>,
    target_node: TsNode<'tree>,
    receiver_node: Option<TsNode<'tree>>,
    method_node: Option<TsNode<'tree>>,
}

fn collect_callables(nodes: &[Node]) -> Vec<CallableNode> {
    nodes
        .iter()
        .filter(|n| {
            matches!(
                n.kind,
                NodeKind::Function | NodeKind::Method | NodeKind::Test
            )
        })
        .map(|n| CallableNode {
            qn: n.qualified_name.clone(),
            name: n.name.clone(),
            parent_qn: n.parent_name.clone().unwrap_or_else(|| n.file_path.clone()),
            line_start: n.line_start,
            line_end: n.line_end,
        })
        .collect()
}

fn extract_rust_call_sites<'tree>(
    root: TsNode<'tree>,
    source: &'tree [u8],
) -> Result<Vec<RustCallSite<'tree>>, String> {
    let matches = rust_query_matches(root, source)?;
    let mut call_sites: HashMap<RustNodeKey, RustCallSite<'tree>> = HashMap::new();

    for group in matches {
        let mut call_node = None;
        let mut target_node = None;
        let mut receiver_node = None;
        let mut method_node = None;

        for capture in &group.captures {
            match capture.name.as_str() {
                "atlas.call" => call_node = Some(capture.node),
                "atlas.call.target" => target_node = Some(capture.node),
                "atlas.call.receiver" => receiver_node = Some(capture.node),
                "atlas.call.method" => method_node = Some(capture.node),
                _ => {}
            }
        }

        if let Some(node) = call_node {
            let site = call_sites.entry(node_key(node)).or_insert(RustCallSite {
                node,
                target_node: node,
                receiver_node: None,
                method_node: None,
            });
            if let Some(target_node) = target_node {
                site.target_node = target_node;
            }
            site.receiver_node = site.receiver_node.or(receiver_node);
            site.method_node = site.method_node.or(method_node);
        }
    }

    let mut call_sites = call_sites
        .into_values()
        .filter(|site| site.target_node != site.node || site.receiver_node.is_some())
        .collect::<Vec<_>>();
    call_sites.sort_by_key(|site| (site.node.start_byte(), site.node.end_byte()));
    Ok(call_sites)
}

fn caller_qn_for_line(callables: &[CallableNode], line: u32) -> Option<&str> {
    callables
        .iter()
        .filter(|callable| callable.line_start <= line && line <= callable.line_end)
        .min_by_key(|callable| {
            (
                callable.line_end.saturating_sub(callable.line_start),
                callable.line_start,
            )
        })
        .map(|callable| callable.qn.as_str())
}

fn resolve_local_callee(caller_qn: &str, name: &str, callables: &[CallableNode]) -> Option<String> {
    let mut candidates = callables.iter().filter(|callable| callable.name == name);
    let first = candidates.next()?;
    let second = candidates.next();
    if second.is_none() {
        return Some(first.qn.clone());
    }

    let caller_parent_chain = scope_chain_for_qn(caller_qn);
    for parent in &caller_parent_chain {
        if let Some(matched) = callables
            .iter()
            .find(|callable| callable.name == name && callable.parent_qn == *parent)
        {
            return Some(matched.qn.clone());
        }
    }

    callables
        .iter()
        .find(|callable| callable.name == name && callable.parent_qn == caller_parent_chain[0])
        .map(|callable| callable.qn.clone())
}

fn scope_chain_for_qn(qn: &str) -> Vec<String> {
    let Some((prefix, tail)) = qn
        .split_once("::fn::")
        .or_else(|| qn.split_once("::method::"))
    else {
        return vec![qn.to_owned()];
    };
    let mut scopes = Vec::new();
    let mut parts: Vec<&str> = tail.split("::").collect();
    if parts.len() > 1 {
        parts.pop();
    }
    while !parts.is_empty() {
        scopes.push(format!("{prefix}::module::{}", parts.join("::")));
        parts.pop();
    }
    scopes.push(prefix.to_owned());
    scopes
}

fn rust_call_target(node: TsNode<'_>, source: &[u8]) -> Option<(String, String, Option<String>)> {
    match node.kind() {
        "identifier" => {
            let name = node_text(node, source).to_owned();
            Some((name.clone(), name, None))
        }
        "generic_function" => node
            .child_by_field_name("function")
            .and_then(|function| rust_call_target(function, source)),
        "field_expression" => {
            let receiver = node.child_by_field_name("value")?;
            let method = node.child_by_field_name("field")?;
            Some((
                node_text(node, source).to_owned(),
                node_text(method, source).to_owned(),
                Some(node_text(receiver, source).to_owned()),
            ))
        }
        "scoped_identifier" => {
            let text = node_text(node, source).to_owned();
            let (receiver_text, callee_name) = text.rsplit_once("::")?;
            let receiver_text = receiver_text.to_owned();
            let callee_name = callee_name.to_owned();
            Some((text, callee_name, Some(receiver_text)))
        }
        _ => None,
    }
}

fn should_emit_rust_call(callee_name: &str) -> bool {
    callee_name
        .chars()
        .next()
        .is_some_and(|ch| !ch.is_uppercase())
}

fn is_self_call(caller_qn: &str, callee_name: &str, receiver: Option<&str>) -> bool {
    if receiver.is_some() {
        return false;
    }
    caller_simple_name(caller_qn) == callee_name
}

fn caller_simple_name(caller_qn: &str) -> &str {
    caller_qn
        .rsplit("::")
        .next()
        .unwrap_or(caller_qn)
        .rsplit('.')
        .next()
        .unwrap_or(caller_qn)
}

fn call_edge(
    caller_qn: &str,
    callee_qn: &str,
    rel_path: &str,
    line: u32,
    text: &str,
    receiver: Option<&str>,
    same_file: bool,
) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::Calls,
        source_qn: caller_qn.to_owned(),
        target_qn: callee_qn.to_owned(),
        file_path: rel_path.to_owned(),
        line: Some(line),
        confidence: if same_file { 0.8 } else { 0.3 },
        confidence_tier: Some(if same_file { "same_file" } else { "text" }.to_owned()),
        extra_json: serde_json::json!({
            "callee_text": text,
            "callee_name": caller_simple_name(callee_qn),
            "receiver_text": receiver,
        }),
    }
}

// ---------------------------------------------------------------------------
// Same-file reference resolution
// ---------------------------------------------------------------------------

fn resolve_same_file_references(
    root: TsNode<'_>,
    source: &[u8],
    rel_path: &str,
    nodes: &[Node],
) -> Vec<Edge> {
    let reference_sites = extract_rust_reference_sites(root, source)
        .unwrap_or_else(|err| panic!("rust reference query failed: {err}"));
    let mut symbol_targets: HashMap<String, Vec<String>> = HashMap::new();
    let mut type_targets: HashMap<String, Vec<String>> = HashMap::new();

    for node in nodes {
        if node.kind == NodeKind::File {
            continue;
        }

        symbol_targets
            .entry(node.name.clone())
            .or_default()
            .push(node.qualified_name.clone());

        if matches!(
            node.kind,
            NodeKind::Module | NodeKind::Struct | NodeKind::Enum | NodeKind::Trait
        ) {
            type_targets
                .entry(node.name.clone())
                .or_default()
                .push(node.qualified_name.clone());
        }
    }

    ReferenceResolver {
        source,
        rel_path,
        nodes,
        symbol_targets,
        type_targets,
        seen: HashSet::new(),
        edges: Vec::new(),
    }
    .resolve_sites(&reference_sites)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RustReferenceKind {
    UseArgument,
    Type,
}

#[derive(Clone, Copy, Debug)]
struct RustReferenceSite<'tree> {
    node: TsNode<'tree>,
    target_node: TsNode<'tree>,
    kind: RustReferenceKind,
}

fn extract_rust_reference_sites<'tree>(
    root: TsNode<'tree>,
    source: &'tree [u8],
) -> Result<Vec<RustReferenceSite<'tree>>, String> {
    let matches = rust_query_matches(root, source)?;
    let mut sites = Vec::new();

    for group in matches {
        let mut use_node = None;
        let mut use_argument = None;
        let mut type_reference = None;

        for capture in &group.captures {
            match capture.name.as_str() {
                "atlas.reference.use" => use_node = Some(capture.node),
                "atlas.reference.use_argument" => use_argument = Some(capture.node),
                "atlas.reference.type" => type_reference = Some(capture.node),
                _ => {}
            }
        }

        if let (Some(node), Some(target_node)) = (use_node, use_argument) {
            sites.push(RustReferenceSite {
                node,
                target_node,
                kind: RustReferenceKind::UseArgument,
            });
        }
        if let Some(target_node) = type_reference {
            sites.push(RustReferenceSite {
                node: target_node,
                target_node,
                kind: RustReferenceKind::Type,
            });
        }
    }

    sites.sort_by_key(|site| (site.node.start_byte(), site.node.end_byte()));
    Ok(sites)
}

struct ReferenceResolver<'a> {
    source: &'a [u8],
    rel_path: &'a str,
    nodes: &'a [Node],
    symbol_targets: HashMap<String, Vec<String>>,
    type_targets: HashMap<String, Vec<String>>,
    seen: HashSet<(String, String, u32)>,
    edges: Vec<Edge>,
}

impl<'a> ReferenceResolver<'a> {
    fn resolve_sites(mut self, sites: &[RustReferenceSite<'_>]) -> Vec<Edge> {
        for site in sites {
            match site.kind {
                RustReferenceKind::UseArgument => {
                    let source_qn =
                        reference_source_qn(self.nodes, self.rel_path, start_line(site.node));
                    for name in use_reference_names(site.target_node, self.source) {
                        let target_qn =
                            unique_target_qn(&self.symbol_targets, &name).map(str::to_owned);
                        self.maybe_push_reference_edge(
                            source_qn,
                            target_qn.as_deref(),
                            start_line(site.node),
                        );
                    }
                }
                RustReferenceKind::Type => {
                    if is_definition_name(site.target_node) {
                        continue;
                    }
                    let source_qn =
                        reference_source_qn(self.nodes, self.rel_path, start_line(site.node));
                    let name = type_reference_name(site.target_node, self.source);
                    let target_qn = unique_target_qn(&self.type_targets, &name).map(str::to_owned);
                    self.maybe_push_reference_edge(
                        source_qn,
                        target_qn.as_deref(),
                        start_line(site.node),
                    );
                }
            }
        }
        self.edges
    }

    fn maybe_push_reference_edge(&mut self, source_qn: &str, target_qn: Option<&str>, line: u32) {
        let Some(target_qn) = target_qn else {
            return;
        };
        if source_qn == target_qn {
            return;
        }

        let key = (source_qn.to_owned(), target_qn.to_owned(), line);
        if !self.seen.insert(key.clone()) {
            return;
        }

        self.edges.push(reference_edge(
            &key.0,
            &key.1,
            self.rel_path,
            line,
            Some("same_file".to_owned()),
        ));
    }
}

fn unique_target_qn<'a>(targets: &'a HashMap<String, Vec<String>>, name: &str) -> Option<&'a str> {
    match targets.get(name) {
        Some(entries) if entries.len() == 1 => entries.first().map(|entry| entry.as_str()),
        _ => None,
    }
}

fn reference_source_qn<'a>(nodes: &'a [Node], rel_path: &'a str, line: u32) -> &'a str {
    nodes
        .iter()
        .filter(|node| {
            node.kind != NodeKind::File && node.line_start <= line && line <= node.line_end
        })
        .min_by_key(|node| {
            (
                node.line_end.saturating_sub(node.line_start),
                node.line_start,
            )
        })
        .map(|node| node.qualified_name.as_str())
        .unwrap_or(rel_path)
}

fn use_reference_names(node: TsNode<'_>, source: &[u8]) -> Vec<String> {
    let mut names = Vec::new();
    if node.kind() == "use_declaration" {
        if let Some(argument) = node.child_by_field_name("argument") {
            collect_use_reference_names(argument, source, &mut names);
        }
    } else {
        collect_use_reference_names(node, source, &mut names);
    }
    names.sort();
    names.dedup();
    names
}

fn collect_use_reference_names(node: TsNode<'_>, source: &[u8], names: &mut Vec<String>) {
    match node.kind() {
        "identifier" => push_reference_name(node_text(node, source), names),
        "scoped_identifier" => {
            push_reference_name(last_path_segment(node_text(node, source)), names)
        }
        "use_as_clause" => {
            if let Some(path) = node.child_by_field_name("path") {
                collect_use_reference_names(path, source, names);
            }
            return;
        }
        "scoped_use_list" => {
            if let Some(path) = node.child_by_field_name("path") {
                collect_use_reference_names(path, source, names);
            }
            if let Some(list) = node.child_by_field_name("list") {
                collect_use_reference_names(list, source, names);
            }
            return;
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_use_reference_names(child, source, names);
    }
}

fn push_reference_name(name: &str, names: &mut Vec<String>) {
    if matches!(name, "crate" | "self" | "super" | "Self") || name.is_empty() {
        return;
    }
    names.push(name.to_owned());
}

fn type_reference_name(node: TsNode<'_>, source: &[u8]) -> String {
    last_path_segment(node_text(node, source)).to_owned()
}

fn last_path_segment(path: &str) -> &str {
    path.split("::")
        .last()
        .unwrap_or(path)
        .split('<')
        .next()
        .unwrap_or(path)
        .trim()
}

fn is_definition_name(node: TsNode<'_>) -> bool {
    node.parent()
        .and_then(|parent| parent.child_by_field_name("name"))
        .is_some_and(|name| name == node)
}

fn reference_edge(
    source_qn: &str,
    target_qn: &str,
    rel_path: &str,
    line: u32,
    confidence_tier: Option<String>,
) -> Edge {
    Edge {
        id: 0,
        kind: EdgeKind::References,
        source_qn: source_qn.to_owned(),
        target_qn: target_qn.to_owned(),
        file_path: rel_path.to_owned(),
        line: Some(line),
        confidence: 0.75,
        confidence_tier,
        extra_json: serde_json::Value::Null,
    }
}

fn rust_query_matches<'tree>(
    root: TsNode<'tree>,
    source: &'tree [u8],
) -> Result<Vec<QueryCaptureGroup<'tree>>, String> {
    let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
    let query = compile_query(language, RUST_DEFINITION_QUERY)?;
    Ok(run_query(&query, root, source))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query_helpers::{compile_query, read_capture_text, run_query};
    use crate::traits::ParseContext;

    fn parse(src: &str) -> ParsedFile {
        let p = RustParser;
        let (pf, _) = p.parse(&ParseContext {
            rel_path: "src/lib.rs",
            file_hash: "deadbeef",
            source: src.as_bytes(),
            old_tree: None,
        });
        pf
    }

    #[test]
    fn extracts_file_node() {
        let pf = parse("fn foo() {}");
        assert!(pf.nodes.iter().any(|n| n.kind == NodeKind::File));
    }

    #[test]
    fn extracts_free_function() {
        let pf = parse("pub fn greet(name: &str) -> String { todo!() }");
        let func = pf
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function)
            .expect("function");
        assert_eq!(func.name, "greet");
        assert!(func.qualified_name.contains("fn::greet"));
    }

    #[test]
    fn extracts_struct() {
        let pf = parse("pub struct Foo { x: i32 }");
        let s = pf
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Struct)
            .expect("struct");
        assert_eq!(s.name, "Foo");
    }

    #[test]
    fn extracts_enum() {
        let pf = parse("enum Color { Red, Green, Blue }");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Enum && n.name == "Color")
        );
    }

    #[test]
    fn extracts_trait() {
        let pf = parse("pub trait Drawable { fn draw(&self); }");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Trait && n.name == "Drawable")
        );
    }

    #[test]
    fn trait_method_declaration_emitted_and_contained_by_trait() {
        let pf = parse("pub trait Drawable { fn draw(&self); }");
        assert!(pf.nodes.iter().any(|n| {
            n.kind == NodeKind::Method
                && n.qualified_name == "src/lib.rs::method::Drawable::draw"
                && n.parent_name.as_deref() == Some("src/lib.rs::trait::Drawable")
        }));
        assert!(pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Contains
                && e.source_qn == "src/lib.rs::trait::Drawable"
                && e.target_qn == "src/lib.rs::method::Drawable::draw"
        }));
    }

    #[test]
    fn free_function_and_trait_method_with_same_name_stay_distinct() {
        let pf = parse("fn draw() {} trait Drawable { fn draw(&self); }");
        assert!(pf
            .nodes
            .iter()
            .any(|n| n.kind == NodeKind::Function && n.qualified_name == "src/lib.rs::fn::draw"));
        assert!(pf.nodes.iter().any(|n| {
            n.kind == NodeKind::Method && n.qualified_name == "src/lib.rs::method::Drawable::draw"
        }));
    }

    #[test]
    fn extracts_method_and_impl_edge() {
        let src = "struct Foo; impl Foo { pub fn bar(&self) {} }";
        let pf = parse(src);
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Module && n.qualified_name == "src/lib.rs::impl::Foo")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Method && n.name == "bar")
        );
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Contains
            && e.source_qn == "src/lib.rs::impl::Foo"
            && e.target_qn == "src/lib.rs::method::Foo::bar"));
    }

    #[test]
    fn implements_edge_for_trait_impl() {
        let src = "trait Greet {} struct Hi; impl Greet for Hi {}";
        let pf = parse(src);
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Implements
            && e.source_qn == "src/lib.rs::struct::Hi"
            && e.target_qn == "src/lib.rs::trait::Greet"));
    }

    #[test]
    fn implements_edge_uses_enum_qn_when_impl_targets_enum() {
        let src = "trait Render {} enum Mode { Fast } impl Render for Mode {}";
        let pf = parse(src);
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Implements
            && e.source_qn == "src/lib.rs::enum::Mode"
            && e.target_qn == "src/lib.rs::trait::Render"));
    }

    #[test]
    fn external_trait_impl_does_not_emit_dangling_implements_edge() {
        let src = "struct Foo; impl std::fmt::Display for Foo {}";
        let pf = parse(src);
        assert!(!pf.edges.iter().any(|e| e.kind == EdgeKind::Implements));
    }

    #[test]
    fn scoped_local_trait_impl_emits_same_file_edge_when_targets_are_unique() {
        let src = r#"
mod local {
    pub trait Trait {}
    pub struct Type;
}

impl local::Trait for local::Type {}
"#;
        let pf = parse(src);
        assert!(pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Implements
                && e.source_qn == "src/lib.rs::struct::local::Type"
                && e.target_qn == "src/lib.rs::trait::local::Trait"
        }));
    }

    #[test]
    fn test_fn_detected() {
        let src = r#"
#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {}
}
"#;
        let pf = parse(src);
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Test && n.name == "it_works")
        );
    }

    #[test]
    fn top_level_test_attr_emits_test_node_kind() {
        let pf = parse("#[test] fn it_works() {}");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Test && n.name == "it_works")
        );
    }

    #[test]
    fn cfg_test_module_marks_nested_helper_as_test() {
        let pf = parse("#[cfg(test)] mod tests { fn helper() {} }");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Test && n.name == "helper")
        );
    }

    #[test]
    fn cfg_not_test_module_does_not_mark_nested_helper_as_test() {
        let pf = parse("#[cfg(not(test))] mod tests { fn helper() {} }");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Function && n.name == "helper")
        );
        assert!(
            !pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Test && n.name == "helper")
        );
    }

    #[test]
    fn custom_attribute_containing_test_does_not_mark_function_as_test() {
        let pf = parse("#[mytest] fn helper() {}");
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Function && n.name == "helper")
        );
        assert!(
            !pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Test && n.name == "helper")
        );
    }

    #[test]
    fn nested_module() {
        let src = "mod outer { mod inner { fn deep() {} } }";
        let pf = parse(src);
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Module && n.name == "outer")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Module && n.name == "inner")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Function && n.name == "deep")
        );
    }

    #[test]
    fn contains_edges_present() {
        let src = "mod foo { fn bar() {} }";
        let pf = parse(src);
        assert!(pf.edges.iter().any(|e| e.kind == EdgeKind::Contains));
    }

    #[test]
    fn same_file_call_resolved() {
        let src = r#"
fn helper() {}
fn caller() { helper(); }
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
                && e.source_qn.contains("caller")
                && e.target_qn.contains("helper")),
            "expected a Calls edge from caller to helper; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn generic_function_call_resolved() {
        let src = r#"
fn helper<T>(value: T) -> T { value }
fn caller() { let _ = helper::<u32>(1); }
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| e.kind == EdgeKind::Calls
                && e.source_qn.contains("caller")
                && e.target_qn.contains("helper")),
            "expected a Calls edge from caller to generic helper; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn method_call_resolved() {
        let src = r#"
fn helper() {}
struct S;
impl S {
    fn do_work(&self) { helper(); }
}
"#;
        let pf = parse(src);
        assert!(
            pf.edges
                .iter()
                .any(|e| e.kind == EdgeKind::Calls && e.target_qn.contains("helper")),
            "expected Calls edge to helper from method"
        );
    }

    #[test]
    fn method_call_syntax_resolved_to_same_file_method() {
        let src = r#"
struct Worker;

impl Worker {
    fn run(&self) {}

    fn execute(&self) {
        self.run();
    }
}
"#;
        let pf = parse(src);
        assert!(pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "src/lib.rs::method::Worker::execute"
                && e.target_qn == "src/lib.rs::method::Worker::run"
        }));
    }

    #[test]
    fn no_self_calls_edge() {
        // A recursive call should not produce a self-loop.
        let src = r#"fn recurse(n: u32) -> u32 { if n == 0 { 0 } else { recurse(n-1) } }"#;
        let pf = parse(src);
        assert!(
            !pf.edges
                .iter()
                .any(|e| e.kind == EdgeKind::Calls && e.source_qn == e.target_qn),
            "recursive call must not produce a self-loop edge"
        );
    }

    #[test]
    fn unresolved_call_keeps_text_target() {
        let src = r#"fn caller() { crate::helper(); }"#;
        let pf = parse(src);
        let edge = pf
            .edges
            .iter()
            .find(|e| e.kind == EdgeKind::Calls)
            .expect("call edge");
        assert_eq!(edge.target_qn, "crate::helper");
        assert_eq!(edge.confidence_tier.as_deref(), Some("text"));
    }

    #[test]
    fn skips_variant_and_option_constructor_false_positives() {
        let src = r#"
enum Value { Object }

fn helper() {}

fn caller() {
    Value::Object();
    Some("x");
    helper();
}
"#;
        let pf = parse(src);
        assert!(pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn.contains("caller")
                && e.target_qn.contains("helper")
        }));
        assert!(
            !pf.edges
                .iter()
                .any(|e| { e.kind == EdgeKind::Calls && e.target_qn == "Value::Object" })
        );
        assert!(
            !pf.edges
                .iter()
                .any(|e| e.kind == EdgeKind::Calls && e.target_qn == "Some")
        );
    }

    #[test]
    fn extracts_generic_function() {
        let pf = parse("pub fn wrap<T: Clone>(value: T) -> Option<T> { Some(value) }");
        let func = pf
            .nodes
            .iter()
            .find(|n| n.kind == NodeKind::Function && n.name == "wrap")
            .expect("generic function");
        assert_eq!(func.return_type.as_deref(), Some("Option<T>"));
        assert_eq!(func.params.as_deref(), Some("(value: T)"));
    }

    #[test]
    fn resolves_same_file_use_and_type_references() {
        let src = r#"
mod support {
    pub struct Helper;
}

use self::support::Helper;

fn build(value: Helper) -> Helper { value }
"#;
        let pf = parse(src);
        assert!(
            pf.edges.iter().any(|e| e.kind == EdgeKind::References
                && e.source_qn == "src/lib.rs"
                && e.target_qn.contains("module::support")),
            "expected use reference to module support; edges: {:?}",
            pf.edges
        );
        assert!(
            pf.edges.iter().any(|e| e.kind == EdgeKind::References
                && e.source_qn.contains("build")
                && e.target_qn.contains("struct::support::Helper")),
            "expected function type reference to Helper; edges: {:?}",
            pf.edges
        );
    }

    #[test]
    fn macro_heavy_file_parses() {
        let src = r#"
macro_rules! call_helper {
    ($value:expr) => {
        helper($value)
    };
}

#[derive(Debug, Clone)]
struct Wrapper<T> {
    value: T,
}

fn helper<T>(value: T) -> T { value }

fn caller() {
    let _ = call_helper!(Wrapper { value: 1 });
}
"#;
        let pf = parse(src);
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Struct && n.name == "Wrapper")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Function && n.name == "helper")
        );
        assert!(
            pf.nodes
                .iter()
                .any(|n| n.kind == NodeKind::Function && n.name == "caller")
        );
    }

    #[test]
    fn nested_impl_scope_tracks_parent_module() {
        let src = r#"
mod outer {
    struct Thing;
    impl Thing {
        fn run(&self) {}
    }
}
"#;
        let pf = parse(src);
        assert!(pf.nodes.iter().any(|n| {
            n.kind == NodeKind::Module && n.qualified_name == "src/lib.rs::impl::outer::Thing"
        }));
        assert!(pf.nodes.iter().any(|n| {
            n.kind == NodeKind::Method
                && n.qualified_name == "src/lib.rs::method::outer::Thing::run"
                && n.parent_name.as_deref() == Some("src/lib.rs::impl::outer::Thing")
        }));
    }

    #[test]
    fn resolves_calls_to_closest_parent_scope() {
        let src = r#"
fn helper() {}

mod alpha {
    fn helper() {}

    fn caller() {
        helper();
    }
}
"#;
        let pf = parse(src);
        assert!(pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "src/lib.rs::fn::alpha::caller"
                && e.target_qn == "src/lib.rs::fn::alpha::helper"
        }));
        assert!(!pf.edges.iter().any(|e| {
            e.kind == EdgeKind::Calls
                && e.source_qn == "src/lib.rs::fn::alpha::caller"
                && e.target_qn == "src/lib.rs::fn::helper"
        }));
    }

    #[test]
    fn malformed_source_keeps_file_node_and_best_effort_symbols() {
        let pf = parse("pub fn broken(value: i32) -> i32 { value + 1 } @");
        assert!(pf.nodes.iter().any(|node| node.kind == NodeKind::File));
        assert!(
            pf.nodes
                .iter()
                .any(|node| node.kind == NodeKind::Function && node.name == "broken")
        );
    }

    #[test]
    fn rust_definition_query_extracts_function_capture() {
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let query = compile_query(language.clone(), RUST_DEFINITION_QUERY)
            .expect("rust definition query should compile");
        let source = b"fn helper() {}";

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language)
            .expect("tree-sitter-rust grammar failed to load");
        let tree = parser
            .parse(source.as_slice(), None)
            .expect("fixture should parse");

        let matches = run_query(&query, tree.root_node(), source);
        assert!(matches.iter().any(|group| {
            group.captures.iter().any(|capture| {
                capture.name == "atlas.definition.function"
                    && read_capture_text(capture, source).contains("fn helper")
            })
        }));
    }

    #[test]
    fn rust_definition_query_extracts_impl_trait_capture() {
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let query = compile_query(language.clone(), RUST_DEFINITION_QUERY)
            .expect("rust definition query should compile");
        let source = b"trait Draw {} struct Shape; impl Draw for Shape {}";

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language)
            .expect("tree-sitter-rust grammar failed to load");
        let tree = parser
            .parse(source.as_slice(), None)
            .expect("fixture should parse");

        let matches = run_query(&query, tree.root_node(), source);
        assert!(matches.iter().any(|group| {
            group.captures.iter().any(|capture| {
                capture.name == "atlas.impl.trait" && read_capture_text(capture, source) == "Draw"
            })
        }));
    }

    #[test]
    fn rust_definition_query_extracts_method_call_receiver_and_name() {
        let language: tree_sitter::Language = tree_sitter_rust::LANGUAGE.into();
        let query = compile_query(language.clone(), RUST_DEFINITION_QUERY)
            .expect("rust definition query should compile");
        let source = b"struct S; impl S { fn run(&self) {} fn call(&self) { self.run(); } }";

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language)
            .expect("tree-sitter-rust grammar failed to load");
        let tree = parser
            .parse(source.as_slice(), None)
            .expect("fixture should parse");

        let matches = run_query(&query, tree.root_node(), source);
        assert!(matches.iter().any(|group| {
            let names = group
                .captures
                .iter()
                .map(|capture| capture.name.as_str())
                .collect::<Vec<_>>();
            names.contains(&"atlas.call.receiver")
                && names.contains(&"atlas.call.method")
                && group.captures.iter().any(|capture| {
                    capture.name == "atlas.call.method"
                        && read_capture_text(capture, source) == "run"
                })
        }));
    }
}
