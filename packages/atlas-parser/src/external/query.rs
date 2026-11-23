//! SCM query/capture extraction for external parsers.
//!
//! Query mode maps `@atlas.*` captures onto the same node and edge shapes as
//! rule mode. Query text is compiled once at construction; malformed matches
//! are skipped instead of panicking so a hostile grammar cannot crash a scan.

use std::borrow::Cow;
use std::collections::HashSet;
use std::fs;

use anyhow::{Context, Result, bail, ensure};
use atlas_core::{Edge, Node, NodeKind, ParsedFile};
use regex::Regex;
use tree_sitter::{Node as TsNode, Query};

use crate::ast_helpers::{end_line, file_node, node_text, start_line};
use crate::lang::common::{call_edge, contains_edge};
use crate::query_helpers::{QueryCaptureGroup, compile_query, run_query};
use crate::traits::{LangParser, ParseContext};

use super::emit::{
    attribute_node, import_node, named_child_text, reference_edge, symbol_parent_name, symbol_qn,
};
use super::{ExternalParserConfig, ExternalQnStyle, supports_extension};

const DEFINITION_PREFIX: &str = "atlas.definition.";

pub(super) struct QueryExternalParser {
    language: tree_sitter::Language,
    language_name: String,
    extensions: Vec<String>,
    query: Query,
    qn_style: ExternalQnStyle,
    owner_node_kinds: Vec<NodeKind>,
    test_name_patterns: Vec<Regex>,
    test_path_patterns: Vec<Regex>,
}

struct RawDefinition<'tree> {
    node: TsNode<'tree>,
    kind: NodeKind,
    name: String,
    params: Option<String>,
    return_type: Option<String>,
    parent: Option<usize>,
    owner: Option<usize>,
    qn: String,
    parent_name: Option<String>,
}

struct RawAttribute<'tree> {
    node: TsNode<'tree>,
    name: String,
    owner_def: Option<usize>,
}

struct RawImport {
    name: String,
    line_start: u32,
    line_end: u32,
}

struct RawCall<'tree> {
    node: TsNode<'tree>,
    callee: String,
    receiver: Option<String>,
}

struct RawReference<'tree> {
    node: TsNode<'tree>,
    name: String,
}

impl QueryExternalParser {
    pub(super) fn new(
        config: &ExternalParserConfig,
        language: tree_sitter::Language,
    ) -> Result<Self> {
        let text = query_text(config)?;
        let query = compile_query(language.clone(), &text).map_err(|error| {
            anyhow::anyhow!(
                "parsers.external '{}' query failed: {error}",
                config.language_name
            )
        })?;
        validate_captures(&query, &config.language_name)?;

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

        Ok(Self {
            language,
            language_name: config.language_name.clone(),
            extensions: config.extensions.clone(),
            query,
            qn_style: config.qn_style,
            owner_node_kinds: config.owner_node_kinds.clone(),
            test_name_patterns,
            test_path_patterns,
        })
    }

    fn is_test(&self, name: &str, rel_path: &str) -> bool {
        self.test_name_patterns
            .iter()
            .any(|pattern| pattern.is_match(name))
            || self
                .test_path_patterns
                .iter()
                .any(|pattern| pattern.is_match(rel_path))
    }

    fn extract<'tree>(
        &self,
        root: TsNode<'tree>,
        rel_path: &str,
        file_hash: &str,
        source: &'tree [u8],
        nodes: &mut Vec<Node>,
        edges: &mut Vec<Edge>,
    ) {
        let groups = run_query(&self.query, root, source);
        let mut definitions: Vec<RawDefinition<'tree>> = Vec::new();
        let mut attributes: Vec<RawAttribute<'tree>> = Vec::new();
        let mut imports: Vec<RawImport> = Vec::new();
        let mut calls: Vec<RawCall<'tree>> = Vec::new();
        let mut references: Vec<RawReference<'tree>> = Vec::new();

        let mut seen_defs = HashSet::new();
        let mut seen_attrs = HashSet::new();
        let mut seen_imports = HashSet::new();
        let mut seen_calls = HashSet::new();
        let mut seen_refs = HashSet::new();

        for group in &groups {
            let name = capture_text(group, "atlas.name", source);
            let mut group_defs = Vec::new();
            for capture in &group.captures {
                let Some(kind_name) = capture.name.strip_prefix(DEFINITION_PREFIX) else {
                    continue;
                };
                let Some(kind) = definition_kind(kind_name) else {
                    continue;
                };
                let Some(name) = name else {
                    continue;
                };
                let key = (capture.node.start_byte(), capture.node.end_byte(), kind);
                if !seen_defs.insert(key) {
                    continue;
                }
                group_defs.push(definitions.len());
                definitions.push(RawDefinition {
                    node: capture.node,
                    kind,
                    name: name.to_owned(),
                    params: capture_text(group, "atlas.parameters", source).map(str::to_owned),
                    return_type: capture_text(group, "atlas.return_type", source)
                        .map(str::to_owned),
                    parent: None,
                    owner: None,
                    qn: String::new(),
                    parent_name: None,
                });
            }

            for capture in &group.captures {
                if capture.name != "atlas.attribute" {
                    continue;
                }
                let Some(name) = name.or_else(|| non_empty(node_text(capture.node, source))) else {
                    continue;
                };
                let key = (
                    capture.node.start_byte(),
                    capture.node.end_byte(),
                    name.to_owned(),
                );
                if !seen_attrs.insert(key) {
                    continue;
                }
                attributes.push(RawAttribute {
                    node: capture.node,
                    name: name.to_owned(),
                    owner_def: group_defs.first().copied(),
                });
            }

            for capture in &group.captures {
                match capture.name.as_str() {
                    "atlas.import" => {
                        let Some(name) = name
                            .or_else(|| named_child_text(capture.node, source))
                            .and_then(non_empty)
                        else {
                            continue;
                        };
                        let key = (
                            capture.node.start_byte(),
                            capture.node.end_byte(),
                            name.to_owned(),
                        );
                        if seen_imports.insert(key) {
                            imports.push(RawImport {
                                name: name.to_owned(),
                                line_start: start_line(capture.node),
                                line_end: end_line(capture.node),
                            });
                        }
                    }
                    "atlas.call" => {
                        let Some(callee) = name
                            .or_else(|| named_child_text(capture.node, source))
                            .and_then(non_empty)
                        else {
                            continue;
                        };
                        let key = (
                            capture.node.start_byte(),
                            capture.node.end_byte(),
                            callee.to_owned(),
                        );
                        if seen_calls.insert(key) {
                            calls.push(RawCall {
                                node: capture.node,
                                callee: callee.to_owned(),
                                receiver: capture_text(group, "atlas.receiver", source)
                                    .map(str::to_owned),
                            });
                        }
                    }
                    "atlas.reference" => {
                        let Some(name) =
                            name.or_else(|| non_empty(node_text(capture.node, source)))
                        else {
                            continue;
                        };
                        let key = (
                            capture.node.start_byte(),
                            capture.node.end_byte(),
                            name.to_owned(),
                        );
                        if seen_refs.insert(key) {
                            references.push(RawReference {
                                node: capture.node,
                                name: name.to_owned(),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

        resolve_definitions(
            &mut definitions,
            self.qn_style,
            &self.owner_node_kinds,
            rel_path,
        );

        for definition in &definitions {
            let line_start = start_line(definition.node);
            nodes.push(Node {
                id: atlas_core::NodeId::UNSET,
                kind: definition.kind,
                name: definition.name.clone(),
                qualified_name: definition.qn.clone(),
                file_path: rel_path.to_owned(),
                line_start,
                line_end: end_line(definition.node),
                language: self.language_name.clone(),
                parent_name: definition.parent_name.clone(),
                params: definition.params.clone(),
                return_type: definition.return_type.clone(),
                modifiers: None,
                is_test: definition.kind == NodeKind::Test
                    || self.is_test(&definition.name, rel_path),
                file_hash: file_hash.to_owned(),
                extra_json: serde_json::Value::Null,
                repo_provenance: None,
            });
            let parent_qn = definition
                .parent
                .map(|index| definitions[index].qn.as_str())
                .unwrap_or(rel_path);
            edges.push(contains_edge(
                parent_qn,
                &definition.qn,
                rel_path,
                line_start,
            ));
        }

        let mut attribute_index = 0usize;
        for attribute in &attributes {
            let owner_qn = attribute
                .owner_def
                .map(|index| definitions[index].qn.as_str())
                .or_else(|| {
                    enclosing_definition(&definitions, attribute.node)
                        .map(|index| definitions[index].qn.as_str())
                })
                .unwrap_or(rel_path);
            attribute_node(
                rel_path,
                file_hash,
                &self.language_name,
                &attribute.name,
                start_line(attribute.node),
                end_line(attribute.node),
                owner_qn,
                &mut attribute_index,
                nodes,
                edges,
            );
        }

        for import in &imports {
            import_node(
                rel_path,
                file_hash,
                &self.language_name,
                &import.name,
                import.line_start,
                import.line_end,
                nodes,
                edges,
            );
        }

        for call in &calls {
            let target = resolve_target(&definitions, &call.callee);
            let same_file = target.is_some();
            let target_qn = target
                .map(|index| definitions[index].qn.clone())
                .unwrap_or_else(|| format!("{}::{}", rel_path, call.callee));
            let source_qn = enclosing_definition(&definitions, call.node)
                .map(|index| definitions[index].qn.clone())
                .unwrap_or_else(|| rel_path.to_owned());
            edges.push(call_edge(
                &source_qn,
                &target_qn,
                rel_path,
                start_line(call.node),
                &call.callee,
                call.receiver.as_deref(),
                same_file,
            ));
        }

        for reference in &references {
            let target = resolve_target(&definitions, &reference.name);
            let same_file = target.is_some();
            let target_qn = target
                .map(|index| definitions[index].qn.clone())
                .unwrap_or_else(|| format!("{}::{}", rel_path, reference.name));
            let source_qn = enclosing_definition(&definitions, reference.node)
                .map(|index| definitions[index].qn.clone())
                .unwrap_or_else(|| rel_path.to_owned());
            edges.push(reference_edge(
                &source_qn,
                &target_qn,
                rel_path,
                start_line(reference.node),
                same_file,
            ));
        }
    }
}

impl LangParser for QueryExternalParser {
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
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        let line_count = ctx.source.iter().filter(|&&byte| byte == b'\n').count() as u32 + 1;
        nodes.push(file_node(
            ctx.rel_path,
            ctx.file_hash,
            line_count,
            &self.language_name,
        ));

        if let Some(tree) = &tree {
            self.extract(
                tree.root_node(),
                ctx.rel_path,
                ctx.file_hash,
                ctx.source,
                &mut nodes,
                &mut edges,
            );
        }

        (
            ParsedFile {
                path: ctx.rel_path.to_owned(),
                language: Some(self.language_name.clone()),
                hash: ctx.file_hash.to_owned(),
                size: Some(ctx.source.len() as i64),
                nodes,
                edges,
            },
            tree,
        )
    }
}

fn query_text(config: &ExternalParserConfig) -> Result<String> {
    if let Some(query) = &config.query {
        return Ok(query.clone());
    }
    let path = config
        .query_path
        .as_deref()
        .context("query mode requires query_path or query")?;
    fs::read_to_string(path).with_context(|| {
        format!(
            "parsers.external '{}' cannot read query file {path}",
            config.language_name
        )
    })
}

fn validate_captures(query: &Query, label: &str) -> Result<()> {
    for name in query.capture_names() {
        let Some(rest) = name.strip_prefix("atlas.") else {
            continue;
        };
        if matches!(
            rest,
            "name"
                | "import"
                | "attribute"
                | "call"
                | "receiver"
                | "parameters"
                | "return_type"
                | "reference"
        ) {
            continue;
        }
        if let Some(kind) = rest.strip_prefix("definition.") {
            ensure!(
                definition_kind(kind).is_some(),
                "parsers.external '{label}' query uses unsupported capture @{name}"
            );
            continue;
        }
        bail!("parsers.external '{label}' query uses unsupported capture @{name}");
    }
    Ok(())
}

fn definition_kind(name: &str) -> Option<NodeKind> {
    match name {
        "function" => Some(NodeKind::Function),
        "method" => Some(NodeKind::Method),
        "class" => Some(NodeKind::Class),
        "interface" => Some(NodeKind::Interface),
        "struct" => Some(NodeKind::Struct),
        "enum" => Some(NodeKind::Enum),
        "trait" => Some(NodeKind::Trait),
        "module" => Some(NodeKind::Module),
        "package" => Some(NodeKind::Package),
        "variable" => Some(NodeKind::Variable),
        "constant" | "const" => Some(NodeKind::Constant),
        "test" => Some(NodeKind::Test),
        _ => None,
    }
}

fn capture_text<'source>(
    group: &QueryCaptureGroup<'_>,
    name: &str,
    source: &'source [u8],
) -> Option<&'source str> {
    group
        .capture(name)
        .map(|capture| node_text(capture.node, source))
        .and_then(non_empty)
}

fn non_empty(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then_some(trimmed)
}

fn resolve_definitions(
    definitions: &mut [RawDefinition<'_>],
    style: ExternalQnStyle,
    owner_node_kinds: &[NodeKind],
    rel_path: &str,
) {
    let mut order: Vec<usize> = (0..definitions.len()).collect();
    order.sort_by_key(|&index| {
        (
            definitions[index].node.start_byte(),
            std::cmp::Reverse(definitions[index].node.end_byte()),
        )
    });

    let mut stack: Vec<usize> = Vec::new();
    for &index in &order {
        while let Some(&top) = stack.last() {
            if definitions[top].node.end_byte() <= definitions[index].node.start_byte() {
                stack.pop();
            } else {
                break;
            }
        }
        let parent = stack.last().copied().filter(|&candidate| {
            let outer = definitions[candidate].node;
            let inner = definitions[index].node;
            outer.start_byte() <= inner.start_byte()
                && outer.end_byte() >= inner.end_byte()
                && (outer.start_byte() < inner.start_byte() || outer.end_byte() > inner.end_byte())
        });
        definitions[index].parent = parent;
        stack.push(index);
    }

    for index in 0..definitions.len() {
        let owner = owner_ancestor(index, definitions, owner_node_kinds);
        definitions[index].owner = owner;
    }

    let mut by_span: Vec<usize> = (0..definitions.len()).collect();
    by_span.sort_by_key(|&index| {
        std::cmp::Reverse(definitions[index].node.end_byte() - definitions[index].node.start_byte())
    });
    for &index in &by_span {
        let kind = definitions[index].kind;
        let name = definitions[index].name.clone();
        let parent_qn = definitions[index]
            .parent
            .map(|parent| definitions[parent].qn.clone());
        let owner_name = definitions[index]
            .owner
            .map(|owner| definitions[owner].name.clone());
        let owner_qn = definitions[index]
            .owner
            .map(|owner| definitions[owner].qn.clone());
        let parent_or_file = parent_qn.as_deref().unwrap_or(rel_path);
        definitions[index].qn = symbol_qn(
            style,
            rel_path,
            kind,
            &name,
            parent_or_file,
            owner_name.as_deref(),
        );
        definitions[index].parent_name =
            symbol_parent_name(style, rel_path, parent_or_file, owner_qn.as_deref());
    }
}

fn owner_ancestor(
    index: usize,
    definitions: &[RawDefinition<'_>],
    owner_node_kinds: &[NodeKind],
) -> Option<usize> {
    let mut current = definitions[index].parent;
    while let Some(candidate) = current {
        if owner_node_kinds.contains(&definitions[candidate].kind) {
            return Some(candidate);
        }
        current = definitions[candidate].parent;
    }
    None
}

fn enclosing_definition(definitions: &[RawDefinition<'_>], node: TsNode<'_>) -> Option<usize> {
    definitions
        .iter()
        .enumerate()
        .filter(|(_, definition)| {
            definition.node.start_byte() <= node.start_byte()
                && definition.node.end_byte() >= node.end_byte()
        })
        .min_by_key(|(_, definition)| definition.node.end_byte() - definition.node.start_byte())
        .map(|(index, _)| index)
}

fn resolve_target(definitions: &[RawDefinition<'_>], name: &str) -> Option<usize> {
    definitions
        .iter()
        .rposition(|definition| definition.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_core::EdgeKind;

    fn query_config(query: &str) -> ExternalParserConfig {
        ExternalParserConfig {
            language_name: "rust".to_owned(),
            extensions: vec!["rs".to_owned()],
            grammar_dir: Some("grammars/tree-sitter-rust".to_owned()),
            query: Some(query.to_owned()),
            ..Default::default()
        }
    }

    fn parse(config: ExternalParserConfig, source: &str) -> ParsedFile {
        let parser = QueryExternalParser::new(&config, tree_sitter_rust::LANGUAGE.into())
            .expect("query parser");
        let context = ParseContext {
            rel_path: "src/test.rs",
            file_hash: "hash:test",
            source: source.as_bytes(),
            old_tree: None,
        };
        let (parsed, _) = parser.parse(&context);
        parsed
    }

    fn node_named<'a>(parsed: &'a ParsedFile, name: &str) -> &'a Node {
        parsed
            .nodes
            .iter()
            .find(|node| node.name == name)
            .unwrap_or_else(|| panic!("missing node {name}"))
    }

    #[test]
    fn extracts_definitions_and_names() {
        let parsed = parse(
            query_config(
                "(function_item name: (identifier) @atlas.name) @atlas.definition.function\n\
                 (struct_item name: (type_identifier) @atlas.name) @atlas.definition.struct",
            ),
            "fn greet() {}\nstruct Widget;\n",
        );

        let greet = node_named(&parsed, "greet");
        assert_eq!(greet.kind, NodeKind::Function);
        assert_eq!(greet.qualified_name, "src/test.rs::function::greet");
        let widget = node_named(&parsed, "Widget");
        assert_eq!(widget.kind, NodeKind::Struct);
        assert_eq!(widget.qualified_name, "src/test.rs::struct::Widget");
        assert!(
            parsed
                .edges
                .iter()
                .any(|edge| edge.kind == EdgeKind::Contains
                    && edge.target_qn == "src/test.rs::function::greet")
        );
    }

    #[test]
    fn extracts_imports() {
        let parsed = parse(
            query_config(
                "(function_item name: (identifier) @atlas.name) @atlas.definition.function\n\
                 (use_declaration) @atlas.import",
            ),
            "use std::fmt;\nfn greet() {}\n",
        );

        let import = parsed
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Import)
            .expect("import node");
        assert_eq!(import.qualified_name, "src/test.rs::import::std::fmt");
        assert!(
            parsed
                .edges
                .iter()
                .any(|edge| edge.kind == EdgeKind::Imports
                    && edge.target_qn == import.qualified_name)
        );
    }

    #[test]
    fn extracts_calls_and_receivers() {
        let parsed = parse(
            query_config(
                "(function_item name: (identifier) @atlas.name) @atlas.definition.function\n\
                 (call_expression function: (identifier) @atlas.name) @atlas.call\n\
                 (call_expression function: (field_expression value: (_) @atlas.receiver field: (field_identifier) @atlas.name)) @atlas.call",
            ),
            "fn callee() {}\nfn caller() { callee(); obj.run(); }\n",
        );

        let direct = parsed
            .edges
            .iter()
            .find(|edge| {
                edge.kind == EdgeKind::Calls && edge.target_qn == "src/test.rs::function::callee"
            })
            .expect("direct call edge");
        assert_eq!(direct.confidence, 0.8);
        assert_eq!(direct.confidence_tier.as_deref(), Some("same_file"));
        assert_eq!(direct.source_qn, "src/test.rs::function::caller");

        let method = parsed
            .edges
            .iter()
            .find(|edge| edge.kind == EdgeKind::Calls && edge.target_qn == "src/test.rs::run")
            .expect("method call edge");
        assert_eq!(method.confidence, 0.3);
        assert_eq!(method.confidence_tier.as_deref(), Some("text"));
        assert_eq!(method.extra_json["receiver_text"].as_str(), Some("obj"));
    }

    #[test]
    fn extracts_parameters_and_return_type() {
        let parsed = parse(
            query_config(
                "(function_item\n\
                   name: (identifier) @atlas.name\n\
                   parameters: (parameters) @atlas.parameters\n\
                   return_type: (_) @atlas.return_type) @atlas.definition.function",
            ),
            "fn add(a: i32, b: i32) -> i32 { a + b }\n",
        );

        let add = node_named(&parsed, "add");
        assert_eq!(add.params.as_deref(), Some("(a: i32, b: i32)"));
        assert_eq!(add.return_type.as_deref(), Some("i32"));
    }

    #[test]
    fn extracts_attributes_at_file_scope() {
        let parsed = parse(
            query_config(
                "(function_item name: (identifier) @atlas.name) @atlas.definition.function\n\
                 (attribute_item (attribute (identifier) @atlas.name)) @atlas.attribute",
            ),
            "#[derive(Debug)]\nstruct Widget;\nfn greet() {}\n",
        );

        let attribute = parsed
            .nodes
            .iter()
            .find(|node| node.modifiers.as_deref() == Some("attribute"))
            .expect("attribute node");
        assert_eq!(attribute.name, "derive");
        assert_eq!(attribute.qualified_name, "src/test.rs::attribute::derive#1");
    }

    #[test]
    fn test_name_patterns_mark_nodes() {
        let mut parser_config = query_config(
            "(function_item name: (identifier) @atlas.name) @atlas.definition.function",
        );
        parser_config.test_name_patterns = vec!["^test_".to_owned()];
        let parsed = parse(parser_config, "fn test_greet() {}\nfn greet() {}\n");

        assert!(node_named(&parsed, "test_greet").is_test);
        assert!(!node_named(&parsed, "greet").is_test);
    }

    #[test]
    fn references_emit_reference_edges() {
        let parsed = parse(
            query_config(
                "(function_item name: (identifier) @atlas.name) @atlas.definition.function\n\
                 (struct_item name: (type_identifier) @atlas.name) @atlas.definition.struct\n\
                 (type_identifier) @atlas.reference",
            ),
            "struct Widget;\nfn make() -> Widget { Widget }\n",
        );

        assert!(
            parsed
                .edges
                .iter()
                .any(|edge| edge.kind == EdgeKind::References
                    && edge.target_qn == "src/test.rs::struct::Widget"
                    && edge.source_qn == "src/test.rs::function::make"),
            "nodes: {:?}\nedges: {:?}",
            parsed.nodes,
            parsed.edges
        );
    }

    #[test]
    fn reads_query_path_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("rust.scm");
        std::fs::write(
            &path,
            "(function_item name: (identifier) @atlas.name) @atlas.definition.function",
        )
        .expect("write query");

        let mut parser_config = query_config("");
        parser_config.query = None;
        parser_config.query_path = Some(path.to_string_lossy().into_owned());
        let parsed = parse(parser_config, "fn greet() {}\n");
        assert_eq!(
            node_named(&parsed, "greet").qualified_name,
            "src/test.rs::function::greet"
        );
    }

    #[test]
    fn external_lang_parser_dispatches_query_mode() {
        let parser_config = query_config(
            "(function_item name: (identifier) @atlas.name) @atlas.definition.function",
        );
        let parser = crate::ExternalLangParser::from_language(
            &parser_config,
            tree_sitter_rust::LANGUAGE.into(),
        )
        .expect("query-mode external parser");
        assert_eq!(parser.language_name(), "rust");
        assert!(parser.supports("src/test.rs"));

        let context = ParseContext {
            rel_path: "src/test.rs",
            file_hash: "hash:test",
            source: b"fn greet() {}\n",
            old_tree: None,
        };
        let (parsed, _) = parser.parse(&context);
        assert!(parsed.nodes.iter().any(|node| node.name == "greet"));
    }

    #[test]
    fn invalid_query_fails_construction() {
        let error = QueryExternalParser::new(
            &query_config("(not_a_real_node) @atlas.definition.function"),
            tree_sitter_rust::LANGUAGE.into(),
        )
        .err()
        .expect("invalid query must fail");
        assert!(
            error.to_string().contains("query failed"),
            "unexpected error: {error}"
        );
    }

    #[test]
    fn unknown_capture_fails_construction() {
        let error = QueryExternalParser::new(
            &query_config("(function_item name: (identifier) @atlas.bogus)"),
            tree_sitter_rust::LANGUAGE.into(),
        )
        .err()
        .expect("unknown capture must fail");
        assert!(
            error.to_string().contains("unsupported capture"),
            "unexpected error: {error}"
        );
    }
}
