use tree_sitter::{Language, Node, Query, QueryCursor, StreamingIterator};

use crate::ast_helpers::node_text;

#[derive(Clone, Debug)]
pub struct QueryCapture<'tree> {
    pub name: String,
    pub node: Node<'tree>,
    ordinal: usize,
}

impl QueryCapture<'_> {
    fn sort_key(&self) -> (usize, usize, usize) {
        (self.node.start_byte(), self.node.end_byte(), self.ordinal)
    }
}

#[derive(Clone, Debug)]
pub struct QueryCaptureGroup<'tree> {
    pub pattern_index: usize,
    pub captures: Vec<QueryCapture<'tree>>,
    match_index: usize,
}

impl<'tree> QueryCaptureGroup<'tree> {
    pub fn capture(&self, name: &str) -> Option<&QueryCapture<'tree>> {
        self.captures.iter().find(|capture| capture.name == name)
    }

    pub fn optional_capture(&self, name: &str) -> Option<&QueryCapture<'tree>> {
        self.capture(name)
    }

    pub fn required_capture(&self, name: &str) -> Result<&QueryCapture<'tree>, String> {
        self.capture(name)
            .ok_or_else(|| format!("query match missing required capture @{name}"))
    }
}

pub fn compile_query(language: Language, query_text: &str) -> Result<Query, String> {
    Query::new(&language, query_text)
        .map_err(|err| format!("failed to compile tree-sitter query: {err}"))
}

pub fn compile_static_query(
    language: Language,
    language_name: &str,
    query_text: &'static str,
) -> Result<Query, String> {
    compile_query(language, query_text)
        .map_err(|err| format!("{language_name} query compile failed: {err}"))
}

pub fn sort_captures_by_byte_range(captures: &mut [QueryCapture<'_>]) {
    captures.sort_by_key(QueryCapture::sort_key);
}

pub fn preserve_match_source_order(groups: &mut [QueryCaptureGroup<'_>]) {
    groups.sort_by_key(|group| group.match_index);
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
        let mut captures = query_match
            .captures
            .iter()
            .enumerate()
            .map(|(ordinal, capture)| QueryCapture {
                name: capture_names[capture.index as usize].to_owned(),
                node: capture.node,
                ordinal,
            })
            .collect::<Vec<_>>();
        sort_captures_by_byte_range(&mut captures);
        groups.push(QueryCaptureGroup {
            pattern_index: query_match.pattern_index,
            captures,
            match_index: groups.len(),
        });
    }

    preserve_match_source_order(&mut groups);
    groups
}

pub fn read_capture_text<'tree>(capture: &QueryCapture<'tree>, source: &'tree [u8]) -> &'tree str {
    node_text(capture.node, source)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_rust(source: &[u8]) -> (Language, tree_sitter::Tree) {
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&language)
            .expect("tree-sitter-rust grammar failed to load");
        let tree = parser.parse(source, None).expect("fixture should parse");
        (language, tree)
    }

    #[test]
    fn invalid_query_reports_clear_error() {
        let language: Language = tree_sitter_rust::LANGUAGE.into();
        let err =
            compile_query(language, "(").expect_err("invalid query text should fail to compile");
        assert!(err.contains("failed to compile tree-sitter query"));
    }

    #[test]
    fn missing_required_capture_reports_clear_error() {
        let source = b"fn helper() {}";
        let (language, tree) = parse_rust(source);
        let query = compile_query(
            language,
            "(function_item name: (identifier) @atlas.name) @atlas.definition.function",
        )
        .expect("query should compile");
        let matches = run_query(&query, tree.root_node(), source);
        let group = matches.first().expect("function match should exist");

        let err = group
            .required_capture("atlas.parameters")
            .expect_err("missing required capture should fail");
        assert!(err.contains("query match missing required capture @atlas.parameters"));
    }

    #[test]
    fn optional_capture_absence_does_not_fail() {
        let source = b"fn helper() {}";
        let (language, tree) = parse_rust(source);
        let query = compile_query(
            language,
            "(function_item name: (identifier) @atlas.name) @atlas.definition.function",
        )
        .expect("query should compile");
        let matches = run_query(&query, tree.root_node(), source);
        let group = matches.first().expect("function match should exist");

        assert!(group.optional_capture("atlas.parameters").is_none());
        assert!(group.optional_capture("atlas.name").is_some());
    }

    #[test]
    fn capture_order_is_deterministic_across_repeated_runs() {
        let source = b"fn helper(arg: usize) -> usize { arg }";
        let (language, tree) = parse_rust(source);
        let query = compile_query(
            language,
            "(function_item\n  name: (identifier) @atlas.name\n  parameters: (parameters) @atlas.parameters\n  body: (block) @atlas.body) @atlas.definition.function",
        )
        .expect("query should compile");

        let expected = run_query(&query, tree.root_node(), source)
            .into_iter()
            .map(|group| {
                group
                    .captures
                    .iter()
                    .map(|capture| {
                        (
                            capture.name.clone(),
                            capture.node.start_byte(),
                            capture.node.end_byte(),
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();

        for _ in 0..5 {
            let actual = run_query(&query, tree.root_node(), source)
                .into_iter()
                .map(|group| {
                    group
                        .captures
                        .iter()
                        .map(|capture| {
                            (
                                capture.name.clone(),
                                capture.node.start_byte(),
                                capture.node.end_byte(),
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>();
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn overlapping_captures_preserve_match_order_before_filtering() {
        let source = b"fn helper() {}";
        let (language, tree) = parse_rust(source);
        let query = compile_query(
            language,
            "(function_item name: (identifier) @atlas.name) @atlas.definition.function\n(function_item parameters: (parameters) @atlas.parameters) @atlas.definition.function",
        )
        .expect("query should compile");

        let matches = run_query(&query, tree.root_node(), source);
        assert_eq!(matches.len(), 2);
        assert!(matches[0].capture("atlas.name").is_some());
        assert!(matches[1].capture("atlas.parameters").is_some());

        let mut reversed = vec![matches[1].clone(), matches[0].clone()];
        preserve_match_source_order(&mut reversed);
        assert!(reversed[0].capture("atlas.name").is_some());
        assert!(reversed[1].capture("atlas.parameters").is_some());
    }

    #[test]
    fn rust_query_captures_function_from_fixture() {
        let (language, tree) = parse_rust(b"fn helper() {}");
        let query = compile_static_query(language, "rust", include_str!("../queries/rust.scm"))
            .expect("rust query should compile");
        let source = b"fn helper() {}";

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
