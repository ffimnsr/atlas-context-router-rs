use super::super::*;
use super::parse;
use clap::Parser;

#[test]
fn parse_memory_store_command() {
    let cli = parse(&[
        "atlas",
        "memory",
        "store",
        "remember hooks",
        "--topic",
        "hooks",
        "--title",
        "Hook notes",
        "--importance",
        "critical",
        "--scope",
        "frontend",
        "--frontend",
        "codex",
        "--source-id",
        "artifact-1",
    ]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand:
                MemoryCommand::Store {
                    ref text,
                    ref topic,
                    ref title,
                    ref importance,
                    ref scope,
                    ref frontend,
                    ref source_id,
                }
        } if text == "remember hooks"
            && topic.as_deref() == Some("hooks")
            && title.as_deref() == Some("Hook notes")
            && importance.as_deref() == Some("critical")
            && scope.as_deref() == Some("frontend")
            && frontend.as_deref() == Some("codex")
            && source_id.as_deref() == Some("artifact-1")
    ));
}
#[test]
fn parse_memory_recall_command() {
    let cli = parse(&[
        "atlas",
        "memory",
        "recall",
        "deploy",
        "--topic",
        "hooks",
        "--importance",
        "high",
        "--scope",
        "project",
        "--limit",
        "5",
    ]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand:
                MemoryCommand::Recall {
                    ref query,
                    ref topic,
                    ref importance,
                    ref scope,
                    shared: false,
                    limit: 5,
                }
        } if query == "deploy"
            && topic.as_deref() == Some("hooks")
            && importance.as_deref() == Some("high")
            && scope.as_deref() == Some("project")
    ));
}
#[test]
fn parse_memory_recall_shared_defaults() {
    let cli = parse(&["atlas", "memory", "recall", "x"]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand: MemoryCommand::Recall {
                shared: false,
                limit: 20,
                ..
            }
        }
    ));
}
#[test]
fn parse_memory_recall_shared_conflicts_with_scope() {
    let error = Cli::try_parse_from([
        "atlas", "memory", "recall", "x", "--shared", "--scope", "frontend",
    ])
    .expect_err("--shared and --scope must conflict");
    assert!(error.to_string().contains("--shared"), "got: {error}");
}
#[test]
fn parse_memory_list_command() {
    let cli = parse(&[
        "atlas",
        "memory",
        "list",
        "--topic",
        "hooks",
        "--importance",
        "low",
        "--scope",
        "global",
        "--older-than",
        "2026-02-01",
        "--newer-than",
        "2026-01-01T00:00:00Z",
    ]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand:
                MemoryCommand::List {
                    ref topic,
                    ref importance,
                    ref scope,
                    ref older_than,
                    ref newer_than,
                }
        } if topic.as_deref() == Some("hooks")
            && importance.as_deref() == Some("low")
            && scope.as_deref() == Some("global")
            && older_than.as_deref() == Some("2026-02-01")
            && newer_than.as_deref() == Some("2026-01-01T00:00:00Z")
    ));
}
#[test]
fn parse_memory_delete_command() {
    let cli = parse(&["atlas", "memory", "delete", "abc123", "--dry-run"]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand: MemoryCommand::Delete {
                ref memory_id,
                dry_run: true,
            }
        } if memory_id == "abc123"
    ));
}
