use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;

use super::{InstallScope, PlatformResult};

#[cfg(test)]
pub(super) fn install_copilot(repo_root: &Path, dry_run: bool) -> Result<PlatformResult> {
    install_copilot_scoped(repo_root, repo_root, InstallScope::Repo, dry_run)
}

pub(super) fn install_copilot_scoped(
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
        "servers",
        "atlas",
        server_entry,
        dry_run,
        "GitHub Copilot",
    )
}

#[cfg(test)]
pub(super) fn install_claude(repo_root: &Path, dry_run: bool) -> Result<PlatformResult> {
    install_claude_scoped(repo_root, repo_root, InstallScope::Repo, dry_run)
}

pub(super) fn install_claude_scoped(
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

#[cfg(test)]
pub(super) fn install_codex(repo_root: &Path, dry_run: bool) -> Result<PlatformResult> {
    install_codex_scoped(repo_root, repo_root, InstallScope::Repo, dry_run)
}

pub(super) fn install_codex_scoped(
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

pub(super) fn stdio_server_args(repo_root: &Path) -> Vec<String> {
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
        .find(|(index, _)| {
            let line = &rest[index + 1..];
            line.starts_with('[')
        })
        .map(|(index, _)| after_header + index + 1)
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

fn copilot_server_entry(repo_root: &Path) -> Value {
    serde_json::json!({
        "type": "stdio",
        "command": "atlas",
        "args": stdio_server_args(repo_root)
    })
}

fn stdio_server_entry(repo_root: &Path) -> Value {
    serde_json::json!({
        "type": "stdio",
        "command": "atlas",
        "args": stdio_server_args(repo_root)
    })
}

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
            Ok(Value::Object(map)) => map,
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
