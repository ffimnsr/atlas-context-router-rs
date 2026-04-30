use super::*;
use clap::Parser;

fn parse(args: &[&str]) -> Cli {
    Cli::try_parse_from(args).expect("parse should succeed")
}

// -------------------------------------------------------------------------
// Global flags
// -------------------------------------------------------------------------

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

// -------------------------------------------------------------------------
// init / build / serve / db-check
// -------------------------------------------------------------------------

#[test]
fn parse_init_command() {
    let cli = parse(&["atlas", "init"]);
    assert!(matches!(
        cli.command,
        Command::Init { ref profile } if profile == "standard"
    ));
}

#[test]
fn parse_init_full_profile() {
    let cli = parse(&["atlas", "init", "--profile", "full"]);
    assert!(matches!(
        cli.command,
        Command::Init { ref profile } if profile == "full"
    ));
}

#[test]
fn parse_migrate_command() {
    let cli = parse(&["atlas", "migrate"]);
    assert!(matches!(cli.command, Command::Migrate));
}

#[test]
fn parse_debug_config_command() {
    let cli = parse(&["atlas", "debug-config"]);
    assert!(matches!(cli.command, Command::DebugConfig));
}

#[test]
fn parse_config_show_command() {
    let cli = parse(&["atlas", "config", "show"]);
    match cli.command {
        Command::Config { subcommand } => match subcommand {
            ConfigCommand::Show => {}
        },
        _ => panic!("expected Config command"),
    }
}

#[test]
fn parse_selfupdate_command() {
    let cli = parse(&["atlas", "selfupdate"]);
    assert!(matches!(cli.command, Command::Selfupdate));
}

#[test]
fn parse_build_command_no_flags() {
    let cli = parse(&["atlas", "build"]);
    assert!(matches!(
        cli.command,
        Command::Build {
            fail_fast: false,
            dry_run: false
        }
    ));
}

#[test]
fn parse_build_fail_fast() {
    let cli = parse(&["atlas", "build", "--fail-fast"]);
    assert!(matches!(
        cli.command,
        Command::Build {
            fail_fast: true,
            dry_run: false
        }
    ));
}

#[test]
fn parse_build_dry_run() {
    let cli = parse(&["atlas", "build", "--dry-run"]);
    assert!(matches!(
        cli.command,
        Command::Build {
            fail_fast: false,
            dry_run: true
        }
    ));
}

#[test]
fn parse_serve_command() {
    let cli = parse(&["atlas", "serve"]);
    assert!(matches!(cli.command, Command::Serve));
}

#[test]
fn parse_db_check_command() {
    let cli = parse(&["atlas", "db-check"]);
    assert!(matches!(cli.command, Command::DbCheck));
}

#[test]
fn parse_doctor_command() {
    let cli = parse(&["atlas", "doctor"]);
    assert!(matches!(cli.command, Command::Doctor));
}

#[test]
fn parse_purge_noncanonical_command() {
    let cli = parse(&["atlas", "purge-noncanonical"]);
    assert!(matches!(cli.command, Command::PurgeNoncanonical));
}

// -------------------------------------------------------------------------
// update
// -------------------------------------------------------------------------

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

// -------------------------------------------------------------------------
// query
// -------------------------------------------------------------------------

#[test]
fn parse_query_text_only() {
    let cli = parse(&["atlas", "query", "ReplaceFileGraph"]);
    if let Command::Query {
        text,
        kind,
        language,
        include_files,
        limit,
        ..
    } = cli.command
    {
        assert_eq!(text, "ReplaceFileGraph");
        assert!(kind.is_none());
        assert!(language.is_none());
        assert!(!include_files);
        assert_eq!(limit, 20);
    } else {
        panic!("expected Query command");
    }
}

#[test]
fn parse_docs_section_heading_selector() {
    let cli = parse(&[
        "atlas",
        "docs-section",
        "README.md",
        "--heading",
        "document.overview.install",
        "--max-bytes",
        "2048",
    ]);
    if let Command::DocsSection {
        path,
        heading,
        line,
        max_bytes,
    } = cli.command
    {
        assert_eq!(path, "README.md");
        assert_eq!(heading.as_deref(), Some("document.overview.install"));
        assert_eq!(line, None);
        assert_eq!(max_bytes, 2048);
    } else {
        panic!("expected DocsSection command");
    }
}

#[test]
fn parse_docs_section_line_selector() {
    let cli = parse(&["atlas", "docs-section", "README.md", "--line", "7"]);
    if let Command::DocsSection {
        path,
        heading,
        line,
        max_bytes,
    } = cli.command
    {
        assert_eq!(path, "README.md");
        assert_eq!(heading, None);
        assert_eq!(line, Some(7));
        assert_eq!(max_bytes, 16_384);
    } else {
        panic!("expected DocsSection command");
    }
}

#[test]
fn parse_query_with_kind_and_language_filters() {
    let cli = parse(&[
        "atlas",
        "query",
        "foo",
        "--kind",
        "function",
        "--language",
        "rust",
    ]);
    if let Command::Query {
        text,
        kind,
        language,
        ..
    } = cli.command
    {
        assert_eq!(text, "foo");
        assert_eq!(kind.as_deref(), Some("function"));
        assert_eq!(language.as_deref(), Some("rust"));
    } else {
        panic!("expected Query command");
    }
}

#[test]
fn parse_query_expand_flag() {
    let cli = parse(&["atlas", "query", "foo", "--expand", "--expand-hops", "3"]);
    if let Command::Query {
        expand,
        expand_hops,
        fuzzy,
        ..
    } = cli.command
    {
        assert!(expand);
        assert_eq!(expand_hops, 3);
        assert!(!fuzzy);
    } else {
        panic!("expected Query command");
    }
}

#[test]
fn parse_query_fuzzy_flag() {
    let cli = parse(&["atlas", "query", "greter", "--fuzzy"]);
    if let Command::Query { fuzzy, .. } = cli.command {
        assert!(fuzzy);
    } else {
        panic!("expected Query command");
    }
}

#[test]
fn parse_query_include_files_flag() {
    let cli = parse(&["atlas", "query", "guide", "--include-files"]);
    if let Command::Query { include_files, .. } = cli.command {
        assert!(include_files);
    } else {
        panic!("expected Query command");
    }
}

// -------------------------------------------------------------------------
// impact
// -------------------------------------------------------------------------

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

// -------------------------------------------------------------------------
// review-context
// -------------------------------------------------------------------------

#[test]
fn parse_review_context_defaults() {
    let cli = parse(&["atlas", "review-context"]);
    if let Command::ReviewContext {
        max_depth,
        max_nodes,
        base,
        files,
        format,
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

// -------------------------------------------------------------------------
// detect-changes
// -------------------------------------------------------------------------

#[test]
fn parse_detect_changes_with_base() {
    let cli = parse(&["atlas", "detect-changes", "--base", "origin/main"]);
    if let Command::DetectChanges { base, staged } = cli.command {
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
fn parse_explain_change_with_base() {
    let cli = parse(&["atlas", "explain-change", "--base", "origin/main"]);
    if let Command::ExplainChange {
        base,
        staged,
        files,
        max_depth,
        max_nodes,
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
            } => {
                assert_eq!(branch.as_deref(), Some("main"));
                assert!(repair);
                assert_eq!(max_commits, Some(25));
            }
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

// -------------------------------------------------------------------------
// unknown / missing required args
// -------------------------------------------------------------------------

#[test]
fn unknown_subcommand_fails() {
    assert!(Cli::try_parse_from(["atlas", "foobar"]).is_err());
}

#[test]
fn query_missing_text_arg_fails() {
    assert!(Cli::try_parse_from(["atlas", "query"]).is_err());
}

// -------------------------------------------------------------------------
// install
// -------------------------------------------------------------------------

#[test]
fn parse_install_defaults() {
    let cli = parse(&["atlas", "install"]);
    if let Command::Install {
        platform,
        scope,
        dry_run,
        validate_only,
        force,
        no_hooks,
        no_instructions,
    } = cli.command
    {
        assert_eq!(platform, "all");
        assert_eq!(scope, "repo");
        assert!(!dry_run);
        assert!(!validate_only);
        assert!(!force);
        assert!(!no_hooks);
        assert!(!no_instructions);
    } else {
        panic!("expected Install command");
    }
}

#[test]
fn parse_install_platform_claude() {
    let cli = parse(&["atlas", "install", "--platform", "claude"]);
    if let Command::Install { platform, .. } = cli.command {
        assert_eq!(platform, "claude");
    } else {
        panic!("expected Install command");
    }
}

#[test]
fn parse_install_dry_run() {
    let cli = parse(&["atlas", "install", "--dry-run"]);
    if let Command::Install { dry_run, .. } = cli.command {
        assert!(dry_run);
    } else {
        panic!("expected Install command");
    }
}

#[test]
fn parse_install_scope_user() {
    let cli = parse(&["atlas", "install", "--scope", "user"]);
    if let Command::Install { scope, .. } = cli.command {
        assert_eq!(scope, "user");
    } else {
        panic!("expected Install command");
    }
}

#[test]
fn parse_install_validate_only() {
    let cli = parse(&["atlas", "install", "--validate-only"]);
    if let Command::Install { validate_only, .. } = cli.command {
        assert!(validate_only);
    } else {
        panic!("expected Install command");
    }
}

#[test]
fn parse_install_force() {
    let cli = parse(&["atlas", "install", "--force"]);
    if let Command::Install { force, .. } = cli.command {
        assert!(force);
    } else {
        panic!("expected Install command");
    }
}

#[test]
fn parse_install_no_hooks_and_no_instructions() {
    let cli = parse(&["atlas", "install", "--no-hooks", "--no-instructions"]);
    if let Command::Install {
        no_hooks,
        no_instructions,
        ..
    } = cli.command
    {
        assert!(no_hooks);
        assert!(no_instructions);
    } else {
        panic!("expected Install command");
    }
}

// -------------------------------------------------------------------------
// completions
// -------------------------------------------------------------------------

#[test]
fn parse_completions_bash() {
    let cli = parse(&["atlas", "completions", "bash"]);
    assert!(matches!(
        cli.command,
        Command::Completions {
            shell: clap_complete::Shell::Bash
        }
    ));
}

#[test]
fn parse_completions_zsh() {
    let cli = parse(&["atlas", "completions", "zsh"]);
    assert!(matches!(
        cli.command,
        Command::Completions {
            shell: clap_complete::Shell::Zsh
        }
    ));
}

#[test]
fn completions_missing_shell_fails() {
    assert!(Cli::try_parse_from(["atlas", "completions"]).is_err());
}

#[test]
fn parse_shell_with_flags() {
    let cli = parse(&["atlas", "shell", "--fuzzy", "--paging"]);
    assert!(matches!(
        cli.command,
        Command::Shell {
            fuzzy: true,
            paging: true
        }
    ));
}

#[test]
fn parse_analyze_dead_code_with_subpath() {
    let cli = parse(&["atlas", "analyze", "dead-code", "--subpath", "src"]);
    if let Command::Analyze { subcommand } = cli.command {
        match subcommand {
            AnalyzeCommand::DeadCode { subpath, limit, .. } => {
                assert_eq!(subpath.as_deref(), Some("src"));
                assert_eq!(limit, 100);
            }
            _ => panic!("expected analyze dead-code"),
        }
    } else {
        panic!("expected Analyze command");
    }
}

#[test]
fn parse_refactor_rename_with_named_flags() {
    let cli = parse(&[
        "atlas",
        "refactor",
        "rename",
        "--symbol",
        "src/lib.rs::fn::helper",
        "--to",
        "helper_renamed",
        "--dry-run",
    ]);
    if let Command::Refactor { subcommand } = cli.command {
        match subcommand {
            RefactorCommand::Rename {
                symbol,
                to,
                dry_run,
                legacy_symbol,
                legacy_to,
            } => {
                assert_eq!(symbol.as_deref(), Some("src/lib.rs::fn::helper"));
                assert_eq!(to.as_deref(), Some("helper_renamed"));
                assert!(dry_run);
                assert!(legacy_symbol.is_none());
                assert!(legacy_to.is_none());
            }
            _ => panic!("expected refactor rename"),
        }
    } else {
        panic!("expected Refactor command");
    }
}

#[test]
fn parse_refactor_rename_legacy_positionals() {
    let cli = parse(&[
        "atlas",
        "refactor",
        "rename",
        "src/lib.rs::fn::helper",
        "helper_renamed",
        "--dry-run",
    ]);
    if let Command::Refactor { subcommand } = cli.command {
        match subcommand {
            RefactorCommand::Rename {
                symbol,
                to,
                dry_run,
                legacy_symbol,
                legacy_to,
            } => {
                assert!(symbol.is_none());
                assert!(to.is_none());
                assert!(dry_run);
                assert_eq!(legacy_symbol.as_deref(), Some("src/lib.rs::fn::helper"));
                assert_eq!(legacy_to.as_deref(), Some("helper_renamed"));
            }
            _ => panic!("expected refactor rename"),
        }
    } else {
        panic!("expected Refactor command");
    }
}

#[test]
fn parse_hidden_hook_command() {
    let cli = parse(&["atlas", "hook", "session-start"]);
    match cli.command {
        Command::Hook { event } => assert_eq!(event, "session-start"),
        _ => panic!("expected Hook command"),
    }
}
