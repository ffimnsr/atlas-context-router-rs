use atlas_core::{Edge, Node, ParsedFile};

use crate::traits::{LangParser, ParseContext};

mod calls;
mod emitter;
mod facts;
mod references;

#[cfg(test)]
mod tests;

use calls::resolve_same_file_calls;
use emitter::{RustDefinitionEmitter, file_node};
use facts::RustSyntaxFacts;
use references::resolve_same_file_references;

pub struct RustParser;

const RUST_DEFINITION_QUERY: &str = include_str!("../../../queries/rust.scm");
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
