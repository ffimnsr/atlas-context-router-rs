//! ICM-G5 — freshness, commit update, and refresh planning.

use std::collections::BTreeSet;

use atlas_core::model::{Edge, Node};
use atlas_core::{AtlasError, Result};
use atlas_repo::{DiffTarget, changed_files};
use atlas_session::{
    OVERVIEW_SCHEMA_VERSION, OverviewFreshness, OverviewListFilter, OverviewSubjectKind,
    SessionStore,
};
use atlas_store_sqlite::Store;
use camino::Utf8Path;

use super::types::{OverviewRefreshItem, OverviewRefreshPlan};
use super::util;

/// Explicit service facade for freshness comparison and refresh planning.
#[derive(Debug, Default, Clone, Copy)]
pub struct OverviewFreshnessService;

impl OverviewFreshnessService {
    /// Build a refresh plan. Freshness marking only happens when `apply` is
    /// explicitly true.
    pub fn plan(
        &self,
        repo_root: &str,
        store: &Store,
        session: &mut SessionStore,
        since: Option<&str>,
        apply: bool,
    ) -> Result<OverviewRefreshPlan> {
        refresh_overview_plan(repo_root, store, session, since, apply)
    }
}

/// Build a read-only refresh plan. Freshness marking only happens when
/// `apply` is explicitly true.
pub fn refresh_overview_plan(
    repo_root: &str,
    store: &Store,
    session: &mut SessionStore,
    since: Option<&str>,
    apply: bool,
) -> Result<OverviewRefreshPlan> {
    let target = match since {
        Some(since) => DiffTarget::BaseRef(since.to_owned()),
        None => DiffTarget::WorkingTree,
    };
    let changed = match changed_files(Utf8Path::new(repo_root), &target) {
        Ok(changed) => changed,
        // No git repository: default working-tree planning is a no-op, not a
        // hard failure. Explicit --since refs still fail clearly below.
        Err(_) if since.is_none() => Vec::new(),
        Err(error) => {
            return Err(AtlasError::Other(format!(
                "cannot read changed files: {error}"
            )));
        }
    };
    let mut changed_paths = BTreeSet::new();
    for change in changed {
        if let Ok(path) = util::canonical_path(repo_root, &change.path) {
            changed_paths.insert(path);
        }
    }

    let nodes = store.list_all_nodes()?;
    let edges = store.list_all_edges()?;
    let (_by_qname, by_file) = util::index_nodes(&nodes);

    let mut changed_qnames = BTreeSet::new();
    for path in &changed_paths {
        if let Some(indexes) = by_file.get(path) {
            for index in indexes {
                changed_qnames.insert(nodes[*index].qualified_name.clone());
            }
        }
    }

    let records = session.list_overview_records(
        repo_root,
        &OverviewListFilter {
            limit: Some(200),
            ..Default::default()
        },
    )?;

    let mut direct_stale = Vec::new();
    let mut possibly_stale = Vec::new();
    let mut orphaned = Vec::new();
    let mut unchanged_count = 0;
    let mut applied_count = 0usize;

    for record in records {
        let subject = record.subject();
        let matched = util::matching_nodes(&subject, &nodes, &by_file);
        let resolved = !matched.is_empty() || subject.kind == OverviewSubjectKind::Project;
        let file_path = record.canonical_file_path.as_deref();
        let file_changed = file_path.is_some_and(|path| changed_paths.contains(path));
        let file_deleted =
            file_path.is_some_and(|path| !Utf8Path::new(repo_root).join(path).exists());
        let content_changed =
            file_path
                .zip(record.content_hash.as_deref())
                .is_some_and(|(path, stored)| {
                    util::file_content_hash(repo_root, path)
                        .is_some_and(|current| current != stored)
                });

        if !resolved || file_deleted {
            orphaned.push(item(&record, "subject_no_longer_resolves", 90));
            if apply && mark_if_changed(session, &record, OverviewFreshness::Orphaned)? {
                applied_count += 1;
            }
            continue;
        }

        if file_changed || content_changed {
            direct_stale.push(item(
                &record,
                if file_changed {
                    "direct_file_change"
                } else {
                    "content_hash_changed"
                },
                100,
            ));
            if apply && mark_if_changed(session, &record, OverviewFreshness::Stale)? {
                applied_count += 1;
            }
            continue;
        }

        if dependent_changed(&record, &changed_paths, &changed_qnames, &edges, &matched) {
            possibly_stale.push(item(&record, "dependent_change", 60));
            if apply && mark_if_changed(session, &record, OverviewFreshness::PossiblyStale)? {
                applied_count += 1;
            }
            continue;
        }

        unchanged_count += 1;
    }

    sort_items(&mut direct_stale);
    sort_items(&mut possibly_stale);
    sort_items(&mut orphaned);

    Ok(OverviewRefreshPlan {
        schema_version: OVERVIEW_SCHEMA_VERSION,
        repo_root: repo_root.to_owned(),
        since: since.map(str::to_owned),
        changed_files: changed_paths.into_iter().collect(),
        apply,
        applied_count,
        direct_stale,
        possibly_stale,
        orphaned,
        unchanged_count,
    })
}

fn mark_if_changed(
    session: &mut SessionStore,
    record: &atlas_session::OverviewRecord,
    target: OverviewFreshness,
) -> Result<bool> {
    if record.freshness == target {
        return Ok(false);
    }
    session.mark_overview_freshness(&record.id, target)?;
    Ok(true)
}

fn dependent_changed(
    record: &atlas_session::OverviewRecord,
    changed_paths: &BTreeSet<String>,
    changed_qnames: &BTreeSet<String>,
    edges: &[Edge],
    matched: &[&Node],
) -> bool {
    match record.subject_kind {
        OverviewSubjectKind::Project => !changed_paths.is_empty(),
        OverviewSubjectKind::Package => {
            let package = record.package_name.as_deref().unwrap_or_default();
            matched.iter().any(|node| {
                changed_paths.contains(&node.file_path)
                    || util::node_package(node).as_deref() == Some(package)
            }) && !changed_paths.is_empty()
        }
        OverviewSubjectKind::Module => {
            let module = record.module_path.as_deref().unwrap_or_default();
            changed_paths
                .iter()
                .any(|path| util::module_path_for_file(path) == module || path.starts_with(module))
        }
        OverviewSubjectKind::File => false,
        OverviewSubjectKind::Symbol
        | OverviewSubjectKind::Function
        | OverviewSubjectKind::Method => {
            let qname = record.qualified_name.as_deref().unwrap_or_default();
            edges
                .iter()
                .any(|edge| edge.source_qn == qname && changed_qnames.contains(&edge.target_qn))
        }
    }
}

fn item(
    record: &atlas_session::OverviewRecord,
    stale_reason: &str,
    priority: u32,
) -> OverviewRefreshItem {
    let label = record
        .qualified_name
        .clone()
        .or_else(|| record.canonical_file_path.clone())
        .or_else(|| record.package_name.clone())
        .or_else(|| record.module_path.clone())
        .unwrap_or_else(|| record.subject_id.clone());
    OverviewRefreshItem {
        overview_id: record.id.clone(),
        subject_id: record.subject_id.clone(),
        subject_kind: record.subject_kind,
        subject_label: label.clone(),
        freshness: record.freshness,
        stale_reason: stale_reason.to_owned(),
        suggested_export_command: format!(
            "atlas overview export --subject {} --include-code-spans --json",
            shell_quote(&label)
        ),
        affected_dependents: Vec::new(),
        previous_record_id: record.supersedes_id.clone(),
        priority: priority
            + u32::from(record.subject_kind.specificity()) * 2
            + (record.confidence * 10.0) as u32,
    }
}

fn sort_items(items: &mut [OverviewRefreshItem]) {
    items.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.subject_id.cmp(&right.subject_id))
    });
}

fn shell_quote(value: &str) -> String {
    if value
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '~'))
    {
        value.to_owned()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

/// Validate that a caller-provided `since` resolves in git. Exposed so CLI can
/// fail before touching storage.
pub fn validate_since(repo_root: &str, since: &str) -> Result<()> {
    let target = DiffTarget::BaseRef(since.to_owned());
    changed_files(Utf8Path::new(repo_root), &target)
        .map(|_| ())
        .map_err(|error| AtlasError::Other(format!("invalid overview --since {since}: {error}")))
}
