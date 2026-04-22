use atlas_core::{EdgeKind, Node, NodeId, NodeKind};
use rusqlite::Row;

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
