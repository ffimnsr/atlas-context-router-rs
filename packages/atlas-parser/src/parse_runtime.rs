use std::ops::ControlFlow;
use std::time::{Duration, Instant};

/// Hard per-file parse deadline for tree-sitter-backed parsers.
pub const DEFAULT_PARSE_TIMEOUT_MICROS: u64 = 1_000_000;

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
}
