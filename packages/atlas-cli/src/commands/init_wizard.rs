//! Interactive TUI wizard for `atlas init`.
//!
//! Runs only when stdin is an interactive terminal and `--json` is not set.
//! Guides the user through generic, tool-agnostic Atlas setup: the managed
//! `AGENTS.md` instruction block, the generic agent hook runner, and git hooks.
//! Per-tool MCP and hook configuration is intentionally not written.

use std::io::{IsTerminal, Write};
use std::path::Path;

use anyhow::Result;
use console::{Style, Term, style};
use dialoguer::{Confirm, theme::ColorfulTheme};

use crate::install::{InstallOptions, InstallSummary, install_hook_runner};

/// Returns `true` when the wizard should run.
pub fn should_run(json: bool) -> bool {
    !json && std::io::stdin().is_terminal()
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the full interactive init wizard.
///
/// Called after the basic atlas directory / database setup is complete.
pub fn run(repo_root: &Path) -> Result<()> {
    let term = Term::stdout();
    let theme = ColorfulTheme::default();

    print_banner(&term)?;

    // ── Step 1: Agent integration ────────────────────────────────────────────
    section(&term, "1", "Agent Integration")?;
    writeln!(
        term.clone(),
        "  Atlas writes a managed block to the generic AGENTS.md file.\n"
    )?;

    let install_agent_hooks = Confirm::with_theme(&theme)
        .with_prompt("Install the generic Atlas agent hook runner?")
        .default(true)
        .interact()?;

    // ── Step 2: Git hooks ────────────────────────────────────────────────────
    writeln!(term.clone())?;
    section(&term, "2", "Git Hooks")?;

    let install_git_hooks = Confirm::with_theme(&theme)
        .with_prompt("Install git hooks for automatic graph updates?")
        .default(false)
        .interact()?;

    // ── Apply ─────────────────────────────────────────────────────────────────
    writeln!(term.clone())?;
    writeln!(
        term.clone(),
        "  {}",
        Style::new().dim().apply_to("─".repeat(52))
    )?;
    writeln!(term.clone())?;
    writeln!(
        term.clone(),
        "  {}",
        style("Applying configuration…").bold()
    )?;
    writeln!(term.clone())?;

    match apply_generic_setup(repo_root, install_agent_hooks, install_git_hooks) {
        Ok(summary) => {
            for f in &summary.instruction_files {
                print_tick(&term, &format!("Instructions → {f}"))?;
            }
            for f in &summary.hook_runner_files {
                print_tick(&term, &format!("Hook runner  → {f}"))?;
            }
            for p in &summary.hook_paths {
                print_tick(&term, &format!("Git hook  → {}", p.display()))?;
            }
            if summary.instruction_files.is_empty()
                && summary.hook_runner_files.is_empty()
                && summary.hook_paths.is_empty()
            {
                print_skip(&term, "Atlas integration already up to date")?;
            }
        }
        Err(e) => print_cross(&term, &format!("Setup failed: {e}"))?,
    }

    // ── Done ──────────────────────────────────────────────────────────────────
    writeln!(term.clone())?;
    writeln!(
        term.clone(),
        "  {}",
        Style::new().dim().apply_to("─".repeat(52))
    )?;
    writeln!(term.clone())?;
    writeln!(
        term.clone(),
        "  {} {}",
        style("✓").green().bold(),
        style("Atlas initialized!").bold()
    )?;
    writeln!(term.clone())?;
    writeln!(term.clone(), "  Next steps:")?;
    writeln!(
        term.clone(),
        "    {}  — scan and index your codebase",
        style("atlas build").cyan().bold()
    )?;
    writeln!(
        term.clone(),
        "    {}  — start the MCP server for AI tools",
        style("atlas serve").cyan().bold()
    )?;
    writeln!(term.clone())?;
    writeln!(term.clone(), "  {}", crate::install::GENERIC_MCP_SETUP)?;
    writeln!(term.clone())?;

    Ok(())
}

/// Apply the tool-agnostic surfaces selected in the wizard.
fn apply_generic_setup(
    repo_root: &Path,
    with_agent_hooks: bool,
    with_git_hooks: bool,
) -> Result<InstallSummary> {
    let mut summary = crate::install::run_install(
        repo_root,
        InstallOptions {
            no_hooks: true,
            ..Default::default()
        },
    )?;

    if with_agent_hooks && let Some(path) = install_hook_runner(repo_root, false)? {
        summary.hook_runner_files.push(path);
    }

    if with_git_hooks {
        summary.hook_paths = crate::install::install_git_hooks(repo_root, false, false)?;
    }

    Ok(summary)
}

// ---------------------------------------------------------------------------
// Banner
// ---------------------------------------------------------------------------

fn print_banner(term: &Term) -> Result<()> {
    let _ = term.clear_screen();
    let accent = Style::new().cyan().bold();
    let dim = Style::new().dim();

    writeln!(term.clone())?;
    writeln!(
        term.clone(),
        "  {}",
        accent.apply_to("█████╗ ████████╗██╗      █████╗ ███████╗")
    )?;
    writeln!(
        term.clone(),
        "  {}",
        accent.apply_to("██╔══██╗╚══██╔══╝██║     ██╔══██╗██╔════╝")
    )?;
    writeln!(
        term.clone(),
        "  {}",
        accent.apply_to("███████║   ██║   ██║     ███████║███████╗")
    )?;
    writeln!(
        term.clone(),
        "  {}",
        accent.apply_to("██╔══██║   ██║   ██║     ██╔══██║╚════██║")
    )?;
    writeln!(
        term.clone(),
        "  {}",
        accent.apply_to("██║  ██║   ██║   ███████╗██║  ██║███████║")
    )?;
    writeln!(
        term.clone(),
        "  {}",
        accent.apply_to("╚═╝  ╚═╝   ╚═╝   ╚══════╝╚═╝  ╚═╝╚══════╝")
    )?;
    writeln!(term.clone())?;
    writeln!(
        term.clone(),
        "  {}  {}",
        style(" graph-aware code context for CLI and MCP workflows").bold(),
        dim.apply_to("· interactive setup")
    )?;
    writeln!(term.clone())?;
    writeln!(term.clone(), "  {}", dim.apply_to("─".repeat(52)))?;
    writeln!(term.clone())?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Section header
// ---------------------------------------------------------------------------

fn section(term: &Term, num: &str, title: &str) -> Result<()> {
    let badge = style(format!("[{num}]")).cyan().bold();
    let heading = style(title).white().bold();
    writeln!(term.clone(), "  {badge} {heading}")?;
    writeln!(term.clone())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Result line helpers
// ---------------------------------------------------------------------------

fn print_tick(term: &Term, msg: &str) -> Result<()> {
    writeln!(term.clone(), "  {}  {msg}", style("✓").green().bold())?;
    Ok(())
}

fn print_skip(term: &Term, msg: &str) -> Result<()> {
    writeln!(term.clone(), "  {}  {msg}", style("·").dim())?;
    Ok(())
}

fn print_cross(term: &Term, msg: &str) -> Result<()> {
    writeln!(term.clone(), "  {}  {msg}", style("✗").red().bold())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::apply_generic_setup;

    #[test]
    fn wizard_installs_generic_agents_md_only() {
        let tmp = TempDir::new().unwrap();

        let summary = apply_generic_setup(tmp.path(), false, false).unwrap();

        assert!(tmp.path().join("AGENTS.md").exists());
        assert!(summary.instruction_files.contains(&"AGENTS.md".to_owned()));
        assert!(summary.hook_runner_files.is_empty());
        assert!(summary.hook_paths.is_empty());
        for removed in [".mcp.json", ".claude", ".codex", ".vscode", "CLAUDE.md"] {
            assert!(
                !tmp.path().join(removed).exists(),
                "tool-specific path {removed} must not be created"
            );
        }
    }

    #[test]
    fn wizard_can_install_generic_hook_runner() {
        let tmp = TempDir::new().unwrap();

        let summary = apply_generic_setup(tmp.path(), true, false).unwrap();

        assert!(
            tmp.path()
                .join(".atlas")
                .join("hooks")
                .join("atlas-hook")
                .exists()
        );
        assert!(
            summary
                .hook_runner_files
                .contains(&".atlas/hooks/atlas-hook".to_owned())
        );
        assert!(!tmp.path().join(".claude").exists());
        assert!(!tmp.path().join(".codex").exists());
    }
}
