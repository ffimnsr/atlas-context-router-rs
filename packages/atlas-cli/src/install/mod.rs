use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::Serialize;

mod git_hooks;
mod hook_runner;
mod init_modes;
mod instructions;
mod validation;

#[cfg(test)]
mod tests;

pub use git_hooks::install_git_hooks;
pub use hook_runner::install_hook_runner;
pub use init_modes::{apply_init_mode, plan_init_mode};
pub use instructions::{AGENTS_FILE, GENERIC_MCP_SETUP, InstructionsMode, inject_instructions};

use validation::validate_install;

#[derive(Debug, Default)]
pub struct InstallSummary {
    pub hook_paths: Vec<PathBuf>,
    pub hook_runner_files: Vec<String>,
    pub instruction_files: Vec<String>,
    pub validation_checks: Vec<InstallValidation>,
    pub validate_only: bool,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct InstallOptions {
    pub dry_run: bool,
    pub validate_only: bool,
    pub force: bool,
    pub no_hooks: bool,
    pub no_instructions: bool,
    pub instructions_mode: InstructionsMode,
}

#[derive(Debug, Clone, Serialize)]
pub struct InstallValidation {
    pub check: String,
    pub ok: bool,
    pub detail: String,
}

/// Install tool-agnostic Atlas integration for a repository.
///
/// Writes only generic assets: a managed `AGENTS.md` instruction block, git
/// hooks, and the generic `.atlas/hooks/atlas-hook` runner. Per-tool MCP and
/// hook configuration is intentionally left to the user; [`GENERIC_MCP_SETUP`]
/// documents how to register the Atlas MCP server with any client.
pub fn run_install(repo_root: &Path, options: InstallOptions) -> Result<InstallSummary> {
    let mut summary = InstallSummary {
        validate_only: options.validate_only,
        ..Default::default()
    };

    if !options.no_hooks && !options.validate_only {
        summary.hook_paths = install_git_hooks(repo_root, options.dry_run, options.force)?;
        if let Some(path) = install_hook_runner(repo_root, options.dry_run)? {
            summary.hook_runner_files.push(path);
        }
    }

    if !options.no_instructions && !options.validate_only {
        summary.instruction_files = inject_instructions(
            repo_root,
            &[AGENTS_FILE],
            options.dry_run,
            options.instructions_mode,
        )?;
    }

    if !options.dry_run || options.validate_only {
        summary.validation_checks =
            validate_install(repo_root, options.no_hooks, options.no_instructions)?;
    }

    Ok(summary)
}
