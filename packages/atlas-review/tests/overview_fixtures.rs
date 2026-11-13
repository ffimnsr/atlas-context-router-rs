//! ICM-G10 — reusable overview fixture validation.

use atlas_session::{
    OVERVIEW_INLINE_BODY_BUDGET_BYTES, OverviewAnalysisInput, OverviewSubjectKind,
};

fn parse(name: &str, raw: &str) -> OverviewAnalysisInput {
    let input: OverviewAnalysisInput =
        serde_json::from_str(raw).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_eq!(input.schema_version, 1, "{name}");
    assert!(!input.summary.trim().is_empty(), "{name}");
    assert!((0.0..=1.0).contains(&input.confidence), "{name}");
    input
}

#[test]
fn overview_fixtures_cover_all_subject_kinds() {
    let fixtures = [
        (
            "project",
            include_str!("overview_fixture_project.json"),
            OverviewSubjectKind::Project,
        ),
        (
            "package",
            include_str!("overview_fixture_package.json"),
            OverviewSubjectKind::Package,
        ),
        (
            "module",
            include_str!("overview_fixture_module.json"),
            OverviewSubjectKind::Module,
        ),
        (
            "file",
            include_str!("overview_fixture_file.json"),
            OverviewSubjectKind::File,
        ),
        (
            "function",
            include_str!("overview_fixture_function.json"),
            OverviewSubjectKind::Function,
        ),
        (
            "stale_function",
            include_str!("overview_fixture_stale_function.json"),
            OverviewSubjectKind::Function,
        ),
        (
            "orphaned_symbol",
            include_str!("overview_fixture_orphaned_symbol.json"),
            OverviewSubjectKind::Symbol,
        ),
        (
            "manual",
            include_str!("overview_fixture_manual.json"),
            OverviewSubjectKind::File,
        ),
    ];

    for (name, raw, kind) in fixtures {
        let input = parse(name, raw);
        let subject = input
            .subject
            .clone()
            .into_structured()
            .unwrap_or_else(|| panic!("{name}: fixture must use a structured subject"));
        assert_eq!(subject.kind, kind, "{name}");
        subject
            .validated()
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
fn oversized_overview_fixture_exceeds_inline_budget() {
    let input = parse("oversized", include_str!("overview_fixture_oversized.json"));
    assert!(input.description.len() > OVERVIEW_INLINE_BODY_BUDGET_BYTES);
}

#[test]
fn stale_fixture_uses_an_older_commit() {
    let current = parse("function", include_str!("overview_fixture_function.json"));
    let stale = parse(
        "stale_function",
        include_str!("overview_fixture_stale_function.json"),
    );
    let current = current
        .subject
        .into_structured()
        .expect("structured subject");
    let stale = stale.subject.into_structured().expect("structured subject");
    assert_ne!(stale.commit_sha, current.commit_sha);
    assert_eq!(
        stale.qualified_name, current.qualified_name,
        "same subject identity must survive commit drift"
    );
}
