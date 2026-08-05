use atlas_core::{Edge, Node, NodeKind, ParsedFile};

use crate::traits::{LangParser, ParseContext};

mod calls;
mod declarations;

#[cfg(test)]
mod tests;

use calls::resolve_go_calls;
use declarations::{
    contains_edge, file_node, find_package, package_node, visit_function, visit_imports,
    visit_method, visit_type_decl, visit_value_decl,
};

// SQ1 migration checklist:
// - manual extraction below owns package, declaration, import, call-edge, scope, and qualified-name semantics
// - keep public parser API, graph schema, and output contracts unchanged during query migration
// - move tree-sitter syntax matching only into shared @atlas.* query captures

pub struct GoParser;
impl LangParser for GoParser {
    fn language_name(&self) -> &'static str {
        "go"
    }

    fn supports(&self, path: &str) -> bool {
        path.ends_with(".go")
    }

    fn parse(&self, ctx: &ParseContext<'_>) -> (ParsedFile, Option<tree_sitter::Tree>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .expect("tree-sitter-go grammar failed to load");

        let tree = crate::parse_runtime::parse_tree(&mut parser, ctx.source, ctx.old_tree);
        let mut nodes: Vec<Node> = Vec::new();
        let mut edges: Vec<Edge> = Vec::new();

        let line_count = ctx.source.iter().filter(|&&b| b == b'\n').count() as u32 + 1;
        nodes.push(file_node(ctx.rel_path, ctx.file_hash, line_count));

        if let Some(ref tree) = tree {
            let root = tree.root_node();
            let package = find_package(root, ctx.source);
            let package_qn = format!("{}::package::{}", ctx.rel_path, package.name);

            nodes.push(package_node(
                ctx.rel_path,
                ctx.file_hash,
                &package.name,
                &package_qn,
                package.line,
            ));
            edges.push(contains_edge(
                ctx.rel_path,
                &package_qn,
                ctx.rel_path,
                package.line,
            ));

            let mut cursor = root.walk();
            for child in root.children(&mut cursor) {
                match child.kind() {
                    "function_declaration" => {
                        visit_function(child, ctx, &package_qn, &mut nodes, &mut edges);
                    }
                    "method_declaration" => {
                        visit_method(child, ctx, &package_qn, &mut nodes, &mut edges);
                    }
                    "type_declaration" => {
                        visit_type_decl(child, ctx, &package_qn, &mut nodes, &mut edges);
                    }
                    "import_declaration" => {
                        visit_imports(child, ctx, &package_qn, &mut nodes, &mut edges);
                    }
                    "const_declaration" => {
                        visit_value_decl(
                            child,
                            ctx,
                            &package_qn,
                            NodeKind::Constant,
                            "const",
                            &mut nodes,
                            &mut edges,
                        );
                    }
                    "var_declaration" => {
                        visit_value_decl(
                            child,
                            ctx,
                            &package_qn,
                            NodeKind::Variable,
                            "var",
                            &mut nodes,
                            &mut edges,
                        );
                    }
                    _ => {}
                }
            }

            // Second pass: same-file call resolution.
            let mut call_edges = resolve_go_calls(root, ctx.source, ctx.rel_path, &nodes);
            edges.append(&mut call_edges);
        }

        let pf = ParsedFile {
            path: ctx.rel_path.to_owned(),
            language: Some("go".to_owned()),
            hash: ctx.file_hash.to_owned(),
            size: Some(ctx.source.len() as i64),
            nodes,
            edges,
        };
        (pf, tree)
    }
}
