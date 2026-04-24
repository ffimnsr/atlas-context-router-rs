use crate::store::history::{StoredCommit, StoredSnapshotFile};
use crate::store::tests::open_in_memory;

#[test]
fn upsert_repo_is_idempotent() {
    let store = open_in_memory();
    let id1 = store.upsert_repo("/repos/my-repo").unwrap();
    let id2 = store.upsert_repo("/repos/my-repo").unwrap();
    assert_eq!(id1, id2, "same path must return same repo_id");
}

#[test]
fn find_repo_id_returns_none_for_unknown_root() {
    let store = open_in_memory();
    let id = store.find_repo_id("/no/such/repo").unwrap();
    assert!(id.is_none());
}

#[test]
fn commit_metadata_stored_and_retrieved_correctly() {
    let store = open_in_memory();
    let repo_id = store.upsert_repo("/repos/test").unwrap();

    let sha = "a".repeat(40);
    let parent = "b".repeat(40);
    let commit = StoredCommit {
        commit_sha: sha.clone(),
        repo_id,
        parent_sha: Some(parent.clone()),
        author_name: Some("Alice".into()),
        author_email: Some("alice@example.com".into()),
        author_time: 1_700_000_000,
        committer_time: 1_700_000_001,
        subject: "feat: add widget".into(),
        message: Some("feat: add widget\n\nLonger body.".into()),
        indexed_at: String::new(),
    };
    store.upsert_commit(&commit).unwrap();

    let found = store
        .find_commit(repo_id, &sha)
        .unwrap()
        .expect("commit must exist");
    assert_eq!(found.commit_sha, sha);
    assert_eq!(found.parent_sha.as_deref(), Some(parent.as_str()));
    assert_eq!(found.author_name.as_deref(), Some("Alice"));
    assert_eq!(found.author_email.as_deref(), Some("alice@example.com"));
    assert_eq!(found.author_time, 1_700_000_000);
    assert_eq!(found.committer_time, 1_700_000_001);
    assert_eq!(found.subject, "feat: add widget");
}

#[test]
fn commit_upsert_replaces_on_duplicate_key() {
    let store = open_in_memory();
    let repo_id = store.upsert_repo("/repos/test").unwrap();
    let sha = "c".repeat(40);

    let mut commit = StoredCommit {
        commit_sha: sha.clone(),
        repo_id,
        parent_sha: None,
        author_name: Some("Alice".into()),
        author_email: None,
        author_time: 1_000,
        committer_time: 1_001,
        subject: "first".into(),
        message: None,
        indexed_at: String::new(),
    };
    store.upsert_commit(&commit).unwrap();
    commit.subject = "updated".into();
    store.upsert_commit(&commit).unwrap();

    let found = store.find_commit(repo_id, &sha).unwrap().unwrap();
    assert_eq!(found.subject, "updated");
}

#[test]
fn find_commit_returns_none_for_unknown_sha() {
    let store = open_in_memory();
    let repo_id = store.upsert_repo("/repos/test").unwrap();
    let sha = "d".repeat(40);
    let found = store.find_commit(repo_id, &sha).unwrap();
    assert!(found.is_none());
}

#[test]
fn snapshot_insert_and_retrieve() {
    let store = open_in_memory();
    let repo_id = store.upsert_repo("/repos/test").unwrap();
    let sha = "e".repeat(40);

    let sid = store
        .insert_snapshot(repo_id, &sha, Some("tree123"), 10, 20, 5, 1.0, 0)
        .unwrap();
    assert!(sid > 0);

    let snap = store
        .find_snapshot(repo_id, &sha)
        .unwrap()
        .expect("snapshot must exist");
    assert_eq!(snap.repo_id, repo_id);
    assert_eq!(snap.commit_sha, sha);
    assert_eq!(snap.node_count, 10);
    assert_eq!(snap.edge_count, 20);
    assert_eq!(snap.file_count, 5);
    assert!((snap.completeness - 1.0).abs() < f64::EPSILON);
    assert_eq!(snap.parse_error_count, 0);
}

#[test]
fn snapshot_files_stored_and_keyed_by_snapshot_id() {
    let store = open_in_memory();
    let repo_id = store.upsert_repo("/repos/test").unwrap();
    let sha = "f".repeat(40);
    let sid = store
        .insert_snapshot(repo_id, &sha, None, 0, 0, 2, 1.0, 0)
        .unwrap();

    let files = vec![
        StoredSnapshotFile {
            snapshot_id: sid,
            file_path: "src/main.rs".into(),
            file_hash: "hash1".into(),
            language: Some("rust".into()),
            size: Some(1024),
        },
        StoredSnapshotFile {
            snapshot_id: sid,
            file_path: "src/lib.rs".into(),
            file_hash: "hash2".into(),
            language: Some("rust".into()),
            size: Some(512),
        },
    ];
    store.insert_snapshot_files(&files).unwrap();
    // No assertion on read-back — write must not error. Full read-back is
    // added when Slice 2 implements snapshot_files queries.
}

#[test]
fn history_status_returns_zeroes_for_unknown_repo() {
    let store = open_in_memory();
    let status = store.history_status("/no/such/repo").unwrap();
    assert!(status.repo_id.is_none());
    assert_eq!(status.indexed_commit_count, 0);
    assert_eq!(status.snapshot_count, 0);
    assert!(status.latest_commit_sha.is_none());
}

#[test]
fn history_status_counts_correctly_after_ingest() {
    let store = open_in_memory();
    let root = "/repos/status-test";
    let repo_id = store.upsert_repo(root).unwrap();

    for i in 0u8..3 {
        let sha = format!("{:040}", i);
        let commit = StoredCommit {
            commit_sha: sha.clone(),
            repo_id,
            parent_sha: None,
            author_name: None,
            author_email: None,
            author_time: i as i64 * 1000,
            committer_time: i as i64 * 1000,
            subject: format!("commit {i}"),
            message: None,
            indexed_at: String::new(),
        };
        store.upsert_commit(&commit).unwrap();
        store
            .insert_snapshot(repo_id, &sha, None, 0, 0, 0, 1.0, 0)
            .unwrap();
    }

    let status = store.history_status(root).unwrap();
    assert_eq!(status.indexed_commit_count, 3);
    assert_eq!(status.snapshot_count, 3);
    // latest by author_time desc — sha "00..02" has highest time
    let latest = status.latest_commit_sha.unwrap();
    assert_eq!(&latest, &format!("{:040}", 2u8));
}
