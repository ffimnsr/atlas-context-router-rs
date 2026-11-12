//! ICM-G6 — readable Markdown export of overview memory.

use atlas_core::Result;
use atlas_session::{OverviewListFilter, OverviewRecord, OverviewSubjectKind, SessionStore};

use super::types::OverviewDocsExport;

const GROUP_ORDER: [OverviewSubjectKind; 7] = [
    OverviewSubjectKind::Project,
    OverviewSubjectKind::Package,
    OverviewSubjectKind::Module,
    OverviewSubjectKind::File,
    OverviewSubjectKind::Symbol,
    OverviewSubjectKind::Function,
    OverviewSubjectKind::Method,
];

/// Export active overview records grouped by project, package, module, file,
/// then symbol. Stale records are labeled visibly.
pub fn export_overview_docs(repo_root: &str, session: &SessionStore) -> Result<OverviewDocsExport> {
    let records = session.list_overview_records(
        repo_root,
        &OverviewListFilter {
            limit: Some(200),
            ..Default::default()
        },
    )?;
    let stale_count = records
        .iter()
        .filter(|record| record.freshness.is_stale())
        .count();

    let mut markdown = String::new();
    markdown.push_str("# Code Overview Memory\n\n");
    markdown.push_str(&format!("Repo: `{repo_root}`\n\n"));
    markdown.push_str(&format!(
        "Records: {} · stale: {}\n\n",
        records.len(),
        stale_count
    ));

    for kind in GROUP_ORDER {
        let group = records
            .iter()
            .filter(|record| record.subject_kind == kind)
            .collect::<Vec<_>>();
        if group.is_empty() {
            continue;
        }
        markdown.push_str(&format!("## {}\n\n", group_heading(kind)));
        for record in group {
            markdown.push_str(&render_record(record));
        }
    }

    Ok(OverviewDocsExport {
        markdown,
        record_count: records.len(),
        stale_count,
    })
}

fn group_heading(kind: OverviewSubjectKind) -> &'static str {
    match kind {
        OverviewSubjectKind::Project => "Project",
        OverviewSubjectKind::Package => "Packages",
        OverviewSubjectKind::Module => "Modules",
        OverviewSubjectKind::File => "Files",
        OverviewSubjectKind::Symbol => "Symbols",
        OverviewSubjectKind::Function => "Functions",
        OverviewSubjectKind::Method => "Methods",
    }
}

fn render_record(record: &OverviewRecord) -> String {
    let mut out = String::new();
    let stale = if record.freshness.is_stale() {
        format!(" **(stale: {})**", record.freshness.as_str())
    } else {
        String::new()
    };
    let title = if record.title.trim().is_empty() {
        record
            .qualified_name
            .clone()
            .or_else(|| record.canonical_file_path.clone())
            .or_else(|| record.package_name.clone())
            .or_else(|| record.module_path.clone())
            .unwrap_or_else(|| record.subject_id.clone())
    } else {
        record.title.clone()
    };
    out.push_str(&format!("### {title}{stale}\n\n"));
    if !record.summary.trim().is_empty() {
        out.push_str(&format!("{}\n\n", record.summary.trim()));
    }
    if !record.description_preview.trim().is_empty() {
        out.push_str(&format!("{}\n\n", record.description_preview.trim()));
    }
    for (label, values) in [
        ("Responsibilities", &record.responsibilities),
        ("Flow", &record.flow),
        ("Inputs", &record.inputs),
        ("Outputs", &record.outputs),
        ("Invariants", &record.invariants),
        ("Gotchas", &record.gotchas),
        ("Examples", &record.examples),
    ] {
        if values.is_empty() {
            continue;
        }
        out.push_str(&format!("**{label}**\n\n"));
        for value in values {
            out.push_str(&format!("- {value}\n"));
        }
        out.push('\n');
    }
    if !record.tags.is_empty() {
        out.push_str(&format!("Tags: {}\n\n", record.tags.join(", ")));
    }
    if let Some(source_id) = record.source_id.as_deref() {
        out.push_str(&format!("Evidence: `source_id={source_id}`\n\n"));
    }
    out.push_str(&format!(
        "Freshness: `{}` · confidence: {:.2} · commit: `{}`\n\n",
        record.freshness.as_str(),
        record.confidence,
        record.commit_sha
    ));
    out
}
