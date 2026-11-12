//! ICM-G7 — overview selection for context payloads.

use atlas_core::Result;
use atlas_repo::CanonicalRepoPath;
use camino::Utf8Path;
use serde_json::Value;

use super::store::{MAX_SEARCH_LIMIT, load_records, recency_rank};
use super::types::{
    OverviewContextHit, OverviewContextQuery, OverviewFreshness, OverviewSubjectKind,
};
use crate::store::SessionStore;

impl SessionStore {
    /// Select fresh overview records for graph/content/session context.
    pub fn select_overviews_for_context(
        &self,
        query: &OverviewContextQuery,
    ) -> Result<Vec<OverviewContextHit>> {
        let canonical_files = query
            .file_paths
            .iter()
            .chain(query.changed_files.iter())
            .filter_map(|path| {
                CanonicalRepoPath::from_cli_argument(
                    Utf8Path::new(&query.repo_root),
                    Utf8Path::new(path),
                )
                .ok()
                .map(|path| path.as_str().to_owned())
            })
            .collect::<Vec<_>>();

        let mut hits = Vec::new();
        for record in load_records(&self.conn, &query.repo_root, true)? {
            if record.freshness == OverviewFreshness::Orphaned {
                continue;
            }
            if record.freshness.is_stale() && !query.include_stale {
                continue;
            }
            if !query.freshness.is_empty() && !query.freshness.contains(&record.freshness) {
                continue;
            }
            if !query.subject_kinds.is_empty()
                && !query.subject_kinds.contains(&record.subject_kind)
            {
                continue;
            }

            let mut reason = None;
            if record
                .qualified_name
                .as_deref()
                .is_some_and(|qname| query.qualified_names.iter().any(|value| value == qname))
            {
                reason = Some(("same_symbol", 5.0_f64));
            } else if record
                .canonical_file_path
                .as_deref()
                .is_some_and(|path| canonical_files.iter().any(|value| value == path))
            {
                let changed = record.canonical_file_path.as_deref().is_some_and(|path| {
                    query.changed_files.iter().any(|changed| {
                        CanonicalRepoPath::from_cli_argument(
                            Utf8Path::new(&query.repo_root),
                            Utf8Path::new(changed),
                        )
                        .ok()
                        .map(|path| path.as_str().to_owned())
                        .as_deref()
                            == Some(path)
                    })
                });
                reason = Some(if changed {
                    ("changed_dependency", 4.8_f64)
                } else {
                    ("same_file", 4.5_f64)
                });
            } else if record
                .module_path
                .as_deref()
                .is_some_and(|module| query.modules.iter().any(|value| value == module))
            {
                reason = Some(("containing_module", 3.0_f64));
            } else if record
                .package_name
                .as_deref()
                .is_some_and(|package| query.packages.iter().any(|value| value == package))
            {
                reason = Some(("package_summary", 2.5_f64));
            } else if record.subject_kind == OverviewSubjectKind::Project {
                reason = Some(("project_summary", 1.0_f64));
            }

            let Some((selection_reason, base)) = reason else {
                continue;
            };
            let score = base
                + f64::from(record.freshness.rank()) * 0.2
                + record.confidence
                + recency_rank(&record.updated_at) * 0.5;
            let stale_reason = if record.freshness.is_stale() {
                Some(
                    record
                        .metadata
                        .get("stale_reason")
                        .and_then(Value::as_str)
                        .unwrap_or("commit_or_graph_changed")
                        .to_owned(),
                )
            } else {
                None
            };
            hits.push(OverviewContextHit {
                record,
                selection_reason: selection_reason.to_owned(),
                score,
                stale_reason,
            });
        }

        hits.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.record.updated_at.cmp(&right.record.updated_at))
                .then_with(|| left.record.id.cmp(&right.record.id))
        });
        hits.truncate(query.limit.clamp(1, MAX_SEARCH_LIMIT));
        Ok(hits)
    }
}
