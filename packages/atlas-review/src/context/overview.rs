//! ICM-G7 — overview memory in context payloads.

use atlas_core::Result;
use atlas_core::model::{ContextOverview, ContextRequest, ContextResult, ContextTarget};
use atlas_session::{OverviewContextQuery, OverviewFreshness, OverviewSubjectKind, SessionStore};

const DEFAULT_OVERVIEW_LIMIT: usize = 10;
const MAX_OVERVIEW_LIMIT: usize = 50;
const DEFAULT_OVERVIEW_MAX_BYTES: usize = 4_096;

pub(super) fn overviews_for_context(
    session: &SessionStore,
    repo_root: &str,
    request: &ContextRequest,
    result: &ContextResult,
) -> Result<Vec<ContextOverview>> {
    let freshness = request
        .overview_freshness
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|value| value.parse::<OverviewFreshness>().ok())
        .collect::<Vec<_>>();
    let subject_kinds = request
        .overview_subject_kinds
        .as_deref()
        .unwrap_or_default()
        .iter()
        .filter_map(|value| value.parse::<OverviewSubjectKind>().ok())
        .collect::<Vec<_>>();
    let include_stale = freshness.iter().any(|state| {
        matches!(
            state,
            OverviewFreshness::PossiblyStale | OverviewFreshness::Stale
        )
    });

    let mut qualified_names = result
        .nodes
        .iter()
        .map(|node| node.node.qualified_name.clone())
        .collect::<Vec<_>>();
    let mut file_paths = result
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let mut changed_files = Vec::new();
    match &request.target {
        ContextTarget::FilePath { path } => file_paths.push(path.clone()),
        ContextTarget::ChangedFiles { paths } => {
            file_paths.extend(paths.iter().cloned());
            changed_files.extend(paths.iter().cloned());
        }
        ContextTarget::QualifiedName { qname } => qualified_names.push(qname.clone()),
        _ => {}
    }
    let mut modules = result
        .nodes
        .iter()
        .map(|node| {
            node.node
                .file_path
                .rsplit_once('/')
                .map(|(parent, _)| parent.to_owned())
                .unwrap_or_else(|| ".".to_owned())
        })
        .collect::<Vec<_>>();
    let mut packages = result
        .nodes
        .iter()
        .filter_map(|node| {
            node.node
                .extra_json
                .get("owner_name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    qualified_names.sort();
    qualified_names.dedup();
    file_paths.sort();
    file_paths.dedup();
    modules.sort();
    modules.dedup();
    packages.sort();
    packages.dedup();

    let query = OverviewContextQuery {
        repo_root: repo_root.to_owned(),
        qualified_names,
        file_paths,
        modules,
        packages,
        changed_files,
        freshness,
        subject_kinds,
        include_stale,
        limit: request
            .overview_limit
            .unwrap_or(DEFAULT_OVERVIEW_LIMIT)
            .clamp(1, MAX_OVERVIEW_LIMIT),
    };
    let hits = session.select_overviews_for_context(&query)?;

    let max_bytes = request
        .overview_max_bytes
        .unwrap_or(DEFAULT_OVERVIEW_MAX_BYTES);
    let mut emitted_bytes = 0usize;
    let mut overviews = Vec::new();
    for hit in hits {
        let preview = hit.record.description_preview.clone();
        let summary = hit.record.summary.clone();
        let added = preview.len() + summary.len();
        if emitted_bytes + added > max_bytes && !overviews.is_empty() {
            break;
        }
        emitted_bytes += added;
        let repo_id = atlas_repo::stable_repo_id(camino::Utf8Path::new(repo_root));
        let uri = hit
            .record
            .qualified_name
            .as_deref()
            .map(|qname| {
                format!(
                    "atlas://repo/{}/symbol/{}",
                    encode_uri_segment(&repo_id),
                    encode_uri_segment(qname)
                )
            })
            .or_else(|| {
                hit.record.canonical_file_path.as_deref().map(|path| {
                    format!(
                        "atlas://repo/{}/file/{}",
                        encode_uri_segment(&repo_id),
                        encode_uri_segment(path)
                    )
                })
            });
        overviews.push(ContextOverview {
            overview_id: hit.record.id,
            subject_id: hit.record.subject_id,
            subject_kind: hit.record.subject_kind.as_str().to_owned(),
            title: hit.record.title,
            summary,
            preview,
            freshness: hit.record.freshness.as_str().to_owned(),
            confidence: hit.record.confidence,
            selection_reason: hit.selection_reason,
            source_id: hit.record.source_id,
            canonical_file_path: hit.record.canonical_file_path,
            qualified_name: hit.record.qualified_name,
            stale_reason: hit.stale_reason,
            level: Some("l1".to_owned()),
            uri,
        });
    }
    Ok(overviews)
}

/// Percent-encode one URI path segment (RFC 3986 unreserved plus `:`).
fn encode_uri_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b':' => {
                out.push(byte as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
