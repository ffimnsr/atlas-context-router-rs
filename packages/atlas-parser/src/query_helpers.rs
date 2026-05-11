use tree_sitter::{Language, Node, Query, QueryCursor, StreamingIterator};

use crate::ast_helpers::node_text;

#[derive(Clone, Debug)]
pub struct QueryCapture<'tree> {
    pub name: String,
    pub node: Node<'tree>,
}

#[derive(Clone, Debug)]
pub struct QueryCaptureGroup<'tree> {
    pub pattern_index: usize,
    pub captures: Vec<QueryCapture<'tree>>,
}

pub fn compile_query(language: Language, query_text: &str) -> Result<Query, String> {
    Query::new(&language, query_text)
        .map_err(|err| format!("failed to compile tree-sitter query: {err}"))
}

pub fn run_query<'tree>(
    query: &Query,
    root: Node<'tree>,
    source: &'tree [u8],
) -> Vec<QueryCaptureGroup<'tree>> {
    let capture_names = query.capture_names();
    let mut cursor = QueryCursor::new();
    let mut matches = cursor.matches(query, root, source);
    let mut groups = Vec::new();

    while let Some(query_match) = matches.next() {
        groups.push(QueryCaptureGroup {
            pattern_index: query_match.pattern_index,
            captures: query_match
                .captures
                .iter()
                .map(|capture| QueryCapture {
                    name: capture_names[capture.index as usize].to_owned(),
                    node: capture.node,
                })
                .collect(),
        });
    }

    groups
}

pub fn read_capture_text<'tree>(capture: &QueryCapture<'tree>, source: &'tree [u8]) -> &'tree str {
    node_text(capture.node, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_query_reports_clear_error() {
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let err =
            compile_query(language, "(").expect_err("invalid query text should fail to compile");
        assert!(err.contains("failed to compile tree-sitter query"));
    }

    #[test]
    fn rust_query_captures_function_from_fixture() {
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let query = compile_query(language.clone(), include_str!("../queries/rust.scm"))
            .expect("rust query should compile");
        let source = b"fn helper() {}";

        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language)
            .expect("tree-sitter-rust grammar failed to load");
        let tree = parser
            .parse(source.as_slice(), None)
            .expect("fixture should parse");

        let matches = run_query(&query, tree.root_node(), source);
        let names = matches
            .iter()
            .flat_map(|group| group.captures.iter())
            .filter(|capture| capture.name == "atlas.name")
            .map(|capture| read_capture_text(capture, source))
            .collect::<Vec<_>>();

        assert!(names.contains(&"helper"));
        assert!(matches.iter().any(|group| {
            group.captures.iter().any(|capture| {
                capture.name == "atlas.definition.function"
                    && read_capture_text(capture, source).contains("fn helper")
            })
        }));
    }
}
