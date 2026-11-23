use super::*;

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
