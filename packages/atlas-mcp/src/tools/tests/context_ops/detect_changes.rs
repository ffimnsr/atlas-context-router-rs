use super::*;

#[test]
fn detect_changes_accepts_change_source_object_and_reports_normalized_kind() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        fixture._dir.path(),
        "src/service.rs",
        "pub fn compute() -> i32 { 2 }\n",
    );

    let args = serde_json::json!({
        "change_source": { "kind": "working_tree" },
        "output_format": "json"
    });
    let resp = call(
        "detect_changes",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("call");
    let payload: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(resp.clone())).expect("parse json");

    assert_eq!(
        payload["change_source"]["kind"],
        serde_json::json!("working_tree")
    );
    assert!(resp["_meta"].get("deprecated_input_fields").is_none());
}

#[test]
fn detect_changes_legacy_mode_is_rejected() {
    let fixture = setup_git_mcp_fixture();
    write_repo_file(
        fixture._dir.path(),
        "src/service.rs",
        "pub fn compute() -> i32 { 2 }\n",
    );

    let args = serde_json::json!({ "mode": "working_tree", "output_format": "json" });
    let resp = call(
        "detect_changes",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("call");
    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["message"],
        serde_json::json!("legacy change_source fields are no longer supported")
    );
}

#[test]
fn detect_changes_rejects_mixed_change_source_and_legacy_fields() {
    let fixture = setup_git_mcp_fixture();
    let resp = call(
        "detect_changes",
        Some(&serde_json::json!({
            "change_source": { "kind": "working_tree" },
            "working_tree": true,
            "output_format": "json"
        })),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("call");

    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["message"],
        serde_json::json!("legacy change_source fields are no longer supported")
    );
}

#[test]
fn detect_changes_legacy_repo_scope_is_rejected() {
    use atlas_repo::{
        RepoRegistration, RepoRegistry, RepoRelationship, RepoRelationshipKind, TrustState,
        VcsMetadata, stable_repo_id,
    };
    use camino::{Utf8Path, Utf8PathBuf};

    let fixture = setup_git_mcp_fixture();
    let root = Utf8Path::new(&fixture.repo_root);
    let dep = Utf8PathBuf::from(format!("{}/dep-repo", root.as_str()));
    std::fs::create_dir_all(dep.as_std_path()).expect("create dep repo dir");
    let dep_repo_id = stable_repo_id(dep.as_path());
    let mut registry = RepoRegistry::new(stable_repo_id(root));
    registry.registrations = vec![
        RepoRegistration {
            repo_id: stable_repo_id(root),
            root: root.to_path_buf(),
            display_alias: ".".to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind: RepoRelationshipKind::Root,
                parent_repo_id: None,
                parent_path: None,
            },
            trust_state: TrustState::Trusted,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: Vec::new(),
        },
        RepoRegistration {
            repo_id: dep_repo_id.clone(),
            root: dep,
            display_alias: "dep-repo".to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind: RepoRelationshipKind::Submodule,
                parent_repo_id: Some(stable_repo_id(root)),
                parent_path: Some("dep-repo".to_owned()),
            },
            trust_state: TrustState::Trusted,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: Vec::new(),
        },
    ];
    registry.save(root).expect("save registry");

    let args = serde_json::json!({
        "repo_id": dep_repo_id,
        "change_source": { "kind": "working_tree" },
        "output_format": "json"
    });
    let resp = call(
        "detect_changes",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("call");
    assert_eq!(resp["isError"], serde_json::json!(true));
    assert_eq!(
        resp["structuredContent"]["message"],
        serde_json::json!("legacy repo scope fields are no longer supported")
    );
}

#[test]
fn detect_changes_all_repos_skips_unavailable_repo_with_warning() {
    use atlas_repo::{
        RepoRegistration, RepoRegistry, RepoRelationship, RepoRelationshipKind, TrustState,
        VcsMetadata, stable_repo_id,
    };
    use camino::{Utf8Path, Utf8PathBuf};

    let fixture = setup_git_mcp_fixture();
    let root = Utf8Path::new(&fixture.repo_root);
    let mut registry = RepoRegistry::new(stable_repo_id(root));
    registry.registrations = vec![
        RepoRegistration {
            repo_id: stable_repo_id(root),
            root: root.to_path_buf(),
            display_alias: ".".to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind: RepoRelationshipKind::Root,
                parent_repo_id: None,
                parent_path: None,
            },
            trust_state: TrustState::Trusted,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: Vec::new(),
        },
        RepoRegistration {
            repo_id: stable_repo_id(Utf8Path::new("/missing/submodule")),
            root: Utf8PathBuf::from("/missing/submodule"),
            display_alias: "missing-submodule".to_owned(),
            vcs: VcsMetadata {
                head: None,
                default_branch: None,
                remote_url: None,
            },
            relationship: RepoRelationship {
                kind: RepoRelationshipKind::Submodule,
                parent_repo_id: Some(stable_repo_id(root)),
                parent_path: Some("vendor/missing-submodule".to_owned()),
            },
            trust_state: TrustState::Missing,
            enabled: true,
            include_globs: None,
            exclude_globs: None,
            dependencies: Vec::new(),
        },
    ];
    registry.save(root).expect("save registry");

    write_repo_file(
        fixture._dir.path(),
        "src/service.rs",
        "pub fn compute() -> i32 { 2 }\n",
    );

    let args = serde_json::json!({ "repo_scope": { "kind": "all" }, "change_source": { "kind": "working_tree" }, "output_format": "json" });
    let resp = call(
        "detect_changes",
        Some(&args),
        &fixture.repo_root,
        &fixture.db_path,
    )
    .expect("call");
    let payload: serde_json::Value =
        serde_json::from_str(&unwrap_tool_text(resp.clone())).expect("parse json");

    assert_eq!(
        payload["repo_scope"]["selected_repo_count"],
        serde_json::json!(2)
    );
    assert_eq!(
        payload["repo_scope"]["processed_repo_count"],
        serde_json::json!(1)
    );
    assert_eq!(
        payload["repo_scope"]["skipped_repo_count"],
        serde_json::json!(1)
    );
    assert!(
        resp["structuredContent"]["warnings"]
            .as_array()
            .is_some_and(|warnings| !warnings.is_empty())
    );
}
