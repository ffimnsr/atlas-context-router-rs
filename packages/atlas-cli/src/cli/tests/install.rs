use super::super::*;
use super::parse;
use crate::cli::InstallMode;
use crate::install::InstructionsMode;
use clap::Parser;

#[test]
fn parse_install_defaults() {
    let cli = parse(&["atlas", "install"]);
    if let Command::Install {
        platform,
        scope,
        dry_run,
        validate_only,
        force,
        instructions_only,
        no_platform_config,
        no_hooks,
        no_instructions,
        instructions_mode,
        mode,
    } = cli.command
    {
        assert_eq!(platform, "all");
        assert_eq!(scope, "repo");
        assert!(!dry_run);
        assert!(!validate_only);
        assert!(!force);
        assert!(!instructions_only);
        assert!(!no_platform_config);
        assert!(!no_hooks);
        assert!(!no_instructions);
        assert_eq!(instructions_mode, InstructionsMode::Refresh);
        assert_eq!(mode, InstallMode::All);
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
#[test]
fn parse_install_instructions_only() {
    let cli = parse(&["atlas", "install", "--instructions-only"]);
    if let Command::Install {
        instructions_only, ..
    } = cli.command
    {
        assert!(instructions_only);
    } else {
        panic!("expected Install command");
    }
}
#[test]
fn parse_install_no_platform_config() {
    let cli = parse(&["atlas", "install", "--no-platform-config"]);
    if let Command::Install {
        no_platform_config, ..
    } = cli.command
    {
        assert!(no_platform_config);
    } else {
        panic!("expected Install command");
    }
}
#[test]
fn parse_install_mode_all_is_default() {
    let cli = parse(&["atlas", "install"]);
    if let Command::Install { mode, .. } = cli.command {
        assert_eq!(mode, InstallMode::All);
    } else {
        panic!("expected Install command");
    }
}
#[test]
fn parse_install_mode_mcp() {
    let cli = parse(&["atlas", "install", "--mode", "mcp"]);
    if let Command::Install { mode, .. } = cli.command {
        assert_eq!(mode, InstallMode::Mcp);
    } else {
        panic!("expected Install command");
    }
}
#[test]
fn parse_install_mode_hook() {
    let cli = parse(&["atlas", "install", "--mode", "hook"]);
    if let Command::Install { mode, .. } = cli.command {
        assert_eq!(mode, InstallMode::Hook);
    } else {
        panic!("expected Install command");
    }
}
#[test]
fn parse_install_mode_cli() {
    let cli = parse(&["atlas", "install", "--mode", "cli"]);
    if let Command::Install { mode, .. } = cli.command {
        assert_eq!(mode, InstallMode::Cli);
    } else {
        panic!("expected Install command");
    }
}
#[test]
fn parse_install_rejects_unknown_mode() {
    assert!(Cli::try_parse_from(["atlas", "install", "--mode", "hooks"]).is_err());
}
#[test]
fn parse_install_replace_file_instructions_mode() {
    let cli = parse(&["atlas", "install", "--instructions-mode", "replace-file"]);
    if let Command::Install {
        instructions_mode, ..
    } = cli.command
    {
        assert_eq!(instructions_mode, InstructionsMode::ReplaceFile);
    } else {
        panic!("expected Install command");
    }
}
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
