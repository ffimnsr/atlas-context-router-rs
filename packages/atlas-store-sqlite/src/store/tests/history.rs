use crate::store::history::{HistoricalEdge, HistoricalNode, StoredCommit, StoredSnapshotFile};
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

// ── Slice 2 — content-addressed historical file graph ─────────────────────────

fn make_node(file_hash: &str, qn: &str) -> HistoricalNode {
    HistoricalNode {
        file_hash: file_hash.to_owned(),
        qualified_name: qn.to_owned(),
        kind: "function".to_owned(),
        name: qn.to_owned(),
        file_path: "src/lib.rs".to_owned(),
        line_start: Some(1),
        line_end: Some(10),
        language: Some("rust".to_owned()),
        parent_name: None,
        params: None,
        return_type: None,
        modifiers: None,
        is_test: false,
        extra_json: None,
    }
}

fn make_edge(file_hash: &str, src: &str, tgt: &str) -> HistoricalEdge {
    HistoricalEdge {
        file_hash: file_hash.to_owned(),
        source_qn: src.to_owned(),
        target_qn: tgt.to_owned(),
        kind: "calls".to_owned(),
        file_path: "src/lib.rs".to_owned(),
        line: Some(5),
        confidence: 1.0,
        confidence_tier: None,
        extra_json: None,
    }
}

#[test]
fn has_historical_file_graph_returns_false_when_empty() {
    let store = open_in_memory();
    let hash = "a".repeat(40);
    assert!(!store.has_historical_file_graph(&hash).unwrap());
}

#[test]
fn insert_historical_nodes_and_has_file_graph() {
    let store = open_in_memory();
    let hash = "b".repeat(40);
    let node = make_node(&hash, "my_crate::foo");
    store.insert_historical_nodes(&[node]).unwrap();
    assert!(store.has_historical_file_graph(&hash).unwrap());
}

#[test]
fn insert_historical_nodes_idempotent() {
    let store = open_in_memory();
    let hash = "c".repeat(40);
    let node = make_node(&hash, "my_crate::bar");
    store
        .insert_historical_nodes(std::slice::from_ref(&node))
        .unwrap();
    // Second insert must not error (INSERT OR IGNORE).
    store
        .insert_historical_nodes(std::slice::from_ref(&node))
        .unwrap();
    assert_eq!(store.count_historical_nodes(&hash).unwrap(), 1);
}

#[test]
fn count_historical_nodes_and_edges() {
    let store = open_in_memory();
    let hash = "d".repeat(40);
    let nodes = vec![make_node(&hash, "crate::a"), make_node(&hash, "crate::b")];
    let edges = vec![make_edge(&hash, "crate::a", "crate::b")];
    store.insert_historical_nodes(&nodes).unwrap();
    store.insert_historical_edges(&edges).unwrap();
    assert_eq!(store.count_historical_nodes(&hash).unwrap(), 2);
    assert_eq!(store.count_historical_edges(&hash).unwrap(), 1);
}

#[test]
fn list_historical_node_qns_returns_all() {
    let store = open_in_memory();
    let hash = "e".repeat(40);
    let nodes = vec![make_node(&hash, "crate::x"), make_node(&hash, "crate::y")];
    store.insert_historical_nodes(&nodes).unwrap();
    let mut qns = store.list_historical_node_qns(&hash).unwrap();
    qns.sort();
    assert_eq!(qns, vec!["crate::x", "crate::y"]);
}

#[test]
fn list_historical_edge_keys_returns_all() {
    let store = open_in_memory();
    let hash = "f".repeat(40);
    let edges = vec![
        make_edge(&hash, "crate::a", "crate::b"),
        make_edge(&hash, "crate::b", "crate::c"),
    ];
    store.insert_historical_edges(&edges).unwrap();
    let mut keys = store.list_historical_edge_keys(&hash).unwrap();
    keys.sort();
    assert_eq!(keys.len(), 2);
    assert_eq!(
        keys[0],
        ("crate::a".into(), "crate::b".into(), "calls".into())
    );
}

#[test]
fn get_historical_file_language_returns_language() {
    let store = open_in_memory();
    let hash = "1".repeat(40);
    let node = make_node(&hash, "crate::hello");
    store.insert_historical_nodes(&[node]).unwrap();
    let lang = store.get_historical_file_language(&hash).unwrap();
    assert_eq!(lang.as_deref(), Some("rust"));
}

#[test]
fn get_historical_file_language_none_for_unknown_hash() {
    let store = open_in_memory();
    let lang = store.get_historical_file_language("no_such_hash").unwrap();
    assert!(lang.is_none());
}

#[test]
fn snapshot_membership_stored_correctly() {
    let store = open_in_memory();
    let repo_id = store.upsert_repo("/repos/snap-test").unwrap();
    let sha = "a0".repeat(20);
    let sid = store
        .insert_snapshot(repo_id, &sha, None, 2, 1, 1, 1.0, 0)
        .unwrap();

    let hash = "aa".repeat(20);
    let qns = vec!["crate::alpha".to_owned(), "crate::beta".to_owned()];
    let edges = vec![("crate::alpha".into(), "crate::beta".into(), "calls".into())];

    store.attach_snapshot_nodes(sid, &hash, &qns).unwrap();
    store.attach_snapshot_edges(sid, &hash, &edges).unwrap();

    assert_eq!(store.count_snapshot_nodes(sid).unwrap(), 2);
    assert_eq!(store.count_snapshot_edges(sid).unwrap(), 1);
}

#[test]
fn snapshot_node_membership_idempotent() {
    let store = open_in_memory();
    let repo_id = store.upsert_repo("/repos/idempotent").unwrap();
    let sha = "b0".repeat(20);
    let sid = store
        .insert_snapshot(repo_id, &sha, None, 1, 0, 1, 1.0, 0)
        .unwrap();
    let hash = "bb".repeat(20);
    let qns = vec!["crate::only".to_owned()];
    store.attach_snapshot_nodes(sid, &hash, &qns).unwrap();
    store.attach_snapshot_nodes(sid, &hash, &qns).unwrap(); // duplicate, must not fail
    assert_eq!(store.count_snapshot_nodes(sid).unwrap(), 1);
}

#[test]
fn unchanged_file_graph_reused_across_snapshots() {
    // Simulate two commits with the same file blob.
    let store = open_in_memory();
    let repo_id = store.upsert_repo("/repos/reuse").unwrap();
    let file_hash = "cc".repeat(20);

    let node = make_node(&file_hash, "crate::stable");
    store.insert_historical_nodes(&[node]).unwrap();

    // Snapshot 1.
    let sha1 = "10".repeat(20);
    let sid1 = store
        .insert_snapshot(repo_id, &sha1, None, 1, 0, 1, 1.0, 0)
        .unwrap();
    store
        .attach_snapshot_nodes(sid1, &file_hash, &["crate::stable".into()])
        .unwrap();

    // Snapshot 2 reuses same node rows — no second insert needed.
    let sha2 = "20".repeat(20);
    let sid2 = store
        .insert_snapshot(repo_id, &sha2, None, 1, 0, 1, 1.0, 0)
        .unwrap();
    store
        .attach_snapshot_nodes(sid2, &file_hash, &["crate::stable".into()])
        .unwrap();

    // Both snapshots reference the same underlying node data.
    assert_eq!(store.count_snapshot_nodes(sid1).unwrap(), 1);
    assert_eq!(store.count_snapshot_nodes(sid2).unwrap(), 1);
    assert_eq!(store.count_historical_nodes(&file_hash).unwrap(), 1);
}

#[test]
fn modified_file_creates_new_membership_state() {
    let store = open_in_memory();
    let repo_id = store.upsert_repo("/repos/modified").unwrap();

    let hash_v1 = "d1".repeat(20);
    let hash_v2 = "d2".repeat(20);

    store
        .insert_historical_nodes(&[make_node(&hash_v1, "crate::func")])
        .unwrap();
    store
        .insert_historical_nodes(&[
            make_node(&hash_v2, "crate::func"),
            make_node(&hash_v2, "crate::new_func"),
        ])
        .unwrap();

    let sha1 = "c1".repeat(20);
    let sid1 = store
        .insert_snapshot(repo_id, &sha1, None, 1, 0, 1, 1.0, 0)
        .unwrap();
    store
        .attach_snapshot_nodes(sid1, &hash_v1, &["crate::func".into()])
        .unwrap();

    let sha2 = "c2".repeat(20);
    let sid2 = store
        .insert_snapshot(repo_id, &sha2, None, 2, 0, 1, 1.0, 0)
        .unwrap();
    store
        .attach_snapshot_nodes(
            sid2,
            &hash_v2,
            &["crate::func".into(), "crate::new_func".into()],
        )
        .unwrap();

    assert_eq!(store.count_snapshot_nodes(sid1).unwrap(), 1);
    assert_eq!(store.count_snapshot_nodes(sid2).unwrap(), 2);
    assert_eq!(store.count_historical_nodes(&hash_v1).unwrap(), 1);
    assert_eq!(store.count_historical_nodes(&hash_v2).unwrap(), 2);
}
