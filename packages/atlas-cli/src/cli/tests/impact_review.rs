use super::super::*;
use super::parse;

#[test]
fn parse_impact_defaults() {
    let cli = parse(&["atlas", "impact"]);
    if let Command::Impact {
        max_depth,
        max_nodes,
        ..
    } = cli.command
    {
        assert_eq!(max_depth, 5);
        assert_eq!(max_nodes, 200);
    } else {
        panic!("expected Impact command");
    }
}
#[test]
fn parse_impact_with_files() {
    let cli = parse(&["atlas", "impact", "--files", "a.rs", "b.rs"]);
    if let Command::Impact { files, .. } = cli.command {
        assert_eq!(files, vec!["a.rs", "b.rs"]);
    } else {
        panic!("expected Impact command");
    }
}
#[test]
fn parse_impact_with_depth_and_nodes() {
    let cli = parse(&["atlas", "impact", "--max-depth", "3", "--max-nodes", "50"]);
    if let Command::Impact {
        max_depth,
        max_nodes,
        ..
    } = cli.command
    {
        assert_eq!(max_depth, 3);
        assert_eq!(max_nodes, 50);
    } else {
        panic!("expected Impact command");
    }
}
#[test]
fn parse_impact_repo_id_flag() {
    let cli = parse(&["atlas", "impact", "--repo-id", "repo_abc"]);
    if let Command::Impact {
        repo_id, all_repos, ..
    } = cli.command
    {
        assert_eq!(repo_id.as_deref(), Some("repo_abc"));
        assert!(!all_repos);
    } else {
        panic!("expected Impact command");
    }
}
#[test]
fn parse_review_context_defaults() {
    let cli = parse(&["atlas", "review-context"]);
    if let Command::ReviewContext {
        max_depth,
        max_nodes,
        base,
        files,
        format,
        ..
    } = cli.command
    {
        assert_eq!(max_depth, 3);
        assert_eq!(max_nodes, 200);
        assert!(base.is_none());
        assert!(files.is_empty());
        assert_eq!(format, ReviewContextFormat::Text);
    } else {
        panic!("expected ReviewContext command");
    }
}
#[test]
fn parse_review_context_markdown_format() {
    let cli = parse(&["atlas", "review-context", "--format", "markdown"]);
    if let Command::ReviewContext { format, .. } = cli.command {
        assert_eq!(format, ReviewContextFormat::Markdown);
    } else {
        panic!("expected ReviewContext command");
    }
}
#[test]
fn parse_session_decisions_command() {
    let cli = parse(&[
        "atlas",
        "session",
        "decisions",
        "verify_token",
        "--current-session",
        "--limit",
        "7",
    ]);
    if let Command::Session { subcommand } = cli.command {
        match subcommand {
            SessionCommand::Decisions {
                query,
                current_session,
                limit,
            } => {
                assert_eq!(query, "verify_token");
                assert!(current_session);
                assert_eq!(limit, 7);
            }
            _ => panic!("expected SessionCommand::Decisions"),
        }
    } else {
        panic!("expected Session command");
    }
}
#[test]
fn parse_detect_changes_with_base() {
    let cli = parse(&["atlas", "detect-changes", "--base", "origin/main"]);
    if let Command::DetectChanges { base, staged, .. } = cli.command {
        assert_eq!(base.as_deref(), Some("origin/main"));
        assert!(!staged);
    } else {
        panic!("expected DetectChanges command");
    }
}
#[test]
fn parse_detect_changes_staged() {
    let cli = parse(&["atlas", "detect-changes", "--staged"]);
    if let Command::DetectChanges { staged, .. } = cli.command {
        assert!(staged);
    } else {
        panic!("expected DetectChanges command");
    }
}
#[test]
fn parse_detect_changes_all_repos() {
    let cli = parse(&["atlas", "detect-changes", "--all-repos"]);
    assert!(matches!(
        cli.command,
        Command::DetectChanges {
            all_repos: true,
            repo_id: None,
            ..
        }
    ));
}
#[test]
fn parse_explain_change_with_base() {
    let cli = parse(&["atlas", "explain-change", "--base", "origin/main"]);
    if let Command::ExplainChange {
        base,
        staged,
        files,
        max_depth,
        max_nodes,
        ..
    } = cli.command
    {
        assert_eq!(base.as_deref(), Some("origin/main"));
        assert!(!staged);
        assert!(files.is_empty());
        assert_eq!(max_depth, 5);
        assert_eq!(max_nodes, 200);
    } else {
        panic!("expected ExplainChange command");
    }
}
