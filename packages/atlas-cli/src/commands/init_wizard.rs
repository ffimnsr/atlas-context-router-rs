//! Interactive TUI wizard for `atlas init`.
//!
//! Runs only when stdin is an interactive terminal and `--json` is not set.
//! Guides the user through platform configuration, git hooks, and shell
//! completions, then prints a concise "done" summary.

use std::io::{IsTerminal, Write};
use std::path::Path;

use anyhow::Result;
use console::{Style, Term, style};
use dialoguer::{Confirm, MultiSelect, theme::ColorfulTheme};

use crate::install::InstallSummary;

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

    // ── Step 1: Platform ─────────────────────────────────────────────────────
    section(&term, "1", "LLM Agent Setup")?;
    writeln!(
        term.clone(),
        "  Select the AI coding tool(s) to configure:\n"
    )?;

    const PLATFORM_NAMES: [&str; 3] = ["GitHub Copilot  (VS Code)", "Claude Code", "OpenAI Codex"];
    const PLATFORM_KEYS: [&str; 3] = ["copilot", "claude", "codex"];

    let platform_selections = MultiSelect::with_theme(&theme)
        .items(&PLATFORM_NAMES)
        .defaults(&[false, false, true])
        .interact()?;

    // ── Step 2: Git hooks ─────────────────────────────────────────────────────
    writeln!(term.clone())?;
    section(&term, "2", "Git Hooks")?;

    let install_hooks = Confirm::with_theme(&theme)
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

    // Platforms
    for &idx in &platform_selections {
        let key = PLATFORM_KEYS[idx];
        let display = PLATFORM_NAMES[idx];
        match install_platform_setup(repo_root, key) {
            Ok(summary) => {
                for name in &summary.configured {
                    print_tick(&term, name)?;
                }
                for name in &summary.already_configured {
                    print_skip(&term, &format!("{name} already configured"))?;
                }
                for f in &summary.instruction_files {
                    print_tick(&term, &format!("Instructions → {f}"))?;
                }
                for f in &summary.platform_hook_files {
                    print_tick(&term, &format!("Agent hooks → {f}"))?;
                }
                if summary.configured.is_empty()
                    && summary.already_configured.is_empty()
                    && summary.instruction_files.is_empty()
                    && summary.platform_hook_files.is_empty()
                {
                    print_tick(&term, display)?;
                }
            }
            Err(e) => print_cross(&term, &format!("{display}: {e}"))?,
        }
    }

    // Git hooks
    if install_hooks {
        match crate::install::install_git_hooks(repo_root, false, false) {
            Ok(paths) if !paths.is_empty() => {
                for p in &paths {
                    print_tick(&term, &format!("Git hook  → {}", p.display()))?;
                }
            }
            Ok(_) => {
                print_skip(&term, "Git hooks: no .git directory found")?;
            }
            Err(e) => print_cross(&term, &format!("Git hooks: {e}"))?,
        }
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

    Ok(())
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

fn install_platform_setup(repo_root: &Path, platform: &str) -> Result<InstallSummary> {
    let mut summary = crate::install::run_install(
        repo_root,
        platform,
        "repo",
        crate::install::InstallOptions {
            no_hooks: true,
            ..Default::default()
        },
    )?;
    summary.platform_hook_files =
        crate::install::install_platform_agent_hooks(repo_root, platform, false)?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::install_platform_setup;

    #[test]
    fn install_platform_setup_adds_agent_hooks_for_claude() {
        let tmp = TempDir::new().unwrap();

        let summary = install_platform_setup(tmp.path(), "claude").unwrap();

        assert!(tmp.path().join(".mcp.json").exists());
        assert!(tmp.path().join("CLAUDE.md").exists());
        assert!(
            tmp.path()
                .join(".atlas")
                .join("hooks")
                .join("atlas-hook")
                .exists()
        );
        assert!(tmp.path().join(".claude").join("settings.json").exists());
        assert!(
            summary
                .platform_hook_files
                .contains(&".atlas/hooks/atlas-hook".to_owned())
        );
        assert!(
            summary
                .platform_hook_files
                .contains(&".claude/settings.json".to_owned())
        );
    }

    #[test]
    fn install_platform_setup_keeps_git_hooks_separate() {
        let tmp = TempDir::new().unwrap();

        let summary = install_platform_setup(tmp.path(), "codex").unwrap();

        assert!(summary.hook_paths.is_empty());
        assert!(
            !tmp.path()
                .join(".git")
                .join("hooks")
                .join("post-commit")
                .exists()
        );
        assert!(tmp.path().join(".codex").join("hooks.json").exists());
    }
}
