use super::super::*;
use super::parse;
use clap::Parser;

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
fn parse_repo_add_command() {
    let cli = parse(&["atlas", "repo", "add", "../sibling"]);
    match cli.command {
        Command::Repo { subcommand } => match subcommand {
            RepoCommand::Add { path } => assert_eq!(path, "../sibling"),
            _ => panic!("expected repo add command"),
        },
        _ => panic!("expected repo command"),
    }
}
#[test]
fn parse_repo_remove_command() {
    let cli = parse(&["atlas", "repo", "remove", "repo_abc"]);
    match cli.command {
        Command::Repo { subcommand } => match subcommand {
            RepoCommand::Remove { repo_id } => assert_eq!(repo_id, "repo_abc"),
            _ => panic!("expected repo remove command"),
        },
        _ => panic!("expected repo command"),
    }
}
#[test]
fn parse_repo_sync_command() {
    let cli = parse(&["atlas", "repo", "sync"]);
    assert!(matches!(
        cli.command,
        Command::Repo {
            subcommand: RepoCommand::Sync
        }
    ));
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
            dry_run: false,
            ..
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
            dry_run: false,
            ..
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
            dry_run: true,
            ..
        }
    ));
}
#[test]
fn parse_build_all_repos() {
    let cli = parse(&["atlas", "build", "--all-repos"]);
    assert!(matches!(
        cli.command,
        Command::Build {
            all_repos: true,
            repo_id: None,
            ..
        }
    ));
}
#[test]
fn parse_build_repo_id() {
    let cli = parse(&["atlas", "build", "--repo-id", "repo_abc"]);
    assert!(matches!(
        cli.command,
        Command::Build {
            repo_id: Some(ref repo_id),
            all_repos: false,
            ..
        } if repo_id == "repo_abc"
    ));
}
#[test]
fn parse_serve_command() {
    let cli = parse(&["atlas", "serve"]);
    assert!(matches!(
        cli.command,
        Command::Serve {
            direct_stdio: false
        }
    ));
}
#[test]
fn parse_serve_direct_stdio_command() {
    let cli = parse(&["atlas", "serve", "--direct-stdio"]);
    assert!(matches!(cli.command, Command::Serve { direct_stdio: true }));
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
#[test]
fn unknown_subcommand_fails() {
    assert!(Cli::try_parse_from(["atlas", "foobar"]).is_err());
}
#[test]
fn query_missing_text_arg_fails() {
    assert!(Cli::try_parse_from(["atlas", "query"]).is_err());
}
