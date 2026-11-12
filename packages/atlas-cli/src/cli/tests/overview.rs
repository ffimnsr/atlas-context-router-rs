use super::super::*;
use super::parse;

#[test]
fn parse_overview_export_command() {
    let cli = parse(&[
        "atlas",
        "overview",
        "export",
        "--scope",
        "functions",
        "--subject",
        "src/lib.rs::fn::compute",
        "--include-code-spans",
        "--include-callers",
        "--limit",
        "5",
        "--max-tokens",
        "512",
    ]);
    assert!(matches!(
        cli.command,
        Command::Overview {
            subcommand:
                OverviewCommand::Export {
                    ref scope,
                    ref subjects,
                    limit: Some(5),
                    include_code_spans: true,
                    include_callers: true,
                    max_tokens: Some(512),
                    ..
                }
        } if scope == "functions" && subjects == &vec!["src/lib.rs::fn::compute".to_owned()]
    ));
}

#[test]
fn parse_overview_ingest_stdin_and_show() {
    let cli = parse(&["atlas", "overview", "ingest", "-"]);
    assert!(matches!(
        cli.command,
        Command::Overview {
            subcommand: OverviewCommand::Ingest { ref input }
        } if input == "-"
    ));

    let cli = parse(&["atlas", "overview", "show", "src/lib.rs"]);
    assert!(matches!(
        cli.command,
        Command::Overview {
            subcommand: OverviewCommand::Show { ref subject }
        } if subject == "src/lib.rs"
    ));
}

#[test]
fn parse_overview_search_list_stale_filters() {
    let cli = parse(&[
        "atlas",
        "overview",
        "search",
        "parser",
        "--subject-kind",
        "file",
        "--freshness",
        "fresh",
        "--limit",
        "3",
    ]);
    assert!(matches!(
        cli.command,
        Command::Overview {
            subcommand:
                OverviewCommand::Search {
                    ref query,
                    ref subject_kind,
                    ref freshness,
                    limit: Some(3),
                    ..
                }
        } if query == "parser"
            && subject_kind.as_deref() == Some("file")
            && freshness.as_deref() == Some("fresh")
    ));

    let cli = parse(&["atlas", "overview", "list", "--freshness", "stale"]);
    assert!(matches!(
        cli.command,
        Command::Overview {
            subcommand: OverviewCommand::List {
                ref freshness,
                ..
            }
        } if freshness.as_deref() == Some("stale")
    ));

    let cli = parse(&["atlas", "overview", "stale", "--file", "src/lib.rs"]);
    assert!(matches!(
        cli.command,
        Command::Overview {
            subcommand: OverviewCommand::Stale { ref file, .. }
        } if file.as_deref() == Some("src/lib.rs")
    ));
}

#[test]
fn parse_overview_refresh_plan_apply_and_delete_dry_run() {
    let cli = parse(&[
        "atlas",
        "overview",
        "refresh-plan",
        "--changed",
        "--since",
        "HEAD~1",
        "--apply",
    ]);
    assert!(matches!(
        cli.command,
        Command::Overview {
            subcommand:
                OverviewCommand::RefreshPlan {
                    changed: true,
                    ref since,
                    apply: true,
                }
        } if since.as_deref() == Some("HEAD~1")
    ));

    let cli = parse(&["atlas", "overview", "delete", "overview-123", "--dry-run"]);
    assert!(matches!(
        cli.command,
        Command::Overview {
            subcommand: OverviewCommand::Delete {
                ref overview_id,
                dry_run: true,
            }
        } if overview_id == "overview-123"
    ));
}

#[test]
fn parse_overview_export_docs_and_run_external() {
    let cli = parse(&[
        "atlas",
        "overview",
        "export-docs",
        "--format",
        "markdown",
        "--output",
        "-",
    ]);
    assert!(matches!(
        cli.command,
        Command::Overview {
            subcommand: OverviewCommand::ExportDocs {
                ref format,
                ref output,
            }
        } if format == "markdown" && output == "-"
    ));

    let cli = parse(&[
        "atlas",
        "overview",
        "run-external",
        "--dry-run",
        "--scope",
        "changed",
        "--since",
        "HEAD~1",
    ]);
    assert!(matches!(
        cli.command,
        Command::Overview {
            subcommand:
                OverviewCommand::RunExternal {
                    dry_run: true,
                    apply: false,
                    ref scope,
                    ref since,
                    ..
                }
        } if scope == "changed" && since.as_deref() == Some("HEAD~1")
    ));
}
