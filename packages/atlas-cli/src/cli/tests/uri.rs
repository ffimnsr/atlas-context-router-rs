use super::*;

#[test]
fn parse_uri_commands() {
    let cli = parse(&[
        "atlas",
        "uri",
        "read",
        "atlas://repo/r/file/src%2Flib.rs",
        "--level",
        "l2",
    ]);
    assert!(matches!(
        cli.command,
        Command::Uri {
            subcommand: UriCommand::Read {
                ref uri,
                ref level
            }
        } if uri == "atlas://repo/r/file/src%2Flib.rs" && level.as_deref() == Some("l2")
    ));

    let cli = parse(&["atlas", "uri", "list", "atlas://repo/r/tree/src"]);
    assert!(matches!(
        cli.command,
        Command::Uri {
            subcommand: UriCommand::List { ref uri }
        } if uri == "atlas://repo/r/tree/src"
    ));

    let cli = parse(&[
        "atlas",
        "uri",
        "grep",
        "atlas://repo/r/file/src%2Flib.rs",
        "token",
        "--regex",
    ]);
    assert!(matches!(
        cli.command,
        Command::Uri {
            subcommand: UriCommand::Grep {
                ref pattern,
                regex: true,
                ..
            }
        } if pattern == "token"
    ));

    let cli = parse(&[
        "atlas",
        "uri",
        "search",
        "atlas://repo/r/tree/src",
        "parser",
    ]);
    assert!(matches!(
        cli.command,
        Command::Uri {
            subcommand: UriCommand::Search { ref query, .. }
        } if query == "parser"
    ));
}

#[test]
fn parse_experience_commands() {
    let cli = parse(&[
        "atlas",
        "experience",
        "inspect",
        "atlas://repo/r/file/src%2Flib.rs",
        "--direction",
        "outgoing",
        "--limit",
        "5",
    ]);
    assert!(matches!(
        cli.command,
        Command::Experience {
            uri: _,
            subcommand: Some(ExperienceCommand::Inspect {
                ref uri,
                ref direction,
                limit: 5
            })
        } if uri == "atlas://repo/r/file/src%2Flib.rs" && direction == "outgoing"
    ));

    let cli = parse(&[
        "atlas",
        "experience",
        "prune",
        "--older-than",
        "7",
        "--dry-run",
    ]);
    assert!(matches!(
        cli.command,
        Command::Experience {
            uri: _,
            subcommand: Some(ExperienceCommand::Prune {
                older_than: 7,
                dry_run: true
            })
        }
    ));
}
