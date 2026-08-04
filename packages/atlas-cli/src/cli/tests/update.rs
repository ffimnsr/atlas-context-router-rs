use super::super::*;
use super::parse;

#[test]
fn parse_update_with_base_ref() {
    let cli = parse(&["atlas", "update", "--base", "origin/main"]);
    if let Command::Update {
        base,
        staged,
        working_tree,
        files,
        fail_fast,
        dry_run,
        ..
    } = cli.command
    {
        assert_eq!(base.as_deref(), Some("origin/main"));
        assert!(!staged);
        assert!(!working_tree);
        assert!(files.is_empty());
        assert!(!fail_fast);
        assert!(!dry_run);
    } else {
        panic!("expected Update command");
    }
}
#[test]
fn parse_update_staged() {
    let cli = parse(&["atlas", "update", "--staged"]);
    if let Command::Update { staged, .. } = cli.command {
        assert!(staged);
    } else {
        panic!("expected Update command");
    }
}
#[test]
fn parse_update_explicit_files() {
    let cli = parse(&["atlas", "update", "--files", "src/a.rs", "src/b.rs"]);
    if let Command::Update { files, .. } = cli.command {
        assert_eq!(files, vec!["src/a.rs", "src/b.rs"]);
    } else {
        panic!("expected Update command");
    }
}
#[test]
fn parse_update_dry_run() {
    let cli = parse(&["atlas", "update", "--dry-run"]);
    if let Command::Update { dry_run, .. } = cli.command {
        assert!(dry_run);
    } else {
        panic!("expected Update command");
    }
}
#[test]
fn parse_update_affected_repos() {
    let cli = parse(&["atlas", "update", "--affected-repos"]);
    assert!(matches!(
        cli.command,
        Command::Update {
            affected_repos: true,
            all_repos: false,
            repo_id: None,
            ..
        }
    ));
}
