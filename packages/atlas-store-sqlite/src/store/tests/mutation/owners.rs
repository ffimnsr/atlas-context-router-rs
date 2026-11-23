use super::*;

#[test]
fn file_owner_round_trips_cargo_owner() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "foo",
        "crates/foo/src/lib.rs::fn::foo",
        "crates/foo/src/lib.rs",
        "rust",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "crates/foo/src/lib.rs",
            "abc",
            Some("rust"),
            None,
            &[node],
            &[],
        )
        .unwrap();

    let owner = PackageOwner {
        owner_id: "cargo:crates/foo/Cargo.toml".to_owned(),
        kind: PackageOwnerKind::Cargo,
        root: "crates/foo".to_owned(),
        manifest_path: "crates/foo/Cargo.toml".to_owned(),
        package_name: Some("foo".to_owned()),
    };
    store
        .upsert_file_owner("crates/foo/src/lib.rs", Some(&owner))
        .unwrap();

    let stored = store
        .file_owner("crates/foo/src/lib.rs")
        .unwrap()
        .expect("owner");
    assert_eq!(stored.owner_id, "cargo:crates/foo/Cargo.toml");
    assert_eq!(stored.kind, PackageOwnerKind::Cargo);
    assert_eq!(stored.root, "crates/foo");
    assert_eq!(stored.manifest_path, "crates/foo/Cargo.toml");
    assert_eq!(stored.package_name.as_deref(), Some("foo"));
}

#[test]
fn file_owner_returns_none_when_not_set() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "bar",
        "scripts/run.py::fn::bar",
        "scripts/run.py",
        "python",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "scripts/run.py",
            "ff",
            Some("python"),
            None,
            &[node],
            &[],
        )
        .unwrap();
    // No upsert → owner should be None.
    assert_eq!(store.file_owner("scripts/run.py").unwrap(), None);
}

#[test]
fn file_owner_id_returns_id_string() {
    let mut store = open_in_memory();
    let node = make_node(
        NodeKind::Function,
        "go_fn",
        "lib/core/core.go::fn::go_fn",
        "lib/core/core.go",
        "go",
    );
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "lib/core/core.go",
            "g1",
            Some("go"),
            None,
            &[node],
            &[],
        )
        .unwrap();

    let owner = PackageOwner {
        owner_id: "go:lib/core/go.mod".to_owned(),
        kind: PackageOwnerKind::Go,
        root: "lib/core".to_owned(),
        manifest_path: "lib/core/go.mod".to_owned(),
        package_name: Some("example.com/core".to_owned()),
    };
    store
        .upsert_file_owner("lib/core/core.go", Some(&owner))
        .unwrap();

    assert_eq!(
        store.file_owner_id("lib/core/core.go").unwrap().as_deref(),
        Some("go:lib/core/go.mod")
    );
}

#[test]
fn upsert_file_owner_for_repo_does_not_modify_other_repo() {
    let mut store = open_in_memory();
    for (repo_id, qname) in [
        ("repo-a", "repo::repo-a::src/lib.rs::fn::run"),
        ("repo-b", "repo::repo-b::src/lib.rs::fn::run"),
    ] {
        let file = ParsedFile {
            path: "src/lib.rs".to_string(),
            language: Some("rust".to_string()),
            hash: format!("{repo_id}-hash"),
            size: None,
            nodes: vec![make_node(
                NodeKind::Function,
                "run",
                qname,
                "src/lib.rs",
                "rust",
            )],
            edges: vec![],
        };
        store
            .replace_files_transactional_for_repo(repo_id, &[file])
            .unwrap();
    }

    let owner = PackageOwner {
        owner_id: "cargo:Cargo.toml".to_owned(),
        kind: PackageOwnerKind::Cargo,
        root: ".".to_owned(),
        manifest_path: "Cargo.toml".to_owned(),
        package_name: Some("repo-a".to_owned()),
    };
    store
        .upsert_file_owner_for_repo("repo-a", "src/lib.rs", Some(&owner))
        .unwrap();

    assert_eq!(
        store
            .file_owner_id_for_repo("repo-a", "src/lib.rs")
            .unwrap()
            .as_deref(),
        Some("cargo:Cargo.toml")
    );
    assert_eq!(
        store
            .file_owner_id_for_repo("repo-b", "src/lib.rs")
            .unwrap(),
        None
    );
}

// file_hash
// -------------------------------------------------------------------------

#[test]
fn file_hash_returns_none_for_unknown_path() {
    let store = open_in_memory();
    assert_eq!(store.file_hash("nonexistent.rs").unwrap(), None);
}

#[test]
fn file_hash_returns_stored_hash() {
    let mut store = open_in_memory();
    let node = make_node(NodeKind::Function, "foo", "a.rs::fn::foo", "a.rs", "rust");
    store
        .replace_file_graph_for_repo(
            "repo_test",
            "a.rs",
            "deadbeef",
            Some("rust"),
            None,
            &[node],
            &[],
        )
        .unwrap();
    assert_eq!(
        store.file_hash("a.rs").unwrap(),
        Some("deadbeef".to_string())
    );
}

// -------------------------------------------------------------------------
// rename_file_graph
// -------------------------------------------------------------------------
