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
/// `no_instructions` — skip injecting platform-specific agent instructions.
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
        let files = inject_instructions(
            repo_root,
            &instruction_targets(platform, repo_root),
            dry_run,
        )?;
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
fn install_claude(repo_root: &Path, dry_run: bool) -> Result<PlatformResult> {
    let config_path = repo_root.join(".mcp.json");
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
fn install_codex(repo_root: &Path, dry_run: bool) -> Result<PlatformResult> {
    let config_path = repo_root.join(".codex").join("config.toml");
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
            .split('`')
            .enumerate()
            .filter(|(idx, _)| idx % 2 == 1)
            .map(|(_, chunk)| chunk.trim())
            .filter(|chunk| {
                !chunk.is_empty() && chunk.chars().all(|ch| ch.is_ascii_lowercase() || ch == '_')
            })
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

        let summary = run_install(tmp.path(), "codex", false, true, false).unwrap();

        assert!(summary.instruction_files.contains(&"AGENTS.md".to_owned()));
        assert!(!summary.instruction_files.contains(&"CLAUDE.md".to_owned()));
        assert!(tmp.path().join("AGENTS.md").exists());
        assert!(!tmp.path().join("CLAUDE.md").exists());
    }

    #[test]
    fn run_install_claude_creates_only_claude_md() {
        let tmp = TempDir::new().unwrap();

        let summary = run_install(tmp.path(), "claude", false, true, false).unwrap();

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
        assert!(!tmp.path().join("CLAUDE.md").exists());
    }
}
