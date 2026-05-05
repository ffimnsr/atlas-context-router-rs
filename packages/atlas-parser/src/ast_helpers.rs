use tree_sitter::{Node, Tree};

/// Walk the tree and collect every node matching `kind`.
pub fn find_all<'a>(tree: &'a Tree, kind: &str) -> Vec<Node<'a>> {
    let mut result = Vec::new();
    collect_nodes(tree.root_node(), kind, &mut result);
    result
}

fn collect_nodes<'a>(node: Node<'a>, kind: &str, out: &mut Vec<Node<'a>>) {
    if node.kind() == kind {
        out.push(node);
    }
    for child in node.children(&mut node.walk()) {
        collect_nodes(child, kind, out);
    }
}

/// Extract the source text slice for a node (UTF-8).
pub fn node_text<'s>(node: Node<'_>, source: &'s [u8]) -> &'s str {
    let start = node.start_byte();
    let end = node.end_byte();
    source
        .get(start..end)
        .and_then(|bytes| std::str::from_utf8(bytes).ok())
        .unwrap_or("")
}

/// 1-based start line.
pub fn start_line(node: Node<'_>) -> u32 {
    node.start_position().row as u32 + 1
}

/// 1-based end line.
pub fn end_line(node: Node<'_>) -> u32 {
    node.end_position().row as u32 + 1
}

/// Find first direct child with the given field name.
pub fn field_text<'s>(node: Node<'_>, field: &str, source: &'s [u8]) -> Option<&'s str> {
    node.child_by_field_name(field)
        .map(|n| node_text(n, source))
}

/// Check whether the node or any ancestor has has a given kind up to depth.
pub fn has_ancestor_kind(mut node: Node<'_>, kind: &str, max_depth: usize) -> bool {
    for _ in 0..max_depth {
        match node.parent() {
            Some(p) => {
                if p.kind() == kind {
                    return true;
                }
                node = p;
            }
            None => break,
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

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

    fn visit_all_nodes(node: Node<'_>, source: &[u8]) {
        let _ = node_text(node, source);

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            visit_all_nodes(child, source);
        }
    }

    #[test]
    fn node_text_handles_incremental_reparse_stale_byte_ranges() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_bash::LANGUAGE.into())
            .expect("tree-sitter-bash grammar failed to load");

        let old_source = [18, 77, 46, 120];
        let old_tree = parser
            .parse(old_source, None)
            .expect("initial parse should return a tree");
        let new_source = [91];
        let new_tree = parser
            .parse(new_source, Some(&old_tree))
            .expect("incremental parse should return a tree");

        visit_all_nodes(new_tree.root_node(), &new_source);
    }

    fn exercise_common_helpers(node: Node<'_>, source: &[u8], ancestor_kind: &str) {
        let _ = node_text(node, source);
        let _ = start_line(node);
        let _ = end_line(node);
        let _ = has_ancestor_kind(node, ancestor_kind, 16);
        for field in COMMON_FIELDS {
            let _ = field_text(node, field, source);
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            exercise_common_helpers(child, source, ancestor_kind);
        }
    }

    #[test]
    fn common_helpers_tolerate_invalid_utf8_json_tree() {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&tree_sitter_json::LANGUAGE.into())
            .expect("tree-sitter-json grammar failed to load");

        let source = [b'{', b'"', b'a', b'"', b':', 0xff, b'}'];
        let tree = parser
            .parse(source, None)
            .expect("parse should return a tree even on malformed input");

        let root = tree.root_node();
        exercise_common_helpers(root, &source, root.kind());
        let _ = find_all(&tree, "pair");
        let _ = find_all(&tree, "string");
    }
}
