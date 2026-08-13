use super::super::*;
use super::parse;
use clap::Parser;

// ── ICM-C2 — feedback command parsing ─────────────────────────────────────────

#[test]
fn parse_feedback_record_command() {
    let cli = parse(&[
        "atlas",
        "feedback",
        "record",
        "--predicted",
        "dead code",
        "--actual",
        "used by macro",
        "--correction",
        "keep it",
        "--tool",
        "cli",
        "--analysis-kind",
        "dead_code",
        "--symbol",
        "src/lib.rs::fn::legacy",
        "--file",
        "src/lib.rs",
        "--source-id",
        "artifact-1",
    ]);
    assert!(matches!(
        cli.command,
        Command::Feedback {
            subcommand:
                FeedbackCommand::Record {
                    ref predicted,
                    ref actual,
                    ref correction,
                    ref tool,
                    ref analysis_kind,
                    ref symbol,
                    ref file,
                    ref source_id,
                }
        } if predicted == "dead code"
            && actual == "used by macro"
            && correction.as_deref() == Some("keep it")
            && tool.as_deref() == Some("cli")
            && analysis_kind.as_deref() == Some("dead_code")
            && symbol.as_deref() == Some("src/lib.rs::fn::legacy")
            && file.as_deref() == Some("src/lib.rs")
            && source_id.as_deref() == Some("artifact-1")
    ));
}

#[test]
fn parse_feedback_record_requires_predicted_and_actual() {
    let missing_predicted =
        Cli::try_parse_from(["atlas", "feedback", "record", "--actual", "alive"])
            .expect_err("--predicted must be required");
    assert!(missing_predicted.to_string().contains("--predicted"));

    let missing_actual =
        Cli::try_parse_from(["atlas", "feedback", "record", "--predicted", "dead"])
            .expect_err("--actual must be required");
    assert!(missing_actual.to_string().contains("--actual"));
}

#[test]
fn parse_feedback_search_command() {
    let cli = parse(&[
        "atlas",
        "feedback",
        "search",
        "macro registry",
        "--tool",
        "cli",
        "--analysis-kind",
        "dead_code",
        "--symbol",
        "src/lib.rs::fn::legacy",
        "--file",
        "src/lib.rs",
        "--limit",
        "5",
    ]);
    assert!(matches!(
        cli.command,
        Command::Feedback {
            subcommand:
                FeedbackCommand::Search {
                    ref query,
                    ref tool,
                    ref analysis_kind,
                    ref symbol,
                    ref file,
                    limit: 5,
                }
        } if query == "macro registry"
            && tool.as_deref() == Some("cli")
            && analysis_kind.as_deref() == Some("dead_code")
            && symbol.as_deref() == Some("src/lib.rs::fn::legacy")
            && file.as_deref() == Some("src/lib.rs")
    ));
}

#[test]
fn parse_feedback_search_defaults() {
    let cli = parse(&["atlas", "feedback", "search", "x"]);
    assert!(matches!(
        cli.command,
        Command::Feedback {
            subcommand: FeedbackCommand::Search { limit: 20, .. }
        }
    ));
}

#[test]
fn parse_feedback_stats_command() {
    let cli = parse(&["atlas", "feedback", "stats"]);
    assert!(matches!(
        cli.command,
        Command::Feedback {
            subcommand: FeedbackCommand::Stats
        }
    ));
}

// ── ICM-D2 — wake-up command parsing ──────────────────────────────────────────

#[test]
fn parse_wake_up_command() {
    let cli = parse(&[
        "atlas",
        "wake-up",
        "--topic",
        "hooks",
        "--session",
        "sess-1",
        "--frontend",
        "zed",
        "--max-items",
        "5",
    ]);
    assert!(matches!(
        cli.command,
        Command::WakeUp {
            ref topic,
            ref session,
            ref frontend,
            max_items: Some(5),
        } if topic.as_deref() == Some("hooks")
            && session.as_deref() == Some("sess-1")
            && frontend.as_deref() == Some("zed")
    ));
}

#[test]
fn parse_wake_up_defaults() {
    let cli = parse(&["atlas", "wake-up"]);
    assert!(matches!(
        cli.command,
        Command::WakeUp {
            topic: None,
            session: None,
            frontend: None,
            max_items: None,
        }
    ));
}

// ── ICM-B — decay, stale, prune, health, consolidate parsing ──────────────────
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

// ── ICM-B — decay, stale, prune, health, consolidate parsing ──────────────────

#[test]
fn parse_memory_decay_command() {
    let cli = parse(&["atlas", "memory", "decay", "--topic", "hooks", "--dry-run"]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand: MemoryCommand::Decay {
                ref topic,
                dry_run: true,
            }
        } if topic.as_deref() == Some("hooks")
    ));
    let cli = parse(&["atlas", "memory", "decay"]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand: MemoryCommand::Decay { dry_run: false, .. }
        }
    ));
}

#[test]
fn parse_memory_stale_command() {
    let cli = parse(&[
        "atlas", "memory", "stale", "--topic", "hooks", "--scope", "project",
    ]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand: MemoryCommand::Stale {
                ref topic,
                ref scope,
            }
        } if topic.as_deref() == Some("hooks") && scope.as_deref() == Some("project")
    ));
}

#[test]
fn parse_memory_prune_command() {
    let cli = parse(&[
        "atlas",
        "memory",
        "prune",
        "--dry-run",
        "--topic",
        "hooks",
        "--importance",
        "low",
        "--older-than",
        "2026-02-01",
        "--allow-critical",
    ]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand:
                MemoryCommand::Prune {
                    dry_run: true,
                    ref topic,
                    ref importance,
                    ref older_than,
                    allow_critical: true,
                }
        } if topic.as_deref() == Some("hooks")
            && importance.as_deref() == Some("low")
            && older_than.as_deref() == Some("2026-02-01")
    ));
}

#[test]
fn parse_memory_prune_defaults() {
    let cli = parse(&["atlas", "memory", "prune"]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand: MemoryCommand::Prune {
                dry_run: false,
                allow_critical: false,
                ..
            }
        }
    ));
}

#[test]
fn parse_memory_health_command() {
    let cli = parse(&["atlas", "memory", "health", "--topic", "hooks"]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand: MemoryCommand::Health {
                ref topic,
                scope: None,
            }
        } if topic.as_deref() == Some("hooks")
    ));
}

#[test]
fn parse_memory_consolidate_command() {
    let cli = parse(&[
        "atlas",
        "memory",
        "consolidate",
        "--topic",
        "hooks",
        "--dry-run",
    ]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand: MemoryCommand::Consolidate {
                ref topic,
                scope: None,
                dry_run: true,
            }
        } if topic.as_deref() == Some("hooks")
    ));
    let cli = parse(&["atlas", "memory", "consolidate"]);
    assert!(matches!(
        cli.command,
        Command::Memory {
            subcommand: MemoryCommand::Consolidate { dry_run: false, .. }
        }
    ));
}
