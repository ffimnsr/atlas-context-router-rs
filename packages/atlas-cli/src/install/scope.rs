use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::InstallScope;

pub(super) fn should_auto_detect(platform: &str, repo_root: &Path) -> bool {
    match platform {
        "copilot" => {
            repo_root.join(".vscode").is_dir()
                || std::env::var("VSCODE_PID").is_ok()
                || std::env::var("TERM_PROGRAM")
                    .map(|value| value == "vscode")
                    .unwrap_or(false)
        }
        "claude" => true,
        "codex" => repo_root.join(".codex").is_dir(),
        _ => false,
    }
}

pub(super) fn scope_root(repo_root: &Path, scope: InstallScope) -> Result<PathBuf> {
    match scope {
        InstallScope::Repo => Ok(repo_root.to_path_buf()),
        InstallScope::User => user_home_dir(),
    }
}

fn user_home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .or_else(|_| {
            #[allow(deprecated)]
            std::env::home_dir().ok_or(std::env::VarError::NotPresent)
        })
        .context("cannot determine home directory")
}

pub(super) fn display_scoped_path(
    path: &Path,
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
) -> String {
    let normalized = match scope {
        InstallScope::Repo => path
            .strip_prefix(repo_root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned(),
        InstallScope::User => path
            .strip_prefix(scope_root)
            .map(|rest| format!("~/{}", rest.to_string_lossy()))
            .unwrap_or_else(|_| path.display().to_string()),
    };
    normalized.replace('\\', "/")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub(super) fn atlas_hook_command(
    install_root: &Path,
    scope: InstallScope,
    frontend: &str,
    arg: &str,
) -> String {
    match scope {
        InstallScope::Repo => format!(".atlas/hooks/atlas-hook {frontend} {arg}"),
        InstallScope::User => {
            let runner = install_root.join(".atlas").join("hooks").join("atlas-hook");
            format!(
                "{} {frontend} {arg}",
                shell_quote(&runner.display().to_string())
            )
        }
    }
}
