use super::super::*;
use super::parse;

#[test]
fn parse_history_update_with_repair() {
    let cli = parse(&[
        "atlas",
        "history",
        "update",
        "--branch",
        "main",
        "--repair",
        "--max-commits",
        "25",
    ]);
    if let Command::History { subcommand } = cli.command {
        match subcommand {
            HistoryCommand::Update {
                branch,
                repair,
                max_commits,
                yes,
            } => {
                assert_eq!(branch.as_deref(), Some("main"));
                assert!(repair);
                assert_eq!(max_commits, Some(25));
                assert!(!yes);
            }
            _ => panic!("expected history update"),
        }
    } else {
        panic!("expected History command");
    }
}
#[test]
fn parse_history_build_with_yes() {
    let cli = parse(&["atlas", "history", "build", "--yes"]);
    if let Command::History { subcommand } = cli.command {
        match subcommand {
            HistoryCommand::Build { yes, .. } => assert!(yes),
            _ => panic!("expected history build"),
        }
    } else {
        panic!("expected History command");
    }
}
#[test]
fn parse_history_update_with_yes() {
    let cli = parse(&["atlas", "history", "update", "--yes"]);
    if let Command::History { subcommand } = cli.command {
        match subcommand {
            HistoryCommand::Update { yes, .. } => assert!(yes),
            _ => panic!("expected history update"),
        }
    } else {
        panic!("expected History command");
    }
}
#[test]
fn parse_history_rebuild_commit() {
    let cli = parse(&[
        "atlas",
        "history",
        "rebuild",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ]);
    if let Command::History { subcommand } = cli.command {
        match subcommand {
            HistoryCommand::Rebuild { commit_sha } => {
                assert_eq!(commit_sha, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
            }
            _ => panic!("expected history rebuild"),
        }
    } else {
        panic!("expected History command");
    }
}
#[test]
fn parse_history_diff_commits() {
    let cli = parse(&[
        "atlas",
        "history",
        "diff",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "--stat-only",
    ]);
    if let Command::History { subcommand } = cli.command {
        match subcommand {
            HistoryCommand::Diff {
                commit_a,
                commit_b,
                stat_only,
                full,
            } => {
                assert_eq!(commit_a, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
                assert_eq!(commit_b, "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
                assert!(stat_only);
                assert!(!full);
            }
            _ => panic!("expected history diff"),
        }
    } else {
        panic!("expected History command");
    }
}
#[test]
fn parse_history_symbol_query() {
    let cli = parse(&[
        "atlas",
        "history",
        "symbol",
        "src/lib.rs::fn::helper",
        "--full",
    ]);
    if let Command::History { subcommand } = cli.command {
        match subcommand {
            HistoryCommand::Symbol {
                qualified_name,
                stat_only,
                full,
            } => {
                assert_eq!(qualified_name, "src/lib.rs::fn::helper");
                assert!(!stat_only);
                assert!(full);
            }
            _ => panic!("expected history symbol"),
        }
    } else {
        panic!("expected History command");
    }
}
#[test]
fn parse_history_file_query() {
    let cli = parse(&["atlas", "history", "file", "src/lib.rs", "--follow-renames"]);
    if let Command::History { subcommand } = cli.command {
        match subcommand {
            HistoryCommand::File {
                path,
                follow_renames,
                stat_only,
                full,
            } => {
                assert_eq!(path, "src/lib.rs");
                assert!(follow_renames);
                assert!(!stat_only);
                assert!(!full);
            }
            _ => panic!("expected history file"),
        }
    } else {
        panic!("expected History command");
    }
}
#[test]
fn parse_history_dependency_query() {
    let cli = parse(&[
        "atlas",
        "history",
        "dependency",
        "src/lib.rs::fn::helper",
        "src/lib.rs::method::Greeter::greet_twice",
        "--stat-only",
    ]);
    if let Command::History { subcommand } = cli.command {
        match subcommand {
            HistoryCommand::Dependency {
                source,
                target,
                stat_only,
                full,
            } => {
                assert_eq!(source, "src/lib.rs::fn::helper");
                assert_eq!(target, "src/lib.rs::method::Greeter::greet_twice");
                assert!(stat_only);
                assert!(!full);
            }
            _ => panic!("expected history dependency"),
        }
    } else {
        panic!("expected History command");
    }
}
#[test]
fn parse_history_module_query() {
    let cli = parse(&["atlas", "history", "module", "src/lib.rs"]);
    if let Command::History { subcommand } = cli.command {
        match subcommand {
            HistoryCommand::Module {
                module,
                stat_only,
                full,
            } => {
                assert_eq!(module, "src/lib.rs");
                assert!(!stat_only);
                assert!(!full);
            }
            _ => panic!("expected history module"),
        }
    } else {
        panic!("expected History command");
    }
}
#[test]
fn parse_history_churn_command() {
    let cli = parse(&["atlas", "history", "churn", "--full"]);
    if let Command::History { subcommand } = cli.command {
        match subcommand {
            HistoryCommand::Churn { stat_only, full } => {
                assert!(!stat_only);
                assert!(full);
            }
            _ => panic!("expected history churn"),
        }
    } else {
        panic!("expected History command");
    }
}
#[test]
fn parse_history_prune_command() {
    let cli = parse(&[
        "atlas",
        "history",
        "prune",
        "--keep-latest",
        "5",
        "--older-than-days",
        "14",
        "--keep-tagged-only",
        "--keep-weekly",
    ]);
    if let Command::History { subcommand } = cli.command {
        match subcommand {
            HistoryCommand::Prune {
                keep_all,
                keep_latest,
                older_than_days,
                keep_tagged_only,
                keep_weekly,
            } => {
                assert!(!keep_all);
                assert_eq!(keep_latest, Some(5));
                assert_eq!(older_than_days, Some(14));
                assert!(keep_tagged_only);
                assert!(keep_weekly);
            }
            _ => panic!("expected history prune"),
        }
    } else {
        panic!("expected History command");
    }
}
