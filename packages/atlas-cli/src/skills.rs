//! Agent skill guides served by the installed Atlas binary.
//!
//! Guides live in `skill-data/` and are embedded at build time, so the content
//! always matches the installed version. The managed `AGENTS.md` block is only
//! a pointer at `atlas skills get core`; it never carries the guide body. Add or
//! edit guide content here, not in the instruction block or the discovery stub.
//!
//! The `cli` and `mcp-tools` references are rendered from the live binary and
//! MCP registry, so they cannot drift from the code they describe.

use std::fmt::Write as _;

use clap::{Arg, Command};

/// One skill guide shipped with this Atlas version.
#[derive(Clone, Copy, Debug)]
pub struct Skill {
    pub name: &'static str,
    pub summary: &'static str,
    pub content: &'static str,
    pub references: &'static [Reference],
}

/// One reference document reachable with `atlas skills get <skill> --ref <name>`.
#[derive(Clone, Copy, Debug)]
pub struct Reference {
    pub name: &'static str,
    pub summary: &'static str,
}

const CORE_REFERENCES: &[Reference] = &[
    Reference {
        name: "cli",
        summary: "Every subcommand, argument, and flag, generated from the installed binary",
    },
    Reference {
        name: "mcp-tools",
        summary: "Full MCP tool inventory and input contracts, generated from the server",
    },
    Reference {
        name: "workflows",
        summary: "Review, impact, symbol, analysis, and refactor recipes",
    },
    Reference {
        name: "continuity",
        summary: "Sessions, artifacts, memories, memoir, overview, history, atlas:// URIs",
    },
    Reference {
        name: "setup",
        summary: "Install and init modes, skills, build/update, serve, health, config",
    },
];

pub const CORE: Skill = Skill {
    name: "core",
    summary: "Atlas-first workflow, tool map, escalation order, and trust checks",
    content: include_str!("../skill-data/core/SKILL.md"),
    references: CORE_REFERENCES,
};

const SKILLS: &[Skill] = &[CORE];

/// Every guide this Atlas version serves, in display order.
pub fn all() -> &'static [Skill] {
    SKILLS
}

/// Resolve one guide by exact name.
pub fn get(name: &str) -> Option<&'static Skill> {
    SKILLS.iter().find(|skill| skill.name == name)
}

/// Comma-separated skill names for error and help output.
pub fn names() -> String {
    SKILLS
        .iter()
        .map(|skill| skill.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Comma-separated reference names for one skill.
pub fn reference_names(skill: &Skill) -> String {
    skill
        .references
        .iter()
        .map(|reference| reference.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Render one reference by name, or `None` when this version does not ship it.
///
/// Rendered references are generated from the binary they ship in, so flags and
/// tools always match the installed version.
pub fn render_reference(name: &str) -> Option<String> {
    Some(match name {
        "cli" => render_cli_reference(),
        "mcp-tools" => atlas_mcp::tool_list_markdown(),
        "workflows" => include_str!("../skill-data/core/references/workflows.md").to_owned(),
        "continuity" => include_str!("../skill-data/core/references/continuity.md").to_owned(),
        "setup" => include_str!("../skill-data/core/references/setup.md").to_owned(),
        _ => return None,
    })
}

/// Render a guide, optionally appending every reference.
pub fn render_skill(skill: &Skill, full: bool) -> String {
    let mut out = skill.content.to_owned();
    if !full {
        return out;
    }
    for reference in skill.references {
        out.push_str("\n---\n\n");
        let _ = writeln!(out, "# Reference: {}\n", reference.name);
        match render_reference(reference.name) {
            Some(content) => {
                out.push_str(content.trim_end());
                out.push('\n');
            }
            None => {
                let _ = writeln!(
                    out,
                    "This version does not ship the `{}` reference.",
                    reference.name
                );
            }
        }
    }
    out
}

fn render_cli_reference() -> String {
    let mut out = String::from(
        "# Atlas CLI reference\n\n\
         Generated from the installed binary, so subcommands, arguments, and flags match this Atlas version. \
         Global options work on every command: `--repo <REPO>`, `--db <DB>`, `-v/--verbose`, `--json`.\n",
    );
    let root = <crate::cli::Cli as clap::CommandFactory>::command();
    for subcommand in visible_subcommands(&root) {
        render_command(subcommand, "atlas", &mut out);
    }
    out
}

fn visible_subcommands(command: &Command) -> impl Iterator<Item = &Command> {
    command
        .get_subcommands()
        .filter(|subcommand| subcommand.get_name() != "help")
}

fn render_command(command: &Command, parent: &str, out: &mut String) {
    let path = format!("{parent} {}", command.get_name());
    let _ = writeln!(out, "\n## `{path}`\n");
    if let Some(about) = command.get_about() {
        let _ = writeln!(out, "{about}\n");
    }
    let arguments: Vec<&Arg> = command
        .get_arguments()
        .filter(|arg| documented_argument(arg))
        .collect();

    let mut usage = path.clone();
    for arg in arguments.iter().filter(|arg| arg.is_positional()) {
        let name = arg.get_id().as_str().to_uppercase();
        if arg.is_required_set() {
            let _ = write!(usage, " <{name}>");
        } else {
            let _ = write!(usage, " [{name}]");
        }
    }
    if command.has_subcommands() {
        usage.push_str(" <COMMAND>");
    }
    if !arguments.is_empty() || command.has_subcommands() {
        usage.push_str(" [OPTIONS]");
    }
    let _ = writeln!(out, "```text\n{usage}\n```\n");
    if !arguments.is_empty() {
        out.push_str("Options:\n");
        for arg in arguments {
            let _ = writeln!(out, "- {}", render_argument(arg));
        }
        out.push('\n');
    }

    let subcommands: Vec<&Command> = visible_subcommands(command).collect();
    if !subcommands.is_empty() {
        out.push_str("Subcommands:\n");
        for subcommand in &subcommands {
            let about = subcommand
                .get_about()
                .map(|about| about.to_string())
                .unwrap_or_default();
            let _ = writeln!(out, "- `{}`: {about}", subcommand.get_name());
        }
        out.push('\n');
        for subcommand in subcommands {
            render_command(subcommand, &path, out);
        }
    }
}

fn documented_argument(arg: &Arg) -> bool {
    !arg.is_global_set() && !matches!(arg.get_id().as_str(), "help" | "version")
}

fn render_argument(arg: &Arg) -> String {
    let mut text = String::new();
    if arg.is_positional() {
        let name = arg.get_id().as_str().to_uppercase();
        let _ = if arg.is_required_set() {
            write!(text, "`<{name}>`")
        } else {
            write!(text, "`[{name}]`")
        };
    } else {
        let long = arg.get_long().map(|long| format!("--{long}"));
        let short = arg.get_short().map(|short| format!("-{short}"));
        let flag = [short, long]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(", ");
        let _ = write!(text, "`{flag}`");
    }

    if arg.get_action().takes_values() && !arg.is_positional() {
        match arg.get_value_names() {
            Some(value_names) => {
                for value_name in value_names {
                    let _ = write!(text, " `{}`", value_name.as_str().to_uppercase());
                }
            }
            None => text.push_str(" `VALUE`"),
        }
    }

    if let Some(help) = arg.get_help() {
        let _ = write!(text, ": {help}");
        if !arg.get_default_values().is_empty()
            && help.to_string().to_ascii_lowercase().contains("default")
        {
            return text;
        }
    }
    if let Some(default) = arg.get_default_values().first() {
        let _ = write!(text, " (default: `{}`)", default.to_string_lossy());
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// MCP tools the static references document.
    const DOCUMENTED_TOOLS: &[&str] = &[
        "analyze_safety",
        "assess_risk",
        "build_graph",
        "compact_session",
        "db_check",
        "debug_graph",
        "detect_changes",
        "doctor",
        "explain_change",
        "get_context",
        "get_docs_section",
        "get_global_memory",
        "get_impact_radius",
        "get_minimal_context",
        "get_review_context",
        "get_session_status",
        "list_graph_stats",
        "memory_recall",
        "memory_store",
        "query_graph",
        "read_file_around_match",
        "read_file_excerpt",
        "record_session_event",
        "resolve_symbol",
        "resume_session",
        "save_context_artifact",
        "search_content",
        "search_decisions",
        "search_files",
        "search_saved_context",
        "search_templates",
        "search_text_assets",
        "status",
        "symbol_neighbors",
        "tool_help",
        "tool_list",
        "tool_search",
        "trace_calls",
        "traverse_graph",
        "update_graph",
        "wake_up",
    ];

    fn static_references() -> Vec<(&'static str, &'static str)> {
        vec![
            ("core", CORE.content),
            (
                "workflows",
                include_str!("../skill-data/core/references/workflows.md"),
            ),
            (
                "continuity",
                include_str!("../skill-data/core/references/continuity.md"),
            ),
            (
                "setup",
                include_str!("../skill-data/core/references/setup.md"),
            ),
        ]
    }

    fn exported_tool_names() -> BTreeSet<String> {
        atlas_mcp::tool_list()["tools"]
            .as_array()
            .expect("tool list array")
            .iter()
            .filter_map(|tool| tool["name"].as_str())
            .map(str::to_owned)
            .collect()
    }

    fn cli_command_paths() -> BTreeSet<String> {
        let root = <crate::cli::Cli as clap::CommandFactory>::command();
        let mut paths = BTreeSet::new();
        collect_command_paths(&root, "atlas", &mut paths);
        paths
    }

    fn collect_command_paths(command: &Command, prefix: &str, paths: &mut BTreeSet<String>) {
        for subcommand in visible_subcommands(command) {
            let path = format!("{prefix} {}", subcommand.get_name());
            paths.insert(path.clone());
            collect_command_paths(subcommand, &path, paths);
        }
    }

    /// `atlas` command paths written in backticks or code fences.
    fn documented_cli_paths(markdown: &str) -> Vec<Vec<String>> {
        let mut tokens = Vec::new();
        for segment in markdown.split('`') {
            let trimmed = segment.trim();
            if !trimmed.starts_with("atlas ") {
                continue;
            }
            let mut path = Vec::new();
            for token in trimmed.split_whitespace().skip(1) {
                let is_identifier = token
                    .chars()
                    .next()
                    .is_some_and(|first| first.is_ascii_alphanumeric())
                    && token
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric() || character == '-');
                if !is_identifier || token.is_empty() {
                    break;
                }
                path.push(token.to_owned());
            }
            if !path.is_empty() {
                tokens.push(path);
            }
        }
        tokens
    }

    fn best_matching_path(path: &[String], known: &BTreeSet<String>) -> Option<usize> {
        let mut matched = None;
        for end in 1..=path.len() {
            let candidate = format!("atlas {}", path[..end].join(" "));
            if known.contains(&candidate) {
                matched = Some(end);
            } else {
                break;
            }
        }
        matched
    }

    #[test]
    fn core_guide_is_embedded() {
        assert!(CORE.content.contains("# Atlas core workflow"));
        assert!(CORE.content.contains("atlas skills get core"));
        assert!(
            CORE.content
                .contains("Do not treat this instruction block as exhaustive inventory.")
        );
        assert!(CORE.content.contains("atlas skills get core --full"));
    }

    #[test]
    fn skill_names_are_unique() {
        let mut seen = BTreeSet::new();
        for skill in all() {
            assert!(
                seen.insert(skill.name),
                "duplicate skill name: {}",
                skill.name
            );
        }
    }

    #[test]
    fn get_resolves_known_and_rejects_unknown() {
        assert_eq!(get("core").map(|skill| skill.name), Some("core"));
        assert!(get("missing").is_none());
    }

    #[test]
    fn every_declared_reference_renders() {
        for reference in CORE.references {
            let content = render_reference(reference.name)
                .unwrap_or_else(|| panic!("reference `{}` has no content", reference.name));
            assert!(
                !content.trim().is_empty(),
                "reference `{}` rendered empty",
                reference.name
            );
        }
    }

    #[test]
    fn unknown_reference_is_rejected() {
        assert!(render_reference("missing").is_none());
    }

    #[test]
    fn full_render_includes_every_reference() {
        let full = render_skill(&CORE, true);
        assert!(full.contains("# Atlas core workflow"));
        for reference in CORE.references {
            assert!(
                full.contains(&format!("# Reference: {}", reference.name)),
                "full render is missing reference `{}`",
                reference.name
            );
        }
        assert!(full.len() > CORE.content.len() * 2);
    }

    #[test]
    fn compact_render_omits_references() {
        let compact = render_skill(&CORE, false);
        assert_eq!(compact, CORE.content);
        assert!(!compact.contains("# Reference:"));
    }

    #[test]
    fn cli_reference_lists_every_top_level_subcommand() {
        let content = render_cli_reference();
        let root = <crate::cli::Cli as clap::CommandFactory>::command();
        for subcommand in visible_subcommands(&root) {
            assert!(
                content.contains(&format!("## `atlas {}`", subcommand.get_name())),
                "cli reference is missing `atlas {}`",
                subcommand.get_name()
            );
        }
        assert!(content.contains("## `atlas skills get`"));
        assert!(content.contains("## `atlas skills list`"));
    }

    #[test]
    fn documented_cli_commands_exist_in_the_binary() {
        let known = cli_command_paths();
        for (name, markdown) in static_references() {
            for path in documented_cli_paths(markdown) {
                assert!(
                    best_matching_path(&path, &known).is_some(),
                    "`{name}` documents unknown command `atlas {}`",
                    path.join(" ")
                );
            }
        }
    }

    #[test]
    fn documented_mcp_tools_exist_in_the_registry() {
        let exported = exported_tool_names();
        for tool in DOCUMENTED_TOOLS {
            assert!(
                exported.contains(*tool),
                "documented MCP tool `{tool}` is not exported by this binary"
            );
        }
    }

    #[test]
    fn static_references_cover_every_documented_tool() {
        let mut combined = String::new();
        for (_, markdown) in static_references() {
            combined.push_str(markdown);
        }
        let missing: Vec<&str> = DOCUMENTED_TOOLS
            .iter()
            .copied()
            .filter(|tool| !combined.contains(&format!("`{tool}`")))
            .collect();
        assert!(
            missing.is_empty(),
            "static references no longer mention: {missing:?}"
        );
    }
}
