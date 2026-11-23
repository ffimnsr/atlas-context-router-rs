use super::types::*;
use super::*;

pub(crate) fn load_rust_complexity(
    repo_root: &Path,
    nodes: &[Node],
) -> Result<HashMap<String, RustComplexityMetrics>> {
    let rust_callables = nodes
        .iter()
        .filter(|node| is_callable(node) && node.language.eq_ignore_ascii_case("rust"))
        .fold(BTreeMap::<String, Vec<&Node>>::new(), |mut acc, node| {
            acc.entry(node.file_path.clone()).or_default().push(node);
            acc
        });

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .map_err(|err| AtlasError::Other(format!("failed to load rust grammar: {err}")))?;

    let mut metrics = HashMap::new();
    for (file_path, file_nodes) in rust_callables {
        let source_path = repo_root.join(&file_path);
        let Ok(source) = fs::read(&source_path) else {
            continue;
        };
        let Some(tree) = parser.parse(&source, None) else {
            continue;
        };
        let mut function_metrics = HashMap::<(String, u32), RustComplexityMetrics>::new();
        collect_rust_function_metrics(tree.root_node(), &source, &mut function_metrics);

        for node in file_nodes {
            if let Some(metric) = function_metrics.get(&(node.name.clone(), node.line_start)) {
                metrics.insert(node.qualified_name.clone(), *metric);
            }
        }
    }

    Ok(metrics)
}

pub(super) fn collect_rust_function_metrics(
    node: TsNode<'_>,
    source: &[u8],
    out: &mut HashMap<(String, u32), RustComplexityMetrics>,
) {
    if node.kind() == "function_item"
        && let Some(name) = child_field_text(node, "name", source)
    {
        out.insert(
            (name.to_owned(), start_line(node)),
            analyze_rust_function(node, source),
        );
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust_function_metrics(child, source, out);
    }
}

pub(super) fn analyze_rust_function(node: TsNode<'_>, source: &[u8]) -> RustComplexityMetrics {
    let mut metrics = RustComplexityMetrics {
        cyclomatic_complexity: 1,
        cognitive_complexity: 0,
        branch_count: 0,
        max_nesting_depth: 0,
    };

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_rust_complexity(child, source, 0, &mut metrics);
    }

    metrics
}

pub(super) fn walk_rust_complexity(
    node: TsNode<'_>,
    source: &[u8],
    nesting_depth: usize,
    metrics: &mut RustComplexityMetrics,
) {
    let kind = node.kind();
    let next_nesting = if is_nesting_node(node) {
        let depth = nesting_depth + 1;
        metrics.max_nesting_depth = metrics.max_nesting_depth.max(depth);
        depth
    } else {
        nesting_depth
    };

    match kind {
        "if_expression" | "while_expression" | "for_expression" | "loop_expression" => {
            metrics.cyclomatic_complexity += 1;
            metrics.cognitive_complexity += 1 + nesting_depth;
            metrics.branch_count += 1;
        }
        "match_expression" => {
            let arms = count_children_of_kind(node, "match_arm").max(1);
            metrics.cyclomatic_complexity += arms;
            metrics.cognitive_complexity += 1 + nesting_depth;
            metrics.branch_count += arms;
        }
        "return_expression" => {
            metrics.cognitive_complexity += 1 + nesting_depth;
            metrics.branch_count += 1;
        }
        "binary_expression" => {
            let boolean_branches = count_boolean_operators(node);
            metrics.cyclomatic_complexity += boolean_branches;
            metrics.cognitive_complexity += boolean_branches;
            metrics.branch_count += boolean_branches;
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_rust_complexity(child, source, next_nesting, metrics);
    }

    let _ = source;
}

pub(super) fn count_boolean_operators(node: TsNode<'_>) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if matches!(child.kind(), "&&" | "||") {
            count += 1;
        }
    }
    count
}

pub(super) fn count_children_of_kind(node: TsNode<'_>, kind: &str) -> usize {
    let mut count = 0;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == kind {
            count += 1;
        }
    }
    count
}

pub(super) fn is_nesting_node(node: TsNode<'_>) -> bool {
    match node.kind() {
        "if_expression" | "while_expression" | "for_expression" | "loop_expression"
        | "match_expression" | "closure_expression" => true,
        "block" => !node.parent().is_some_and(|parent| {
            matches!(
                parent.kind(),
                "function_item"
                    | "if_expression"
                    | "else_clause"
                    | "while_expression"
                    | "for_expression"
                    | "loop_expression"
                    | "match_expression"
                    | "match_arm"
                    | "closure_expression"
            )
        }),
        _ => false,
    }
}

fn child_field_text<'a>(node: TsNode<'_>, field: &str, source: &'a [u8]) -> Option<&'a str> {
    node.child_by_field_name(field)
        .and_then(|child| std::str::from_utf8(&source[child.start_byte()..child.end_byte()]).ok())
}

pub(super) fn start_line(node: TsNode<'_>) -> u32 {
    node.start_position().row as u32 + 1
}
