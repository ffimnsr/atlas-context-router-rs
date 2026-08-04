use super::build::{MAX_MULTI_REPO_SELECTION, enabled_registration_targets};
use atlas_repo::{
    RepoRegistration, RepoRegistry, RepoRelationship, RepoRelationshipKind, TrustState,
    VcsMetadata, stable_repo_id,
};
use camino::{Utf8Path, Utf8PathBuf};

fn registration(root: &Utf8Path, alias: &str, kind: RepoRelationshipKind) -> RepoRegistration {
    RepoRegistration {
        repo_id: stable_repo_id(root),
        root: root.to_path_buf(),
        display_alias: alias.to_owned(),
        vcs: VcsMetadata {
            head: None,
            default_branch: None,
            remote_url: None,
        },
        relationship: RepoRelationship {
            kind,
            parent_repo_id: None,
            parent_path: None,
        },
        trust_state: TrustState::Trusted,
        enabled: true,
        include_globs: None,
        exclude_globs: None,
        dependencies: Vec::new(),
    }
}

#[test]
fn enabled_registration_targets_all_repos_excludes_manual_registrations() {
    let temp = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let sub = root.join("submodule");
    let manual = root.join("../manual-sibling");
    let mut registry = RepoRegistry::new(stable_repo_id(root));
    registry.registrations = vec![
        registration(root, ".", RepoRelationshipKind::Root),
        registration(sub.as_path(), "submodule", RepoRelationshipKind::Submodule),
        registration(manual.as_path(), "manual", RepoRelationshipKind::Manual),
    ];
    registry.save(root).unwrap();

    let (selected, excluded_manual) = enabled_registration_targets(root, None, true).unwrap();

    assert_eq!(selected.len(), 2);
    assert_eq!(excluded_manual, 1);
    assert!(
        selected
            .iter()
            .all(|entry| entry.relationship.kind != RepoRelationshipKind::Manual)
    );
}

#[test]
fn enabled_registration_targets_rejects_excessive_all_repo_fanout() {
    let temp = tempfile::tempdir().unwrap();
    let root = Utf8Path::from_path(temp.path()).unwrap();
    let mut registry = RepoRegistry::new(stable_repo_id(root));
    registry.registrations = (0..(MAX_MULTI_REPO_SELECTION + 1))
        .map(|index| {
            let repo_root = Utf8PathBuf::from(format!("{}/repo-{index}", root.as_str()));
            registration(
                repo_root.as_path(),
                &format!("repo-{index}"),
                RepoRelationshipKind::Submodule,
            )
        })
        .collect();
    registry.save(root).unwrap();

    let error = enabled_registration_targets(root, None, true).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("all_repos scope exceeds max supported repo fan-out")
    );
}
