use super::super::*;
use super::parse;

#[test]
fn global_verbose_and_json_flags() {
    let cli = parse(&["atlas", "--verbose", "--json", "status"]);
    assert!(cli.verbose);
    assert!(cli.json);
    assert!(matches!(cli.command, Command::Status { .. }));
}
#[test]
fn global_repo_and_db_flags() {
    let cli = parse(&[
        "atlas",
        "--repo",
        "/tmp/proj",
        "--db",
        "/tmp/w.sqlite",
        "status",
    ]);
    assert_eq!(cli.repo.as_deref(), Some("/tmp/proj"));
    assert_eq!(cli.db.as_deref(), Some("/tmp/w.sqlite"));
}
#[test]
fn defaults_are_none_and_false() {
    let cli = parse(&["atlas", "init"]);
    assert!(cli.repo.is_none());
    assert!(cli.db.is_none());
    assert!(!cli.verbose);
    assert!(!cli.json);
}
