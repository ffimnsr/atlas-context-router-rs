use std::fs;

use atlas_core::{EdgeKind, GraphStoreHealthClass, NodeKind};

use super::*;

#[test]
fn corrupt_graph_db_is_quarantined_and_fresh_db_created() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("worldtree.db");
    fs::write(&db_path, b"garbage sqlite bytes").unwrap();
    fs::write(dir.path().join("worldtree.db-wal"), b"wal").unwrap();
    fs::write(dir.path().join("worldtree.db-shm"), b"shm").unwrap();

    let recovery = Store::prepare_graph_store_rebuild(
        db_path.to_str().unwrap(),
        crate::GraphRecoveryMode::AutoQuarantineAndRebuild,
        true,
    )
    .expect("auto quarantine must succeed");

    assert_eq!(
        recovery.health_class,
        Some(GraphStoreHealthClass::SqliteCorrupt)
    );
    assert!(recovery.full_rebuild_required);
    let quarantine = recovery.quarantine_path.expect("quarantine path");
    assert!(std::path::Path::new(&quarantine).exists());

    let reopened = Store::open(db_path.to_str().unwrap()).expect("fresh db must open");
    assert_eq!(
        reopened.schema_version().unwrap(),
        crate::migrations::LATEST_VERSION
    );
}

#[test]
fn logical_inconsistency_triggers_quarantine_and_full_rebuild() {
    let (dir, path, mut store) = open_file_backed();
    let node = make_node(
        NodeKind::Function,
        "caller",
        "src/lib.rs::fn::caller",
        "src/lib.rs",
        "rust",
    );
    store
        .replace_file_graph("src/lib.rs", "hash", Some("rust"), None, &[node], &[])
        .unwrap();
    store.conn.execute(
        "INSERT INTO edges(kind, source_qualified, target_qualified, file_path, confidence, confidence_tier, extra_json, source_repo_id)
         VALUES (?1, ?2, ?3, ?4, 1.0, NULL, '{}', 'legacy')",
        rusqlite::params![
            EdgeKind::Calls.as_str(),
            "src/lib.rs::fn::caller",
            "src/lib.rs::fn::missing",
            "src/lib.rs"
        ],
    ).unwrap();
    drop(store);

    let recovery = Store::prepare_graph_store_rebuild(
        &path,
        crate::GraphRecoveryMode::AutoQuarantineAndRebuild,
        true,
    )
    .expect("logical inconsistency should quarantine");

    assert_eq!(
        recovery.health_class,
        Some(GraphStoreHealthClass::LogicalInconsistency)
    );
    assert!(recovery.full_rebuild_required);
    let quarantine = recovery.quarantine_path.expect("quarantine path");
    assert!(std::path::Path::new(&quarantine).exists());
    assert!(dir.path().join("test.sqlite").exists());
}

#[test]
fn block_only_mode_reports_corruption_without_mutating_db() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("worldtree.db");
    fs::write(&db_path, b"garbage sqlite bytes").unwrap();

    let error = Store::prepare_graph_store_rebuild(
        db_path.to_str().unwrap(),
        crate::GraphRecoveryMode::BlockOnly,
        false,
    )
    .unwrap_err();

    assert_eq!(error.health_class, GraphStoreHealthClass::SqliteCorrupt);
    assert_eq!(error.recovery_mode, crate::GraphRecoveryMode::BlockOnly);
    assert!(db_path.exists(), "block_only must not mutate graph db");
    assert!(error.quarantine_path.is_none());
}

#[test]
fn auto_quarantine_requires_explicit_opt_in_outside_build_update() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("worldtree.db");
    fs::write(&db_path, b"garbage sqlite bytes").unwrap();

    let error = Store::prepare_graph_store_rebuild(
        db_path.to_str().unwrap(),
        crate::GraphRecoveryMode::AutoQuarantineAndRebuild,
        false,
    )
    .unwrap_err();

    assert_eq!(error.health_class, GraphStoreHealthClass::SqliteCorrupt);
    assert_eq!(
        error.recovery_mode,
        crate::GraphRecoveryMode::ManualRebuildRequired
    );
    assert!(
        db_path.exists(),
        "auto mode without explicit opt-in must not mutate db"
    );
    assert!(error.quarantine_path.is_none());
    assert!(
        error
            .failure_reason
            .as_deref()
            .is_some_and(|text| { text.contains("requires explicit opt-in") })
    );
}
