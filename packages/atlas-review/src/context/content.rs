// Graph/Content Companion Patch — N2 Unified bounded selection policy
// Graph/Content Companion Patch — N3 Coordinated ranking and evidence
//
// Retrieves non-code content assets (docs, config, templates, SQL, prompts)
// adjacent to changed files or related to the request target. Results are
// merged into `ContextResult::content_assets` under the unified selection
// ordering policy:
//   1. adjacent_to_changed_file (same directory or path prefix match)
//   2. related_to_changed_symbol (chunk content mentions a changed symbol)
//   3. content_match (general query match)
//
// Deterministic tie-breakers at equal relevance score:
//   - Graph nodes take priority over content assets (handled at payload level).
//   - Among content assets: priority() descending, then alphabetical by path.
//
// N3 extends evidence with normalized ranking signals so each asset carries
// a `source_kind` discriminant plus individual boost/score fields that are
// consistent with graph-node evidence across mixed result sets.

use super::*;
use atlas_core::model::{ContentAsset, ContentAssetReason, MixedResultKind};

/// Source types produced by Atlas session/runtime flows that should NOT be
/// returned as content assets. Non-code project file assets have other types.
const SESSION_ARTIFACT_SOURCE_TYPES: &[&str] = &[
    "review_context",
    "impact_result",
    "command_output",
    "bridge_artifact",
    "hook_event",
    "hook_handoff",
    "mcp_artifact",
    "test",
    "reasoning_result",
    "file_read",
    "file_write",
];

fn is_session_artifact(source_type: &str) -> bool {
    SESSION_ARTIFACT_SOURCE_TYPES.contains(&source_type)
}

/// Derive a content asset category from a file path extension.
fn content_type_from_path(path: &str) -> &'static str {
    let lower = path.to_lowercase();
    let ext = lower.rsplit('.').next().unwrap_or("");
    match ext {
        "md" | "markdown" | "rst" | "txt" | "adoc" | "asciidoc" => "doc",
        "toml" | "yaml" | "yml" | "json" | "ini" | "cfg" | "env" | "conf" | "config" => "config",
        "sql" => "sql",
        "html" | "jinja2" | "j2" | "mustache" | "handlebars" | "tera" | "twig" => "template",
        "prompt" => "prompt",
        _ => {
            // Treat files in a "prompts/" directory as prompts.
            if lower.contains("/prompts/") || lower.starts_with("prompts/") {
                "prompt"
            } else if lower.contains("/templates/") || lower.starts_with("templates/") {
                "template"
            } else {
                "other"
            }
        }
    }
}

/// Determine if a content asset path is adjacent to any of the changed files.
/// "Adjacent" means same directory or path prefix match.
fn is_adjacent_to_changed_file(asset_path: &str, changed_paths: &[String]) -> bool {
    let asset_dir = asset_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    for changed in changed_paths {
        let changed_dir = changed.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        if !asset_dir.is_empty() && !changed_dir.is_empty() && asset_dir == changed_dir {
            return true;
        }
        // Path prefix: asset is inside a subdirectory of the changed file's
        // parent (e.g. config/ next to src/).
        if !changed_dir.is_empty()
            && (asset_path.starts_with(changed_dir) || changed.starts_with(asset_dir))
        {
            return true;
        }
    }
    false
}

/// Retrieve content assets adjacent to the current context request.
///
/// Queries the content store using terms derived from changed file paths and
/// changed symbol names.  Results are classified by selection reason and ranked
/// by a composite score:
///   base_rank_score + adjacency_boost + symbol_mention_boost
///
/// Returns at most `max_content_assets` items ordered by descending score.
/// Equal-scored items are deterministically ordered by path.
pub(super) fn retrieve_content_assets(
    content_store: &ContentStore,
    request: &ContextRequest,
    result: &ContextResult,
    max_content_assets: usize,
) -> Vec<ContentAsset> {
    let changed_paths = extract_changed_paths_for_content(request, result);
    let symbol_names = extract_symbol_names(result);

    // Build search query from changed file base-names and top symbol names.
    let mut terms: Vec<String> = changed_paths
        .iter()
        .filter_map(|p| p.rsplit('/').next())
        .map(String::from)
        .collect();
    terms.extend(symbol_names.iter().take(5).cloned());
    terms.dedup();
    if terms.is_empty() {
        return Vec::new();
    }
    let query = terms.join(" ");

    let filters = SearchFilters {
        repo_root: None,
        session_id: None,
        agent_id: None,
        source_type: None,
    };

    let chunks = match content_store.search_with_fallback(&query, &filters) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Collect unique source IDs that are NOT session artifacts.
    let mut seen_source_ids: Vec<String> = Vec::new();
    for chunk in &chunks {
        if seen_source_ids.contains(&chunk.source_id) {
            continue;
        }
        // Peek at source metadata to filter session artifacts.
        if let Ok(Some(meta)) = content_store.get_source(&chunk.source_id)
            && !is_session_artifact(&meta.source_type)
        {
            seen_source_ids.push(chunk.source_id.clone());
            if seen_source_ids.len() >= max_content_assets * 3 {
                // Gather enough candidates before scoring.
                break;
            }
        }
    }

    if seen_source_ids.is_empty() {
        return Vec::new();
    }

    // Score and classify each candidate.
    let mut scored: Vec<ContentAsset> = Vec::new();
    for (rank, source_id) in seen_source_ids.iter().enumerate() {
        let meta = match content_store.get_source(source_id) {
            Ok(Some(m)) => m,
            _ => continue,
        };

        let path = meta.identity_value.clone();
        let preview: String = chunks
            .iter()
            .find(|c| &c.source_id == source_id)
            .map(|c| c.content.chars().take(512).collect())
            .unwrap_or_default();

        // Base score decaying with rank (same formula as saved context).
        let base_score = 1.0_f32 / (1.0 + rank as f32);
        let mut score = base_score;
        let mut selection_reason = ContentAssetReason::ContentMatch;

        // --- N3: normalized ranking signals ---

        // BM25 rank score as a proxy for the content match quality.
        let bm25_score = base_score;

        // Adjacency boost — same directory or path prefix.
        let mut changed_file_boost: Option<f32> = None;
        if is_adjacent_to_changed_file(&path, &changed_paths) {
            let boost = 0.5_f32;
            score += boost;
            changed_file_boost = Some(boost);
            selection_reason = ContentAssetReason::AdjacentToChangedFile;
        }

        // Symbol mention boost — check if the preview chunk mentions a changed
        // symbol name.  Also set exact_symbol_match when the mention is exact.
        let mut exact_symbol_match = false;
        let mut same_package_dir_boost: Option<f32> = None;
        if selection_reason != ContentAssetReason::AdjacentToChangedFile {
            let preview_lower = preview.to_lowercase();
            for sym in &symbol_names {
                if sym.is_empty() {
                    continue;
                }
                let sym_lower = sym.to_lowercase();
                if preview_lower.contains(&sym_lower) {
                    let boost = 0.3_f32;
                    score += boost;
                    same_package_dir_boost = Some(boost);
                    selection_reason = ContentAssetReason::RelatedToChangedSymbol;
                    // Exact match: symbol appears as a whole word in the preview.
                    // Simple word-boundary check using surrounding chars.
                    exact_symbol_match = preview_lower.match_indices(&sym_lower).any(|(idx, _)| {
                        let before_ok = idx == 0
                            || !preview_lower[..idx]
                                .chars()
                                .next_back()
                                .map(char::is_alphanumeric)
                                .unwrap_or(false);
                        let after_idx = idx + sym_lower.len();
                        let after_ok = after_idx >= preview_lower.len()
                            || !preview_lower[after_idx..]
                                .chars()
                                .next()
                                .map(char::is_alphanumeric)
                                .unwrap_or(false);
                        before_ok && after_ok
                    });
                    if exact_symbol_match {
                        // Additional boost for exact word match.
                        score += 0.1;
                    }
                    break;
                }
            }
        }

        // Path proximity boost — asset path shares a common prefix segment
        // with the changed file paths beyond the root.
        let path_proximity_boost: Option<f32> = {
            let asset_seg = path.split('/').next().unwrap_or("");
            if !asset_seg.is_empty()
                && changed_paths
                    .iter()
                    .any(|p| p.starts_with(asset_seg) || p.split('/').next() == Some(asset_seg))
            {
                let boost = 0.1_f32;
                score += boost;
                Some(boost)
            } else {
                None
            }
        };

        let content_type = content_type_from_path(&path).to_owned();
        // N3: derive surface kind from content type for the mixed-result discriminant.
        let source_kind = MixedResultKind::from_content_type(&content_type);

        let evidence = ContextRankingEvidence {
            base_score: Some(base_score),
            final_score: Some(score),
            source_kind: Some(source_kind),
            exact_symbol_match,
            bm25_score: Some(bm25_score),
            changed_file_boost,
            same_package_dir_boost,
            path_proximity_boost,
            ..ContextRankingEvidence::default()
        };

        scored.push(ContentAsset {
            source_id: source_id.clone(),
            path,
            content_type,
            preview,
            selection_reason,
            relevance_score: score,
            context_ranking_evidence: Some(evidence),
        });
    }

    // Sort: descending score; tie-break by priority then path (alphabetical).
    scored.sort_by(|a, b| {
        b.relevance_score
            .partial_cmp(&a.relevance_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.selection_reason
                    .priority()
                    .cmp(&a.selection_reason.priority())
            })
            .then_with(|| a.path.cmp(&b.path))
    });

    scored.truncate(max_content_assets);
    scored
}

/// Collect changed file paths from the request target or the already-built
/// result's direct-target files.
fn extract_changed_paths_for_content(
    request: &ContextRequest,
    result: &ContextResult,
) -> Vec<String> {
    match &request.target {
        ContextTarget::ChangedFiles { paths } => paths.clone(),
        ContextTarget::ChangedSymbols { qnames: _ } => result
            .nodes
            .iter()
            .filter(|n| n.selection_reason == SelectionReason::DirectTarget)
            .map(|n| n.node.file_path.clone())
            .collect(),
        _ => result
            .files
            .iter()
            .filter(|f| f.selection_reason == SelectionReason::DirectTarget)
            .map(|f| f.path.clone())
            .collect(),
    }
}

/// Collect symbol names from direct-target nodes in the result.
fn extract_symbol_names(result: &ContextResult) -> Vec<String> {
    let mut names: Vec<String> = result
        .nodes
        .iter()
        .filter(|n| n.selection_reason == SelectionReason::DirectTarget)
        .map(|n| n.node.name.clone())
        .collect();
    names.dedup();
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_type_from_known_extensions() {
        assert_eq!(content_type_from_path("README.md"), "doc");
        assert_eq!(content_type_from_path("config.toml"), "config");
        assert_eq!(content_type_from_path("schema.sql"), "sql");
        assert_eq!(content_type_from_path("layout.html"), "template");
        assert_eq!(content_type_from_path("system.prompt"), "prompt");
        assert_eq!(content_type_from_path("unknown.xyz"), "other");
    }

    #[test]
    fn content_type_from_directory_hints() {
        // Extension takes priority over directory hint.
        // .md in prompts/ → still "doc" because extension match wins.
        assert_eq!(content_type_from_path("prompts/system.md"), "doc");
        // Extension-less or unknown extensions fall through to directory hint.
        assert_eq!(content_type_from_path("prompts/system.prompt"), "prompt");
        assert_eq!(content_type_from_path("templates/layout.txt"), "doc");
        // Unknown extension uses directory hint.
        assert_eq!(content_type_from_path("prompts/system.xyz"), "prompt");
        assert_eq!(content_type_from_path("templates/layout.xyz"), "template");
    }

    #[test]
    fn adjacent_same_directory() {
        let changed = vec!["src/lib.rs".to_string()];
        assert!(is_adjacent_to_changed_file("src/README.md", &changed));
        assert!(!is_adjacent_to_changed_file("docs/guide.md", &changed));
    }

    #[test]
    fn adjacent_path_prefix_match() {
        let changed = vec!["src/handlers/auth.rs".to_string()];
        assert!(is_adjacent_to_changed_file(
            "src/handlers/auth.sql",
            &changed
        ));
    }

    #[test]
    fn content_asset_reason_priority_ordering() {
        assert!(
            ContentAssetReason::AdjacentToChangedFile.priority()
                > ContentAssetReason::RelatedToChangedSymbol.priority()
        );
        assert!(
            ContentAssetReason::RelatedToChangedSymbol.priority()
                > ContentAssetReason::ContentMatch.priority()
        );
    }

    #[test]
    fn session_artifact_filter_covers_known_types() {
        for t in SESSION_ARTIFACT_SOURCE_TYPES {
            assert!(
                is_session_artifact(t),
                "expected {t} to be session artifact"
            );
        }
        assert!(!is_session_artifact("file"));
        assert!(!is_session_artifact("doc"));
        assert!(!is_session_artifact("sql"));
    }
}
