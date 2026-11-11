use std::fs;
use std::path::Path;

use anyhow::Result;

use super::hook_runner::ATLAS_HOOK_RUNNER_REL;
use super::instructions::{AGENTS_FILE, INSTRUCTIONS_END_MARKER, INSTRUCTIONS_MARKER};
use super::{InstallValidation, git_hooks};

pub(super) fn validate_install(
    repo_root: &Path,
    no_hooks: bool,
    no_instructions: bool,
) -> Result<Vec<InstallValidation>> {
    let mut checks = Vec::new();

    if !no_hooks {
        let runner = repo_root.join(ATLAS_HOOK_RUNNER_REL);
        checks.push(validate_runner(&runner));

        if repo_root.join(".git").is_dir() {
            for hook_name in [
                "pre-commit",
                "post-commit",
                "post-checkout",
                "post-merge",
                "post-rewrite",
            ] {
                let hook_path = repo_root.join(".git").join("hooks").join(hook_name);
                checks.push(validate_git_hook(&hook_path, hook_name));
            }
        }
    }

    if !no_instructions {
        checks.push(validate_instruction_file(
            &repo_root.join(AGENTS_FILE),
            AGENTS_FILE,
        ));
    }

    Ok(checks)
}

fn validate_runner(path: &Path) -> InstallValidation {
    match fs::read_to_string(path) {
        Ok(text) => InstallValidation {
            check: "atlas_hook_runner".to_owned(),
            ok: text.contains("atlas hook \"$ATLAS_EVENT\""),
            detail: ATLAS_HOOK_RUNNER_REL.to_owned(),
        },
        Err(_) => InstallValidation {
            check: "atlas_hook_runner".to_owned(),
            ok: false,
            detail: format!("{ATLAS_HOOK_RUNNER_REL} (missing)"),
        },
    }
}

fn validate_git_hook(path: &Path, hook_name: &str) -> InstallValidation {
    match fs::read_to_string(path) {
        Ok(text) => InstallValidation {
            check: format!("git_hook_{hook_name}"),
            ok: text.contains(git_hooks::HOOK_START_MARKER)
                && text.contains(git_hooks::HOOK_END_MARKER),
            detail: path.display().to_string(),
        },
        Err(_) => InstallValidation {
            check: format!("git_hook_{hook_name}"),
            ok: false,
            detail: format!("{} (missing)", path.display()),
        },
    }
}

fn validate_instruction_file(path: &Path, filename: &str) -> InstallValidation {
    match fs::read_to_string(path) {
        Ok(text) => InstallValidation {
            check: format!("instruction_{filename}"),
            ok: text.contains(INSTRUCTIONS_MARKER) && text.contains(INSTRUCTIONS_END_MARKER),
            detail: filename.to_owned(),
        },
        Err(_) => InstallValidation {
            check: format!("instruction_{filename}"),
            ok: false,
            detail: format!("{filename} (missing)"),
        },
    }
}
