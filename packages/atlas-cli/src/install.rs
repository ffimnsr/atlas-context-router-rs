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
use serde::Serialize;
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
    /// Platform agent hook config files written or updated.
    pub platform_hook_files: Vec<String>,
    /// Validation checks for installed or existing files.
    pub validation_checks: Vec<InstallValidation>,
    /// Selected install scope.
    pub scope: String,
    /// Whether install ran in validate-only mode.
    pub validate_only: bool,
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

/// Run the full install flow for the given `platform` target.
///
/// `repo_root` — the repository root directory.
/// `platform`  — `"all"`, `"copilot"`, `"claude"`, or `"codex"`.
/// `platform`  — `"all"`, `"copilot"`, `"claude"`, or `"codex"`.
/// `dry_run`   — when `true`, describe changes without writing files.
/// `no_hooks`  — skip git hooks and platform agent hook config installation.
/// `no_instructions` — skip injecting platform-specific agent instructions.
pub fn run_install(
    repo_root: &Path,
    platform: &str,
    scope: &str,
    dry_run: bool,
    validate_only: bool,
    no_hooks: bool,
    no_instructions: bool,
) -> Result<InstallSummary> {
    let mut summary = InstallSummary::default();
    let scope = InstallScope::parse(scope);
    let scope_root = scope_root(repo_root, scope)?;
    summary.scope = scope.as_str().to_owned();
    summary.validate_only = validate_only;

    for name in ["copilot", "claude", "codex"] {
        let skip = match platform {
            "all" => !should_auto_detect(name, repo_root),
            p => p != name,
        };
        if skip {
            continue;
        }

        if validate_only {
            continue;
        }

        let result = match name {
            "copilot" => install_copilot_scoped(repo_root, &scope_root, scope, dry_run),
            "claude" => install_claude_scoped(repo_root, &scope_root, scope, dry_run),
            "codex" => install_codex_scoped(repo_root, &scope_root, scope, dry_run),
            _ => unreachable!(),
        }?;

        match result {
            PlatformResult::Configured(label) => summary.configured.push(label),
            PlatformResult::AlreadyConfigured(label) => summary.already_configured.push(label),
        }
    }

    if !no_hooks {
        summary.hook_paths = install_git_hooks(repo_root, dry_run)?;
        if !validate_only {
            summary.platform_hook_files = install_platform_agent_hooks_scoped(
                repo_root,
                &scope_root,
                platform,
                scope,
                dry_run,
            )?;
        }
    }

    if !no_instructions && !validate_only {
        let files = inject_instructions(
            repo_root,
            &instruction_targets(platform, repo_root),
            dry_run,
        )?;
        summary.instruction_files = files;
    }

    if !dry_run || validate_only {
        summary.validation_checks = validate_install(
            repo_root,
            &scope_root,
            platform,
            scope,
            no_hooks,
            no_instructions,
        )?;
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

fn scope_root(repo_root: &Path, scope: InstallScope) -> Result<PathBuf> {
    match scope {
        InstallScope::Repo => Ok(repo_root.to_path_buf()),
        InstallScope::User => user_home_dir(),
    }
}

fn user_home_dir() -> Result<PathBuf> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .or_else(|_| {
            #[allow(deprecated)]
            std::env::home_dir().ok_or(std::env::VarError::NotPresent)
        })
        .context("cannot determine home directory")
}

fn display_scoped_path(
    path: &Path,
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
) -> String {
    let normalized = match scope {
        InstallScope::Repo => path
            .strip_prefix(repo_root)
            .unwrap_or(path)
            .to_string_lossy()
            .into_owned(),
        InstallScope::User => path
            .strip_prefix(scope_root)
            .map(|rest| format!("~/{}", rest.to_string_lossy()))
            .unwrap_or_else(|_| path.display().to_string()),
    };
    normalized.replace('\\', "/")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn atlas_hook_command(
    install_root: &Path,
    scope: InstallScope,
    frontend: &str,
    arg: &str,
) -> String {
    match scope {
        InstallScope::Repo => format!(".atlas/hooks/atlas-hook {frontend} {arg}"),
        InstallScope::User => {
            let runner = install_root.join(".atlas").join("hooks").join("atlas-hook");
            format!(
                "{} {frontend} {arg}",
                shell_quote(&runner.display().to_string())
            )
        }
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
#[cfg(test)]
fn install_copilot(repo_root: &Path, dry_run: bool) -> Result<PlatformResult> {
    install_copilot_scoped(repo_root, repo_root, InstallScope::Repo, dry_run)
}

fn install_copilot_scoped(
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
    dry_run: bool,
) -> Result<PlatformResult> {
    let config_path = match scope {
        InstallScope::Repo => scope_root.join(".vscode").join("mcp.json"),
        InstallScope::User => scope_root
            .join(".config")
            .join("Code")
            .join("User")
            .join("mcp.json"),
    };
    let server_entry = copilot_server_entry(repo_root);
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
#[cfg(test)]
fn install_claude(repo_root: &Path, dry_run: bool) -> Result<PlatformResult> {
    install_claude_scoped(repo_root, repo_root, InstallScope::Repo, dry_run)
}

fn install_claude_scoped(
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
    dry_run: bool,
) -> Result<PlatformResult> {
    let config_path = match scope {
        InstallScope::Repo => scope_root.join(".mcp.json"),
        InstallScope::User => scope_root.join(".mcp.json"),
    };
    let server_entry = stdio_server_entry(repo_root);
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
#[cfg(test)]
fn install_codex(repo_root: &Path, dry_run: bool) -> Result<PlatformResult> {
    install_codex_scoped(repo_root, repo_root, InstallScope::Repo, dry_run)
}

fn install_codex_scoped(
    repo_root: &Path,
    scope_root: &Path,
    scope: InstallScope,
    dry_run: bool,
) -> Result<PlatformResult> {
    let config_path = match scope {
        InstallScope::Repo => scope_root.join(".codex").join("config.toml"),
        InstallScope::User => scope_root.join(".codex").join("config.toml"),
    };
    merge_toml_mcp(&config_path, repo_root, "atlas", dry_run, "Codex")
}

// ---------------------------------------------------------------------------
// MCP JSON config helpers
// ---------------------------------------------------------------------------

fn stdio_server_args(repo_root: &Path) -> Vec<String> {
    vec![
        "--repo".to_owned(),
        repo_root.display().to_string(),
        "--db".to_owned(),
        repo_root
            .join(".atlas")
            .join("worldtree.db")
            .display()
            .to_string(),
        "serve".to_owned(),
    ]
}

fn toml_basic_string(value: &str) -> String {
    format!(
        "\"{}\"",
        value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r")
            .replace('\t', "\\t")
    )
}

fn toml_string_array(values: &[String]) -> String {
    format!(
        "[{}]",
        values
            .iter()
            .map(|value| toml_basic_string(value))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn is_legacy_json_server_entry(entry: &Value) -> bool {
    let Value::Object(map) = entry else {
        return false;
    };
    map.get("command").and_then(Value::as_str) == Some("atlas")
        && map.get("args") == Some(&serde_json::json!(["serve"]))
}

fn section_range(existing: &str, header: &str) -> Option<(usize, usize)> {
    let start = existing.find(header)?;
    let after_header = start + header.len();
    let rest = &existing[after_header..];
    let next_section = rest
        .match_indices('\n')
        .find(|(idx, _)| {
            let line = &rest[idx + 1..];
            line.starts_with('[')
        })
        .map(|(idx, _)| after_header + idx + 1)
        .unwrap_or(existing.len());
    Some((start, next_section))
}

fn parse_toml_string_array(value: &str) -> Option<Vec<String>> {
    let trimmed = value.trim();
    if !trimmed.starts_with('[') || !trimmed.ends_with(']') {
        return None;
    }

    let inner = &trimmed[1..trimmed.len() - 1];
    if inner.trim().is_empty() {
        return Some(Vec::new());
    }

    let mut values = Vec::new();
    let mut chars = inner.chars().peekable();

    while let Some(ch) = chars.peek() {
        if ch.is_whitespace() || *ch == ',' {
            chars.next();
            continue;
        }
        if *ch != '"' {
            return None;
        }
        chars.next();

        let mut value = String::new();
        while let Some(ch) = chars.next() {
            match ch {
                '"' => break,
                '\\' => {
                    let escaped = chars.next()?;
                    value.push(match escaped {
                        '\\' => '\\',
                        '"' => '"',
                        'n' => '\n',
                        'r' => '\r',
                        't' => '\t',
                        _ => return None,
                    });
                }
                other => value.push(other),
            }
        }
        values.push(value);
    }

    Some(values)
}

fn is_legacy_toml_section(section: &str) -> bool {
    let mut command = None;
    let mut args = None;

    for line in section.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with('[') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        match key.trim() {
            "command" => {
                command = Some(value.trim().trim_matches('"').to_owned());
            }
            "args" => {
                args = parse_toml_string_array(value);
            }
            _ => {}
        }
    }

    command.as_deref() == Some("atlas") && args.as_deref() == Some(&["serve".to_owned()])
}

/// Server entry for VS Code / GitHub Copilot (uses `"type": "stdio"`).
fn copilot_server_entry(repo_root: &Path) -> Value {
    serde_json::json!({
        "type": "stdio",
        "command": "atlas",
        "args": stdio_server_args(repo_root)
    })
}

/// Generic stdio server entry for other platforms (Claude Code, etc.).
fn stdio_server_entry(repo_root: &Path) -> Value {
    serde_json::json!({
        "type": "stdio",
        "command": "atlas",
        "args": stdio_server_args(repo_root)
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

    let mut changed = false;

    if let Value::Object(map) = servers {
        if let Some(existing) = map.get(server_name) {
            if existing == &server_entry {
                return Ok(PlatformResult::AlreadyConfigured(display_name.to_owned()));
            }
            if is_legacy_json_server_entry(existing) {
                map.insert(server_name.to_owned(), server_entry);
                changed = true;
            } else {
                return Ok(PlatformResult::AlreadyConfigured(display_name.to_owned()));
            }
        } else {
            map.insert(server_name.to_owned(), server_entry);
            changed = true;
        }
    }

    if !changed {
        return Ok(PlatformResult::AlreadyConfigured(display_name.to_owned()));
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
    repo_root: &Path,
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

    let args = toml_string_array(&stdio_server_args(repo_root));
    let section = format!(
        "\n{section_header}\ncommand = \"atlas\"\nargs = {}\ntype = \"stdio\"\n",
        args
    );

    if let Some((start, end)) = section_range(&existing, &section_header) {
        let current_section = &existing[start..end];
        if !is_legacy_toml_section(current_section) {
            return Ok(PlatformResult::AlreadyConfigured(display_name.to_owned()));
        }

        let mut content = String::new();
        content.push_str(&existing[..start]);
        content.push_str(section.trim_start_matches('\n'));
        if end < existing.len() && !existing[end..].starts_with('\n') && !content.ends_with('\n') {
            content.push('\n');
        }
        content.push_str(&existing[end..]);

        if dry_run {
            println!(
                "  [dry-run] {display_name}: would update {}",
                path.display()
            );
            return Ok(PlatformResult::Configured(display_name.to_owned()));
        }

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create directory {}", parent.display()))?;
        }

        fs::write(path, content).with_context(|| format!("cannot write {}", path.display()))?;

        return Ok(PlatformResult::Configured(display_name.to_owned()));
    }

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

const LEGACY_HOOK_MARKER: &str = "atlas update # atlas-hook";
const HOOK_START_MARKER: &str = "# atlas-hook start";
const HOOK_END_MARKER: &str = "# atlas-hook end";
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

    let next_content = upsert_hook_block(&existing, hook_script);
    if next_content == existing {
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

    fs::write(&hook_path, &next_content)
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

fn upsert_hook_block(existing: &str, hook_script: &str) -> String {
    let managed_block = format!("{HOOK_START_MARKER}\n{hook_script}{HOOK_END_MARKER}\n");

    if let Some((start, end)) = managed_hook_range(existing) {
        let mut updated = String::new();
        updated.push_str(&existing[..start]);
        updated.push_str(&managed_block);
        updated.push_str(&existing[end..]);
        return updated;
    }

    if let Some(start) = legacy_hook_start(existing) {
        let mut updated = String::new();
        updated.push_str(&existing[..start]);
        updated.push_str(&managed_block);
        return updated;
    }

    if existing.is_empty() {
        return format!("#!/bin/sh\n{managed_block}");
    }

    let prefix = if existing.ends_with('\n') {
        existing.to_owned()
    } else {
        format!("{existing}\n")
    };
    format!("{prefix}{managed_block}")
}

fn managed_hook_range(existing: &str) -> Option<(usize, usize)> {
    let start = existing.find(HOOK_START_MARKER)?;
    let mut end = existing[start..]
        .find(HOOK_END_MARKER)
        .map(|offset| start + offset + HOOK_END_MARKER.len())?;
    if existing[end..].starts_with("\r\n") {
        end += 2;
    } else if existing[end..].starts_with('\n') {
        end += 1;
    }
    Some((start, end))
}

fn legacy_hook_start(existing: &str) -> Option<usize> {
    existing.find(LEGACY_HOOK_MARKER).map(|offset| {
        existing[..offset]
            .rfind('\n')
            .map(|idx| idx + 1)
            .unwrap_or(0)
    })
}

// ---------------------------------------------------------------------------
// Instruction injection (AGENTS.md / CLAUDE.md)
// ---------------------------------------------------------------------------

const INSTRUCTIONS_MARKER: &str = "<!-- atlas MCP tools -->";
const INSTRUCTIONS_END_MARKER: &str = "<!-- /atlas MCP tools -->";

const INSTRUCTIONS_SECTION: &str = r#"<!-- atlas MCP tools -->
## MCP Tools: atlas

**IMPORTANT: This project has a code knowledge graph. ALWAYS use the
atlas MCP tools BEFORE using file-search or grep to explore the codebase.**
The graph is faster, cheaper (fewer tokens), and gives you structural context
(callers, dependents, test coverage) that file scanning cannot.

### When to use atlas tools first

- **Exploring code**: `query_graph` to find candidate symbols, then `symbol_neighbors`, `traverse_graph`, or `get_context` for callers/callees and usage relationships
- **Searching non-symbol content**: `search_files`, `search_content`, `search_templates`, and `search_text_assets` when graph lookup is the wrong tool
- **Understanding impact**: `get_impact_radius` for blast radius, `explain_change` for richer risk analysis
- **Code review**: `detect_changes` + `get_review_context`, or `get_minimal_context` when tokens matter
- **Finding relationships**: `symbol_neighbors` for immediate usage edges, `traverse_graph` for broader callers/callees, and `get_context` for intent-aware usage lookup
- **Repo health**: `list_graph_stats`, `status`, `doctor`, `db_check`, and `debug_graph` before trusting graph-backed answers
- **Session continuity**: `get_session_status`, `resume_session`, `search_saved_context`, and `save_context_artifact`

Do not treat `query_graph` as caller/callee search. Fall back to file tools **only** after graph relationship tools do not cover what you need.

### Tool list

| Tool | Use when |
| ---- | -------- |
| `list_graph_stats` | Overall graph metrics and language breakdown |
| `query_graph` | Search graph nodes by keyword, kind, or language; returns symbol matches, not usage edges |
| `batch_query_graph` | Run up to 20 query_graph searches in one call |
| `search_files` | Find config, template, SQL, Markdown, and other files by path or glob |
| `search_content` | Search file contents when you need text matches instead of graph symbols |
| `search_templates` | Discover template files by engine or extension |
| `search_text_assets` | Find SQL, config, env, and prompt files outside graph-symbol lookup |
| `status` | Check graph health and basic build state before trusting graph-backed answers |
| `doctor` | Run deeper repo, config, DB, and index health checks |
| `db_check` | Validate SQLite integrity and detect orphan or dangling graph records |
| `debug_graph` | Inspect node and edge breakdowns plus structural anomalies |
| `explain_query` | See how query_graph tokenizes and executes a search |
| `resolve_symbol` | Resolve a symbol or alias-qualified name to a canonical qualified_name |
| `analyze_safety` | Score refactor safety using callers, fan-out, and test adjacency |
| `analyze_remove` | Estimate removal impact with bounded evidence and warnings |
| `analyze_dead_code` | Find likely dead-code candidates with certainty and blockers |
| `analyze_dependency` | Check whether a symbol can be removed without remaining references |
| `get_impact_radius` | Understand blast radius from a changed file set |
| `get_review_context` | Build review bundle with symbols, neighbors, edges, and risk summary |
| `detect_changes` | Ask Atlas for changed files instead of shelling out to git |
| `build_or_update_graph` | Trigger full graph build or incremental update |
| `traverse_graph` | Walk callers, callees, and nearby nodes from known qualified name |
| `get_minimal_context` | Get lower-token review context with auto-detected changes |
| `explain_change` | Get deterministic risk analysis, change kinds, and test gaps |
| `get_context` | Build bounded context around symbol, file, or change-set |
| `get_session_status` | Inspect current session identity, event count, and resume state |
| `resume_session` | Restore prior session snapshot after reconnect or restart |
| `search_saved_context` | Search saved artifacts from earlier large outputs |
| `read_saved_context` | Retrieve full artifact content by source_id with optional paging |
| `save_context_artifact` | Persist large context payloads for later retrieval |
| `get_context_stats` | Inspect session and content-store stats |
| `purge_saved_context` | Remove saved artifacts by session or age |
| `symbol_neighbors` | Inspect immediate callers, callees, tests, and local graph neighborhood |
| `cross_file_links` | Find files coupled to a file through shared symbol references |
| `concept_clusters` | Group related files around seed files by coupling density |

### Workflow

1. Start with MCP graph tools to get structural context.
2. For usage questions, use `query_graph` to find the qualified name, then `symbol_neighbors`, `traverse_graph`, or `get_context`.
3. Use `detect_changes` to identify changed files.
4. Use `get_review_context` or `get_minimal_context` for review.
5. Use `get_impact_radius` or `explain_change` to assess change risk.

### Trust Metadata

- Every MCP tool response includes `atlas_provenance` with `repo_root`, `db_path`, `indexed_file_count`, and `last_indexed_at`.
- Some graph-backed tools also include `atlas_freshness` when working-tree edits may make graph results stale.
- If `atlas_provenance.repo_root` does not match current workspace, stop and verify session wiring.
- If `atlas_provenance.db_path` points at unexpected database, stop and call `status` or `doctor` before trusting results.
- If `atlas_freshness.stale` is true, prefer `build_or_update_graph` before making claims about current code.
- Treat missing optional `atlas_*` fields as "not applicable", not as tool failure.
<!-- /atlas MCP tools -->
"#;

fn instruction_targets(platform: &str, repo_root: &Path) -> Vec<&'static str> {
    let mut targets = Vec::new();

    let wants_agents = match platform {
        "copilot" | "codex" => true,
        "all" => should_auto_detect("copilot", repo_root) || should_auto_detect("codex", repo_root),
        _ => false,
    };
    let wants_claude = match platform {
        "claude" => true,
        "all" => should_auto_detect("claude", repo_root),
        _ => false,
    };

    if wants_agents {
        targets.push("AGENTS.md");
    }
    if wants_claude {
        targets.push("CLAUDE.md");
    }

    targets
}

/// Inject atlas instructions into `AGENTS.md` and `CLAUDE.md` if present or
/// if the repo root directory needs the file created.  Returns the list of
/// files written or updated.
pub fn inject_instructions(
    repo_root: &Path,
    filenames: &[&str],
    dry_run: bool,
) -> Result<Vec<String>> {
    let mut updated: Vec<String> = Vec::new();

    for filename in filenames {
        let path = repo_root.join(filename);

        // Only create AGENTS.md / CLAUDE.md when it does not exist yet;
        // always append when the file is already present.
        let existing = if path.exists() {
            fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?
        } else {
            // Only create the file if it does not exist.
            String::new()
        };

        let next_content = upsert_instructions_section(&existing);
        if next_content == existing {
            continue;
        }

        if dry_run {
            if existing.is_empty() {
                println!("  [dry-run] would create {filename}");
            } else if existing.contains(INSTRUCTIONS_MARKER) {
                println!("  [dry-run] would refresh atlas instructions in {filename}");
            } else {
                println!("  [dry-run] would append atlas instructions to {filename}");
            }
            updated.push(filename.to_string());
            continue;
        }

        fs::write(&path, next_content)
            .with_context(|| format!("cannot write {}", path.display()))?;

        updated.push(filename.to_string());
    }

    Ok(updated)
}

fn upsert_instructions_section(existing: &str) -> String {
    if let Some((start, end)) = instruction_section_range(existing) {
        let mut updated = String::new();
        let prefix = &existing[..start];
        updated.push_str(prefix);
        if !prefix.is_empty() && !prefix.ends_with('\n') {
            updated.push_str("\n\n");
        }
        updated.push_str(INSTRUCTIONS_SECTION);
        let suffix = &existing[end..];
        if !suffix.is_empty() {
            if !updated.ends_with('\n') && !suffix.starts_with('\n') {
                updated.push('\n');
            }
            updated.push_str(suffix);
        }
        return updated;
    }

    let separator = if existing.is_empty() {
        ""
    } else if existing.ends_with('\n') {
        "\n"
    } else {
        "\n\n"
    };

    format!("{existing}{separator}{INSTRUCTIONS_SECTION}")
}

fn instruction_section_range(existing: &str) -> Option<(usize, usize)> {
    let start = existing.find(INSTRUCTIONS_MARKER)?;
    let mut end = existing[start..]
        .find(INSTRUCTIONS_END_MARKER)
        .map(|offset| start + offset + INSTRUCTIONS_END_MARKER.len())
        .unwrap_or(existing.len());
    if existing[end..].starts_with("\r\n") {
        end += 2;
    } else if existing[end..].starts_with('\n') {
        end += 1;
    }
    Some((start, end))
}

// ---------------------------------------------------------------------------
// Platform agent hooks
// ---------------------------------------------------------------------------

/// Atlas hook runner script installed at `.atlas/hooks/atlas-hook`.
///
/// Called by platform hook configs with one event argument.
/// Reads JSON from stdin (capped at 64 KiB), calls atlas CLI commands.
/// All failures are non-blocking.
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

/// Marker used to detect whether a platform hook config already has atlas hooks.
const ATLAS_HOOK_MARKER: &str = "atlas-hook";
const ATLAS_HOOK_LIB_REL: &str = ".atlas/hooks/lib/";

/// Copilot hook events — VS Code PascalCase names.
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

/// Claude hook event — name, atlas-hook event arg, and optional matcher.
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

/// Codex hook event — name, atlas-hook event arg, and optional matcher.
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

/// Install the shared atlas hook runner and all platform-specific hook configs.
pub fn install_platform_agent_hooks(
    repo_root: &Path,
    platform: &str,
    dry_run: bool,
) -> Result<Vec<String>> {
    install_platform_agent_hooks_scoped(repo_root, repo_root, platform, InstallScope::Repo, dry_run)
}

fn install_platform_agent_hooks_scoped(
    repo_root: &Path,
    scope_root: &Path,
    platform: &str,
    scope: InstallScope,
    dry_run: bool,
) -> Result<Vec<String>> {
    let mut files = Vec::new();

    let run_for = |name: &str| -> bool {
        match platform {
            "all" => should_auto_detect(name, repo_root),
            p => p == name,
        }
    };

    let needs_any = run_for("copilot") || run_for("claude") || run_for("codex");
    if !needs_any {
        return Ok(files);
    }

    // Install the shared hook runner first.
    if let Some(runner_path) =
        install_atlas_hook_runner_scoped(repo_root, scope_root, scope, dry_run)?
    {
        files.push(runner_path);
    }

    if run_for("copilot")
        && let Some(p) = install_copilot_agent_hooks_scoped(repo_root, scope_root, scope, dry_run)?
    {
        files.push(p);
    }
    if run_for("claude")
        && let Some(p) = install_claude_agent_hooks_scoped(repo_root, scope_root, scope, dry_run)?
    {
        files.push(p);
    }
    if run_for("codex")
        && let Some(p) = install_codex_agent_hooks_scoped(repo_root, scope_root, scope, dry_run)?
    {
        files.push(p);
    }

    Ok(files)
}

/// Install `.atlas/hooks/atlas-hook` in `repo_root`.
/// Returns the display path when a write occurred (or dry-run describes it).
#[cfg(test)]
fn install_atlas_hook_runner(repo_root: &Path, dry_run: bool) -> Result<Option<String>> {
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

/// Generate the Copilot hook config JSON value.
#[cfg(test)]
fn copilot_hooks_value() -> Value {
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

/// Install `.github/hooks/atlas-copilot.json`.
#[cfg(test)]
fn install_copilot_agent_hooks(repo_root: &Path, dry_run: bool) -> Result<Option<String>> {
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
            return Ok(None); // already configured
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

/// Generate the Claude `settings.json` hooks object value.
#[cfg(test)]
fn claude_hooks_value() -> Value {
    claude_hooks_value_scoped(Path::new("."), InstallScope::Repo)
}

fn claude_hooks_value_scoped(install_root: &Path, scope: InstallScope) -> Value {
    let mut hooks_map = serde_json::Map::new();
    for (event, frontend, arg, matcher) in CLAUDE_HOOKS {
        let cmd = atlas_hook_command(install_root, scope, frontend, arg);
        let entry = if let Some(m) = matcher {
            serde_json::json!([{
                "matcher": m,
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

/// Install or merge atlas hooks into `.claude/settings.json`.
#[cfg(test)]
fn install_claude_agent_hooks(repo_root: &Path, dry_run: bool) -> Result<Option<String>> {
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
            return Ok(None); // already configured
        }
        match serde_json::from_str::<Value>(&text) {
            Ok(Value::Object(m)) => m,
            _ => serde_json::Map::new(),
        }
    } else {
        serde_json::Map::new()
    };

    // Merge atlas hooks into any existing hooks object.
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
        for (k, v) in atlas_hooks_map {
            existing_hooks.insert(k, v);
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

/// Generate the Codex `hooks.json` value.
#[cfg(test)]
fn codex_hooks_value() -> Value {
    codex_hooks_value_scoped(Path::new("."), InstallScope::Repo)
}

fn codex_hooks_value_scoped(install_root: &Path, scope: InstallScope) -> Value {
    let mut hooks_map = serde_json::Map::new();
    for (event, frontend, arg, matcher) in CODEX_HOOKS {
        let cmd = atlas_hook_command(install_root, scope, frontend, arg);
        let entry = if let Some(m) = matcher {
            serde_json::json!([{
                "matcher": m,
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

/// Install `.codex/hooks.json`.
#[cfg(test)]
fn install_codex_agent_hooks(repo_root: &Path, dry_run: bool) -> Result<Option<String>> {
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

fn validate_install(
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
            p => p != name,
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
                p => p != name,
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
                ok: parses && text.contains("atlas-hook"),
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
            ok: text.contains(HOOK_START_MARKER) && text.contains(HOOK_END_MARKER),
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn repo_with_git(dir: &Path) {
        fs::create_dir_all(dir.join(".git").join("hooks")).unwrap();
    }

    #[test]
    fn inject_instructions_creates_agents_md() {
        let tmp = TempDir::new().unwrap();
        // No AGENTS.md file initially.
        let files = inject_instructions(tmp.path(), &["AGENTS.md"], false).unwrap();
        assert!(files.contains(&"AGENTS.md".to_owned()));
        let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(content.contains(INSTRUCTIONS_MARKER));
    }

    #[test]
    fn inject_instructions_idempotent() {
        let tmp = TempDir::new().unwrap();
        inject_instructions(tmp.path(), &["AGENTS.md"], false).unwrap();
        let files2 = inject_instructions(tmp.path(), &["AGENTS.md"], false).unwrap();
        // Second call should return empty — already injected.
        assert!(!files2.contains(&"AGENTS.md".to_owned()));
    }

    #[test]
    fn inject_instructions_appends_to_existing() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), b"# Existing content\n").unwrap();
        inject_instructions(tmp.path(), &["AGENTS.md"], false).unwrap();
        let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
        assert!(content.starts_with("# Existing content\n"));
        assert!(content.contains(INSTRUCTIONS_MARKER));
    }

    #[test]
    fn instruction_targets_match_platform() {
        let tmp = TempDir::new().unwrap();

        assert_eq!(instruction_targets("codex", tmp.path()), vec!["AGENTS.md"]);
        assert_eq!(
            instruction_targets("copilot", tmp.path()),
            vec!["AGENTS.md"]
        );
        assert_eq!(instruction_targets("claude", tmp.path()), vec!["CLAUDE.md"]);
    }

    #[test]
    fn instructions_section_mentions_current_mcp_tools() {
        let documented = instruction_section_tool_names();
        let exported = exported_mcp_tool_names();

        assert_eq!(documented, exported);
    }

    #[test]
    fn instructions_section_documents_trust_metadata() {
        assert!(INSTRUCTIONS_SECTION.contains("atlas_provenance"));
        assert!(INSTRUCTIONS_SECTION.contains("atlas_freshness"));
        assert!(INSTRUCTIONS_SECTION.contains("status` or `doctor"));
    }

    #[test]
    fn inject_instructions_replaces_stale_section() {
        let tmp = TempDir::new().unwrap();
        let stale = format!("# Existing content\n\n{INSTRUCTIONS_MARKER}\nold stale block\n");
        fs::write(tmp.path().join("AGENTS.md"), stale).unwrap();

        let files = inject_instructions(tmp.path(), &["AGENTS.md"], false).unwrap();
        let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();

        assert!(files.contains(&"AGENTS.md".to_owned()));
        assert!(content.contains(INSTRUCTIONS_END_MARKER));
        assert!(content.contains("`concept_clusters`"));
        assert!(!content.contains("old stale block"));
    }

    #[test]
    fn inject_instructions_replaces_marked_section_without_touching_suffix() {
        let tmp = TempDir::new().unwrap();
        let stale = format!(
            "# Existing content\n\n{INSTRUCTIONS_MARKER}\nold stale block\n{INSTRUCTIONS_END_MARKER}\n\n## User Notes\nkeep me\n"
        );
        fs::write(tmp.path().join("AGENTS.md"), stale).unwrap();

        inject_instructions(tmp.path(), &["AGENTS.md"], false).unwrap();
        let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();

        assert!(content.contains("## User Notes\nkeep me\n"));
        assert!(!content.contains("old stale block"));
    }

    #[test]
    fn readme_mcp_tools_match_exported_registry() {
        assert_doc_mcp_tools_match_exported_registry(
            &repo_root().join("README.md"),
            "## MCP Tools",
            true,
        );
    }

    #[test]
    fn wiki_mcp_reference_tools_match_exported_registry() {
        assert_doc_mcp_tools_match_exported_registry(
            &repo_root().join("wiki").join("mcp-reference.md"),
            "## Tool List",
            false,
        );
    }

    fn instruction_section_tool_names() -> BTreeSet<String> {
        INSTRUCTIONS_SECTION
            .lines()
            .filter(|line| line.starts_with("| `"))
            .filter_map(|line| line.split('`').nth(1))
            .map(str::to_owned)
            .collect()
    }

    fn exported_mcp_tool_names() -> BTreeSet<String> {
        atlas_mcp::tool_list()["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .map(str::to_owned)
            .collect()
    }

    fn assert_doc_mcp_tools_match_exported_registry(path: &Path, heading: &str, required: bool) {
        let Some(documented) = markdown_table_tool_names(path, heading, required) else {
            return;
        };

        assert_eq!(documented, exported_mcp_tool_names());
    }

    fn markdown_table_tool_names(
        path: &Path,
        heading: &str,
        required: bool,
    ) -> Option<BTreeSet<String>> {
        if !path.is_file() {
            if required {
                panic!("documentation file missing: {}", path.display());
            }
            eprintln!(
                "skipping MCP registry documentation check; file not present at {}",
                path.display()
            );
            return None;
        }

        let text = fs::read_to_string(path).unwrap();
        let section = text
            .split(heading)
            .nth(1)
            .unwrap()
            .split("\n## ")
            .next()
            .unwrap();

        Some(
            section
                .lines()
                .filter(|line| line.starts_with("| `"))
                .filter_map(|line| line.split('`').nth(1))
                .map(str::to_owned)
                .collect(),
        )
    }

    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf()
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
        assert_eq!(
            val["mcpServers"]["atlas"]["args"],
            serde_json::json!([
                "--repo",
                tmp.path().display().to_string(),
                "--db",
                tmp.path()
                    .join(".atlas")
                    .join("worldtree.db")
                    .display()
                    .to_string(),
                "serve"
            ])
        );
    }

    #[test]
    fn install_claude_idempotent() {
        let tmp = TempDir::new().unwrap();
        install_claude(tmp.path(), false).unwrap();
        let result = install_claude(tmp.path(), false).unwrap();
        assert!(matches!(result, PlatformResult::AlreadyConfigured(_)));
    }

    #[test]
    fn install_claude_migrates_legacy_mcp_json() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join(".mcp.json"),
            r#"{
    "mcpServers": {
        "atlas": {
            "type": "stdio",
            "command": "atlas",
            "args": ["serve"]
        }
    }
}
"#,
        )
        .unwrap();

        let result = install_claude(tmp.path(), false).unwrap();
        assert!(matches!(result, PlatformResult::Configured(_)));

        let val: Value =
            serde_json::from_str(&fs::read_to_string(tmp.path().join(".mcp.json")).unwrap())
                .unwrap();
        assert_eq!(
            val["mcpServers"]["atlas"]["args"],
            serde_json::json!([
                "--repo",
                tmp.path().display().to_string(),
                "--db",
                tmp.path()
                    .join(".atlas")
                    .join("worldtree.db")
                    .display()
                    .to_string(),
                "serve"
            ])
        );
    }

    #[test]
    fn install_claude_preserves_custom_mcp_json() {
        let tmp = TempDir::new().unwrap();
        let custom = r#"{
    "mcpServers": {
        "atlas": {
            "type": "stdio",
            "command": "atlas",
            "args": ["--repo", "/custom/repo", "serve"]
        }
    }
}
"#;
        fs::write(tmp.path().join(".mcp.json"), custom).unwrap();

        let result = install_claude(tmp.path(), false).unwrap();
        assert!(matches!(result, PlatformResult::AlreadyConfigured(_)));
        assert_eq!(
            fs::read_to_string(tmp.path().join(".mcp.json")).unwrap(),
            custom
        );
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
        assert_eq!(
            val["servers"]["atlas"]["args"],
            serde_json::json!([
                "--repo",
                tmp.path().display().to_string(),
                "--db",
                tmp.path()
                    .join(".atlas")
                    .join("worldtree.db")
                    .display()
                    .to_string(),
                "serve"
            ])
        );
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
        assert!(content.contains(&format!(
            "args = [\"--repo\", \"{}\", \"--db\", \"{}\", \"serve\"]",
            tmp.path().display(),
            tmp.path().join(".atlas").join("worldtree.db").display()
        )));
    }

    #[test]
    fn install_codex_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        install_codex(tmp.path(), false).unwrap();
        let result = install_codex(tmp.path(), false).unwrap();
        assert!(matches!(result, PlatformResult::AlreadyConfigured(_)));
    }

    #[test]
    fn install_codex_migrates_legacy_config_toml() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        fs::write(
            tmp.path().join(".codex").join("config.toml"),
            "[mcp_servers.atlas]\ncommand = \"atlas\"\nargs = [\"serve\"]\ntype = \"stdio\"\n",
        )
        .unwrap();

        let result = install_codex(tmp.path(), false).unwrap();
        assert!(matches!(result, PlatformResult::Configured(_)));

        let content = fs::read_to_string(tmp.path().join(".codex").join("config.toml")).unwrap();
        assert!(content.contains(&format!(
            "args = [\"--repo\", \"{}\", \"--db\", \"{}\", \"serve\"]",
            tmp.path().display(),
            tmp.path().join(".atlas").join("worldtree.db").display()
        )));
        assert_eq!(content.matches("[mcp_servers.atlas]").count(), 1);
    }

    #[test]
    fn install_codex_preserves_custom_config_toml() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".codex")).unwrap();
        let custom = "[mcp_servers.atlas]\ncommand = \"atlas\"\nargs = [\"--repo\", \"/custom/repo\", \"serve\"]\ntype = \"stdio\"\n";
        fs::write(tmp.path().join(".codex").join("config.toml"), custom).unwrap();

        let result = install_codex(tmp.path(), false).unwrap();
        assert!(matches!(result, PlatformResult::AlreadyConfigured(_)));
        assert_eq!(
            fs::read_to_string(tmp.path().join(".codex").join("config.toml")).unwrap(),
            custom
        );
    }

    #[test]
    fn run_install_codex_creates_only_agents_md() {
        let tmp = TempDir::new().unwrap();

        let summary = run_install(tmp.path(), "codex", "repo", false, false, true, false).unwrap();

        assert!(summary.instruction_files.contains(&"AGENTS.md".to_owned()));
        assert!(!summary.instruction_files.contains(&"CLAUDE.md".to_owned()));
        assert!(tmp.path().join("AGENTS.md").exists());
        assert!(!tmp.path().join("CLAUDE.md").exists());
    }

    #[test]
    fn run_install_claude_creates_only_claude_md() {
        let tmp = TempDir::new().unwrap();

        let summary = run_install(tmp.path(), "claude", "repo", false, false, true, false).unwrap();

        assert!(summary.instruction_files.contains(&"CLAUDE.md".to_owned()));
        assert!(!summary.instruction_files.contains(&"AGENTS.md".to_owned()));
        assert!(tmp.path().join("CLAUDE.md").exists());
        assert!(!tmp.path().join("AGENTS.md").exists());
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
            assert!(content.contains(HOOK_START_MARKER));
            assert!(content.contains(HOOK_END_MARKER));
            assert_eq!(content.matches("atlas update || true").count(), 1);
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
            assert_eq!(content.matches(HOOK_START_MARKER).count(), 1);
            assert_eq!(content.matches("atlas update || true").count(), 1);
        }
    }

    #[test]
    fn install_git_hooks_replaces_legacy_marker_block() {
        let tmp = TempDir::new().unwrap();
        repo_with_git(tmp.path());
        let hook = tmp.path().join(".git/hooks/post-checkout");
        fs::write(
            &hook,
            "#!/bin/sh\natlas update # atlas-hook\n\n# Installed by atlas. Remove these lines to disable atlas graph updates.\nif command -v atlas >/dev/null 2>&1; then\n    atlas update || true\n    atlas detect-changes --brief || true\nfi\n",
        )
        .unwrap();

        install_git_hooks(tmp.path(), false).unwrap();

        let content = fs::read_to_string(hook).unwrap();
        assert!(!content.contains(LEGACY_HOOK_MARKER));
        assert_eq!(content.matches("atlas update || true").count(), 1);
        assert!(content.contains(HOOK_START_MARKER));
        assert!(content.contains(HOOK_END_MARKER));
    }

    #[test]
    fn install_summary_reports_all_git_hooks() {
        let tmp = TempDir::new().unwrap();
        repo_with_git(tmp.path());
        let summary = run_install(tmp.path(), "claude", "repo", false, false, false, true).unwrap();
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
        run_install(tmp.path(), "claude", "repo", true, false, false, false).unwrap();
        assert!(!tmp.path().join(".mcp.json").exists());
        assert!(!tmp.path().join("AGENTS.md").exists());
        assert!(!tmp.path().join("CLAUDE.md").exists());
    }

    // ------------------------------------------------------------------
    // Platform agent hook tests
    // ------------------------------------------------------------------

    #[test]
    fn install_atlas_hook_runner_creates_executable_script() {
        let tmp = TempDir::new().unwrap();
        let result = install_atlas_hook_runner(tmp.path(), false).unwrap();
        assert!(result.is_some());

        let runner = tmp.path().join(".atlas").join("hooks").join("atlas-hook");
        let lib_dir = tmp.path().join(".atlas").join("hooks").join("lib");
        assert!(runner.exists());
        assert!(lib_dir.is_dir());
        let content = fs::read_to_string(&runner).unwrap();
        assert!(content.starts_with("#!/bin/sh"));
        assert!(content.contains("atlas-hook"));
        assert!(content.contains("ATLAS_HOOK_SCRIPT_PATH=\"$0\""));
        assert!(content.contains("atlas hook \"$ATLAS_EVENT\""));
        assert!(content.contains("[ -n \"$ATLAS_EVENT\" ]"));
        assert!(content.contains("[ \"$ATLAS_EVENT\" != \"unknown\" ]"));
        assert!(!content.contains("ATLAS_REPO_ROOT"));
        assert!(!content.contains("head -c 65536"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&runner).unwrap().permissions().mode();
            assert_ne!(mode & 0o111, 0, "runner should be executable");
        }
    }

    #[test]
    fn install_atlas_hook_runner_idempotent() {
        let tmp = TempDir::new().unwrap();
        install_atlas_hook_runner(tmp.path(), false).unwrap();
        // Second call should return None (no change).
        let result = install_atlas_hook_runner(tmp.path(), false).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn install_atlas_hook_runner_recreates_missing_lib_dir() {
        let tmp = TempDir::new().unwrap();
        install_atlas_hook_runner(tmp.path(), false).unwrap();
        fs::remove_dir_all(tmp.path().join(".atlas").join("hooks").join("lib")).unwrap();

        let result = install_atlas_hook_runner(tmp.path(), false).unwrap();
        assert!(result.is_none());
        assert!(tmp.path().join(".atlas").join("hooks").join("lib").is_dir());
    }

    #[test]
    fn install_copilot_agent_hooks_writes_github_hooks_json() {
        let tmp = TempDir::new().unwrap();
        let result = install_copilot_agent_hooks(tmp.path(), false).unwrap();
        assert!(result.is_some());

        let config = tmp
            .path()
            .join(".github")
            .join("hooks")
            .join("atlas-copilot.json");
        assert!(config.exists());
        let val: Value = serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        let hooks = val["hooks"].as_object().unwrap();
        assert!(!hooks.is_empty());
        assert!(hooks.contains_key("SessionStart"));
        assert!(hooks.contains_key("PostToolUse"));
        assert!(hooks.contains_key("Stop"));
        for entries in hooks.values() {
            let cmd = entries[0]["command"].as_str().unwrap();
            assert_eq!(entries[0]["type"], "command");
            assert!(cmd.contains("atlas-hook"), "missing atlas-hook in: {cmd}");
        }
    }

    #[test]
    fn install_copilot_agent_hooks_idempotent() {
        let tmp = TempDir::new().unwrap();
        install_copilot_agent_hooks(tmp.path(), false).unwrap();
        let result = install_copilot_agent_hooks(tmp.path(), false).unwrap();
        assert!(result.is_none(), "second call should be idempotent");
    }

    #[test]
    fn install_claude_agent_hooks_writes_settings_json() {
        let tmp = TempDir::new().unwrap();
        let result = install_claude_agent_hooks(tmp.path(), false).unwrap();
        assert!(result.is_some());

        let config = tmp.path().join(".claude").join("settings.json");
        assert!(config.exists());
        let val: Value = serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        let hooks = val["hooks"].as_object().unwrap();
        // Core lifecycle events.
        assert!(hooks.contains_key("SessionStart"));
        assert!(hooks.contains_key("UserPromptSubmit"));
        assert!(hooks.contains_key("UserPromptExpansion"));
        assert!(hooks.contains_key("PreToolUse"));
        assert!(hooks.contains_key("PermissionRequest"));
        assert!(hooks.contains_key("PermissionDenied"));
        assert!(hooks.contains_key("PostToolUse"));
        assert!(hooks.contains_key("PostToolUseFailure"));
        // Nested-agent and task events.
        assert!(hooks.contains_key("SubagentStart"));
        assert!(hooks.contains_key("SubagentStop"));
        assert!(hooks.contains_key("TaskCreated"));
        assert!(hooks.contains_key("TaskCompleted"));
        // Freshness and config events.
        assert!(hooks.contains_key("ConfigChange"));
        assert!(hooks.contains_key("CwdChanged"));
        assert!(hooks.contains_key("FileChanged"));
        assert!(hooks.contains_key("WorktreeCreate"));
        assert!(hooks.contains_key("WorktreeRemove"));
        // Compact / session boundary events.
        assert!(hooks.contains_key("PreCompact"));
        assert!(hooks.contains_key("PostCompact"));
        assert!(hooks.contains_key("Stop"));
        assert!(hooks.contains_key("StopFailure"));
        assert!(hooks.contains_key("SessionEnd"));
    }

    #[test]
    fn install_claude_agent_hooks_merges_with_existing_settings() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        fs::write(
            tmp.path().join(".claude").join("settings.json"),
            r#"{"model":"claude-opus-4-5"}"#,
        )
        .unwrap();

        install_claude_agent_hooks(tmp.path(), false).unwrap();

        let val: Value = serde_json::from_str(
            &fs::read_to_string(tmp.path().join(".claude").join("settings.json")).unwrap(),
        )
        .unwrap();
        // Existing key preserved.
        assert_eq!(val["model"], "claude-opus-4-5");
        // Atlas hooks added.
        assert!(val["hooks"].as_object().is_some());
    }

    #[test]
    fn install_claude_agent_hooks_idempotent() {
        let tmp = TempDir::new().unwrap();
        install_claude_agent_hooks(tmp.path(), false).unwrap();
        let result = install_claude_agent_hooks(tmp.path(), false).unwrap();
        assert!(result.is_none(), "second call should be idempotent");
    }

    #[test]
    fn install_codex_agent_hooks_writes_hooks_json() {
        let tmp = TempDir::new().unwrap();
        let result = install_codex_agent_hooks(tmp.path(), false).unwrap();
        assert!(result.is_some());

        let config = tmp.path().join(".codex").join("hooks.json");
        assert!(config.exists());
        let val: Value = serde_json::from_str(&fs::read_to_string(&config).unwrap()).unwrap();
        let hooks = val["hooks"].as_object().unwrap();
        assert!(hooks.contains_key("SessionStart"));
        assert!(hooks.contains_key("PostToolUse"));
        assert!(hooks.contains_key("Stop"));
    }

    #[test]
    fn install_codex_agent_hooks_idempotent() {
        let tmp = TempDir::new().unwrap();
        install_codex_agent_hooks(tmp.path(), false).unwrap();
        let result = install_codex_agent_hooks(tmp.path(), false).unwrap();
        assert!(result.is_none(), "second call should be idempotent");
    }

    #[test]
    fn platform_hook_configs_are_valid_json() {
        serde_json::to_string_pretty(&copilot_hooks_value()).unwrap();
        serde_json::to_string_pretty(&claude_hooks_value()).unwrap();
        serde_json::to_string_pretty(&codex_hooks_value()).unwrap();
    }

    #[test]
    fn platform_hook_fixtures_match_generated_configs() {
        let cases = [
            (
                "copilot",
                copilot_hooks_value(),
                "tests/fixtures/hooks/copilot/atlas-copilot.json",
            ),
            (
                "claude",
                claude_hooks_value(),
                "tests/fixtures/hooks/claude/settings.json",
            ),
            (
                "codex",
                codex_hooks_value(),
                "tests/fixtures/hooks/codex/hooks.json",
            ),
        ];

        for (label, generated, relative_path) in cases {
            let fixture =
                fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path))
                    .unwrap_or_else(|e| panic!("cannot read {label} fixture: {e}"));
            let fixture: Value = serde_json::from_str(&fixture)
                .unwrap_or_else(|e| panic!("cannot parse {label} fixture: {e}"));
            assert_eq!(generated, fixture, "{label} fixture drifted");
        }
    }

    #[test]
    fn install_platform_agent_hooks_dry_run_writes_nothing() {
        let tmp = TempDir::new().unwrap();
        let files = install_platform_agent_hooks(tmp.path(), "claude", true).unwrap();
        // Dry run should report what it would do but write nothing.
        assert!(!files.is_empty(), "dry-run should report planned files");
        assert!(
            !tmp.path()
                .join(".atlas")
                .join("hooks")
                .join("atlas-hook")
                .exists()
        );
        assert!(!tmp.path().join(".claude").join("settings.json").exists());
    }

    #[test]
    fn install_platform_agent_hooks_copilot_creates_files() {
        let tmp = TempDir::new().unwrap();
        let files = install_platform_agent_hooks(tmp.path(), "copilot", false).unwrap();
        assert!(files.contains(&".atlas/hooks/atlas-hook".to_owned()));
        assert!(files.contains(&".github/hooks/atlas-copilot.json".to_owned()));
    }

    #[test]
    fn install_platform_agent_hooks_claude_creates_files() {
        let tmp = TempDir::new().unwrap();
        let files = install_platform_agent_hooks(tmp.path(), "claude", false).unwrap();
        assert!(files.contains(&".atlas/hooks/atlas-hook".to_owned()));
        assert!(files.contains(&".claude/settings.json".to_owned()));
    }

    #[test]
    fn install_platform_agent_hooks_codex_creates_files() {
        let tmp = TempDir::new().unwrap();
        let files = install_platform_agent_hooks(tmp.path(), "codex", false).unwrap();
        assert!(files.contains(&".atlas/hooks/atlas-hook".to_owned()));
        assert!(files.contains(&".codex/hooks.json".to_owned()));
    }

    #[test]
    fn run_install_validate_only_reports_missing_targets_without_writing() {
        let tmp = TempDir::new().unwrap();
        repo_with_git(tmp.path());

        let summary = run_install(tmp.path(), "claude", "repo", false, true, false, false).unwrap();

        assert!(summary.validate_only);
        assert!(summary.configured.is_empty());
        assert!(summary.already_configured.is_empty());
        assert!(!tmp.path().join(".mcp.json").exists());
        assert!(
            summary
                .validation_checks
                .iter()
                .any(|check| check.check == "claude_config" && !check.ok)
        );
    }

    #[test]
    fn user_scope_installs_platform_files_under_home_root() {
        let repo = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();

        let result =
            install_claude_scoped(repo.path(), home.path(), InstallScope::User, false).unwrap();
        assert!(matches!(result, PlatformResult::Configured(_)));

        let files = install_platform_agent_hooks_scoped(
            repo.path(),
            home.path(),
            "claude",
            InstallScope::User,
            false,
        )
        .unwrap();

        assert!(home.path().join(".mcp.json").exists());
        assert!(home.path().join(".claude").join("settings.json").exists());
        assert!(
            home.path()
                .join(".atlas")
                .join("hooks")
                .join("atlas-hook")
                .exists()
        );
        assert!(!repo.path().join(".mcp.json").exists());
        assert!(!repo.path().join(".claude").join("settings.json").exists());
        assert!(files.contains(&"~/.atlas/hooks/atlas-hook".to_owned()));
        assert!(files.contains(&"~/.claude/settings.json".to_owned()));

        let settings =
            fs::read_to_string(home.path().join(".claude").join("settings.json")).unwrap();
        assert!(
            settings.contains(
                &home
                    .path()
                    .join(".atlas")
                    .join("hooks")
                    .join("atlas-hook")
                    .display()
                    .to_string()
            )
        );
    }

    #[test]
    fn user_scope_validation_passes_after_install() {
        let repo = TempDir::new().unwrap();
        let home = TempDir::new().unwrap();

        install_codex_scoped(repo.path(), home.path(), InstallScope::User, false).unwrap();
        install_platform_agent_hooks_scoped(
            repo.path(),
            home.path(),
            "codex",
            InstallScope::User,
            false,
        )
        .unwrap();

        let checks = validate_install(
            repo.path(),
            home.path(),
            "codex",
            InstallScope::User,
            false,
            true,
        )
        .unwrap();
        assert!(checks.iter().all(|check| check.ok), "checks: {checks:?}");
        assert!(
            checks
                .iter()
                .any(|check| check.detail == "~/.codex/config.toml")
        );
        assert!(
            checks
                .iter()
                .any(|check| check.detail == "~/.codex/hooks.json")
        );
    }
}
