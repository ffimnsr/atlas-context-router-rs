use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

mod git_hooks;
mod instructions;
mod mcp;
mod platform_hooks;
mod scope;
mod validation;

#[cfg(test)]
mod tests;

pub use git_hooks::install_git_hooks;
pub use instructions::inject_instructions;
pub use platform_hooks::install_platform_agent_hooks;

use instructions::instruction_targets;
use mcp::{install_claude_scoped, install_codex_scoped, install_copilot_scoped};
use platform_hooks::install_platform_agent_hooks_scoped;
use scope::{scope_root, should_auto_detect};
use validation::validate_install;

#[derive(Debug, Default)]
pub struct InstallSummary {
    pub configured: Vec<String>,
    pub already_configured: Vec<String>,
    pub hook_paths: Vec<PathBuf>,
    pub instruction_files: Vec<String>,
    pub platform_hook_files: Vec<String>,
    pub validation_checks: Vec<InstallValidation>,
    pub scope: String,
    pub validate_only: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct InstallOptions {
    pub dry_run: bool,
    pub validate_only: bool,
    pub force: bool,
    pub no_hooks: bool,
    pub no_instructions: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallValidation {
    pub check: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstallScope {
    Repo,
    User,
}

impl InstallScope {
    fn parse(scope: &str) -> Self {
        match scope {
            "user" => Self::User,
            _ => Self::Repo,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Repo => "repo",
            Self::User => "user",
        }
    }
}

enum PlatformResult {
    Configured(String),
    AlreadyConfigured(String),
}

pub fn run_install(
    repo_root: &Path,
    platform: &str,
    scope: &str,
    options: InstallOptions,
) -> Result<InstallSummary> {
    let mut summary = InstallSummary::default();
    let scope = InstallScope::parse(scope);
    let scope_root = scope_root(repo_root, scope)?;
    summary.scope = scope.as_str().to_owned();
    summary.validate_only = options.validate_only;

    for name in ["copilot", "claude", "codex"] {
        let skip = match platform {
            "all" => !should_auto_detect(name, repo_root),
            p => p != name,
        };
        if skip {
            continue;
        }

        if options.validate_only {
            continue;
        }

        let result = match name {
            "copilot" => install_copilot_scoped(repo_root, &scope_root, scope, options.dry_run),
            "claude" => install_claude_scoped(repo_root, &scope_root, scope, options.dry_run),
            "codex" => install_codex_scoped(repo_root, &scope_root, scope, options.dry_run),
            _ => unreachable!(),
        }?;

        match result {
            PlatformResult::Configured(label) => summary.configured.push(label),
            PlatformResult::AlreadyConfigured(label) => summary.already_configured.push(label),
        }
    }

    if !options.no_hooks {
        summary.hook_paths = install_git_hooks(repo_root, options.dry_run, options.force)?;
        if !options.validate_only {
            summary.platform_hook_files = install_platform_agent_hooks_scoped(
                repo_root,
                &scope_root,
                platform,
                scope,
                options.dry_run,
            )?;
        }
    }

    if !options.no_instructions && !options.validate_only {
        summary.instruction_files = inject_instructions(
            repo_root,
            &instruction_targets(platform, repo_root),
            options.dry_run,
        )?;
    }

    if !options.dry_run || options.validate_only {
        summary.validation_checks = validate_install(
            repo_root,
            &scope_root,
            platform,
            scope,
            options.no_hooks,
            options.no_instructions,
        )?;
    }

    Ok(summary)
}
