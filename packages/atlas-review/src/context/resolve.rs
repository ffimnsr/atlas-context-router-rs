use super::*;

/// Outcome of resolving a [`ContextTarget`].
#[derive(Debug)]
pub enum ResolvedTarget {
    /// Exactly one node matched.
    Node(Box<atlas_core::model::Node>),
    /// Exactly one file path matched (used for `FilePath` targets).
    File(String),
    /// Multiple candidates were found; a ranked list is provided.
    Ambiguous(AmbiguityMeta),
    /// No match found, but suggestions are available if the fallback search
    /// returned anything.
    NotFound { suggestions: Vec<String> },
}

/// Resolve a [`ContextTarget`] to a concrete node or file using the store.
///
/// Resolution order (exact paths first, FTS fallback last):
/// 1. `QualifiedName` → exact `node_by_qname` lookup.
/// 2. `SymbolName`    → exact `nodes_by_name`, take unique or mark ambiguous.
/// 3. `FilePath`      → `nodes_by_file` check; returns `File` if non-empty.
/// 4. `ChangedFiles`  → not resolved to a single node; callers handle directly.
///
/// Falls back to FTS search (capped at 8 candidates) only when exact paths
/// yield no result.
pub fn resolve_target(store: &Store, target: &ContextTarget) -> Result<ResolvedTarget> {
    match target {
        ContextTarget::QualifiedName { qname } => resolve_by_qname(store, qname),
        ContextTarget::SymbolName { name } => resolve_by_name(store, name),
        ContextTarget::FilePath { path } => resolve_by_file(store, path),
        ContextTarget::ChangedFiles { .. }
        | ContextTarget::ChangedSymbols { .. }
        | ContextTarget::EdgeQuerySeed { .. } => Ok(ResolvedTarget::NotFound {
            suggestions: vec![],
        }),
    }
}

/// Normalise user-entered kind tokens inside a qualified name so that common
/// aliases resolve to the canonical token used by the parsers.
pub fn normalize_qn_kind_tokens(qname: &str) -> String {
    let Some(after_file) = qname.find("::") else {
        return qname.to_owned();
    };
    let (file_part, rest) = qname.split_at(after_file);
    let rest = &rest[2..];

    let (kind_token, symbol_rest) = if let Some(pos) = rest.find("::") {
        (&rest[..pos], &rest[pos..])
    } else {
        return qname.to_owned();
    };

    let kind_lower = kind_token.to_ascii_lowercase();
    let canonical_kind = match kind_lower.as_str() {
        "function" | "func" => "fn",
        "meth" => "method",
        "constant" => "const",
        other => other,
    };

    if canonical_kind == kind_token {
        return qname.to_owned();
    }
    format!("{file_part}::{canonical_kind}{symbol_rest}")
}

fn resolve_by_qname(store: &Store, qname: &str) -> Result<ResolvedTarget> {
    if let Some(node) = store.node_by_qname(qname)? {
        return Ok(ResolvedTarget::Node(Box::new(node)));
    }
    let normalised = normalize_qn_kind_tokens(qname);
    if normalised != qname
        && let Some(node) = store.node_by_qname(&normalised)?
    {
        return Ok(ResolvedTarget::Node(Box::new(node)));
    }
    fts_fallback(store, qname)
}

fn resolve_by_name(store: &Store, name: &str) -> Result<ResolvedTarget> {
    const CANDIDATE_CAP: usize = 9;
    let nodes = store.nodes_by_name(name, CANDIDATE_CAP)?;
    match nodes.len() {
        0 => fts_fallback(store, name),
        1 => Ok(ResolvedTarget::Node(Box::new(
            nodes.into_iter().next().unwrap(),
        ))),
        _ => {
            let candidates: Vec<String> = nodes.iter().map(|n| n.qualified_name.clone()).collect();
            Ok(ResolvedTarget::Ambiguous(AmbiguityMeta {
                query: name.to_owned(),
                candidates,
                resolved: false,
            }))
        }
    }
}

fn resolve_by_file(store: &Store, path: &str) -> Result<ResolvedTarget> {
    let nodes = store.nodes_by_file(path)?;
    if nodes.is_empty() {
        return Ok(ResolvedTarget::NotFound {
            suggestions: vec![],
        });
    }
    Ok(ResolvedTarget::File(path.to_owned()))
}

fn fts_fallback(store: &Store, text: &str) -> Result<ResolvedTarget> {
    use atlas_core::SearchQuery;
    use atlas_search::search as fts_search;

    let query = SearchQuery {
        text: text.to_owned(),
        limit: 8,
        fuzzy_match: true,
        ..SearchQuery::default()
    };
    let results = fts_search(store, &query)?;
    if results.is_empty() {
        return Ok(ResolvedTarget::NotFound {
            suggestions: vec![],
        });
    }
    let candidates: Vec<String> = results
        .iter()
        .map(|r| r.node.qualified_name.clone())
        .collect();
    Ok(ResolvedTarget::Ambiguous(AmbiguityMeta {
        query: text.to_owned(),
        candidates,
        resolved: false,
    }))
}
