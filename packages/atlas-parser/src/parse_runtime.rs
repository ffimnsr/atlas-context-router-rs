use std::ops::ControlFlow;
use std::time::{Duration, Instant};

/// Hard per-file parse deadline for tree-sitter-backed parsers.
pub const DEFAULT_PARSE_TIMEOUT_MICROS: u64 = 1_000_000;

fn compatible_old_tree<'a>(
    parser: &tree_sitter::Parser,
    old_tree: Option<&'a tree_sitter::Tree>,
) -> Option<&'a tree_sitter::Tree> {
    let old_tree = old_tree?;
    let parser_language = parser.language()?;

    if *old_tree.language() == *parser_language {
        Some(old_tree)
    } else {
        None
    }
}

pub(crate) fn parse_tree(
    parser: &mut tree_sitter::Parser,
    source: &[u8],
    old_tree: Option<&tree_sitter::Tree>,
) -> Option<tree_sitter::Tree> {
    let timeout = Duration::from_micros(DEFAULT_PARSE_TIMEOUT_MICROS);
    let started = Instant::now();
    let mut progress = |_state: &tree_sitter::ParseState| {
        if started.elapsed() >= timeout {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    };
    let options = tree_sitter::ParseOptions::new().progress_callback(&mut progress);
    let old_tree = compatible_old_tree(parser, old_tree);
    parser.parse_with_options(
        &mut |byte_offset, _position| &source[byte_offset.min(source.len())..],
        old_tree,
        Some(options),
    )
}

#[cfg(test)]
mod tests {
    use super::parse_tree;

    #[test]
    fn parse_tree_parses_small_input() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("tree-sitter-rust grammar failed to load");

        let tree = parse_tree(&mut parser, b"fn main() {}", None);
        assert!(tree.is_some());
    }

    #[test]
    fn parse_tree_ignores_old_tree_from_other_language() {
        let mut bash_parser = tree_sitter::Parser::new();
        bash_parser
            .set_language(&tree_sitter_bash::LANGUAGE.into())
            .expect("tree-sitter-bash grammar failed to load");
        let old_tree = parse_tree(&mut bash_parser, b"echo hi\n", None)
            .expect("initial bash parse should return a tree");

        let mut json_parser = tree_sitter::Parser::new();
        json_parser
            .set_language(&tree_sitter_json::LANGUAGE.into())
            .expect("tree-sitter-json grammar failed to load");

        let new_tree = parse_tree(&mut json_parser, b"{}", Some(&old_tree))
            .expect("parse should ignore mismatched old tree instead of crashing");

        assert_eq!(new_tree.root_node().kind(), "document");
    }
}
