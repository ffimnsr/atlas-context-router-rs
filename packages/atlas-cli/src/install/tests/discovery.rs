use super::*;

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
