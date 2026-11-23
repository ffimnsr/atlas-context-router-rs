use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

use super::agent_skill::{SKILL_REL, SKILL_STUB, install_skill};
use super::git_hooks::{
    HOOK_END_MARKER, HOOK_START_MARKER, HOOK_VERSION_MARKER, LEGACY_HOOK_MARKER, install_git_hooks,
};
use super::instructions::{
    AGENTS_FILE, GENERIC_MCP_SETUP, INSTRUCTIONS_END_MARKER, INSTRUCTIONS_MARKER,
    INSTRUCTIONS_SECTION, InstructionsMode, inject_instructions,
};
use super::validation::validate_install;
use super::{InstallOptions, install_hook_runner, run_install};
use crate::skills::CORE;

fn repo_with_git(dir: &Path) {
    fs::create_dir_all(dir.join(".git").join("hooks")).unwrap();
}

/// Guide body served by `atlas skills get core`.
const GUIDE: &str = CORE.content;

/// One rendered reference served by `atlas skills get core --ref <name>`.
fn guide_reference(name: &str) -> String {
    crate::skills::render_reference(name).unwrap_or_else(|| panic!("missing reference {name}"))
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

/// Extract `event: "name"` literals from a text block (instructions, wiki
/// fallback page).
fn extract_event_literals(text: &str) -> Vec<String> {
    let mut events = Vec::new();
    let mut rest = text;
    while let Some(idx) = rest.find("event: \"") {
        let after = &rest[idx + "event: \"".len()..];
        let end = after.find('"').unwrap_or(after.len());
        let name = after[..end].to_owned();
        if !name.is_empty() && !events.contains(&name) {
            events.push(name);
        }
        rest = after;
    }
    events
}

/// Extract PascalCase event names from markdown table cells (`| `Name` |`),
/// used by the hooks wiki pages.
fn extract_pascal_table_event_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim_start().strip_prefix("| `") else {
            continue;
        };
        let Some(name) = rest.split('`').next() else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let looks_like_event = name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            && name.chars().all(|c| c.is_ascii_alphanumeric());
        if looks_like_event && !names.contains(&name.to_owned()) {
            names.push(name.to_owned());
        }
    }
    names
}

mod discovery;
mod hooks;
mod instructions;
mod registry;
