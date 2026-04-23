use std::fs;
use std::path::Path;

use anyhow::Result;
use serde_json::Value;

use super::instructions::{INSTRUCTIONS_END_MARKER, INSTRUCTIONS_MARKER, instruction_targets};
use super::platform_hooks::ATLAS_HOOK_MARKER;
use super::scope::{display_scoped_path, should_auto_detect};
use super::{InstallScope, InstallValidation};

pub(super) fn validate_install(
    repo_root: &Path,
    scope_root: &Path,
    platform: &str,
    scope: InstallScope,
    no_hooks: bool,
    no_instructions: bool,
) -> Result<Vec<InstallValidation>> {
    let mut checks = Vec::new();

    for name in ["copilot", "claude", "codex"] {
        let skip = match platform {
            "all" => !should_auto_detect(name, repo_root),
            current => current != name,
        };
        if skip {
            continue;
        }

        checks.push(match name {
            "copilot" => validate_json_server_entry(
                "copilot_config",
                &match scope {
                    InstallScope::Repo => scope_root.join(".vscode").join("mcp.json"),
                    InstallScope::User => scope_root
                        .join(".config")
                        .join("Code")
                        .join("User")
                        .join("mcp.json"),
                },
                repo_root,
                scope_root,
                scope,
                "servers",
            ),
            "claude" => validate_json_server_entry(
                "claude_config",
                &scope_root.join(".mcp.json"),
                repo_root,
                scope_root,
                scope,
                "mcpServers",
            ),
            "codex" => validate_toml_section(
                "codex_config",
                &scope_root.join(".codex").join("config.toml"),
                repo_root,
                scope_root,
                scope,
                "[mcp_servers.atlas]",
            ),
            _ => unreachable!(),
        });
    }

    if !no_hooks {
        let runner = scope_root.join(".atlas").join("hooks").join("atlas-hook");
        checks.push(validate_runner(&runner, repo_root, scope_root, scope));

        for name in ["copilot", "claude", "codex"] {
            let skip = match platform {
                "all" => !should_auto_detect(name, repo_root),
                current => current != name,
            };
            if skip {
                continue;
            }

            let hook_path = match (name, scope) {
                ("copilot", InstallScope::Repo) => scope_root
                    .join(".github")
                    .join("hooks")
                    .join("atlas-copilot.json"),
                ("copilot", InstallScope::User) => scope_root
                    .join(".copilot")
                    .join("hooks")
                    .join("atlas-copilot.json"),
                ("claude", _) => scope_root.join(".claude").join("settings.json"),
                ("codex", _) => scope_root.join(".codex").join("hooks.json"),
                _ => unreachable!(),
            };
            checks.push(validate_hook_config(
                name, &hook_path, repo_root, scope_root, scope,
            ));
        }

        if repo_root.join(".git").is_dir() {
            for hook_name in ["pre-commit", "post-checkout", "post-merge", "post-rewrite"] {
                let hook_path = repo_root.join(".git").join("hooks").join(hook_name);
                checks.push(validate_git_hook(&hook_path, hook_name));
            }
        }
    }

    if !no_instructions {
        for filename in instruction_targets(platform, repo_root) {
            let path = repo_root.join(filename);
            checks.push(validate_instruction_file(&path, filename));
        }
    }

    Ok(checks)
}

fn validate_json_server_entry(
    check: &str,
    path: &Path,
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
    top_key: &str,
) -> InstallValidation {
    match fs::read_to_string(path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(root)) => {
                let ok = root
                    .get(top_key)
                    .and_then(Value::as_object)
                    .is_some_and(|servers| servers.contains_key("atlas"));
                InstallValidation {
                    check: check.to_owned(),
                    ok,
                    detail: display_scoped_path(path, repo_root, scope_root, scope),
                }
            }
            _ => InstallValidation {
                check: check.to_owned(),
                ok: false,
                detail: format!(
                    "{} (invalid json)",
                    display_scoped_path(path, repo_root, scope_root, scope)
                ),
            },
        },
        Err(_) => InstallValidation {
            check: check.to_owned(),
            ok: false,
            detail: format!(
                "{} (missing)",
                display_scoped_path(path, repo_root, scope_root, scope)
            ),
        },
    }
}

fn validate_toml_section(
    check: &str,
    path: &Path,
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
    section_header: &str,
) -> InstallValidation {
    match fs::read_to_string(path) {
        Ok(text) => InstallValidation {
            check: check.to_owned(),
            ok: text.contains(section_header),
            detail: display_scoped_path(path, repo_root, scope_root, scope),
        },
        Err(_) => InstallValidation {
            check: check.to_owned(),
            ok: false,
            detail: format!(
                "{} (missing)",
                display_scoped_path(path, repo_root, scope_root, scope)
            ),
        },
    }
}

fn validate_runner(
    path: &Path,
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
) -> InstallValidation {
    match fs::read_to_string(path) {
        Ok(text) => InstallValidation {
            check: "atlas_hook_runner".to_owned(),
            ok: text.contains("atlas hook \"$ATLAS_EVENT\""),
            detail: display_scoped_path(path, repo_root, scope_root, scope),
        },
        Err(_) => InstallValidation {
            check: "atlas_hook_runner".to_owned(),
            ok: false,
            detail: format!(
                "{} (missing)",
                display_scoped_path(path, repo_root, scope_root, scope)
            ),
        },
    }
}

fn validate_hook_config(
    name: &str,
    path: &Path,
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
) -> InstallValidation {
    match fs::read_to_string(path) {
        Ok(text) => {
            let parses = serde_json::from_str::<Value>(&text).is_ok();
            InstallValidation {
                check: format!("{name}_hooks"),
                ok: parses && text.contains(ATLAS_HOOK_MARKER),
                detail: display_scoped_path(path, repo_root, scope_root, scope),
            }
        }
        Err(_) => InstallValidation {
            check: format!("{name}_hooks"),
            ok: false,
            detail: format!(
                "{} (missing)",
                display_scoped_path(path, repo_root, scope_root, scope)
            ),
        },
    }
}

fn validate_git_hook(path: &Path, hook_name: &str) -> InstallValidation {
    match fs::read_to_string(path) {
        Ok(text) => InstallValidation {
            check: format!("git_hook_{hook_name}"),
            ok: text.contains(super::git_hooks::HOOK_START_MARKER)
                && text.contains(super::git_hooks::HOOK_END_MARKER),
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
