use atlas_core::{AtlasError, Edge, EdgeKind, Node, NodeId, NodeKind, ParsedFile, Result};
use atlas_repo::CanonicalRepoPath;
use rusqlite::Row;

#[derive(Debug, Clone)]
pub(super) struct CanonicalizedGraphData {
    pub(super) path: String,
    pub(super) nodes: Vec<Node>,
    pub(super) edges: Vec<Edge>,
}

pub(super) fn canonicalize_repo_path(path: &str) -> Result<String> {
    // Central gate for any graph-store path used as persisted identity, reuse
    // key, or lookup key. Keep all file-derived graph keys on canonical
    // repo-relative spelling before hashing, persistence, or dedupe.
    CanonicalRepoPath::from_repo_relative(path)
        .map(|path| path.as_str().to_owned())
        .map_err(|err| AtlasError::Other(format!("invalid repo-relative path '{path}': {err}")))
}

pub(super) fn canonicalize_graph_slice(
    path: &str,
    nodes: &[Node],
    edges: &[Edge],
) -> Result<CanonicalizedGraphData> {
    let canonical_path = canonicalize_repo_path(path)?;
    let normalized_nodes = nodes
        .iter()
        .map(|node| canonicalize_node(node, path, &canonical_path))
        .collect::<Result<Vec<_>>>()?;
    let normalized_edges = edges
        .iter()
        .map(|edge| canonicalize_edge(edge, path, &canonical_path))
        .collect::<Result<Vec<_>>>()?;

    Ok(CanonicalizedGraphData {
        path: canonical_path,
        nodes: normalized_nodes,
        edges: normalized_edges,
    })
}

pub(super) fn canonicalize_parsed_file(file: &ParsedFile) -> Result<ParsedFile> {
    let normalized = canonicalize_graph_slice(&file.path, &file.nodes, &file.edges)?;
    Ok(ParsedFile {
        path: normalized.path,
        language: file.language.clone(),
        hash: file.hash.clone(),
        size: file.size,
        nodes: normalized.nodes,
        edges: normalized.edges,
    })
}

fn canonicalize_node(node: &Node, raw_path: &str, canonical_path: &str) -> Result<Node> {
    let raw_node_path = node.file_path.as_str();
    let normalized_node_path = canonicalize_repo_path(raw_node_path)?;
    if normalized_node_path != canonical_path {
        return Err(AtlasError::Other(format!(
            "node file_path '{}' does not match file '{}'",
            node.file_path, canonical_path
        )));
    }

    let mut normalized = node.clone();
    normalized.file_path = normalized_node_path;
    normalized.qualified_name = rewrite_known_path_prefixes(
        &node.qualified_name,
        &[(raw_path, canonical_path), (raw_node_path, canonical_path)],
    );
    normalized.parent_name = node.parent_name.as_deref().map(|parent| {
        rewrite_known_path_prefixes(
            parent,
            &[(raw_path, canonical_path), (raw_node_path, canonical_path)],
        )
    });
    Ok(normalized)
}

fn canonicalize_edge(edge: &Edge, raw_path: &str, canonical_path: &str) -> Result<Edge> {
    let raw_edge_path = edge.file_path.as_str();
    let normalized_edge_path = canonicalize_repo_path(raw_edge_path)?;

    let mut normalized = edge.clone();
    normalized.file_path = normalized_edge_path;
    normalized.source_qn = rewrite_known_path_prefixes(
        &edge.source_qn,
        &[
            (raw_path, canonical_path),
            (raw_edge_path, normalized.file_path.as_str()),
        ],
    );
    normalized.target_qn = rewrite_known_path_prefixes(
        &edge.target_qn,
        &[
            (raw_path, canonical_path),
            (raw_edge_path, normalized.file_path.as_str()),
        ],
    );
    Ok(normalized)
}

fn rewrite_known_path_prefixes(value: &str, mappings: &[(&str, &str)]) -> String {
    for (_, canonical_path) in mappings {
        if value == *canonical_path {
            return (*canonical_path).to_owned();
        }

        let canonical_prefix = format!("{canonical_path}::");
        if value.starts_with(&canonical_prefix) {
            return value.to_owned();
        }
    }

    for (raw_prefix, canonical_path) in mappings {
        if raw_prefix.is_empty() || *raw_prefix == *canonical_path {
            continue;
        }
        if value == *raw_prefix {
            return (*canonical_path).to_owned();
        }
        if let Some(rest) = value.strip_prefix(raw_prefix)
            && rest.starts_with("::")
        {
            return format!("{canonical_path}{rest}");
        }
    }

    value.to_owned()
}

pub(super) fn row_to_node(row: &Row<'_>) -> rusqlite::Result<Node> {
    let kind_str: String = row.get(1)?;
    let kind = kind_str.parse::<NodeKind>().unwrap_or(NodeKind::Function);

    let extra_str: Option<String> = row.get(14)?;
    let extra_json = extra_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);

    Ok(Node {
        id: NodeId(row.get(0)?),
        kind,
        name: row.get(2)?,
        qualified_name: row.get(3)?,
        file_path: row.get(4)?,
        line_start: row.get(5)?,
        line_end: row.get(6)?,
        language: row.get::<_, Option<String>>(7)?.unwrap_or_default(),
        parent_name: row.get(8)?,
        params: row.get(9)?,
        return_type: row.get(10)?,
        modifiers: row.get(11)?,
        is_test: row.get::<_, i32>(12)? != 0,
        file_hash: row.get::<_, Option<String>>(13)?.unwrap_or_default(),
        extra_json,
    })
}

pub(super) fn row_to_edge(row: &Row<'_>) -> rusqlite::Result<atlas_core::Edge> {
    let kind_str: String = row.get(1)?;
    let kind = kind_str.parse::<EdgeKind>().unwrap_or(EdgeKind::References);

    let extra_str: Option<String> = row.get(8)?;
    let extra_json = extra_str
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or(serde_json::Value::Null);

    Ok(atlas_core::Edge {
        id: row.get(0)?,
        kind,
        source_qn: row.get(2)?,
        target_qn: row.get(3)?,
        file_path: row.get(4)?,
        line: row.get(5)?,
        confidence: row.get(6)?,
        confidence_tier: row.get(7)?,
        extra_json,
    })
}

pub(super) fn repeat_placeholders(n: usize) -> String {
    (0..n).map(|_| "?").collect::<Vec<_>>().join(",")
}

/// Wrap a user-provided FTS5 query so special characters don't break syntax.
/// Simple approach: if the string has FTS5 operators, quote it as a phrase.
pub(super) fn fts5_escape(input: &str) -> String {
    if looks_like_safe_fts_query(input) {
        return input.to_string();
    }

    // If it looks like a plain word/words without FTS5 syntax, leave it as-is
    // so users can still use operators intentionally.  Otherwise wrap in "".
    let has_special = input
        .chars()
        .any(|c| matches!(c, '"' | '(' | ')' | '^' | '-' | '*'));
    if has_special {
        // Escape internal double-quotes and wrap as phrase.
        format!("\"{}\"", input.replace('"', "\"\""))
    } else {
        input.to_string()
    }
}

pub(super) fn looks_like_safe_fts_query(input: &str) -> bool {
    !input.is_empty()
        && input.split_whitespace().all(|token| {
            token == "OR"
                || token
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '*')
        })
}
