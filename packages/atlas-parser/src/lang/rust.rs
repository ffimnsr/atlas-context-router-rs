use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};
use std::collections::{HashMap, HashSet};
use tree_sitter::Node as TsNode;

use crate::ast_helpers::{end_line, field_text, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

pub struct RustParser;

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

        let tree = parser.parse(ctx.source, ctx.old_tree);
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
            let mut walker = Walker {
                source: ctx.source,
                rel_path: ctx.rel_path,
                file_hash: ctx.file_hash,
                nodes: &mut nodes,
                edges: &mut edges,
                // Parent qualified-name stack; starts with the file node.
                scope_stack: vec![ctx.rel_path.to_owned()],
                in_test_mod: false,
            };
            walker.walk_block(tree.root_node());

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
// Internal walker
// ---------------------------------------------------------------------------

struct Walker<'s, 'o> {
    source: &'s [u8],
    rel_path: &'s str,
    file_hash: &'s str,
    nodes: &'o mut Vec<Node>,
    edges: &'o mut Vec<Edge>,
    /// Current parent qualified-name (top = innermost scope).
    scope_stack: Vec<String>,
    /// True when we're inside a `#[cfg(test)]` module.
    in_test_mod: bool,
}

impl<'s, 'o> Walker<'s, 'o> {
    fn current_parent_qn(&self) -> &str {
        self.scope_stack
            .last()
            .map(|s| s.as_str())
            .unwrap_or(self.rel_path)
    }

    fn local_type_qn(&self, name: &str) -> Option<String> {
        self.nodes
            .iter()
            .rev()
            .find(|node| {
                matches!(
                    node.kind,
                    NodeKind::Struct | NodeKind::Enum | NodeKind::Trait
                ) && node.name == name
            })
            .map(|node| node.qualified_name.clone())
    }

    fn local_trait_qn(&self, name: &str) -> Option<String> {
        self.nodes
            .iter()
            .rev()
            .find(|node| node.kind == NodeKind::Trait && node.name == name)
            .map(|node| node.qualified_name.clone())
    }

    /// Walk all children of a block/source_file.
    fn walk_block(&mut self, node: TsNode<'_>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child);
        }
    }

    fn visit(&mut self, node: TsNode<'_>) {
        match node.kind() {
            "function_item" => self.visit_fn(node, false),
            "mod_item" => self.visit_mod(node),
            "struct_item" => self.visit_named_item(node, NodeKind::Struct, "struct"),
            "enum_item" => self.visit_named_item(node, NodeKind::Enum, "enum"),
            "trait_item" => self.visit_named_item(node, NodeKind::Trait, "trait"),
            "const_item" => self.visit_named_item(node, NodeKind::Constant, "const"),
            "static_item" => self.visit_named_item(node, NodeKind::Constant, "const"),
            "impl_item" => self.visit_impl(node),
            _ => {}
        }
    }

    fn visit_fn(&mut self, node: TsNode<'_>, is_method: bool) {
        let Some(name) = field_text(node, "name", self.source) else {
            return;
        };
        let is_test = self.in_test_mod || has_test_attr(node, self.source);
        let kind = if is_test {
            NodeKind::Test
        } else if is_method {
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
        let params = field_text(node, "parameters", self.source).map(|s| s.to_owned());
        let ret = field_text(node, "return_type", self.source).map(|s| s.to_owned());

        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind,
            name: name.to_owned(),
            qualified_name: qn.clone(),
            file_path: self.rel_path.to_owned(),
            line_start: start_line(node),
            line_end: end_line(node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params,
            return_type: ret,
            modifiers: visibility_modifier(node, self.source),
            is_test: is_test || self.in_test_mod,
            file_hash: self.file_hash.to_owned(),
            extra_json: serde_json::Value::Null,
        });
        self.edges.push(contains_edge(
            &parent_qn,
            &qn,
            self.rel_path,
            start_line(node),
        ));
    }

    fn visit_mod(&mut self, node: TsNode<'_>) {
        let Some(name) = field_text(node, "name", self.source) else {
            return;
        };
        let parent_qn = self.current_parent_qn().to_owned();
        let suffix = qualified_suffix(&parent_qn, self.rel_path, name);
        let qn = format!("{}::module::{}", self.rel_path, suffix);

        // Detect #[cfg(test)] attribute on this mod.
        let was_test_mod = self.in_test_mod;
        let is_test_mod = self.in_test_mod || has_cfg_test(node, self.source);

        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Module,
            name: name.to_owned(),
            qualified_name: qn.clone(),
            file_path: self.rel_path.to_owned(),
            line_start: start_line(node),
            line_end: end_line(node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params: None,
            return_type: None,
            modifiers: visibility_modifier(node, self.source),
            is_test: is_test_mod,
            file_hash: self.file_hash.to_owned(),
            extra_json: serde_json::Value::Null,
        });
        self.edges.push(contains_edge(
            &parent_qn,
            &qn,
            self.rel_path,
            start_line(node),
        ));

        // Recurse into inline module body.
        if let Some(body) = node.child_by_field_name("body") {
            self.scope_stack.push(qn);
            self.in_test_mod = is_test_mod;
            self.walk_block(body);
            self.scope_stack.pop();
            self.in_test_mod = was_test_mod;
        }
    }

    fn visit_named_item(&mut self, node: TsNode<'_>, kind: NodeKind, type_prefix: &str) {
        let Some(name) = field_text(node, "name", self.source) else {
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
            line_start: start_line(node),
            line_end: end_line(node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params: None,
            return_type: None,
            modifiers: visibility_modifier(node, self.source),
            is_test: self.in_test_mod,
            file_hash: self.file_hash.to_owned(),
            extra_json: serde_json::Value::Null,
        });
        self.edges.push(contains_edge(
            &parent_qn,
            &qn,
            self.rel_path,
            start_line(node),
        ));
    }

    fn visit_impl(&mut self, node: TsNode<'_>) {
        let Some(type_name) = field_text(node, "type", self.source) else {
            return;
        };
        let trait_name = field_text(node, "trait", self.source);
        let parent_qn = self.current_parent_qn().to_owned();
        let impl_scope = format!("{}::impl::{}", self.rel_path, type_name);

        self.nodes.push(Node {
            id: NodeId::UNSET,
            kind: NodeKind::Module,
            name: format!("impl {type_name}"),
            qualified_name: impl_scope.clone(),
            file_path: self.rel_path.to_owned(),
            line_start: start_line(node),
            line_end: end_line(node),
            language: "rust".to_owned(),
            parent_name: Some(parent_qn.clone()),
            params: None,
            return_type: None,
            modifiers: None,
            is_test: self.in_test_mod,
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
            start_line(node),
        ));

        // Emit an `Implements` edge if this is `impl Trait for Type`.
        if let Some(trait_name) = trait_name
            && let (Some(type_qn), Some(trait_qn)) = (
                self.local_type_qn(type_name),
                self.local_trait_qn(trait_name),
            )
        {
            self.edges.push(Edge {
                id: 0,
                kind: EdgeKind::Implements,
                source_qn: type_qn,
                target_qn: trait_qn,
                file_path: self.rel_path.to_owned(),
                line: Some(start_line(node)),
                confidence: 0.9,
                confidence_tier: Some("same_file".to_owned()),
                extra_json: serde_json::Value::Null,
            });
        }

        // Walk methods in the impl body.
        if let Some(body) = node.child_by_field_name("body") {
            self.scope_stack.push(impl_scope);
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                if child.kind() == "function_item" {
                    self.visit_fn(child, true);
                }
            }
            self.scope_stack.pop();
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

/// Returns true if the node has a preceding `#[test]` attribute sibling.
fn has_test_attr(node: TsNode<'_>, source: &[u8]) -> bool {
    let mut sib = node.prev_named_sibling();
    while let Some(s) = sib {
        if s.kind() != "attribute_item" {
            break;
        }
        let text = node_text(s, source);
        if text.contains("test") && !text.contains("cfg") {
            return true;
        }
        sib = s.prev_named_sibling();
    }
    false
}

/// Returns true if the node has a preceding `#[cfg(test)]` attribute sibling.
fn has_cfg_test(node: TsNode<'_>, source: &[u8]) -> bool {
    let mut sib = node.prev_named_sibling();
    while let Some(s) = sib {
        if s.kind() != "attribute_item" {
            break;
        }
        let text = node_text(s, source);
        if text.contains("cfg") && text.contains("test") {
            return true;
        }
        sib = s.prev_named_sibling();
    }
    false
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
    // Build callable name → qualified_name map.
    let mut callables: HashMap<String, String> = HashMap::new();
    for n in nodes {
        if matches!(
            n.kind,
            NodeKind::Function | NodeKind::Method | NodeKind::Test
        ) {
            callables.insert(n.name.clone(), n.qualified_name.clone());
        }
    }

    let mut edges = Vec::new();
    let mut scope: Vec<String> = Vec::new();
    walk_for_rust_calls(root, source, rel_path, &callables, &mut scope, &mut edges);
    edges
}

fn walk_for_rust_calls<'a>(
    node: TsNode<'a>,
    source: &[u8],
    rel_path: &str,
    callables: &HashMap<String, String>,
    scope: &mut Vec<String>,
    edges: &mut Vec<Edge>,
) {
    match node.kind() {
        "function_item" => {
            let caller_qn = node
                .child_by_field_name("name")
                .and_then(|n| callables.get(node_text(n, source)));
            let pushed = if let Some(qn) = caller_qn {
                scope.push(qn.clone());
                true
            } else {
                false
            };
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                walk_for_rust_calls(child, source, rel_path, callables, scope, edges);
            }
            if pushed {
                scope.pop();
            }
            return; // already recursed
        }
        "call_expression" => {
            if let Some(caller_qn) = scope.last() {
                let called = node
                    .child_by_field_name("function")
                    .and_then(|f| rust_call_target(f, source));
                if let Some((text, name, receiver)) = called {
                    if !should_emit_rust_call(&name) {
                        return;
                    }
                    if is_self_call(caller_qn, &name, receiver.as_deref()) {
                        return;
                    }
                    if let Some(callee_qn) = callables.get(&name)
                        && callee_qn != caller_qn
                    {
                        edges.push(call_edge(
                            caller_qn,
                            callee_qn,
                            rel_path,
                            start_line(node),
                            &text,
                            receiver.as_deref(),
                            true,
                        ));
                    } else {
                        edges.push(call_edge(
                            caller_qn,
                            &text,
                            rel_path,
                            start_line(node),
                            &text,
                            receiver.as_deref(),
                            false,
                        ));
                    }
                }
            }
        }
        "method_call_expression" => {
            if let Some(caller_qn) = scope.last() {
                let called = rust_method_call_target(node, source);
                if let Some((text, name, receiver)) = called {
                    if is_self_call(caller_qn, &name, receiver.as_deref()) {
                        return;
                    }
                    if let Some(callee_qn) = callables.get(&name)
                        && callee_qn != caller_qn
                    {
                        edges.push(call_edge(
                            caller_qn,
                            callee_qn,
                            rel_path,
                            start_line(node),
                            &text,
                            receiver.as_deref(),
                            true,
                        ));
                    } else {
                        edges.push(call_edge(
                            caller_qn,
                            &text,
                            rel_path,
                            start_line(node),
                            &text,
                            receiver.as_deref(),
                            false,
                        ));
                    }
                }
            }
        }
        _ => {}
    }
    // Default recursive walk.
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_for_rust_calls(child, source, rel_path, callables, scope, edges);
    }
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

fn rust_method_call_target(
    node: TsNode<'_>,
    source: &[u8],
) -> Option<(String, String, Option<String>)> {
    let method = node.child_by_field_name("method")?;
    let receiver = node.child_by_field_name("receiver")?;
    let method_name = node_text(method, source).to_owned();
    let receiver_text = node_text(receiver, source).to_owned();
    Some((
        node_text(node, source).to_owned(),
        method_name,
        Some(receiver_text),
    ))
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
    .resolve(root)
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
    fn resolve(mut self, root: TsNode<'_>) -> Vec<Edge> {
        self.walk(root);
        self.edges
    }

    fn walk(&mut self, node: TsNode<'_>) {
        match node.kind() {
            "use_declaration" => {
                let source_qn = reference_source_qn(self.nodes, self.rel_path, start_line(node));
                for name in use_reference_names(node, self.source) {
                    let target_qn =
                        unique_target_qn(&self.symbol_targets, &name).map(str::to_owned);
                    self.maybe_push_reference_edge(
                        source_qn,
                        target_qn.as_deref(),
                        start_line(node),
                    );
                }
            }
            "type_identifier" | "scoped_type_identifier" if !is_definition_name(node) => {
                let source_qn = reference_source_qn(self.nodes, self.rel_path, start_line(node));
                let name = type_reference_name(node, self.source);
                let target_qn = unique_target_qn(&self.type_targets, &name).map(str::to_owned);
                self.maybe_push_reference_edge(source_qn, target_qn.as_deref(), start_line(node));
            }
            _ => {}
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.walk(child);
        }
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
    if let Some(argument) = node.child_by_field_name("argument") {
        collect_use_reference_names(argument, source, &mut names);
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
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
}
