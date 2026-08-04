use super::*;

pub(super) fn symbol_summary(node: &Node, module_id: String) -> InsightSymbolSummary {
    InsightSymbolSummary {
        qualified_name: node.qualified_name.clone(),
        display_name: node.name.clone(),
        file_path: node.file_path.clone(),
        line_start: node.line_start,
        line_end: node.line_end,
        language: node.language.clone(),
        node_kind: node.kind.as_str().to_owned(),
        module_id,
    }
}

pub(super) fn is_callable_node(node: &Node) -> bool {
    matches!(node.kind, NodeKind::Function | NodeKind::Method)
}

pub(super) fn parse_arity(params: Option<&str>) -> usize {
    let Some(params) = params.map(str::trim) else {
        return 0;
    };
    let inner = params
        .strip_prefix('(')
        .and_then(|value| value.strip_suffix(')'))
        .unwrap_or(params)
        .trim();
    if inner.is_empty() {
        0
    } else {
        inner.split(',').count()
    }
}

pub(super) fn signature_tokens(node: &Node) -> BTreeSet<String> {
    let mut tokens = tokenize_identifier(&node.name);
    for source in [
        node.params.as_deref(),
        node.return_type.as_deref(),
        node.modifiers.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        tokens.extend(tokenize_source(source));
    }
    tokens
}

pub(super) fn source_excerpt_from_text(source: &str, node: &Node) -> Option<String> {
    let start = usize::try_from(node.line_start.saturating_sub(1)).ok()?;
    let end = usize::try_from(node.line_end).ok()?;
    let lines = source.lines().skip(start).take(end.saturating_sub(start));
    Some(lines.collect::<Vec<_>>().join("\n"))
}

pub(super) fn tokenize_identifier(text: &str) -> BTreeSet<String> {
    let mut normalized = String::with_capacity(text.len() * 2);
    let mut previous_is_lower = false;
    for ch in text.chars() {
        if ch.is_ascii_uppercase() && previous_is_lower {
            normalized.push(' ');
        }
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(' ');
        }
        previous_is_lower = ch.is_ascii_lowercase();
    }
    normalized
        .split_whitespace()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
}

pub(super) fn tokenize_source(text: &str) -> Vec<String> {
    let mut normalized = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            normalized.push(ch.to_ascii_lowercase());
        } else {
            normalized.push(' ');
        }
    }
    normalized
        .split_whitespace()
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(super) fn normalize_duplicate_tokens(text: &str) -> Vec<String> {
    tokenize_source(text)
        .into_iter()
        .map(|token| {
            if token.chars().all(|ch| ch.is_ascii_digit()) {
                "<num>".to_owned()
            } else if is_keyword(&token) {
                token
            } else {
                "<id>".to_owned()
            }
        })
        .collect()
}

pub(super) fn shingles(tokens: &[String], size: usize) -> BTreeSet<String> {
    if tokens.is_empty() {
        return BTreeSet::new();
    }
    if tokens.len() <= size {
        return [tokens.join(" ")].into_iter().collect();
    }
    let mut items = BTreeSet::new();
    for window in tokens.windows(size) {
        items.insert(window.join(" "));
    }
    items
}

pub(super) fn jaccard(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f64 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count() as f64;
    let union = left.union(right).count() as f64;
    if union == 0.0 {
        0.0
    } else {
        intersection / union
    }
}

pub(super) fn overlap_ratio(left: usize, right: usize) -> f64 {
    if left == 0 || right == 0 {
        return 0.0;
    }
    let min = left.min(right) as f64;
    let max = left.max(right) as f64;
    min / max
}

pub(super) fn summarize_duplicate_pattern(tokens: &[String]) -> String {
    tokens
        .iter()
        .take(12)
        .cloned()
        .collect::<Vec<_>>()
        .join(" ")
}

pub(super) fn common_parent_qname(items: &[CallableFingerprint]) -> Option<String> {
    let mut parts = items
        .iter()
        .filter_map(|item| item.summary.qualified_name.rsplit_once("::"))
        .map(|(parent, _)| parent.to_owned());
    let first = parts.next()?;
    if parts.all(|item| item == first) {
        Some(first)
    } else {
        None
    }
}

pub(super) fn common_parent_path(files: &[String]) -> Option<String> {
    let mut parts = files
        .iter()
        .filter_map(|file| file.rsplit_once('/').map(|(prefix, _)| prefix));
    let first = parts.next()?.to_owned();
    if parts.all(|item| item == first) {
        Some(first)
    } else {
        None
    }
}

pub(super) fn normalize_paths(paths: Option<&[String]>) -> Result<Vec<String>> {
    paths
        .map(|paths| {
            paths
                .iter()
                .map(|path| {
                    CanonicalRepoPath::from_repo_relative(path)
                        .map(|canonical| canonical.as_str().to_owned())
                        .map_err(|error| AtlasError::Other(error.to_string()))
                })
                .collect()
        })
        .unwrap_or_else(|| Ok(Vec::new()))
}

pub(super) fn is_keyword(token: &str) -> bool {
    matches!(
        token,
        "if" | "else"
            | "for"
            | "while"
            | "loop"
            | "match"
            | "return"
            | "let"
            | "const"
            | "fn"
            | "pub"
            | "impl"
            | "struct"
            | "enum"
            | "trait"
            | "class"
            | "interface"
            | "switch"
            | "case"
            | "break"
            | "continue"
            | "try"
            | "catch"
            | "throw"
            | "await"
            | "async"
            | "new"
            | "use"
            | "import"
            | "export"
            | "mod"
            | "where"
            | "in"
            | "true"
            | "false"
            | "none"
            | "some"
            | "ok"
            | "err"
    )
}
