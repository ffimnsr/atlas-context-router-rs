//! Config-driven external language support backed by `tree-sitter-loader`.
//!
//! Users register extra languages in `.atlas/config.toml` under
//! `[parsers.external]` by pointing at a tree-sitter grammar checkout (compiled
//! on first use and cached) or a prebuilt grammar shared library, and by
//! describing which tree-sitter node kinds map to graph symbols and calls.
//! Built-in handlers always win for overlapping extensions; external parsers
//! are appended after them in the registry.

use std::borrow::Cow;
use std::path::{Path, PathBuf};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use tree_sitter_loader::{CompileConfig, Loader};

use atlas_core::{Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile};

use crate::ast_helpers::{end_line, field_text, node_text, start_line};
use crate::traits::{LangParser, ParseContext};

/// Rule mapping one tree-sitter node kind to a graph symbol node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalSymbolRule {
    /// Tree-sitter node kind treated as a symbol (e.g. `"function_item"`).
    pub tree_kind: String,
    /// Graph node kind emitted for this tree kind.
    pub node_kind: NodeKind,
    /// Tree-sitter field holding the symbol name (default `"name"`).
    #[serde(default = "default_name_field")]
    pub name_field: String,
}

fn default_name_field() -> String {
    "name".to_owned()
}

/// One external language registration (serde-compatible with
/// `[parsers.external]` in `.atlas/config.toml`).
///
/// Relative `grammar_dir` / `lib_path` / `grammar_lib_dir` paths are resolved
/// against the atlas dir (`.atlas/`) by `Config::load` before the registry is
/// built.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalParserConfig {
    /// Language name stored on graph nodes (e.g. `"zig"`).
    pub language_name: String,
    /// File extensions registered for this parser (no leading dot).
    pub extensions: Vec<String>,
    /// Directory containing the grammar checkout (must hold `src/parser.c`
    /// and `src/grammar.json`). The grammar is compiled with a C compiler on
    /// first use and cached in `grammar_lib_dir`.
    #[serde(default)]
    pub grammar_dir: Option<String>,
    /// Path to a prebuilt grammar shared library (e.g.
    /// `/opt/grammars/libtree-sitter-zig.so`). Mutually exclusive with
    /// `grammar_dir`.
    #[serde(default)]
    pub lib_path: Option<String>,
    /// Exported symbol inside `lib_path`. Defaults to
    /// `tree_sitter_<language_name>` (hyphens replaced with underscores).
    #[serde(default)]
    pub lib_function: Option<String>,
    /// Directory where compiled grammars are cached. Defaults to the
    /// tree-sitter user cache (overridable with `TREE_SITTER_LIBDIR`).
    #[serde(default)]
    pub grammar_lib_dir: Option<String>,
    /// Symbol rules: which tree node kinds become graph symbols.
    pub symbols: Vec<ExternalSymbolRule>,
    /// Tree node kinds treated as call sites. Edges connect the enclosing
    /// symbol to a same-file symbol whose name matches the callee text.
    #[serde(default)]
    pub call_node_kinds: Vec<String>,
    /// Field of a call node that names the callee (e.g. `"function"`).
    /// Defaults to the last named child.
    #[serde(default)]
    pub call_target_field: Option<String>,
}

/// A runtime-loaded, config-driven language parser.
pub struct ExternalLangParser {
    language: tree_sitter::Language,
    language_name: String,
    extensions: Vec<String>,
    symbol_rules: Vec<ExternalSymbolRule>,
    call_node_kinds: Vec<String>,
    call_target_field: Option<String>,
}

impl ExternalLangParser {
    /// Load the grammar and validate it against the runtime ABI range.
    pub fn new(config: &ExternalParserConfig) -> Result<Self> {
        ensure!(
            !config.language_name.is_empty(),
            "external parser language_name must not be empty"
        );
        ensure!(
            !config.extensions.is_empty(),
            "external parser '{}' must declare at least one extension",
            config.language_name
        );
        ensure!(
            !config.symbols.is_empty(),
            "external parser '{}' must declare at least one symbol rule",
            config.language_name
        );
        ensure!(
            config.grammar_dir.is_some() != config.lib_path.is_some(),
            "external parser '{}' must configure exactly one of grammar_dir or lib_path",
            config.language_name
        );

        let language = if let Some(lib_path) = &config.lib_path {
            let function = config
                .lib_function
                .clone()
                .unwrap_or_else(|| lib_function_name(&config.language_name));
            Loader::load_language(Path::new(lib_path), &function).map_err(|error| {
                anyhow::anyhow!(
                    "cannot load external grammar library {} (symbol {function}): {error}",
                    lib_path
                )
            })?
        } else {
            let grammar_dir = config.grammar_dir.as_deref().expect("validated above");
            let src_path = PathBuf::from(grammar_dir).join("src");
            ensure!(
                src_path.join("parser.c").exists(),
                "grammar_dir '{grammar_dir}' has no src/parser.c (point at a tree-sitter grammar checkout)"
            );
            let loader = match &config.grammar_lib_dir {
                Some(cache) => Loader::with_parser_lib_path(PathBuf::from(cache)),
                None => Loader::new().map_err(|error| {
                    anyhow::anyhow!("cannot initialize grammar loader: {error}")
                })?,
            };
            loader
                .load_language_at_path(CompileConfig::new(&src_path, None, None))
                .map_err(|error| {
                    anyhow::anyhow!(
                        "cannot build external grammar '{}' from {grammar_dir}: {error}",
                        config.language_name
                    )
                })?
        };

        let abi = language.abi_version();
        ensure!(
            (tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
                .contains(&abi),
            "external parser '{}' grammar ABI {abi} outside supported range {}..={}",
            config.language_name,
            tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION,
            tree_sitter::LANGUAGE_VERSION
        );

        Ok(Self {
            language,
            language_name: config.language_name.clone(),
            extensions: config.extensions.clone(),
            symbol_rules: config.symbols.clone(),
            call_node_kinds: config.call_node_kinds.clone(),
            call_target_field: config.call_target_field.clone(),
        })
    }
}

fn lib_function_name(language_name: &str) -> String {
    format!("tree_sitter_{}", language_name.replace('-', "_"))
}

impl LangParser for ExternalLangParser {
    fn language_name(&self) -> Cow<'static, str> {
        Cow::Owned(self.language_name.clone())
    }

    fn supports(&self, path: &str) -> bool {
        let Some(extension) = path.rsplit('.').next() else {
            return false;
        };
        self.extensions
            .iter()
            .any(|candidate| candidate == extension)
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
            &self.language_name,
            line_count,
        ));

        if let Some(tree) = &tree {
            let root = tree.root_node();
            collect_symbols(
                root,
                ctx.rel_path,
                ctx.file_hash,
                &self.language_name,
                &self.symbol_rules,
                ctx.source,
                ctx.rel_path,
                &mut nodes,
                &mut edges,
            );
            collect_calls(
                root,
                ctx.rel_path,
                ctx.source,
                &self.call_node_kinds,
                self.call_target_field.as_deref(),
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

fn file_node(rel_path: &str, file_hash: &str, language: &str, line_end: u32) -> Node {
    Node {
        id: NodeId::UNSET,
        kind: NodeKind::File,
        name: rel_path.rsplit('/').next().unwrap_or(rel_path).to_owned(),
        qualified_name: rel_path.to_owned(),
        file_path: rel_path.to_owned(),
        line_start: 1,
        line_end,
        language: language.to_owned(),
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

/// Recursively emit symbols. Nested symbols nest their qualified name under
/// the nearest enclosing symbol, mirroring built-in handler conventions.
#[allow(clippy::too_many_arguments)]
fn collect_symbols(
    node: tree_sitter::Node<'_>,
    rel_path: &str,
    file_hash: &str,
    language: &str,
    rules: &[ExternalSymbolRule],
    source: &[u8],
    parent_qn: &str,
    nodes: &mut Vec<Node>,
    edges: &mut Vec<Edge>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(rule) = rules.iter().find(|rule| rule.tree_kind == child.kind()) {
            let Some(name) = field_text(child, &rule.name_field, source) else {
                continue;
            };
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let qualified_name = format!("{parent_qn}::{}::{name}", rule.node_kind.as_str());
            let line_start = start_line(child);
            nodes.push(Node {
                id: NodeId::UNSET,
                kind: rule.node_kind,
                name: name.to_owned(),
                qualified_name: qualified_name.clone(),
                file_path: rel_path.to_owned(),
                line_start,
                line_end: end_line(child),
                language: language.to_owned(),
                parent_name: (parent_qn != rel_path).then_some(parent_qn.to_owned()),
                params: None,
                return_type: None,
                modifiers: None,
                is_test: false,
                file_hash: file_hash.to_owned(),
                extra_json: serde_json::Value::Null,
                repo_provenance: None,
            });
            edges.push(contains_edge(
                parent_qn,
                &qualified_name,
                rel_path,
                line_start,
            ));
            collect_symbols(
                child,
                rel_path,
                file_hash,
                language,
                rules,
                source,
                &qualified_name,
                nodes,
                edges,
            );
        } else {
            collect_symbols(
                child, rel_path, file_hash, language, rules, source, parent_qn, nodes, edges,
            );
        }
    }
}

/// Emit best-effort call edges: callee text resolved to a same-file symbol by
/// name; unresolved references fall back to a file-scoped name that later
/// call-target reconciliation can rewrite.
fn collect_calls(
    node: tree_sitter::Node<'_>,
    rel_path: &str,
    source: &[u8],
    call_kinds: &[String],
    target_field: Option<&str>,
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
                let target_qn = nodes
                    .iter()
                    .rev()
                    .find(|node| node.name == callee)
                    .map(|node| node.qualified_name.clone())
                    .unwrap_or_else(|| format!("{rel_path}::{callee}"));
                let call_line = start_line(child);
                let source_qn = nodes
                    .iter()
                    .rev()
                    .find(|node| {
                        node.kind != NodeKind::File
                            && node.line_start <= call_line
                            && node.line_end >= call_line
                    })
                    .map(|node| node.qualified_name.clone())
                    .unwrap_or_else(|| rel_path.to_owned());
                edges.push(Edge {
                    id: 0,
                    kind: EdgeKind::Calls,
                    source_qn,
                    target_qn,
                    file_path: rel_path.to_owned(),
                    line: Some(call_line),
                    confidence: 1.0,
                    confidence_tier: Some("definite".to_owned()),
                    extra_json: serde_json::Value::Null,
                    repo_provenance: None,
                });
            }
        }
        collect_calls(
            child,
            rel_path,
            source,
            call_kinds,
            target_field,
            nodes,
            edges,
        );
    }
}

fn named_child_text<'s>(node: tree_sitter::Node<'_>, source: &'s [u8]) -> Option<&'s str> {
    let mut cursor = node.walk();
    let mut last: Option<&str> = None;
    for child in node.children(&mut cursor) {
        if child.is_named() {
            last = Some(node_text(child, source));
        }
    }
    last
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Copy the vendored tree-sitter-rust grammar C sources into a tempdir so
    /// the loader compiles it with the system C compiler (same path a user
    /// grammar checkout would take).
    fn vendored_rust_grammar() -> (tempfile::TempDir, String) {
        let dir = tempfile::tempdir().unwrap();
        let src_dir = dir.path().join("grammar").join("src");
        fs::create_dir_all(&src_dir).unwrap();

        let registry_dir = PathBuf::from(env!("CARGO_HOME")).join("registry/src");
        let mut found = None;
        for vendor_dir in fs::read_dir(&registry_dir).unwrap() {
            let vendor_dir = vendor_dir.unwrap().path();
            if !vendor_dir.is_dir() {
                continue;
            }
            for entry in fs::read_dir(&vendor_dir).unwrap() {
                let entry = entry.unwrap();
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("tree-sitter-rust-")
                    && entry.path().join("src/parser.c").exists()
                {
                    found = Some(entry.path().join("src"));
                    break;
                }
            }
            if found.is_some() {
                break;
            }
        }
        let src = found.unwrap_or_else(|| {
            panic!(
                "vendored tree-sitter-rust grammar source not found under {}",
                registry_dir.display()
            )
        });
        for file in ["parser.c", "scanner.c", "grammar.json", "node-types.json"] {
            fs::copy(src.join(file), src_dir.join(file)).unwrap();
        }
        // parser.c/scanner.c include tree_sitter/*.h shipped next to them.
        fs::create_dir_all(src_dir.join("tree_sitter")).unwrap();
        for entry in fs::read_dir(src.join("tree_sitter")).unwrap() {
            let entry = entry.unwrap();
            fs::copy(
                entry.path(),
                src_dir.join("tree_sitter").join(entry.file_name()),
            )
            .unwrap();
        }
        let grammar_path = dir.path().join("grammar");
        let grammar_dir = grammar_path.to_string_lossy().to_string();
        (dir, grammar_dir)
    }

    fn rust_external_config(grammar_dir: &str, lib_cache: &str) -> ExternalParserConfig {
        ExternalParserConfig {
            language_name: "rust".to_owned(),
            extensions: vec!["myrs".to_owned()],
            grammar_dir: Some(grammar_dir.to_owned()),
            lib_path: None,
            lib_function: None,
            grammar_lib_dir: Some(lib_cache.to_owned()),
            symbols: vec![
                ExternalSymbolRule {
                    tree_kind: "function_item".to_owned(),
                    node_kind: NodeKind::Function,
                    name_field: "name".to_owned(),
                },
                ExternalSymbolRule {
                    tree_kind: "struct_item".to_owned(),
                    node_kind: NodeKind::Struct,
                    name_field: "name".to_owned(),
                },
                ExternalSymbolRule {
                    tree_kind: "impl_item".to_owned(),
                    node_kind: NodeKind::Trait,
                    name_field: "type".to_owned(),
                },
            ],
            call_node_kinds: vec!["call_expression".to_owned()],
            call_target_field: Some("function".to_owned()),
        }
    }

    #[test]
    fn external_parser_loads_grammar_and_extracts_symbols() {
        let (_dir, grammar_dir) = vendored_rust_grammar();
        let cache_dir = tempfile::tempdir().unwrap();
        let parser = ExternalLangParser::new(&rust_external_config(
            &grammar_dir,
            cache_dir.path().to_string_lossy().as_ref(),
        ))
        .unwrap();
        assert_eq!(parser.language_name(), "rust");
        assert!(parser.supports("src/main.myrs"));
        assert!(!parser.supports("src/main.rs"));

        let source =
            b"struct Greeter;\nfn greet() -> i32 {\n    Greeter\n}\nfn call() {\n    greet();\n}\n";
        let ctx = ParseContext {
            rel_path: "src/main.myrs",
            file_hash: "hash-1",
            source,
            old_tree: None,
        };
        let (parsed, tree) = parser.parse(&ctx);
        assert!(tree.is_some());
        let functions: Vec<&str> = parsed
            .nodes
            .iter()
            .filter(|node| node.kind == NodeKind::Function)
            .map(|node| node.name.as_str())
            .collect();
        assert_eq!(functions, ["greet", "call"]);
        assert!(
            parsed
                .nodes
                .iter()
                .any(|node| node.kind == NodeKind::Struct && node.name == "Greeter")
        );
        // Struct nests under the file qname.
        assert!(parsed.nodes.iter().any(|node| {
            node.kind == NodeKind::Struct && node.qualified_name == "src/main.myrs::struct::Greeter"
        }));
        // call edge greet() -> fn greet
        let call_edge = parsed
            .edges
            .iter()
            .find(|edge| edge.kind == EdgeKind::Calls)
            .expect("call edge");
        assert!(call_edge.target_qn.ends_with("::function::greet"));
        assert_eq!(call_edge.source_qn, "src/main.myrs::function::call");
    }

    #[test]
    fn external_parser_config_validation() {
        let (_dir, grammar_dir) = vendored_rust_grammar();
        let cache_dir = tempfile::tempdir().unwrap();
        let mut config =
            rust_external_config(&grammar_dir, cache_dir.path().to_string_lossy().as_ref());
        config.extensions.clear();
        assert!(ExternalLangParser::new(&config).is_err());
        config.extensions = vec!["myrs".to_owned()];
        config.grammar_dir = None;
        assert!(ExternalLangParser::new(&config).is_err());
    }
}
