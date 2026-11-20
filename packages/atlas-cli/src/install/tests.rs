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

#[test]
fn inject_instructions_creates_agents_md() {
    let tmp = TempDir::new().unwrap();
    let files =
        inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    assert!(files.contains(&AGENTS_FILE.to_owned()));
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();
    assert!(content.contains(INSTRUCTIONS_MARKER));
}

#[test]
fn inject_instructions_idempotent() {
    let tmp = TempDir::new().unwrap();
    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let files =
        inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    assert!(!files.contains(&AGENTS_FILE.to_owned()));
}

#[test]
fn inject_instructions_appends_to_existing() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(AGENTS_FILE), b"# Existing content\n").unwrap();
    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();
    assert!(content.starts_with("# Existing content\n"));
    assert!(content.contains(INSTRUCTIONS_MARKER));
}

#[test]
fn instructions_section_excludes_generic_mcp_setup() {
    assert!(
        !INSTRUCTIONS_SECTION.contains(GENERIC_MCP_SETUP),
        "managed AGENTS.md block must not embed the generic MCP setup guidance"
    );
    assert!(GENERIC_MCP_SETUP.contains("command: atlas"));
    assert!(GENERIC_MCP_SETUP.contains("\"mcpServers\""));
    assert!(GENERIC_MCP_SETUP.contains("does not write per-tool configuration files"));
}

#[test]
fn instructions_section_stays_a_stable_pointer() {
    assert!(INSTRUCTIONS_SECTION.contains("atlas skills get core"));
    assert!(INSTRUCTIONS_SECTION.contains("atlas skills list"));
    assert!(INSTRUCTIONS_SECTION.contains("MANDATORY"));
    assert!(INSTRUCTIONS_SECTION.contains("STRICT RULE"));
    assert!(
        INSTRUCTIONS_SECTION.lines().count() < 40,
        "managed AGENTS.md block must stay a small pointer, not a growing guide"
    );
    for moved in [
        "Minimal-first escalation order",
        "`tool_list`",
        "Session memory fallback for hosts without hooks",
        "### Add Atlas as an MCP server",
    ] {
        assert!(
            !INSTRUCTIONS_SECTION.contains(moved),
            "`{moved}` belongs in skill-data guide content, not AGENTS.md"
        );
    }
}

#[test]
fn core_skill_mentions_tool_discovery_helpers() {
    assert!(GUIDE.contains("`tool_list`"));
    assert!(GUIDE.contains("`tool_search`"));
    assert!(GUIDE.contains("`tool_help`"));
    assert!(GUIDE.contains("Runtime docs are canonical"));
    assert!(GUIDE.contains("input contracts"));
}

#[test]
fn core_skill_puts_continuity_before_graph_lookup() {
    let continuity = GUIDE
        .find("## Continuity first")
        .expect("core guide needs a leading continuity section");
    let atlas_first = GUIDE
        .find("## Atlas first")
        .expect("core guide keeps the Atlas-first section");
    assert!(
        continuity < atlas_first,
        "continuity must be the first workflow, before graph lookup"
    );
    assert!(GUIDE.contains("`wake_up` once at the start of every session"));
    assert!(GUIDE.contains("Record, save, and feed back in every session"));
    for tool in [
        "`record_session_event`",
        "`save_context_artifact`",
        "`memory_store`",
        "`feedback_record`",
        "`compact_session`",
    ] {
        assert!(GUIDE.contains(tool), "core guide must document {tool}");
    }
}

#[test]
fn core_skill_documents_workflow_patterns() {
    for heading in ["## Workflow patterns", "## Plan and handoff"] {
        assert!(
            GUIDE.contains(heading),
            "core guide must expose the {heading} section"
        );
    }
    for tool in [
        "`batch_query_graph`",
        "`concept_clusters`",
        "`cross_file_links`",
        "`cross_session_search`",
        "`analyze_architecture`",
        "`find_duplicates`",
        "`read_saved_context`",
    ] {
        assert!(
            GUIDE.contains(tool),
            "workflow patterns must suggest {tool}"
        );
    }
    assert!(GUIDE.contains("source_type: \"plan\""));
    assert!(GUIDE.contains("512 B"));
    assert!(GUIDE.contains("gitignored"));
}

#[test]
fn core_skill_documents_split_graph_build_tools() {
    assert!(GUIDE.contains("`build_graph` with `{}`"));
    assert!(GUIDE.contains("`update_graph` with direct `change_source`"));
    assert!(
        GUIDE
            .contains("`update_graph` with `{ \"change_source\": { \"kind\": \"working_tree\" } }")
    );
    assert!(!GUIDE.contains("build_or_update_graph"));
}

#[test]
fn continuity_reference_documents_session_memory_fallback() {
    let guide = guide_reference("continuity");
    assert!(guide.contains("Session memory fallback for hosts without hooks"));
    assert!(guide.contains("`record_session_event`"));
    assert!(guide.contains("`wake_up` when available"));
    assert!(guide.contains("`resume_session`"));
    assert!(guide.contains("`save_context_artifact`"));
    assert!(guide.contains("source_type: \"plan\""));
    assert!(guide.contains("`read_saved_context`"));
    assert!(guide.contains("`compact_session`"));
    assert!(guide.contains("`search_decisions`"));
    assert!(guide.contains("`get_global_memory`"));
    for event in [
        "user-prompt",
        "post-tool-use",
        "file-changed",
        "tool-failure",
        "pre-compact",
        "stop",
        "session-end",
    ] {
        assert!(
            guide.contains(&format!("\"{event}\"")),
            "fallback protocol must document event {event}"
        );
    }
    assert!(guide.contains("Do NOT store"));
    assert!(guide.contains("Never write any Atlas database directly"));
}

#[test]
fn inject_instructions_agents_md_points_at_core_skill() {
    let tmp = TempDir::new().unwrap();
    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();
    assert!(content.contains("atlas skills get core"));
    assert!(content.contains("atlas skills list"));
    assert!(
        !content.contains("Session memory fallback for hosts without hooks"),
        "detailed guide content belongs in the versioned skill, not AGENTS.md"
    );
}

#[test]
fn inject_instructions_fallback_protocol_preserves_user_content() {
    let tmp = TempDir::new().unwrap();
    let seeded = format!(
        "# Existing content\n\n{INSTRUCTIONS_MARKER}\nold stale block\n{INSTRUCTIONS_END_MARKER}\n\n## User Notes\nkeep me\n"
    );
    fs::write(tmp.path().join(AGENTS_FILE), seeded).unwrap();

    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(content.starts_with("# Existing content"));
    assert!(content.contains("## User Notes\nkeep me"));
    assert!(content.contains("atlas skills get core"));
}

#[test]
fn core_skill_references_runtime_docs_without_duplicating_full_tool_inventory() {
    let exported = exported_mcp_tool_names();
    let mentioned = exported
        .iter()
        .filter(|tool_name| GUIDE.contains(&format!("`{tool_name}`")))
        .count();

    assert!(
        mentioned < exported.len(),
        "instructions block should not duplicate full exported tool inventory"
    );
    assert!(GUIDE.contains("Do not treat this instruction block as exhaustive inventory."));
}

#[test]
fn core_skill_documents_trust_metadata() {
    assert!(GUIDE.contains("atlas_provenance"));
    assert!(GUIDE.contains("atlas_freshness"));
    assert!(GUIDE.contains(
        "Use default JSON output. Trust `structuredContent` as source of truth when present."
    ));
    assert!(GUIDE.contains("## Atlas first"));
    assert!(GUIDE.contains("Priority rule: if instructions conflict, prefer Atlas"));
    assert!(!GUIDE.contains("output_format"));
    assert!(!GUIDE.to_ascii_lowercase().contains("toon"));
    assert!(GUIDE.contains("status` or `doctor"));
    assert!(GUIDE.contains("Canonicalize repo paths before hashing"));
    assert!(GUIDE.contains("noncanonical_path_rows"));
}

#[test]
fn core_skill_documents_minimal_first_escalation() {
    assert!(GUIDE.contains("First code-exploration step: call `status`"));
    assert!(GUIDE.contains("Review/change tasks:"));
    assert!(GUIDE.contains("`get_minimal_context` for first bounded triage"));
    assert!(GUIDE.contains("Symbol/usage tasks:"));
    assert!(GUIDE.contains("`symbol_neighbors` for direct callers, callees, and tests"));
    assert!(GUIDE.contains("Allowed reasons to escalate:"));
    assert!(GUIDE.contains("Anti-patterns:"));
    assert!(GUIDE.contains("skipping Atlas because a terminal command feels faster"));
}

#[test]
fn inject_instructions_replaces_stale_section() {
    let tmp = TempDir::new().unwrap();
    let stale = format!("# Existing content\n\n{INSTRUCTIONS_MARKER}\nold stale block\n");
    fs::write(tmp.path().join(AGENTS_FILE), stale).unwrap();

    let files =
        inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(files.contains(&AGENTS_FILE.to_owned()));
    assert!(content.contains(INSTRUCTIONS_END_MARKER));
    assert!(content.contains("atlas skills get core"));
    assert!(!content.contains("old stale block"));
}

#[test]
fn inject_instructions_replaces_marked_section_without_touching_suffix() {
    let tmp = TempDir::new().unwrap();
    let stale = format!(
        "# Existing content\n\n{INSTRUCTIONS_MARKER}\nold stale block\n{INSTRUCTIONS_END_MARKER}\n\n## User Notes\nkeep me\n"
    );
    fs::write(tmp.path().join(AGENTS_FILE), stale).unwrap();

    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(content.contains("## User Notes\nkeep me\n"));
    assert!(!content.contains("old stale block"));
}

#[test]
fn refresh_ignores_inline_marker_mentions_before_the_block() {
    let tmp = TempDir::new().unwrap();
    let seeded = format!(
        "# Notes\n\nMention `{INSTRUCTIONS_MARKER}` inline, not as a boundary.\n\n{INSTRUCTIONS_MARKER}\nold stale block\n{INSTRUCTIONS_END_MARKER}\n\n## User Notes\nkeep me\n"
    );
    fs::write(tmp.path().join(AGENTS_FILE), seeded).unwrap();

    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(content.contains("Mention `<!-- atlas MCP tools -->` inline"));
    assert!(content.contains("## User Notes\nkeep me"));
    assert!(!content.contains("old stale block"));
    assert!(content.contains("atlas skills get core"));
}

#[test]
fn refresh_appends_when_only_an_inline_marker_mention_exists() {
    let tmp = TempDir::new().unwrap();
    let seeded = "# Notes\n\nLiteral `<!-- atlas MCP tools -->` mention only.\n";
    fs::write(tmp.path().join(AGENTS_FILE), seeded).unwrap();

    inject_instructions(tmp.path(), &[AGENTS_FILE], false, InstructionsMode::Refresh).unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(content.starts_with("# Notes"));
    assert!(content.contains("Literal `<!-- atlas MCP tools -->` mention only."));
    assert!(content.contains(INSTRUCTIONS_END_MARKER));
}

#[test]
fn inject_instructions_replace_file_overwrites_existing_content() {
    let tmp = TempDir::new().unwrap();
    fs::write(
        tmp.path().join(AGENTS_FILE),
        "# Existing content\n\n## User Notes\nkeep me\n",
    )
    .unwrap();

    let files = inject_instructions(
        tmp.path(),
        &[AGENTS_FILE],
        false,
        InstructionsMode::ReplaceFile,
    )
    .unwrap();
    let content = fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap();

    assert!(files.contains(&AGENTS_FILE.to_owned()));
    assert_eq!(content, INSTRUCTIONS_SECTION);
}

#[test]
fn run_install_replace_file_overwrites_agents_md() {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join(AGENTS_FILE), "# Existing content\n").unwrap();

    let summary = run_install(
        tmp.path(),
        InstallOptions {
            no_hooks: true,
            instructions_mode: InstructionsMode::ReplaceFile,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(summary.instruction_files.contains(&AGENTS_FILE.to_owned()));
    assert_eq!(
        fs::read_to_string(tmp.path().join(AGENTS_FILE)).unwrap(),
        INSTRUCTIONS_SECTION
    );
}

#[test]
fn generic_install_writes_only_tool_agnostic_files() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());

    let summary = run_install(tmp.path(), InstallOptions::default()).unwrap();

    assert!(tmp.path().join(AGENTS_FILE).exists());
    assert_eq!(
        fs::read_to_string(tmp.path().join(SKILL_REL)).unwrap(),
        SKILL_STUB
    );
    assert!(
        tmp.path()
            .join(".git")
            .join("hooks")
            .join("pre-commit")
            .exists()
    );
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

    for removed in [".mcp.json", ".claude", ".codex", ".vscode", "CLAUDE.md"] {
        assert!(
            !tmp.path().join(removed).exists(),
            "generic install must not create tool-specific path {removed}"
        );
    }
}

#[test]
fn install_writes_discovery_skill_stub() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());

    let summary = run_install(tmp.path(), InstallOptions::default()).unwrap();

    assert!(summary.skill_files.contains(&SKILL_REL.to_owned()));
    let content = fs::read_to_string(tmp.path().join(SKILL_REL)).unwrap();
    assert_eq!(content, SKILL_STUB);
    assert!(content.contains("atlas skills get core"));
    assert!(
        summary
            .validation_checks
            .iter()
            .any(|check| check.check == "agent_skill" && check.ok),
        "install must validate the discovery skill it wrote"
    );
}

#[test]
fn install_is_idempotent_for_discovery_skill() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());

    run_install(tmp.path(), InstallOptions::default()).unwrap();
    let summary = run_install(tmp.path(), InstallOptions::default()).unwrap();

    assert!(summary.skill_files.is_empty());
}

#[test]
fn install_preserves_user_edited_discovery_skill_without_force() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());
    let skill_path = tmp.path().join(SKILL_REL);
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    fs::write(&skill_path, "# my own atlas skill\n").unwrap();

    let summary = run_install(tmp.path(), InstallOptions::default()).unwrap();

    assert!(summary.skill_files.is_empty());
    assert_eq!(
        fs::read_to_string(&skill_path).unwrap(),
        "# my own atlas skill\n"
    );
}

#[test]
fn install_force_refreshes_discovery_skill() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());
    let skill_path = tmp.path().join(SKILL_REL);
    fs::create_dir_all(skill_path.parent().unwrap()).unwrap();
    fs::write(&skill_path, "# stale atlas skill\n").unwrap();

    let summary = run_install(
        tmp.path(),
        InstallOptions {
            force: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(summary.skill_files.contains(&SKILL_REL.to_owned()));
    assert_eq!(fs::read_to_string(&skill_path).unwrap(), SKILL_STUB);
}

#[test]
fn install_dry_run_does_not_write_discovery_skill() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());

    let summary = run_install(
        tmp.path(),
        InstallOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(summary.skill_files.contains(&SKILL_REL.to_owned()));
    assert!(!tmp.path().join(SKILL_REL).exists());
}

#[test]
fn install_skill_helper_creates_parent_dirs() {
    let tmp = TempDir::new().unwrap();

    let written = install_skill(tmp.path(), false, false).unwrap();

    assert_eq!(written.as_deref(), Some(SKILL_REL));
    assert!(tmp.path().join(SKILL_REL).exists());
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

#[test]
fn generated_mcp_tools_markdown_matches_exported_registry() {
    let path = repo_root().join("MCP_TOOLS.md");
    let actual = fs::read_to_string(&path).unwrap_or_else(|err| {
        panic!(
            "read generated MCP tools file {} failed: {err}",
            path.display()
        )
    });

    assert_eq!(actual, atlas_mcp::tool_list_markdown());
    assert!(actual.contains("serialized from `rmcp::model::Tool`"));
    assert!(actual.contains(
        "| Tool | Title | Input schema | Result contract | Output schema | Description |"
    ));
    assert!(!actual.to_ascii_lowercase().contains("toon"));
}

#[test]
fn continuity_reference_only_uses_supported_fallback_events() {
    let guide = guide_reference("continuity");
    let events = extract_event_literals(&guide);
    assert!(
        events.len() >= 7,
        "fallback protocol must reference its capture events: {events:?}"
    );
    for event in &events {
        assert!(
            atlas_mcp::is_supported_event_name(event),
            "installed instructions reference unsupported event name: {event}"
        );
    }
}

#[test]
fn wiki_hook_docs_and_fallback_page_agree_on_event_names() {
    for (path, pascal_cells) in [
        ("wiki/hooks-claude.md", true),
        ("wiki/hooks-codex.md", true),
        ("wiki/session-memory-fallback.md", false),
    ] {
        let full = repo_root().join(path);
        let text =
            fs::read_to_string(&full).unwrap_or_else(|err| panic!("read {path} failed: {err}"));
        let names: Vec<String> = if pascal_cells {
            extract_pascal_table_event_names(&text)
        } else {
            extract_event_literals(&text)
        };
        assert!(!names.is_empty(), "{path} must document event names");
        for name in &names {
            assert!(
                atlas_mcp::is_supported_event_name(name),
                "{path} describes unsupported event name: {name}"
            );
        }
    }
}

#[test]
fn dry_run_reports_instruction_files_without_writing() {
    let tmp = TempDir::new().unwrap();

    let summary = run_install(
        tmp.path(),
        InstallOptions {
            dry_run: true,
            no_hooks: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(
        summary.instruction_files.contains(&AGENTS_FILE.to_owned()),
        "dry-run must report the instruction file it would update: {:?}",
        summary.instruction_files
    );
    assert!(
        !tmp.path().join(AGENTS_FILE).exists(),
        "dry-run must not write instruction files"
    );
    assert!(
        summary.hook_paths.is_empty() && summary.hook_runner_files.is_empty(),
        "dry-run with instruction-only surface must not report hooks"
    );
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

#[test]
fn git_hooks_installed_and_executable() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());
    let hooks = install_git_hooks(tmp.path(), false, false).unwrap();
    assert_eq!(hooks.len(), 5);
    for hook in hooks {
        let file_name = hook
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap()
            .to_owned();
        let content = fs::read_to_string(&hook).unwrap();
        assert!(content.contains(HOOK_START_MARKER));
        assert!(content.contains(HOOK_END_MARKER));
        assert!(content.contains(HOOK_VERSION_MARKER));
        assert_eq!(content.matches("atlas update || true").count(), 1);
        if file_name == "pre-commit" {
            assert!(content.contains("atlas detect-changes || true"));
            assert!(content.contains("[pre-commit] complete"));
        } else {
            assert!(content.contains("atlas detect-changes || true"));
            assert!(!content.contains("atlas detect-changes --brief || true"));
        }
    }
}

#[test]
fn git_hooks_idempotent() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());
    install_git_hooks(tmp.path(), false, false).unwrap();
    let hooks = install_git_hooks(tmp.path(), false, false).unwrap();
    for hook in hooks {
        let content = fs::read_to_string(hook).unwrap();
        assert_eq!(content.matches(HOOK_START_MARKER).count(), 1);
        assert_eq!(content.matches(HOOK_VERSION_MARKER).count(), 1);
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
        "#!/bin/sh\natlas update # atlas-hook\n\n# Installed by atlas. Remove these lines to disable atlas graph updates.\nif command -v atlas >/dev/null 2>&1; then\n    atlas update || true\n    atlas detect-changes || true\nfi\n",
    )
    .unwrap();

    install_git_hooks(tmp.path(), false, false).unwrap();

    let content = fs::read_to_string(hook).unwrap();
    assert!(!content.contains(LEGACY_HOOK_MARKER));
    assert_eq!(content.matches("atlas update || true").count(), 1);
    assert!(content.contains(HOOK_START_MARKER));
    assert!(content.contains(HOOK_END_MARKER));
    assert!(content.contains(HOOK_VERSION_MARKER));
}

#[test]
fn install_git_hooks_refuses_to_overwrite_non_atlas_hook_without_force() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());
    let hook = tmp.path().join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\necho custom\n").unwrap();

    let error = install_git_hooks(tmp.path(), false, false).unwrap_err();

    assert!(error.to_string().contains("atlas install --force"));
    assert_eq!(
        fs::read_to_string(hook).unwrap(),
        "#!/bin/sh\necho custom\n"
    );
}

#[test]
fn install_git_hooks_force_replaces_non_atlas_hook() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());
    let hook = tmp.path().join(".git/hooks/pre-commit");
    fs::write(&hook, "#!/bin/sh\necho custom\n").unwrap();

    let hooks = install_git_hooks(tmp.path(), false, true).unwrap();

    assert_eq!(hooks.len(), 5);
    let content = fs::read_to_string(hook).unwrap();
    assert!(!content.contains("echo custom"));
    assert!(content.contains(HOOK_START_MARKER));
    assert!(content.contains(HOOK_VERSION_MARKER));
    assert!(content.contains(HOOK_END_MARKER));
}

#[test]
fn install_summary_reports_all_git_hooks() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());
    let summary = run_install(
        tmp.path(),
        InstallOptions {
            no_instructions: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(summary.hook_paths.len(), 5);
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
            .any(|path| path.ends_with("post-commit"))
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
    assert!(
        summary
            .hook_runner_files
            .contains(&".atlas/hooks/atlas-hook".to_owned())
    );
}

#[test]
fn dry_run_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());
    run_install(
        tmp.path(),
        InstallOptions {
            dry_run: true,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!tmp.path().join(AGENTS_FILE).exists());
    assert!(!tmp.path().join(".atlas").exists());
    for removed in [".mcp.json", ".claude", ".codex", ".vscode", "CLAUDE.md"] {
        assert!(!tmp.path().join(removed).exists());
    }
}

#[test]
fn install_hook_runner_creates_executable_script() {
    let tmp = TempDir::new().unwrap();
    let result = install_hook_runner(tmp.path(), false).unwrap();
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
fn install_hook_runner_calls_cli_and_never_writes_sqlite_directly() {
    let tmp = TempDir::new().unwrap();
    install_hook_runner(tmp.path(), false).unwrap();

    let content =
        fs::read_to_string(tmp.path().join(".atlas").join("hooks").join("atlas-hook")).unwrap();

    // Thin launcher only: all real work stays in the Rust `atlas hook` CLI.
    assert!(
        content.contains("atlas hook"),
        "runner must call `atlas hook`, got: {content}"
    );
    assert!(content.contains("|| true"), "runner must be non-blocking");

    // Regression guard: generated runner must never perform direct graph/db
    // work from shell; everything goes through the shared CLI service.
    for banned in [
        "atlas update",
        "atlas detect-changes",
        "worldtree",
        "session.db",
        "context.db",
        "sqlite",
    ] {
        assert!(
            !content.contains(banned),
            "runner must not contain {banned:?}; got: {content}"
        );
    }
}

#[test]
fn install_hook_runner_idempotent() {
    let tmp = TempDir::new().unwrap();
    install_hook_runner(tmp.path(), false).unwrap();
    let result = install_hook_runner(tmp.path(), false).unwrap();
    assert!(result.is_none());
}

#[test]
fn install_hook_runner_recreates_missing_lib_dir() {
    let tmp = TempDir::new().unwrap();
    install_hook_runner(tmp.path(), false).unwrap();
    fs::remove_dir_all(tmp.path().join(".atlas").join("hooks").join("lib")).unwrap();

    let result = install_hook_runner(tmp.path(), false).unwrap();
    assert!(result.is_none());
    assert!(tmp.path().join(".atlas").join("hooks").join("lib").is_dir());
}

#[test]
fn run_install_validate_only_reports_missing_targets_without_writing() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());

    let summary = run_install(
        tmp.path(),
        InstallOptions {
            validate_only: true,
            ..Default::default()
        },
    )
    .unwrap();

    assert!(summary.validate_only);
    assert!(!tmp.path().join(AGENTS_FILE).exists());
    assert!(
        summary
            .validation_checks
            .iter()
            .any(|check| check.check == "instruction_AGENTS.md" && !check.ok)
    );
    assert!(
        summary
            .validation_checks
            .iter()
            .any(|check| check.check == "atlas_hook_runner" && !check.ok)
    );
}

#[test]
fn validate_install_passes_after_generic_install() {
    let tmp = TempDir::new().unwrap();
    repo_with_git(tmp.path());
    run_install(tmp.path(), InstallOptions::default()).unwrap();

    let checks = validate_install(tmp.path(), false, false).unwrap();

    assert!(
        checks.iter().all(|check| check.ok),
        "validation should pass after install: {checks:?}"
    );
}
