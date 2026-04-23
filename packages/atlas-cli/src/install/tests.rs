use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use tempfile::TempDir;

use super::git_hooks::{HOOK_END_MARKER, HOOK_START_MARKER, LEGACY_HOOK_MARKER, install_git_hooks};
use super::instructions::{
    INSTRUCTIONS_END_MARKER, INSTRUCTIONS_MARKER, INSTRUCTIONS_SECTION, inject_instructions,
    instruction_targets,
};
use super::mcp::{install_claude, install_codex, install_copilot};
use super::platform_hooks::{
    claude_hooks_value, codex_hooks_value, copilot_hooks_value, install_atlas_hook_runner,
    install_claude_agent_hooks, install_codex_agent_hooks, install_copilot_agent_hooks,
    install_platform_agent_hooks, install_platform_agent_hooks_scoped,
};
use super::validation::validate_install;
use super::{InstallScope, PlatformResult, run_install};

fn repo_with_git(dir: &Path) {
    fs::create_dir_all(dir.join(".git").join("hooks")).unwrap();
}

#[test]
fn inject_instructions_creates_agents_md() {
    let tmp = TempDir::new().unwrap();
    let files = inject_instructions(tmp.path(), &["AGENTS.md"], false).unwrap();
    assert!(files.contains(&"AGENTS.md".to_owned()));
    let content = fs::read_to_string(tmp.path().join("AGENTS.md")).unwrap();
    assert!(content.contains(INSTRUCTIONS_MARKER));
}

#[test]
fn inject_instructions_idempotent() {
    let tmp = TempDir::new().unwrap();
    inject_instructions(tmp.path(), &["AGENTS.md"], false).unwrap();
    let files = inject_instructions(tmp.path(), &["AGENTS.md"], false).unwrap();
    assert!(!files.contains(&"AGENTS.md".to_owned()));
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
    assert_eq!(instruction_section_tool_names(), exported_mcp_tool_names());
}

#[test]
fn instructions_section_documents_trust_metadata() {
    assert!(INSTRUCTIONS_SECTION.contains("atlas_provenance"));
    assert!(INSTRUCTIONS_SECTION.contains("atlas_freshness"));
    assert!(INSTRUCTIONS_SECTION.contains("status` or `doctor"));
    assert!(INSTRUCTIONS_SECTION.contains("Canonicalize repo paths before hashing"));
    assert!(INSTRUCTIONS_SECTION.contains("noncanonical_path_rows"));
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

fn expected_stdio_args(repo_root: &Path) -> Value {
    serde_json::json!([
        "--repo",
        repo_root.display().to_string(),
        "--db",
        repo_root
            .join(".atlas")
            .join("worldtree.db")
            .display()
            .to_string(),
        "serve"
    ])
}

fn assert_json_stdio_server_entry(server: &Value, repo_root: &Path) {
    assert_eq!(server["type"], serde_json::json!("stdio"));
    assert_eq!(server["command"], serde_json::json!("atlas"));
    assert_eq!(server["args"], expected_stdio_args(repo_root));
}

#[test]
fn install_claude_writes_mcp_json() {
    let tmp = TempDir::new().unwrap();
    let result = install_claude(tmp.path(), false).unwrap();
    assert!(matches!(result, PlatformResult::Configured(_)));
    let mcp_path = tmp.path().join(".mcp.json");
    assert!(mcp_path.exists());
    let val: Value = serde_json::from_str(&fs::read_to_string(&mcp_path).unwrap()).unwrap();
    assert_json_stdio_server_entry(&val["mcpServers"]["atlas"], tmp.path());
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
        serde_json::from_str(&fs::read_to_string(tmp.path().join(".mcp.json")).unwrap()).unwrap();
    assert_json_stdio_server_entry(&val["mcpServers"]["atlas"], tmp.path());
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
    assert_json_stdio_server_entry(&val["servers"]["atlas"], tmp.path());
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
    assert!(content.contains("type = \"stdio\""));
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
    assert!(content.contains("type = \"stdio\""));
    assert!(content.contains("command = \"atlas\""));
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
            assert!(content.contains("[pre-commit] complete"));
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
    assert!(hooks.contains_key("SessionStart"));
    assert!(hooks.contains_key("UserPromptSubmit"));
    assert!(hooks.contains_key("UserPromptExpansion"));
    assert!(hooks.contains_key("PreToolUse"));
    assert!(hooks.contains_key("PermissionRequest"));
    assert!(hooks.contains_key("PermissionDenied"));
    assert!(hooks.contains_key("PostToolUse"));
    assert!(hooks.contains_key("PostToolUseFailure"));
    assert!(hooks.contains_key("SubagentStart"));
    assert!(hooks.contains_key("SubagentStop"));
    assert!(hooks.contains_key("TaskCreated"));
    assert!(hooks.contains_key("TaskCompleted"));
    assert!(hooks.contains_key("ConfigChange"));
    assert!(hooks.contains_key("CwdChanged"));
    assert!(hooks.contains_key("FileChanged"));
    assert!(hooks.contains_key("WorktreeCreate"));
    assert!(hooks.contains_key("WorktreeRemove"));
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
    assert_eq!(val["model"], "claude-opus-4-5");
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
        let fixture = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(relative_path))
            .unwrap_or_else(|error| panic!("cannot read {label} fixture: {error}"));
        let fixture: Value = serde_json::from_str(&fixture)
            .unwrap_or_else(|error| panic!("cannot parse {label} fixture: {error}"));
        assert_eq!(generated, fixture, "{label} fixture drifted");
    }
}

#[test]
fn install_platform_agent_hooks_dry_run_writes_nothing() {
    let tmp = TempDir::new().unwrap();
    let files = install_platform_agent_hooks(tmp.path(), "claude", true).unwrap();
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
        super::mcp::install_claude_scoped(repo.path(), home.path(), InstallScope::User, false)
            .unwrap();
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

    let settings = fs::read_to_string(home.path().join(".claude").join("settings.json")).unwrap();
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

    super::mcp::install_codex_scoped(repo.path(), home.path(), InstallScope::User, false).unwrap();
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
