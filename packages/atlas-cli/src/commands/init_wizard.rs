//! Interactive TUI wizard for `atlas init`.
//!
//! Runs only when stdin is an interactive terminal and `--json` is not set.
//! Guides the user through platform configuration, git hooks, and shell
//! completions, then prints a concise "done" summary.

use std::io::{IsTerminal, Write};
use std::path::Path;

use anyhow::{Context, Result};
use clap_complete::Shell;
use console::{Style, Term, style};
use dialoguer::{Confirm, MultiSelect, Select, theme::ColorfulTheme};

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

    // ── Step 3: Shell completions ─────────────────────────────────────────────
    writeln!(term.clone())?;
    section(&term, "3", "Shell Completions")?;

    let install_completions = Confirm::with_theme(&theme)
        .with_prompt("Install shell completions?")
        .default(false)
        .interact()?;

    let chosen_shell: Option<Shell> = if install_completions {
        const SHELL_NAMES: [&str; 4] = ["bash", "zsh", "fish", "powershell"];
        const SHELL_VALS: [Shell; 4] = [Shell::Bash, Shell::Zsh, Shell::Fish, Shell::PowerShell];
        let default = detect_shell_idx(&SHELL_NAMES);

        let idx = Select::with_theme(&theme)
            .with_prompt("Select your shell")
            .items(&SHELL_NAMES)
            .default(default)
            .interact()?;

        Some(SHELL_VALS[idx])
    } else {
        None
    };

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
        match crate::install::run_install(repo_root, key, false, true, false) {
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
                if summary.configured.is_empty()
                    && summary.already_configured.is_empty()
                    && summary.instruction_files.is_empty()
                {
                    print_tick(&term, display)?;
                }
            }
            Err(e) => print_cross(&term, &format!("{display}: {e}"))?,
        }
    }

    // Git hooks
    if install_hooks {
        match crate::install::install_git_hooks(repo_root, false) {
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

    // Shell completions
    if let Some(shell) = chosen_shell {
        match write_completions(shell) {
            Ok(path) => print_tick(&term, &format!("Completions → {path}"))?,
            Err(e) => print_cross(&term, &format!("Completions: {e}"))?,
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

// ---------------------------------------------------------------------------
// Shell detection
// ---------------------------------------------------------------------------

fn detect_shell_idx(names: &[&str]) -> usize {
    let shell_bin = std::env::var("SHELL").unwrap_or_default().to_lowercase();
    let hint = shell_bin.rsplit('/').next().unwrap_or("").to_lowercase();

    names.iter().position(|&n| n == hint).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Completions installation
// ---------------------------------------------------------------------------

/// Generate completions for `shell` and write them to the conventional
/// location for that shell. Returns the path written (as a display string).
fn write_completions(shell: Shell) -> Result<String> {
    use clap::CommandFactory;
    use clap_complete::generate;

    let mut cmd = crate::cli::Cli::command();
    let mut buf = Vec::new();
    generate(shell, &mut cmd, "atlas", &mut buf);
    let completions = String::from_utf8(buf).context("completions are not valid UTF-8")?;

    let (path, preamble) = completions_path(shell)?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    match shell {
        Shell::Bash | Shell::Zsh => {
            // Append eval line to shell rc if not already present.
            let rc = std::fs::read_to_string(&path).unwrap_or_default();
            if !rc.contains("atlas completions") {
                let mut f = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                    .with_context(|| format!("cannot open {}", path.display()))?;
                writeln!(f, "\n{preamble}")?;
            }
        }
        _ => {
            // Write (or overwrite) the completions file.
            std::fs::write(&path, &completions)
                .with_context(|| format!("cannot write {}", path.display()))?;
        }
    }

    Ok(path.display().to_string())
}

/// Return `(file_path, eval_line_or_source)` for the given shell.
fn completions_path(shell: Shell) -> Result<(std::path::PathBuf, String)> {
    let home = home_dir()?;
    match shell {
        Shell::Bash => {
            let rc = home.join(".bashrc");
            Ok((rc, r#"eval "$(atlas completions bash)""#.to_owned()))
        }
        Shell::Zsh => {
            let rc = home.join(".zshrc");
            Ok((rc, r#"eval "$(atlas completions zsh)""#.to_owned()))
        }
        Shell::Fish => {
            let p = home
                .join(".config")
                .join("fish")
                .join("completions")
                .join("atlas.fish");
            Ok((p, String::new()))
        }
        Shell::PowerShell => {
            // $PROFILE equivalent — use a well-known default location.
            let docs = home.join("Documents").join("PowerShell");
            let p = docs.join("atlas.ps1");
            Ok((p, String::new()))
        }
        _ => anyhow::bail!("unsupported shell variant"),
    }
}

fn home_dir() -> Result<std::path::PathBuf> {
    #[allow(deprecated)]
    std::env::home_dir().context("cannot determine home directory")
}
