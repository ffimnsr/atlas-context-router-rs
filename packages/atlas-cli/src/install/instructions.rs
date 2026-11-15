use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use clap::ValueEnum;

pub(super) const INSTRUCTIONS_MARKER: &str = "<!-- atlas MCP tools -->";
pub(super) const INSTRUCTIONS_END_MARKER: &str = "<!-- /atlas MCP tools -->";

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, ValueEnum)]
pub enum InstructionsMode {
    #[default]
    Refresh,
    ReplaceFile,
}

impl InstructionsMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Refresh => "refresh",
            Self::ReplaceFile => "replace-file",
        }
    }
}

/// The only instruction file Atlas manages. It is read by many AI coding tools
/// through the cross-tool `AGENTS.md` convention, so no per-tool files are
/// written.
pub const AGENTS_FILE: &str = "AGENTS.md";

/// Canonical, tool-agnostic instructions for registering the Atlas MCP server.
///
/// `atlas install` prints this guidance and embeds it verbatim in `AGENTS.md`;
/// a test asserts the managed instruction block keeps the two in sync.
pub const GENERIC_MCP_SETUP: &str = r#"### Add Atlas as an MCP server

Atlas is a stdio MCP server. Register it with whatever MCP-capable coding tool
you use — Atlas does not write per-tool configuration files.

  command: atlas
  args:    ["serve"]   # add ["--repo", "<absolute-repo-path>", "serve"] to pin a repository

Most MCP clients accept a JSON entry shaped like:

  {
    "mcpServers": {
      "atlas": { "type": "stdio", "command": "atlas", "args": ["serve"] }
    }
  }

Place that entry in your tool's MCP configuration and restart the tool. Consult
the tool's documentation for the exact config file location and key name."#;

/// Managed AGENTS.md block.
///
/// Keep this block a stable pointer. The detailed agent workflow ships inside
/// the binary (`atlas skills get core`, sourced from `skill-data/`), so release
/// updates do not churn user instruction files. A test enforces both the pointer
/// and the small size of this block.
pub(super) const INSTRUCTIONS_SECTION: &str = r#"<!-- atlas MCP tools -->
## MCP Tools: atlas

**Atlas first.** This repository has a code knowledge graph. For code exploration, use Atlas MCP tools before `grep`, file search, broad reads, or exploratory terminal commands. Atlas returns structure text search cannot: symbols, callers, callees, tests, dependents, impact radius, and review context.

Before the first Atlas tool call, run `atlas skills get core` and follow it. That guide is served by the installed Atlas binary, so it always matches your version: Atlas-first workflow, tool map, escalation order, trust-metadata checks, and path invariants. Run `atlas skills list` to see every guide this version serves.

Use default JSON output. Trust `structuredContent` as source of truth when present.

### Add Atlas as an MCP server

Atlas is a stdio MCP server. Register it with whatever MCP-capable coding tool
you use — Atlas does not write per-tool configuration files.

  command: atlas
  args:    ["serve"]   # add ["--repo", "<absolute-repo-path>", "serve"] to pin a repository

Most MCP clients accept a JSON entry shaped like:

  {
    "mcpServers": {
      "atlas": { "type": "stdio", "command": "atlas", "args": ["serve"] }
    }
  }

Place that entry in your tool's MCP configuration and restart the tool. Consult
the tool's documentation for the exact config file location and key name.
<!-- /atlas MCP tools -->
"#;

pub fn inject_instructions(
    repo_root: &Path,
    filenames: &[&str],
    dry_run: bool,
    mode: InstructionsMode,
) -> Result<Vec<String>> {
    let mut updated = Vec::new();

    for filename in filenames {
        let path = repo_root.join(filename);
        let existing = if path.exists() {
            fs::read_to_string(&path).with_context(|| format!("cannot read {}", path.display()))?
        } else {
            String::new()
        };

        let next_content = render_instructions_content(&existing, mode);
        if next_content == existing {
            continue;
        }

        if dry_run {
            if existing.is_empty() {
                println!("  [dry-run] would create instruction fallback file {filename}");
            } else {
                match mode {
                    InstructionsMode::Refresh if existing.contains(INSTRUCTIONS_MARKER) => {
                        println!(
                            "  [dry-run] would refresh instruction fallback block in {filename}"
                        );
                    }
                    InstructionsMode::Refresh => {
                        println!(
                            "  [dry-run] would append instruction fallback block to {filename}"
                        );
                    }
                    InstructionsMode::ReplaceFile => {
                        println!(
                            "  [dry-run] would replace all contents of {filename} with instruction fallback block"
                        );
                    }
                }
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

fn render_instructions_content(existing: &str, mode: InstructionsMode) -> String {
    match mode {
        InstructionsMode::Refresh => upsert_instructions_section(existing),
        InstructionsMode::ReplaceFile => INSTRUCTIONS_SECTION.to_string(),
    }
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

/// Locate the managed block.
///
/// Markers only count on their own line. Prose or code spans that mention the
/// marker inline must never be treated as the block boundary, or a refresh
/// would truncate user content around the real block.
fn instruction_section_range(existing: &str) -> Option<(usize, usize)> {
    let mut start = None;
    let mut offset = 0usize;
    for line in existing.split_inclusive('\n') {
        let trimmed = line.trim_end();
        if start.is_none() {
            if trimmed.trim_start() == INSTRUCTIONS_MARKER {
                start = Some(offset);
            }
        } else if trimmed.trim_start() == INSTRUCTIONS_END_MARKER {
            let mut end = offset + trimmed.len();
            if existing[end..].starts_with("\r\n") {
                end += 2;
            } else if existing[end..].starts_with('\n') {
                end += 1;
            }
            return Some((start?, end));
        }
        offset += line.len();
    }
    start.map(|start| (start, existing.len()))
}
