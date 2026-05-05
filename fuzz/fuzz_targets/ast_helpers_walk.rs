#![no_main]

use atlas_fuzz::{SupportedPathKind, hash_bytes, source_seed_case_from_bytes};
use atlas_parser::ParserRegistry;
use atlas_parser::ast_helpers::{end_line, field_text, find_all, has_ancestor_kind, node_text, start_line};
use libfuzzer_sys::fuzz_target;
use tree_sitter::{Node, Tree};

const MAX_SOURCE_BYTES: usize = 16 * 1024;
const MAX_ANCESTOR_DEPTH: usize = 32;
const COMMON_FIELDS: [&str; 9] = [
    "name",
    "parameters",
    "return_type",
    "body",
    "value",
    "type",
    "result",
    "object",
    "function",
];
const ALL_PATH_KINDS: [SupportedPathKind; 18] = [
    SupportedPathKind::Rust,
    SupportedPathKind::Go,
    SupportedPathKind::Python,
    SupportedPathKind::JavaScript,
    SupportedPathKind::TypeScript,
    SupportedPathKind::Json,
    SupportedPathKind::Toml,
    SupportedPathKind::Html,
    SupportedPathKind::Css,
    SupportedPathKind::Bash,
    SupportedPathKind::Markdown,
    SupportedPathKind::Java,
    SupportedPathKind::CSharp,
    SupportedPathKind::Php,
    SupportedPathKind::C,
    SupportedPathKind::Cpp,
    SupportedPathKind::Scala,
    SupportedPathKind::Ruby,
];

fuzz_target!(|data: &[u8]| {
    let Some(case) = source_seed_case_from_bytes(data) else {
        return;
    };

    let source = bounded_source(&case.source);
    let registry = ParserRegistry::with_defaults();

    for path_kind in ALL_PATH_KINDS {
        let rel_path = path_kind.rel_path();
        let file_hash = hash_bytes(&source);
        let Some((_parsed, tree)) = registry.parse(rel_path, &file_hash, &source, None) else {
            continue;
        };
        let Some(tree) = tree else {
            continue;
        };

        exercise_tree(&tree, &source, path_kind);
    }
});

fn bounded_source(source: &[u8]) -> Vec<u8> {
    source[..source.len().min(MAX_SOURCE_BYTES)].to_vec()
}

fn exercise_tree(tree: &Tree, source: &[u8], path_kind: SupportedPathKind) {
    let root = tree.root_node();
    walk_node(root, source, root.kind());

    for kind in relevant_kinds(path_kind) {
        std::hint::black_box(find_all(tree, kind));
    }
}

fn walk_node(node: Node<'_>, source: &[u8], ancestor_kind: &str) {
    std::hint::black_box(node_text(node, source));
    std::hint::black_box(start_line(node));
    std::hint::black_box(end_line(node));
    std::hint::black_box(has_ancestor_kind(node, ancestor_kind, MAX_ANCESTOR_DEPTH));

    for field in COMMON_FIELDS {
        std::hint::black_box(field_text(node, field, source));
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, source, ancestor_kind);
    }
}

fn relevant_kinds(path_kind: SupportedPathKind) -> &'static [&'static str] {
    match path_kind {
        SupportedPathKind::Rust => &["function_item", "struct_item", "impl_item", "identifier"],
        SupportedPathKind::Go => &["function_declaration", "method_declaration", "type_declaration", "identifier"],
        SupportedPathKind::Python => &["function_definition", "class_definition", "parameters", "identifier"],
        SupportedPathKind::JavaScript | SupportedPathKind::TypeScript => {
            &["function_declaration", "class_declaration", "method_definition", "identifier"]
        }
        SupportedPathKind::Json => &["object", "array", "pair", "string"],
        SupportedPathKind::Toml => &["document", "pair", "bare_key", "string"],
        SupportedPathKind::Html => &["element", "start_tag", "attribute", "text"],
        SupportedPathKind::Css => &["stylesheet", "rule_set", "declaration", "identifier"],
        SupportedPathKind::Bash => &["program", "function_definition", "command", "word"],
        SupportedPathKind::Markdown => &["document", "section", "atx_heading", "paragraph"],
        SupportedPathKind::Java => &["class_declaration", "method_declaration", "field_declaration", "identifier"],
        SupportedPathKind::CSharp => &["class_declaration", "method_declaration", "property_declaration", "identifier"],
        SupportedPathKind::Php => &["program", "function_definition", "class_declaration", "name"],
        SupportedPathKind::C => &["translation_unit", "function_definition", "declaration", "identifier"],
        SupportedPathKind::Cpp => &["translation_unit", "function_definition", "class_specifier", "identifier"],
        SupportedPathKind::Scala => &["compilation_unit", "function_definition", "class_definition", "identifier"],
        SupportedPathKind::Ruby => &["program", "method", "class", "identifier"],
    }
}
