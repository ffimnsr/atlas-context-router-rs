//! Platform MCP installation and hook injection for `atlas install`.
//!
//! Supported platforms:
//! - `copilot`  — `.vscode/mcp.json` (VS Code / GitHub Copilot)
//! - `claude`   — `.mcp.json` in repo root (Claude Code)
//! - `codex`    — `.codex/config.toml` in repo root (OpenAI Codex project config)
//!
//! Each platform detection check returns `true` when the tool's config
//! directory already exists, so we never silently clobber a non-user system.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::Value;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Result summary returned after running `atlas install`.
#[derive(Debug, Default)]
pub struct InstallSummary {
    /// Platform names that were successfully configured.
    pub configured: Vec<String>,
    /// Platform names that were already configured (no write needed).
    pub already_configured: Vec<String>,
    /// Git hook paths written or already present.
    pub hook_paths: Vec<PathBuf>,
    /// Instruction files that were created or updated.
    pub instruction_files: Vec<String>,
}

/// Run the full install flow for the given `platform` target.
///
/// `repo_root` — the repository root directory.
/// `platform`  — `"all"`, `"copilot"`, `"claude"`, or `"codex"`.
/// `dry_run`   — when `true`, describe changes without writing files.
/// `no_hooks`  — skip git hook installation.
/// `no_instructions` — skip injecting into AGENTS.md / CLAUDE.md.
pub fn run_install(
    repo_root: &Path,
    platform: &str,
    dry_run: bool,
    no_hooks: bool,
    no_instructions: bool,
) -> Result<InstallSummary> {
    let mut summary = InstallSummary::default();

    #[allow(clippy::type_complexity)]
    let platforms: &[(&str, &dyn Fn(&Path, bool) -> Result<PlatformResult>)] = &[
        ("copilot", &install_copilot),
        ("claude", &install_claude),
        ("codex", &install_codex),
    ];

    for (name, installer) in platforms {
        let skip = match platform {
            "all" => !should_auto_detect(name, repo_root),
            p => p != *name,
        };
        if skip {
            continue;
        }

        match installer(repo_root, dry_run)? {
            PlatformResult::Configured(label) => summary.configured.push(label),
            PlatformResult::AlreadyConfigured(label) => summary.already_configured.push(label),
        }
    }

    if !no_hooks {
        summary.hook_paths = install_git_hooks(repo_root, dry_run)?;
    }

    if !no_instructions {
        let files = inject_instructions(repo_root, dry_run)?;
        summary.instruction_files = files;
    }

    Ok(summary)
}

// ---------------------------------------------------------------------------
// Platform auto-detection
// ---------------------------------------------------------------------------

fn should_auto_detect(platform: &str, repo_root: &Path) -> bool {
    match platform {
        "copilot" => {
            // VS Code workspace exists, or VSCODE_PID env is set (running inside Code).
            repo_root.join(".vscode").is_dir()
                || std::env::var("VSCODE_PID").is_ok()
                || std::env::var("TERM_PROGRAM")
                    .map(|v| v == "vscode")
                    .unwrap_or(false)
        }
        "claude" => {
            // Claude Code is always a reasonable default — writes to repo root.
            true
        }
        "codex" => repo_root.join(".codex").is_dir(),
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Per-platform installers
// ---------------------------------------------------------------------------

enum PlatformResult {
    Configured(String),
    AlreadyConfigured(String),
}

/// GitHub Copilot — writes `.vscode/mcp.json` in the repo root.
fn install_copilot(repo_root: &Path, dry_run: bool) -> Result<PlatformResult> {
    let config_path = repo_root.join(".vscode").join("mcp.json");
    let server_entry = copilot_server_entry();
    merge_json_mcp(
        &config_path,
        "servers", // VS Code uses "servers" not "mcpServers"
        "atlas",
        server_entry,
        dry_run,
        "GitHub Copilot",
    )
}

/// Claude Code — writes `.mcp.json` in the repo root.
fn install_claude(repo_root: &Path, dry_run: bool) -> Result<PlatformResult> {
    let config_path = repo_root.join(".mcp.json");
    let server_entry = stdio_server_entry();
    merge_json_mcp(
        &config_path,
        "mcpServers",
        "atlas",
        server_entry,
        dry_run,
        "Claude Code",
    )
}

/// OpenAI Codex CLI — writes/appends to repo-local `.codex/config.toml`.
fn install_codex(repo_root: &Path, dry_run: bool) -> Result<PlatformResult> {
    let config_path = repo_root.join(".codex").join("config.toml");
    merge_toml_mcp(&config_path, "atlas", dry_run, "Codex")
}

// ---------------------------------------------------------------------------
// MCP JSON config helpers
// ---------------------------------------------------------------------------

/// Server entry for VS Code / GitHub Copilot (uses `"type": "stdio"`).
fn copilot_server_entry() -> Value {
    serde_json::json!({
        "type": "stdio",
        "command": "atlas",
        "args": ["serve"]
    })
}

/// Generic stdio server entry for other platforms (Claude Code, etc.).
fn stdio_server_entry() -> Value {
    serde_json::json!({
        "type": "stdio",
        "command": "atlas",
        "args": ["serve"]
    })
}

/// Read existing JSON object from `path`, merge a new MCP server under
/// `top_key."atlas"`, and write back.  Returns whether a write occurred.
fn merge_json_mcp(
    path: &Path,
    top_key: &str,
    server_name: &str,
    server_entry: Value,
    dry_run: bool,
    display_name: &str,
) -> Result<PlatformResult> {
    let mut root: serde_json::Map<String, Value> = if path.exists() {
        let text =
            fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        }
    } else {
        serde_json::Map::new()
    };

    let servers = root
        .entry(top_key)
        .or_insert_with(|| Value::Object(serde_json::Map::new()));

    if let Value::Object(map) = servers {
        if map.contains_key(server_name) {
            return Ok(PlatformResult::AlreadyConfigured(display_name.to_owned()));
        }
        map.insert(server_name.to_owned(), server_entry);
    }

    if dry_run {
        println!("  [dry-run] {display_name}: would write {}", path.display());
        return Ok(PlatformResult::Configured(display_name.to_owned()));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create directory {}", parent.display()))?;
    }
    let json = serde_json::to_string_pretty(&Value::Object(root))
        .context("cannot serialise MCP config")?;
    fs::write(path, format!("{json}\n"))
        .with_context(|| format!("cannot write {}", path.display()))?;

    Ok(PlatformResult::Configured(display_name.to_owned()))
}

// ---------------------------------------------------------------------------
// MCP TOML config helpers (Codex)
// ---------------------------------------------------------------------------

fn merge_toml_mcp(
    path: &Path,
    server_name: &str,
    dry_run: bool,
    display_name: &str,
) -> Result<PlatformResult> {
    let section_header = format!("[mcp_servers.{server_name}]");

    let existing = if path.exists() {
        fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?
    } else {
        String::new()
    };

    if existing.contains(&section_header) {
        return Ok(PlatformResult::AlreadyConfigured(display_name.to_owned()));
    }

    let section =
        format!("\n{section_header}\ncommand = \"atlas\"\nargs = [\"serve\"]\ntype = \"stdio\"\n");

    if dry_run {
        println!(
            "  [dry-run] {display_name}: would append to {}",
            path.display()
        );
        return Ok(PlatformResult::Configured(display_name.to_owned()));
    }

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create directory {}", parent.display()))?;
    }

    let content = if existing.is_empty() {
        section.trim_start_matches('\n').to_owned()
    } else {
        let prefix = if existing.ends_with('\n') {
            existing.clone()
        } else {
            format!("{existing}\n")
        };
        format!("{prefix}{section}")
    };

    fs::write(path, content).with_context(|| format!("cannot write {}", path.display()))?;

    Ok(PlatformResult::Configured(display_name.to_owned()))
}

// ---------------------------------------------------------------------------
// Git hooks
// ---------------------------------------------------------------------------

const HOOK_MARKER: &str = "atlas update # atlas-hook";
const PRE_COMMIT_HOOK_SCRIPT: &str = r#"
# Installed by atlas. Remove these lines to disable atlas graph updates.
if command -v atlas >/dev/null 2>&1; then
    atlas update || true
    atlas detect-changes || true
fi
"#;
const QUIET_HOOK_SCRIPT: &str = r#"
# Installed by atlas. Remove these lines to disable atlas graph updates.
if command -v atlas >/dev/null 2>&1; then
    atlas update || true
    atlas detect-changes --brief || true
fi
"#;

/// Install supported git hooks and return their paths.
pub fn install_git_hooks(repo_root: &Path, dry_run: bool) -> Result<Vec<PathBuf>> {
    let git_dir = repo_root.join(".git");
    if !git_dir.is_dir() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    for hook_name in ["pre-commit", "post-checkout", "post-merge", "post-rewrite"] {
        paths.push(install_git_hook(
            git_dir.join("hooks").join(hook_name),
            dry_run,
        )?);
    }

    Ok(paths)
}

fn install_git_hook(hook_path: PathBuf, dry_run: bool) -> Result<PathBuf> {
    let hook_name = hook_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("git hook");
    let hook_script = if hook_name == "pre-commit" {
        PRE_COMMIT_HOOK_SCRIPT
    } else {
        QUIET_HOOK_SCRIPT
    };

    let existing = if hook_path.exists() {
        fs::read_to_string(&hook_path)
            .with_context(|| format!("cannot read {}", hook_path.display()))?
    } else {
        String::new()
    };

    if existing.contains(HOOK_MARKER) {
        return Ok(hook_path);
    }

    if dry_run {
        println!(
            "  [dry-run] {hook_name}: would write {}",
            hook_path.display()
        );
        return Ok(hook_path);
    }

    if let Some(parent) = hook_path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }

    let content = if existing.is_empty() {
        format!("#!/bin/sh\n{HOOK_MARKER}\n{hook_script}")
    } else {
        let prefix = if existing.ends_with('\n') {
            existing.clone()
        } else {
            format!("{existing}\n")
        };
        format!("{prefix}{HOOK_MARKER}\n{hook_script}")
    };

    fs::write(&hook_path, &content)
        .with_context(|| format!("cannot write {}", hook_path.display()))?;

    // Make the hook executable on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&hook_path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("cannot chmod {}", hook_path.display()))?;
    }

    Ok(hook_path)
}

// ---------------------------------------------------------------------------
// Instruction injection (AGENTS.md / CLAUDE.md)
// ---------------------------------------------------------------------------

const INSTRUCTIONS_MARKER: &str = "<!-- atlas MCP tools -->";

const INSTRUCTIONS_SECTION: &str = r#"<!-- atlas MCP tools -->
## MCP Tools: atlas

**IMPORTANT: This project has a code knowledge graph. ALWAYS use the
atlas MCP tools BEFORE using file-search or grep to explore the codebase.**
The graph is faster, cheaper (fewer tokens), and gives you structural context
(callers, dependents, test coverage) that file scanning cannot.

### When to use atlas tools first

- **Exploring code**: `query` instead of Grep/Glob
- **Understanding impact**: `impact` instead of manually tracing imports
- **Code review**: `detect-changes` + `review-context` instead of reading entire files
- **Finding relationships**: graph traversal for callers/callees/tests

Fall back to file tools **only** when the graph does not cover what you need.

### Key tools

| Tool | Use when |
| ---- | -------- |
| `detect_changes` | Reviewing code changes — gives risk-scored analysis |
| `review_context` | Need source snippets for review — token-efficient |
| `impact_radius` | Understanding blast radius of a change |
| `query_graph` | Tracing callers, callees, imports, tests |
| `graph_stats` | Overall codebase metrics |

### Workflow

1. Start with MCP graph tools to get structural context.
2. Use `detect_changes` for code review.
3. Use `impact_radius` to assess change risk.
"#;

/// Inject atlas instructions into `AGENTS.md` and `CLAUDE.md` if present or
/// if the repo root directory needs the file created.  Returns the list of
/// files written or updated.
pub fn inject_instructions(repo_root: &Path, dry_run: bool) -> Result<Vec<String>> {
    let mut updated: Vec<String> = Vec::new();

    for filename in &["AGENTS.md", "CLAUDE.md"] {
        let path = repo_root.join(filename);

        // Only create AGENTS.md / CLAUDE.md when it does not exist yet;
        // always append when the file is already present.
        let existing = if path.exists() {
            fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?
        } else {
            // Only create the file if it does not exist.
            String::new()
        };

        if existing.contains(INSTRUCTIONS_MARKER) {
            // Already injected.
            continue;
        }

        if dry_run {
            if existing.is_empty() {
                println!("  [dry-run] would create {filename}");
            } else {
                println!("  [dry-run] would append atlas instructions to {filename}");
            }
            updated.push(filename.to_string());
            continue;
        }

        let separator = if existing.is_empty() {
            ""
        } else if existing.ends_with('\n') {
            "\n"
        } else {
            "\n\n"
        };

        fs::write(
            &path,
            format!("{existing}{separator}{INSTRUCTIONS_SECTION}"),
        )
        .with_context(|| format!("cannot write {}", path.display()))?;

        updated.push(filename.to_string());
    }

    Ok(updated)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn repo_with_git(dir: &Path) {
        fs::create_dir_all(dir.join(".git").join("hooks")).unwrap();
    }

    #[test]
    fn inject_instructions_creates_agents_md() {
        let tmp = TempDir::new().unwrap();
        // No AGENTS.md file initially.
        let files = inject_instructions(tmp.path(), false).unwrap();
        assert!(files.contains(&"AGENTS.md".to_owned()));
        let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(content.contains(INSTRUCTIONS_MARKER));
    }

    #[test]
    fn inject_instructions_idempotent() {
        let tmp = TempDir::new().unwrap();
        inject_instructions(tmp.path(), false).unwrap();
        let files2 = inject_instructions(tmp.path(), false).unwrap();
        // Second call should return empty — already injected.
        assert!(!files2.contains(&"AGENTS.md".to_owned()));
    }

    #[test]
    fn inject_instructions_appends_to_existing() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), b"# Existing content\n").unwrap();
        inject_instructions(tmp.path(), false).unwrap();
        let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(content.starts_with("# Existing content\n"));
        assert!(content.contains(INSTRUCTIONS_MARKER));
    }

    #[test]
    fn install_claude_writes_mcp_json() {
        let tmp = TempDir::new().unwrap();
        let result = install_claude(tmp.path(), false).unwrap();
        assert!(matches!(result, PlatformResult::Configured(_)));
        let mcp_path = tmp.path().join(".mcp.json");
        assert!(mcp_path.exists());
        let val: Value = serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
        assert!(val["mcpServers"]["atlas"]["command"] == "atlas");
    }

    #[test]
    fn install_claude_idempotent() {
        let tmp = TempDir::new().unwrap();
        install_claude(tmp.path(), false).unwrap();
        let result = install_claude(tmp.path(), false).unwrap();
        assert!(matches!(result, PlatformResult::AlreadyConfigured(_)));
    }

    #[test]
    fn install_copilot_writes_vscode_mcp_json() {
        let tmp = TempDir::new().unwrap();
        let result = install_copilot(tmp.path(), false).unwrap();
        assert!(matches!(result, PlatformResult::Configured(_)));
        let vscode_path = tmp.path().join(".vscode").join("mcp.json");
        assert!(vscode_path.exists());
        let val: Value = serde_json::from_str(&fs::read_to_string(&vscode_path).unwrap()).unwrap();
        assert!(val["servers"]["atlas"]["command"] == "atlas");
    }

    #[test]
    fn install_codex_writes_repo_local_config_toml() {
        let tmp = TempDir::new().unwrap();
        let result = install_codex(tmp.path(), false).unwrap();
        assert!(matches!(result, PlatformResult::Configured(_)));
        let codex_path = tmp.path().join(".codex").join("config.toml");
        assert!(codex_path.exists());
        let content = fs::read_to_string(&codex_path).unwrap();
        assert!(content.contains("[mcp_servers.atlas]"));
        assert!(content.contains("command = \"atlas\""));
    }

    #[test]
    fn install_codex_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        install_codex(tmp.path(), false).unwrap();
        let result = install_codex(tmp.path(), false).unwrap();
        assert!(matches!(result, PlatformResult::AlreadyConfigured(_)));
    }

    #[test]
    fn git_hooks_installed_and_executable() {
        let tmp = TempDir::new().unwrap();
        repo_with_git(tmp.path());
        let hooks = install_git_hooks(tmp.path(), false).unwrap();
        assert_eq!(hooks.len(), 4);
        for hook in hooks {
            let file_name = hook
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap()
                .to_owned();
            let content = fs::read_to_string(&hook).unwrap();
            assert!(content.contains(HOOK_MARKER));
            if file_name == "pre-commit" {
                assert!(content.contains("atlas detect-changes || true"));
                assert!(!content.contains("atlas detect-changes --brief || true"));
            } else {
                assert!(content.contains("atlas detect-changes --brief || true"));
            }
        }
    }

    #[test]
    fn git_hooks_idempotent() {
        let tmp = TempDir::new().unwrap();
        repo_with_git(tmp.path());
        install_git_hooks(tmp.path(), false).unwrap();
        let hooks = install_git_hooks(tmp.path(), false).unwrap();
        for hook in hooks {
            let content = fs::read_to_string(hook).unwrap();
            assert_eq!(content.matches(HOOK_MARKER).count(), 1);
        }
    }

    #[test]
    fn install_summary_reports_all_git_hooks() {
        let tmp = TempDir::new().unwrap();
        repo_with_git(tmp.path());
        let summary = run_install(tmp.path(), "claude", false, false, true).unwrap();
        assert_eq!(summary.hook_paths.len(), 4);
        assert!(
            summary
                .hook_paths
                .iter()
                .any(|path| path.ends_with("pre-commit"))
        );
        assert!(
            summary
                .hook_paths
                .iter()
                .any(|path| path.ends_with("post-checkout"))
        );
        assert!(
            summary
                .hook_paths
                .iter()
                .any(|path| path.ends_with("post-merge"))
        );
        assert!(
            summary
                .hook_paths
                .iter()
                .any(|path| path.ends_with("post-rewrite"))
        );
    }

    #[test]
    fn dry_run_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        repo_with_git(tmp.path());
        run_install(tmp.path(), "claude", true, false, false).unwrap();
        assert!(!tmp.path().join(".mcp.json").exists());
        assert!(!tmp.path().join("AGENTS.md").exists());
    }
}
