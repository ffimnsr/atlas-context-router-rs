use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use super::InstallScope;
use super::scope::{atlas_hook_command, display_scoped_path, should_auto_detect};

const ATLAS_HOOK_RUNNER_SCRIPT: &str = r#"#!/bin/sh
# Atlas hook runner — thin launcher for Rust `atlas hook`.
# Installed by `atlas install` at .atlas/hooks/atlas-hook.
# All failures are non-blocking.

if [ "$#" -ge 2 ]; then
    ATLAS_FRONTEND="$1"
    ATLAS_EVENT="$2"
else
    ATLAS_FRONTEND="hook"
    ATLAS_EVENT="${1:-unknown}"
fi

# Skip all work when atlas binary is not on PATH.
command -v atlas >/dev/null 2>&1 || exit 0

if [ -n "$ATLAS_EVENT" ] && [ "$ATLAS_EVENT" != "unknown" ]; then
    ATLAS_HOOK_FRONTEND="$ATLAS_FRONTEND" \
    ATLAS_HOOK_SCRIPT_PATH="$0" \
    atlas hook "$ATLAS_EVENT" >/dev/null 2>&1 || true
fi
"#;

pub(super) const ATLAS_HOOK_MARKER: &str = "atlas-hook";
const ATLAS_HOOK_LIB_REL: &str = ".atlas/hooks/lib/";

const COPILOT_VSCODE_EVENTS: &[(&str, &str, &str)] = &[
    ("SessionStart", "copilot", "session-start"),
    ("UserPromptSubmit", "copilot", "user-prompt"),
    ("PreToolUse", "copilot", "pre-tool-use"),
    ("PostToolUse", "copilot", "post-tool-use"),
    ("PreCompact", "copilot", "pre-compact"),
    ("SubagentStart", "copilot", "subagent-start"),
    ("SubagentStop", "copilot", "subagent-stop"),
    ("Stop", "copilot", "stop"),
];

const CLAUDE_HOOKS: &[(&str, &str, &str, Option<&str>)] = &[
    ("SessionStart", "claude", "session-start", None),
    ("UserPromptSubmit", "claude", "user-prompt", None),
    (
        "UserPromptExpansion",
        "claude",
        "user-prompt-expansion",
        None,
    ),
    (
        "PreToolUse",
        "claude",
        "pre-tool-use",
        Some("Bash|Edit|Write|MultiEdit"),
    ),
    ("PermissionRequest", "claude", "permission-request", None),
    ("PermissionDenied", "claude", "permission-denied", None),
    (
        "PostToolUse",
        "claude",
        "post-tool-use",
        Some("Edit|Write|MultiEdit|Bash"),
    ),
    ("PostToolUseFailure", "claude", "tool-failure", None),
    ("Notification", "claude", "notification", None),
    ("SubagentStart", "claude", "subagent-start", None),
    ("SubagentStop", "claude", "subagent-stop", None),
    ("TaskCreated", "claude", "task-created", None),
    ("TaskCompleted", "claude", "task-completed", None),
    ("Stop", "claude", "stop", None),
    ("StopFailure", "claude", "stop-failure", None),
    ("InstructionsLoaded", "claude", "instructions-loaded", None),
    ("ConfigChange", "claude", "config-change", None),
    ("CwdChanged", "claude", "cwd-changed", None),
    ("FileChanged", "claude", "file-changed", None),
    ("WorktreeCreate", "claude", "worktree-create", None),
    ("WorktreeRemove", "claude", "worktree-remove", None),
    ("PreCompact", "claude", "pre-compact", None),
    ("PostCompact", "claude", "post-compact", None),
    ("Elicitation", "claude", "elicitation", None),
    ("ElicitationResult", "claude", "elicitation-result", None),
    ("SessionEnd", "claude", "session-end", None),
];

const CODEX_HOOKS: &[(&str, &str, &str, Option<&str>)] = &[
    (
        "SessionStart",
        "codex",
        "session-start",
        Some("startup|resume"),
    ),
    ("UserPromptSubmit", "codex", "user-prompt", None),
    ("PreToolUse", "codex", "pre-tool-use", Some("Bash")),
    (
        "PermissionRequest",
        "codex",
        "permission-request",
        Some("Bash"),
    ),
    ("PostToolUse", "codex", "post-tool-use", Some("Bash")),
    ("Stop", "codex", "stop", None),
];

pub fn install_platform_agent_hooks(
    repo_root: &Path,
    platform: &str,
    dry_run: bool,
) -> Result<Vec<String>> {
    install_platform_agent_hooks_scoped(repo_root, repo_root, platform, InstallScope::Repo, dry_run)
}

pub(super) fn install_platform_agent_hooks_scoped(
    repo_root: &Path,
    scope_root: &Path,
    platform: &str,
    scope: InstallScope,
    dry_run: bool,
) -> Result<Vec<String>> {
    let mut files = Vec::new();

    let run_for = |name: &str| match platform {
        "all" => should_auto_detect(name, repo_root),
        current => current == name,
    };

    let needs_any = run_for("copilot") || run_for("claude") || run_for("codex");
    if !needs_any {
        return Ok(files);
    }

    if let Some(runner_path) =
        install_atlas_hook_runner_scoped(repo_root, scope_root, scope, dry_run)?
    {
        files.push(runner_path);
    }

    if run_for("copilot")
        && let Some(path) =
            install_copilot_agent_hooks_scoped(repo_root, scope_root, scope, dry_run)?
    {
        files.push(path);
    }
    if run_for("claude")
        && let Some(path) =
            install_claude_agent_hooks_scoped(repo_root, scope_root, scope, dry_run)?
    {
        files.push(path);
    }
    if run_for("codex")
        && let Some(path) = install_codex_agent_hooks_scoped(repo_root, scope_root, scope, dry_run)?
    {
        files.push(path);
    }

    Ok(files)
}

#[cfg(test)]
pub(super) fn install_atlas_hook_runner(repo_root: &Path, dry_run: bool) -> Result<Option<String>> {
    install_atlas_hook_runner_scoped(repo_root, repo_root, InstallScope::Repo, dry_run)
}

fn install_atlas_hook_runner_scoped(
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
    dry_run: bool,
) -> Result<Option<String>> {
    let hooks_dir = scope_root.join(".atlas").join("hooks");
    let hook_lib_dir = hooks_dir.join("lib");
    let runner_path = hooks_dir.join("atlas-hook");

    let existing = if runner_path.exists() {
        fs::read_to_string(&runner_path)
            .with_context(|| format!("cannot read {}", runner_path.display()))?
    } else {
        String::new()
    };

    if !hook_lib_dir.is_dir() {
        if dry_run {
            println!("  [dry-run] would create {ATLAS_HOOK_LIB_REL}");
        } else {
            fs::create_dir_all(&hook_lib_dir)
                .with_context(|| format!("cannot create {}", hook_lib_dir.display()))?;
        }
    }

    if existing == ATLAS_HOOK_RUNNER_SCRIPT {
        return Ok(None);
    }

    let rel = display_scoped_path(&runner_path, repo_root, scope_root, scope);
    if dry_run {
        if existing.is_empty() {
            println!("  [dry-run] would create {rel}");
        } else {
            println!("  [dry-run] would update {rel}");
        }
        return Ok(Some(rel.to_owned()));
    }

    fs::create_dir_all(&hooks_dir)
        .with_context(|| format!("cannot create {}", hooks_dir.display()))?;
    fs::write(&runner_path, ATLAS_HOOK_RUNNER_SCRIPT)
        .with_context(|| format!("cannot write {}", runner_path.display()))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runner_path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("cannot chmod {}", runner_path.display()))?;
    }

    Ok(Some(rel.to_owned()))
}

#[cfg(test)]
pub(super) fn copilot_hooks_value() -> Value {
    copilot_hooks_value_scoped(Path::new("."), InstallScope::Repo)
}

fn copilot_hooks_value_scoped(install_root: &Path, scope: InstallScope) -> Value {
    let mut hooks = serde_json::Map::new();
    for (event, frontend, arg) in COPILOT_VSCODE_EVENTS {
        hooks.insert(
            (*event).to_owned(),
            serde_json::json!([{
                "type": "command",
                "command": atlas_hook_command(install_root, scope, frontend, arg)
            }]),
        );
    }
    serde_json::json!({ "hooks": Value::Object(hooks) })
}

#[cfg(test)]
pub(super) fn install_copilot_agent_hooks(
    repo_root: &Path,
    dry_run: bool,
) -> Result<Option<String>> {
    install_copilot_agent_hooks_scoped(repo_root, repo_root, InstallScope::Repo, dry_run)
}

fn install_copilot_agent_hooks_scoped(
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
    dry_run: bool,
) -> Result<Option<String>> {
    let config_dir = match scope {
        InstallScope::Repo => scope_root.join(".github").join("hooks"),
        InstallScope::User => scope_root.join(".copilot").join("hooks"),
    };
    let config_path = config_dir.join("atlas-copilot.json");
    let rel = display_scoped_path(&config_path, repo_root, scope_root, scope);

    if config_path.exists() {
        let existing = fs::read_to_string(&config_path)
            .with_context(|| format!("cannot read {}", config_path.display()))?;
        if existing.contains(ATLAS_HOOK_MARKER) {
            return Ok(None);
        }
    }

    let content = serde_json::to_string_pretty(&copilot_hooks_value_scoped(scope_root, scope))
        .context("cannot serialise Copilot hook config")?;
    let content = format!("{content}\n");

    if dry_run {
        println!("  [dry-run] would write {rel}");
        return Ok(Some(rel.to_owned()));
    }

    fs::create_dir_all(&config_dir)
        .with_context(|| format!("cannot create {}", config_dir.display()))?;
    fs::write(&config_path, &content)
        .with_context(|| format!("cannot write {}", config_path.display()))?;

    Ok(Some(rel.to_owned()))
}

#[cfg(test)]
pub(super) fn claude_hooks_value() -> Value {
    claude_hooks_value_scoped(Path::new("."), InstallScope::Repo)
}

fn claude_hooks_value_scoped(install_root: &Path, scope: InstallScope) -> Value {
    let mut hooks_map = serde_json::Map::new();
    for (event, frontend, arg, matcher) in CLAUDE_HOOKS {
        let cmd = atlas_hook_command(install_root, scope, frontend, arg);
        let entry = if let Some(matcher) = matcher {
            serde_json::json!([{
                "matcher": matcher,
                "hooks": [{ "type": "command", "command": cmd }]
            }])
        } else {
            serde_json::json!([{
                "hooks": [{ "type": "command", "command": cmd }]
            }])
        };
        hooks_map.insert((*event).to_owned(), entry);
    }
    serde_json::json!({ "hooks": Value::Object(hooks_map) })
}

#[cfg(test)]
pub(super) fn install_claude_agent_hooks(
    repo_root: &Path,
    dry_run: bool,
) -> Result<Option<String>> {
    install_claude_agent_hooks_scoped(repo_root, repo_root, InstallScope::Repo, dry_run)
}

fn install_claude_agent_hooks_scoped(
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
    dry_run: bool,
) -> Result<Option<String>> {
    let config_path = scope_root.join(".claude").join("settings.json");
    let rel = display_scoped_path(&config_path, repo_root, scope_root, scope);

    let mut root: serde_json::Map<String, Value> = if config_path.exists() {
        let text = fs::read_to_string(&config_path)
            .with_context(|| format!("cannot read {}", config_path.display()))?;
        if text.contains(ATLAS_HOOK_MARKER) {
            return Ok(None);
        }
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(map)) => map,
            _ => serde_json::Map::new(),
        }
    } else {
        serde_json::Map::new()
    };

    let atlas_hooks = claude_hooks_value_scoped(scope_root, scope);
    let Value::Object(atlas_obj) = &atlas_hooks else {
        unreachable!()
    };
    let Value::Object(atlas_hooks_map) = atlas_obj.get("hooks").cloned().unwrap_or_default() else {
        unreachable!()
    };

    let hooks_entry = root
        .entry("hooks")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    if let Value::Object(existing_hooks) = hooks_entry {
        for (key, value) in atlas_hooks_map {
            existing_hooks.insert(key, value);
        }
    }

    let content = serde_json::to_string_pretty(&Value::Object(root))
        .context("cannot serialise Claude hooks")?;
    let content = format!("{content}\n");

    if dry_run {
        println!("  [dry-run] would write {rel}");
        return Ok(Some(rel.to_owned()));
    }

    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    fs::write(&config_path, &content)
        .with_context(|| format!("cannot write {}", config_path.display()))?;

    Ok(Some(rel.to_owned()))
}

#[cfg(test)]
pub(super) fn codex_hooks_value() -> Value {
    codex_hooks_value_scoped(Path::new("."), InstallScope::Repo)
}

fn codex_hooks_value_scoped(install_root: &Path, scope: InstallScope) -> Value {
    let mut hooks_map = serde_json::Map::new();
    for (event, frontend, arg, matcher) in CODEX_HOOKS {
        let cmd = atlas_hook_command(install_root, scope, frontend, arg);
        let entry = if let Some(matcher) = matcher {
            serde_json::json!([{
                "matcher": matcher,
                "commands": [{ "command": cmd }]
            }])
        } else {
            serde_json::json!([{
                "commands": [{ "command": cmd }]
            }])
        };
        hooks_map.insert((*event).to_owned(), entry);
    }
    serde_json::json!({ "hooks": Value::Object(hooks_map) })
}

#[cfg(test)]
pub(super) fn install_codex_agent_hooks(repo_root: &Path, dry_run: bool) -> Result<Option<String>> {
    install_codex_agent_hooks_scoped(repo_root, repo_root, InstallScope::Repo, dry_run)
}

fn install_codex_agent_hooks_scoped(
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
    dry_run: bool,
) -> Result<Option<String>> {
    let config_dir = scope_root.join(".codex");
    let config_path = config_dir.join("hooks.json");
    let rel = display_scoped_path(&config_path, repo_root, scope_root, scope);

    if config_path.exists() {
        let existing = fs::read_to_string(&config_path)
            .with_context(|| format!("cannot read {}", config_path.display()))?;
        if existing.contains(ATLAS_HOOK_MARKER) {
            return Ok(None);
        }
    }

    let content = serde_json::to_string_pretty(&codex_hooks_value_scoped(scope_root, scope))
        .context("cannot serialise Codex hook config")?;
    let content = format!("{content}\n");

    if dry_run {
        println!("  [dry-run] would write {rel}");
        return Ok(Some(rel.to_owned()));
    }

    fs::create_dir_all(&config_dir)
        .with_context(|| format!("cannot create {}", config_dir.display()))?;
    fs::write(&config_path, &content)
        .with_context(|| format!("cannot write {}", config_path.display()))?;

    Ok(Some(rel.to_owned()))
}
