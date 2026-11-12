//! VR4 — activate query-conditioned experience edges during context assembly.
//!
//! Edges are hints only: every shortcut points at a real graph node and carries
//! the edge provenance so callers can audit why it appeared.

use super::*;
use atlas_core::model::{ExperienceShortcutEvidence, SelectionReason};

/// Extract the lexical query text used to condition experience edges.
pub(super) fn request_query_text(request: &ContextRequest) -> String {
    match &request.target {
        ContextTarget::SymbolName { name } => name.clone(),
        ContextTarget::QualifiedName { qname } => qname.clone(),
        ContextTarget::FilePath { path } => path.clone(),
        ContextTarget::ChangedFiles { paths } => paths.join(" "),
        ContextTarget::ChangedSymbols { qnames } => qnames.join(" "),
        ContextTarget::EdgeQuerySeed { source_qname, .. } => source_qname.clone(),
    }
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Resolve an `atlas://repo/{id}/file|symbol/{...}` URI to a graph node.
fn resolve_target_uri(store: &Store, uri: &str) -> Result<Option<atlas_core::model::Node>> {
    let Some(rest) = uri.strip_prefix("atlas://repo/") else {
        return Ok(None);
    };
    let Some((_, remainder)) = rest.split_once('/') else {
        return Ok(None);
    };
    let Some((kind, tail)) = remainder.split_once('/') else {
        return Ok(None);
    };
    let Some(target) = percent_decode(tail) else {
        return Ok(None);
    };
    match kind {
        "file" => Ok(store.nodes_by_file(&target)?.into_iter().next()),
        "symbol" => store.node_by_qname(&target),
        _ => Ok(None),
    }
}

/// Add fresh experience-edge targets to a context result.
///
/// Returns the number of shortcuts added. Stale or invalid edges are never
/// surfaced because activation already filters to `fresh`.
pub(super) fn apply_experience_shortcuts(
    store: &Store,
    session: &atlas_session::SessionStore,
    repo_root: &str,
    request: &ContextRequest,
    max_edges: usize,
    result: &mut ContextResult,
) -> Result<usize> {
    let query = request_query_text(request);
    if query.trim().is_empty() || max_edges == 0 {
        return Ok(0);
    }
    let intent = request.intent;
    let intent_label = format!("{intent:?}").to_ascii_lowercase();
    let edges = session.activate_experience_edges(repo_root, &query, &intent_label, max_edges)?;
    if edges.is_empty() {
        return Ok(0);
    }

    let mut seen: HashSet<String> = result
        .nodes
        .iter()
        .map(|node| node.node.qualified_name.clone())
        .collect();
    let mut added = 0usize;
    for edge in edges {
        let Some(node) = resolve_target_uri(store, &edge.target_uri)? else {
            continue;
        };
        if !seen.insert(node.qualified_name.clone()) {
            continue;
        }
        let mut evidence =
            ContextRankingEvidence::from_selection_reason(SelectionReason::ExperienceShortcut);
        evidence.experience_shortcut = Some(ExperienceShortcutEvidence {
            edge_id: edge.id,
            confidence: edge.confidence,
            trace_summary: edge.trace_summary,
            commit_sha: edge.commit_sha,
        });
        result.nodes.push(SelectedNode {
            node,
            selection_reason: SelectionReason::ExperienceShortcut,
            distance: 0,
            relevance_score: (edge.confidence * 10.0) as f32,
            context_ranking_evidence: Some(evidence),
        });
        added += 1;
    }

    if added > 0 {
        result.files = super::symbol::collect_files(&result.nodes);
        super::symbol::update_file_node_counts(result);
        super::rank::rank_context(result);
        super::rank::trim_context(result);
    }
    Ok(added)
}
