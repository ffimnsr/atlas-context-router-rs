use super::*;

#[test]
fn best_effort_open_in_nonexistent_dir_creates_path() {
    let dir = TempDir::new().unwrap();
    let nested = dir.path().join("deep").join("nested").join(".atlas");
    let path = nested.join(DEFAULT_SESSION_DB);
    let result = SessionStore::open(path.to_str().unwrap());
    assert!(result.is_ok(), "store open must create missing dirs");
}

#[test]
fn corrupt_db_is_quarantined_on_open() {
    let dir = TempDir::new().unwrap();
    let atlas_dir = dir.path().join(".atlas");
    std::fs::create_dir_all(&atlas_dir).unwrap();
    let path = atlas_dir.join(DEFAULT_SESSION_DB);

    std::fs::write(&path, b"this is not a sqlite database").unwrap();

    let result = SessionStore::open(path.to_str().unwrap());
    assert!(result.is_err(), "corrupt DB must return error");

    let quarantine = atlas_dir.join(format!("{}.quarantine", DEFAULT_SESSION_DB));
    assert!(
        quarantine.exists(),
        "quarantine file must be created for corrupt DB"
    );
}

#[test]
fn quarantine_allows_fresh_open_after_corruption() {
    let dir = TempDir::new().unwrap();
    let atlas_dir = dir.path().join(".atlas");
    std::fs::create_dir_all(&atlas_dir).unwrap();
    let path = atlas_dir.join(DEFAULT_SESSION_DB);

    std::fs::write(&path, b"not a database").unwrap();
    let _ = SessionStore::open(path.to_str().unwrap());

    let store = SessionStore::open(path.to_str().unwrap());
    assert!(
        store.is_ok(),
        "fresh open after quarantine must succeed: {:?}",
        store.err()
    );
}

#[test]
fn is_corruption_error_matches_known_strings() {
    let cases = [
        "database disk image is malformed",
        "file is not a database",
        "not a database",
    ];
    for msg in cases {
        let err = atlas_core::AtlasError::Db(msg.to_string());
        assert!(
            util::is_corruption_error(&err),
            "must detect corruption in: {msg}"
        );
    }
}

#[test]
fn is_corruption_error_does_not_match_normal_errors() {
    let err = atlas_core::AtlasError::Db("disk I/O error (SQLITE_IOERR)".to_string());
    assert!(!util::is_corruption_error(&err));
}
