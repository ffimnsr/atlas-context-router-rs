use std::borrow::Cow;

use atlas_core::{Edge, Node, NodeId, NodeKind, ParsedFile};
use regex::Regex;

use crate::ast_helpers::{end_line, field_text, file_node, node_text, start_line};
use crate::lang::common::{call_edge, contains_edge};
use crate::traits::{LangParser, ParseContext};

use super::emit::{attribute_node, import_node, named_child_text, symbol_parent_name, symbol_qn};
use super::{
    ExternalAttributeRule, ExternalAttributeTarget, ExternalImportRule, ExternalParserConfig,
    ExternalQnStyle, ExternalSymbolRule, supports_extension,
};

pub(super) struct GenericExternalParser {
    language: tree_sitter::Language,
    language_name: String,
    extensions: Vec<String>,
    symbol_rules: Vec<ExternalSymbolRule>,
    import_rules: Vec<ExternalImportRule>,
    attribute_rules: Vec<ExternalAttributeRule>,
    qn_style: ExternalQnStyle,
    owner_node_kinds: Vec<NodeKind>,
    test_name_patterns: Vec<Regex>,
    test_path_patterns: Vec<Regex>,
    call_node_kinds: Vec<String>,
    call_target_field: Option<String>,
    call_receiver_field: Option<String>,
}

impl GenericExternalParser {
    pub(super) fn new(config: &ExternalParserConfig, language: tree_sitter::Language) -> Self {
        let test_name_patterns = config
            .test_name_patterns
            .iter()
            .map(|pattern| {
                Regex::new(pattern).expect("test_name_patterns validated before parser build")
            })
            .collect();
        let test_path_patterns = config
            .test_path_patterns
            .iter()
            .map(|pattern| {
                Regex::new(pattern).expect("test_path_patterns validated before parser build")
            })
            .collect();

        Self {
            language,
            language_name: config.language_name.clone(),
            extensions: config.extensions.clone(),
            symbol_rules: config.symbols.clone(),
            import_rules: config.imports.clone(),
            attribute_rules: config.attributes.clone(),
            qn_style: config.qn_style,
            owner_node_kinds: config.owner_node_kinds.clone(),
            test_name_patterns,
            test_path_patterns,
            call_node_kinds: config.call_node_kinds.clone(),
            call_target_field: config.call_target_field.clone(),
            call_receiver_field: config.call_receiver_field.clone(),
        }
    }
}

impl LangParser for GenericExternalParser {
    fn language_name(&self) -> Cow<'static, str> {
        Cow::Owned(self.language_name.clone())
    }

    fn supports(&self, path: &str) -> bool {
        supports_extension(path, &self.extensions)
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&self.language)
            .expect("external grammar ABI validated at load time");

        let tree = crate::parse_runtime::parse_tree(&mut parser, ctx.source, ctx.old_tree);
        let mut nodes: Vec<Node> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();

        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(
            ctx.rel_path,
            ctx.file_hash,
            line_count,
            &self.language_name,
        ));

        if let Some(tree) = &tree {
            let root = tree.root_node();
            collect_imports(
                root,
                ctx.rel_path,
                ctx.file_hash,
                &self.language_name,
                &self.import_rules,
                ctx.source,
                &mut nodes,
                &mut edges,
            );

            let symbols = SymbolCollector {
                rel_path: ctx.rel_path,
                file_hash: ctx.file_hash,
                language: &self.language_name,
                source: ctx.source,
                rules: &self.symbol_rules,
                attributes: &self.attribute_rules,
                qn_style: self.qn_style,
                owner_node_kinds: &self.owner_node_kinds,
                test_name_patterns: &self.test_name_patterns,
                test_path_patterns: &self.test_path_patterns,
            };
            let mut attribute_index = 0usize;
            symbols.collect(
                root,
                ctx.rel_path,
                None,
                None,
                &mut attribute_index,
                &mut nodes,
                &mut edges,
            );

            collect_calls(
                root,
                ctx.rel_path,
                ctx.source,
                &self.call_node_kinds,
                self.call_target_field.as_deref(),
                self.call_receiver_field.as_deref(),
                &nodes,
                &mut edges,
            );
        }

        let pf = ParsedFile {
            path: ctx.rel_path.to_owned(),
            language: Some(self.language_name.clone()),
            hash: ctx.file_hash.to_owned(),
            size: Some(ctx.source.len() as i64),
            nodes,
            edges,
        };
        (pf, tree)
    }
}

struct SymbolCollector<'a> {
    rel_path: &'a str,
    file_hash: &'a str,
    language: &'a str,
    source: &'a [u8],
    rules: &'a [ExternalSymbolRule],
    attributes: &'a [ExternalAttributeRule],
    qn_style: ExternalQnStyle,
    owner_node_kinds: &'a [NodeKind],
    test_name_patterns: &'a [Regex],
    test_path_patterns: &'a [Regex],
}

struct PendingAttribute {
    name: String,
    line_start: u32,
    line_end: u32,
}

impl SymbolCollector<'_> {
    #[allow(clippy::too_many_arguments)]
    fn collect(
        &self,
        node: tree_sitter::Node<'_>,
        parent_qn: &str,
        owner_name: Option<&str>,
        owner_qn: Option<&str>,
        attribute_index: &mut usize,
        nodes: &mut Vec<Node>,
        edges: &mut Vec<Edge>,
    ) {
        let mut pending: Vec<PendingAttribute> = Vec::new();
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if let Some(rule) = self
                .attributes
                .iter()
                .find(|rule| rule.tree_kind == child.kind())
            {
                let Some(name) = self.attribute_name(child, rule) else {
                    continue;
                };
                match rule.attach_to {
                    ExternalAttributeTarget::Scope => self.emit_attribute(
                        name,
                        start_line(child),
                        end_line(child),
                        parent_qn,
                        attribute_index,
                        nodes,
                        edges,
                    ),
                    ExternalAttributeTarget::NextSymbol => pending.push(PendingAttribute {
                        name: name.to_owned(),
                        line_start: start_line(child),
                        line_end: end_line(child),
                    }),
                }
                continue;
            }

            if let Some(rule) = self
                .rules
                .iter()
                .find(|rule| rule.tree_kind == child.kind())
            {
                let Some(name) = field_text(child, &rule.name_field, self.source)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                else {
                    continue;
                };
                let qualified_name = self.symbol_qn(rule, name, parent_qn, owner_name);
                let (params, return_type, modifiers) = self.symbol_metadata(child, rule);
                let line_start = start_line(child);
                nodes.push(Node {
                    id: NodeId::UNSET,
                    kind: rule.node_kind,
                    name: name.to_owned(),
                    qualified_name: qualified_name.clone(),
                    file_path: self.rel_path.to_owned(),
                    line_start,
                    line_end: end_line(child),
                    language: self.language.to_owned(),
                    parent_name: self.symbol_parent_name(parent_qn, owner_qn),
                    params,
                    return_type,
                    modifiers,
                    is_test: self.is_test(name),
                    file_hash: self.file_hash.to_owned(),
                    extra_json: serde_json::Value::Null,
                    repo_provenance: None,
                });
                edges.push(contains_edge(
                    parent_qn,
                    &qualified_name,
                    self.rel_path,
                    line_start,
                ));

                for attribute in pending.drain(..) {
                    self.emit_attribute(
                        &attribute.name,
                        attribute.line_start,
                        attribute.line_end,
                        &qualified_name,
                        attribute_index,
                        nodes,
                        edges,
                    );
                }

                let owns_children = self.owner_node_kinds.contains(&rule.node_kind);
                let next_owner_name = if owns_children {
                    Some(name.to_owned())
                } else {
                    owner_name.map(str::to_owned)
                };
                let next_owner_qn = if owns_children {
                    Some(qualified_name.clone())
                } else {
                    owner_qn.map(str::to_owned)
                };
                self.collect(
                    child,
                    &qualified_name,
                    next_owner_name.as_deref(),
                    next_owner_qn.as_deref(),
                    attribute_index,
                    nodes,
                    edges,
                );
                continue;
            }

            pending.clear();
            self.collect(
                child,
                parent_qn,
                owner_name,
                owner_qn,
                attribute_index,
                nodes,
                edges,
            );
        }
    }

    fn symbol_qn(
        &self,
        rule: &ExternalSymbolRule,
        name: &str,
        parent_qn: &str,
        owner_name: Option<&str>,
    ) -> String {
        symbol_qn(
            self.qn_style,
            self.rel_path,
            rule.node_kind,
            name,
            parent_qn,
            owner_name,
        )
    }

    fn symbol_parent_name(&self, parent_qn: &str, owner_qn: Option<&str>) -> Option<String> {
        symbol_parent_name(self.qn_style, self.rel_path, parent_qn, owner_qn)
    }

    fn symbol_metadata(
        &self,
        node: tree_sitter::Node<'_>,
        rule: &ExternalSymbolRule,
    ) -> (Option<String>, Option<String>, Option<String>) {
        let params = rule
            .params_field
            .as_deref()
            .and_then(|field| field_text(node, field, self.source))
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        let return_type = rule
            .return_type_field
            .as_deref()
            .and_then(|field| field_text(node, field, self.source))
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);

        let mut modifiers = Vec::new();
        if !rule.modifier_node_kinds.is_empty() {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if rule
                    .modifier_node_kinds
                    .iter()
                    .any(|kind| kind == child.kind())
                {
                    let text = node_text(child, self.source).trim();
                    if !text.is_empty() {
                        modifiers.push(text.to_owned());
                    }
                }
            }
        }

        (
            params,
            return_type,
            (!modifiers.is_empty()).then(|| modifiers.join(" ")),
        )
    }

    fn attribute_name<'s>(
        &'s self,
        node: tree_sitter::Node<'_>,
        rule: &ExternalAttributeRule,
    ) -> Option<&'s str> {
        field_text(node, &rule.name_field, self.source)
            .or_else(|| named_child_text(node, self.source))
            .map(str::trim)
            .filter(|name| !name.is_empty())
    }

    #[allow(clippy::too_many_arguments)]
    fn emit_attribute(
        &self,
        name: &str,
        line_start: u32,
        line_end: u32,
        owner_qn: &str,
        attribute_index: &mut usize,
        nodes: &mut Vec<Node>,
        edges: &mut Vec<Edge>,
    ) {
        attribute_node(
            self.rel_path,
            self.file_hash,
            self.language,
            name,
            line_start,
            line_end,
            owner_qn,
            attribute_index,
            nodes,
            edges,
        );
    }

    fn is_test(&self, name: &str) -> bool {
        self.test_name_patterns
            .iter()
            .any(|pattern| pattern.is_match(name))
            || self
                .test_path_patterns
                .iter()
                .any(|pattern| pattern.is_match(self.rel_path))
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_imports(
    node: tree_sitter::Node<'_>,
    rel_path: &str,
    file_hash: &str,
    language: &str,
    rules: &[ExternalImportRule],
    source: &[u8],
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    if rules.is_empty() {
        return;
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(rule) = rules.iter().find(|rule| rule.tree_kind == child.kind()) {
            let name = match &rule.name_field {
                Some(field) => field_text(child, field, source),
                None => named_child_text(child, source),
            };
            if let Some(name) = name.map(str::trim).filter(|name| !name.is_empty()) {
                import_node(
                    rel_path,
                    file_hash,
                    language,
                    name,
                    start_line(child),
                    end_line(child),
                    nodes,
                    edges,
                );
            }
        }
        collect_imports(
            child, rel_path, file_hash, language, rules, source, nodes, edges,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn collect_calls(
    node: tree_sitter::Node<'_>,
    rel_path: &str,
    source: &[u8],
    call_kinds: &[String],
    target_field: Option<&str>,
    receiver_field: Option<&str>,
    nodes: &[Node],
    edges: &mut Vec<Edge>,
) {
    if call_kinds.is_empty() {
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if call_kinds.iter().any(|kind| kind == child.kind()) {
            let callee = target_field
                .and_then(|field| field_text(child, field, source))
                .filter(|text| !text.trim().is_empty())
                .map(str::trim)
                .or_else(|| {
                    named_child_text(child, source)
                        .map(str::trim)
                        .filter(|text| !text.is_empty())
                });
            if let Some(callee) = callee {
                let receiver = receiver_field
                    .and_then(|field| field_text(child, field, source))
                    .map(str::trim)
                    .filter(|text| !text.is_empty());
                let resolved = nodes
                    .iter()
                    .rev()
                    .find(|node| {
                        matches!(
                            node.kind,
                            NodeKind::Function
                                | NodeKind::Method
                                | NodeKind::Test
                                | NodeKind::Class
                                | NodeKind::Struct
                                | NodeKind::Trait
                                | NodeKind::Module
                                | NodeKind::Package
                        ) && node.name == callee
                    })
                    .map(|node| node.qualified_name.clone());
                let same_file = resolved.is_some();
                let target_qn = resolved.unwrap_or_else(|| format!("{rel_path}::{callee}"));
                let call_line = start_line(child);
                let source_qn = nodes
                    .iter()
                    .rev()
                    .find(|node| {
                        matches!(
                            node.kind,
                            NodeKind::Function
                                | NodeKind::Method
                                | NodeKind::Test
                                | NodeKind::Class
                                | NodeKind::Struct
                                | NodeKind::Trait
                                | NodeKind::Module
                                | NodeKind::Package
                        ) && node.line_start <= call_line
                            && node.line_end >= call_line
                    })
                    .map(|node| node.qualified_name.clone())
                    .unwrap_or_else(|| rel_path.to_owned());
                edges.push(call_edge(
                    &source_qn, &target_qn, rel_path, call_line, callee, receiver, same_file,
                ));
            }
        }
        collect_calls(
            child,
            rel_path,
            source,
            call_kinds,
            target_field,
            receiver_field,
            nodes,
            edges,
        );
    }
}
