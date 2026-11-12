//! ICM-H1 — shell-first `atlas init --mode` asset generation.
//!
//! Modes are idempotent and fully previewable: `plan_init_mode` reports the
//! exact files a mode would create or refresh without touching the filesystem,
//! and `apply_init_mode` performs the same work through the shared install
//! helpers. No host-specific command generators are introduced here: the MCP
//! asset is a generic stdio server entry that any client can copy.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::Serialize;

use super::git_hooks::install_git_hooks;
use super::hook_runner::{ATLAS_HOOK_RUNNER_REL, install_hook_runner};
use super::instructions::{
    AGENTS_FILE, INSTRUCTIONS_MARKER, InstructionsMode, inject_instructions,
};
use crate::cli::InitMode;

/// Repo-relative generic MCP registration asset written by `--mode mcp`.
pub const MCP_ASSET_REL: &str = ".atlas/agent/mcp.json";

/// Generic stdio server entry; the user copies it into their host config.
pub const MCP_ASSET_CONTENT: &str = r#"{
  "mcpServers": {
    "atlas": {
      "type": "stdio",
      "command": "atlas",
      "args": ["serve"]
    }
  }
}
"#;

/// Git hooks installed by `--mode hook`.
const GIT_HOOK_NAMES: &[&str] = &[
    "pre-commit",
    "post-commit",
    "post-checkout",
    "post-merge",
    "post-rewrite",
];

/// One planned asset write.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InitPlanEntry {
    /// Asset family: `mcp`, `hook`, `hook_runner`, or `instructions`.
    pub asset: String,
    /// Repo-relative path the mode writes.
    pub path: String,
    /// `create`, `refresh`, `unchanged`, or `skipped`.
    pub action: String,
}

/// Outcome of one `atlas init --mode` run.
#[derive(Debug, Clone, Default, Serialize)]
pub struct InitModeSummary {
    pub mode: String,
    pub dry_run: bool,
    /// Every asset the mode considered, with its planned or applied action.
    pub files: Vec<InitPlanEntry>,
}

impl InitModeSummary {
    pub fn touched(&self) -> Vec<&InitPlanEntry> {
        self.files
            .iter()
            .filter(|entry| entry.action == "create" || entry.action == "refresh")
            .collect()
    }
}

/// Enumerate every asset a mode would write, without touching the filesystem.
pub fn plan_init_mode(repo_root: &Path, mode: InitMode) -> Vec<InitPlanEntry> {
    let mut entries = Vec::new();

    if mode.includes_mcp() {
        let path = repo_root.join(MCP_ASSET_REL);
        let action = match fs::read_to_string(&path) {
            Ok(existing) if existing == MCP_ASSET_CONTENT => "unchanged",
            Ok(_) => "refresh",
            Err(_) => "create",
        };
        entries.push(InitPlanEntry {
            asset: "mcp".to_owned(),
            path: MCP_ASSET_REL.to_owned(),
            action: action.to_owned(),
        });
    }

    if mode.includes_hooks() {
        if repo_root.join(".git").is_dir() {
            for hook in GIT_HOOK_NAMES {
                let relative = format!(".git/hooks/{hook}");
                let path = repo_root.join(&relative);
                let action = match fs::read_to_string(&path) {
                    Ok(existing) if existing.contains("# atlas-hook start") => "unchanged",
                    Ok(_) => "refresh",
                    Err(_) => "create",
                };
                entries.push(InitPlanEntry {
                    asset: "hook".to_owned(),
                    path: relative,
                    action: action.to_owned(),
                });
            }
        }
        let runner_path = repo_root.join(ATLAS_HOOK_RUNNER_REL);
        let action = match fs::read_to_string(&runner_path) {
            Ok(existing) if existing.contains("Atlas hook runner") => "unchanged",
            Ok(_) => "refresh",
            Err(_) => "create",
        };
        entries.push(InitPlanEntry {
            asset: "hook_runner".to_owned(),
            path: ATLAS_HOOK_RUNNER_REL.to_owned(),
            action: action.to_owned(),
        });
    }

    if mode.includes_cli() {
        let path = repo_root.join(AGENTS_FILE);
        let action = match fs::read_to_string(&path) {
            Ok(existing) if existing.contains(INSTRUCTIONS_MARKER) => "unchanged",
            Ok(_) => "refresh",
            Err(_) => "create",
        };
        entries.push(InitPlanEntry {
            asset: "instructions".to_owned(),
            path: AGENTS_FILE.to_owned(),
            action: action.to_owned(),
        });
    }

    entries
}

/// Apply one init mode. `dry_run` returns the plan without writing anything.
pub fn apply_init_mode(
    repo_root: &Path,
    mode: InitMode,
    dry_run: bool,
    force: bool,
) -> Result<InitModeSummary> {
    let plan = plan_init_mode(repo_root, mode);
    let mut summary = InitModeSummary {
        mode: mode.as_str().to_owned(),
        dry_run,
        files: plan.clone(),
    };
    if dry_run {
        return Ok(summary);
    }

    if mode.includes_mcp() {
        let path = repo_root.join(MCP_ASSET_REL);
        let considered = plan
            .iter()
            .find(|entry| entry.asset == "mcp")
            .map(|entry| entry.action.as_str())
            .unwrap_or("create");
        let applied = match considered {
            "unchanged" => "unchanged",
            // Existing user-edited assets are never clobbered without --force.
            "refresh" if !force => "skipped",
            _ => {
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("cannot create {}", parent.display()))?;
                }
                fs::write(&path, MCP_ASSET_CONTENT)
                    .with_context(|| format!("cannot write {}", path.display()))?;
                "create"
            }
        };
        set_action(&mut summary.files, MCP_ASSET_REL, applied);
    }

    if mode.includes_hooks() {
        let hook_paths = install_git_hooks(repo_root, false, force)
            .context("cannot install git hooks for init --mode hook")?;
        for path in &hook_paths {
            let relative = repo_relative(repo_root, path);
            set_action(&mut summary.files, &relative, "create");
        }
        if install_hook_runner(repo_root, false)
            .context("cannot install the Atlas hook runner")?
            .is_some()
        {
            set_action(&mut summary.files, ATLAS_HOOK_RUNNER_REL, "create");
        }
    }

    if mode.includes_cli() {
        let updated =
            inject_instructions(repo_root, &[AGENTS_FILE], false, InstructionsMode::Refresh)
                .context("cannot inject the managed AGENTS.md instruction block")?;
        if updated.iter().any(|file| file == AGENTS_FILE) {
            set_action(&mut summary.files, AGENTS_FILE, "refresh");
        }
    }

    Ok(summary)
}

fn set_action(entries: &mut [InitPlanEntry], path: &str, action: &str) {
    for entry in entries.iter_mut() {
        if entry.path == path {
            entry.action = action.to_owned();
        }
    }
}

fn repo_relative(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn repo() -> TempDir {
        let dir = TempDir::new().expect("temp dir");
        fs::create_dir_all(dir.path().join(".git").join("hooks")).expect("git dir");
        dir
    }

    #[test]
    fn plan_lists_only_requested_assets() {
        let repo = repo();
        let mcp = plan_init_mode(repo.path(), InitMode::Mcp);
        assert_eq!(mcp.len(), 1);
        assert_eq!(mcp[0].path, MCP_ASSET_REL);
        assert_eq!(mcp[0].action, "create");

        let hooks = plan_init_mode(repo.path(), InitMode::Hook);
        assert!(hooks.iter().all(|entry| entry.asset != "mcp"));
        assert_eq!(
            hooks.iter().filter(|entry| entry.asset == "hook").count(),
            GIT_HOOK_NAMES.len()
        );

        let cli = plan_init_mode(repo.path(), InitMode::Cli);
        assert_eq!(cli.len(), 1);
        assert_eq!(cli[0].path, AGENTS_FILE);

        let all = plan_init_mode(repo.path(), InitMode::All);
        assert_eq!(all.len(), 1 + GIT_HOOK_NAMES.len() + 1 + 1);
    }

    #[test]
    fn dry_run_writes_nothing_and_reports_plan() {
        let repo = repo();
        let summary =
            apply_init_mode(repo.path(), InitMode::All, true, false).expect("dry run plan");
        assert!(summary.dry_run);
        assert!(!repo.path().join(MCP_ASSET_REL).exists());
        assert!(!repo.path().join(AGENTS_FILE).exists());
        assert!(!summary.touched().is_empty());
    }

    #[test]
    fn mcp_mode_is_idempotent_and_preserves_user_edits() {
        let repo = repo();
        let first = apply_init_mode(repo.path(), InitMode::Mcp, false, false).expect("apply");
        assert_eq!(
            first
                .files
                .iter()
                .find(|entry| entry.asset == "mcp")
                .map(|entry| entry.action.as_str()),
            Some("create")
        );
        let content =
            fs::read_to_string(repo.path().join(MCP_ASSET_REL)).expect("mcp asset content");
        assert_eq!(content, MCP_ASSET_CONTENT);

        let second = apply_init_mode(repo.path(), InitMode::Mcp, false, false).expect("re-apply");
        assert_eq!(
            second
                .files
                .iter()
                .find(|entry| entry.asset == "mcp")
                .map(|entry| entry.action.as_str()),
            Some("unchanged")
        );

        fs::write(repo.path().join(MCP_ASSET_REL), "{\"custom\":true}\n").expect("user edit");
        let third = apply_init_mode(repo.path(), InitMode::Mcp, false, false).expect("re-apply");
        assert_eq!(
            third
                .files
                .iter()
                .find(|entry| entry.asset == "mcp")
                .map(|entry| entry.action.as_str()),
            Some("skipped")
        );
        assert_eq!(
            fs::read_to_string(repo.path().join(MCP_ASSET_REL)).expect("content"),
            "{\"custom\":true}\n"
        );

        apply_init_mode(repo.path(), InitMode::Mcp, false, true).expect("forced apply");
        assert_eq!(
            fs::read_to_string(repo.path().join(MCP_ASSET_REL)).expect("content"),
            MCP_ASSET_CONTENT
        );
    }

    #[test]
    fn cli_mode_creates_then_refreshes_agent_instructions() {
        let repo = repo();
        apply_init_mode(repo.path(), InitMode::Cli, false, false).expect("apply");
        let content = fs::read_to_string(repo.path().join(AGENTS_FILE)).expect("agents");
        assert!(content.contains(INSTRUCTIONS_MARKER));

        let second = apply_init_mode(repo.path(), InitMode::Cli, false, false).expect("re-apply");
        assert_eq!(
            second
                .files
                .iter()
                .find(|entry| entry.asset == "instructions")
                .map(|entry| entry.action.as_str()),
            Some("unchanged")
        );
    }
}
