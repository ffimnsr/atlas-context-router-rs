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
    std::str::from_utf8(&source[start..end]).unwrap_or("<invalid utf8>")
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
