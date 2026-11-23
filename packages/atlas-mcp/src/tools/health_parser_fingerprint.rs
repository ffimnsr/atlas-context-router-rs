//! Parser fingerprint visibility for the MCP `status` and `doctor` tools.
//!
//! Mirrors the CLI surfaces so an MCP client sees the same `current | stale |
//! missing | unknown` state that `atlas status` and `atlas doctor` report.

use atlas_engine::parser_fingerprint::{PARSER_FINGERPRINT_REVISION, compute};
use atlas_store_sqlite::Store;
use camino::Utf8Path;
use serde_json::{Value, json};

/// Outcome of a doctor-style fingerprint probe.
pub(super) struct FingerprintCheck {
    pub ok: bool,
    pub detail: String,
    pub issue_code: Option<&'static str>,
}

/// Compare the stored graph fingerprint with the current parser configuration.
pub(super) fn state(store: Option<&Store>, repo_root: &str) -> &'static str {
    let Some(store) = store else {
        return "unknown";
    };
    let Ok(config) = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)) else {
        return "unknown";
    };
    let expected = compute(&config.parsers.external);
    let source_repo_id = atlas_repo::stable_repo_id(Utf8Path::new(repo_root));
    match store.parser_fingerprint(&source_repo_id) {
        Ok(Some(stored)) if stored == expected => "current",
        Ok(Some(_)) => "stale",
        Ok(None) => "missing",
        Err(_) => "unknown",
    }
}

/// `graph_state.parser_fingerprint` block for the MCP `status` payload.
pub(super) fn status_block(store: Option<&Store>, repo_root: &str) -> Value {
    json!({
        "state": state(store, repo_root),
        "revision": PARSER_FINGERPRINT_REVISION,
    })
}

/// Doctor probe; returns `None` when the graph store or config cannot be read,
/// because the `db_open` / `config_file` checks already report that failure.
pub(super) fn doctor_check(repo_root: &str, db_path: &str) -> Option<FingerprintCheck> {
    let config = atlas_engine::Config::load(&atlas_engine::paths::atlas_dir(repo_root)).ok()?;
    let store = Store::open(db_path).ok()?;
    let source_repo_id = atlas_repo::stable_repo_id(Utf8Path::new(repo_root));
    let check = atlas_engine::doctor::parser_fingerprint_check(
        &store,
        &source_repo_id,
        &config.parsers.external,
    );
    Some(FingerprintCheck {
        ok: check.ok,
        detail: check.message,
        issue_code: check.issue_code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_with_store() -> (tempfile::TempDir, String, String, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let repo_root = dir.path().to_string_lossy().into_owned();
        let db_path = dir.path().join("atlas.db").to_string_lossy().into_owned();
        let store = Store::open(&db_path).expect("open store");
        (dir, repo_root, db_path, store)
    }

    #[test]
    fn status_reports_missing_without_fingerprint() {
        let (_dir, repo_root, _db_path, store) = repo_with_store();
        let block = status_block(Some(&store), &repo_root);
        assert_eq!(block["state"], json!("missing"));
        assert_eq!(block["revision"], json!(PARSER_FINGERPRINT_REVISION));
    }

    #[test]
    fn status_reports_current_after_fingerprint_written() {
        let (_dir, repo_root, _db_path, store) = repo_with_store();
        let expected = compute(&[]);
        store
            .set_parser_fingerprint(
                &atlas_repo::stable_repo_id(Utf8Path::new(&repo_root)),
                &expected,
            )
            .expect("write fingerprint");
        assert_eq!(
            status_block(Some(&store), &repo_root)["state"],
            json!("current")
        );
    }

    #[test]
    fn status_reports_stale_for_other_fingerprint() {
        let (_dir, repo_root, _db_path, store) = repo_with_store();
        store
            .set_parser_fingerprint(
                &atlas_repo::stable_repo_id(Utf8Path::new(&repo_root)),
                "0000",
            )
            .expect("write fingerprint");
        assert_eq!(
            status_block(Some(&store), &repo_root)["state"],
            json!("stale")
        );
    }

    #[test]
    fn status_reports_unknown_without_store() {
        let (_dir, repo_root, _db_path, _store) = repo_with_store();
        assert_eq!(status_block(None, &repo_root)["state"], json!("unknown"));
    }

    #[test]
    fn doctor_check_tracks_missing_then_current() {
        let (_dir, repo_root, db_path, store) = repo_with_store();
        let missing = doctor_check(&repo_root, &db_path).expect("doctor check");
        assert!(!missing.ok);
        assert_eq!(missing.issue_code, Some("parser_fingerprint_missing"));

        let expected = compute(&[]);
        store
            .set_parser_fingerprint(
                &atlas_repo::stable_repo_id(Utf8Path::new(&repo_root)),
                &expected,
            )
            .expect("write fingerprint");
        let current = doctor_check(&repo_root, &db_path).expect("doctor check");
        assert!(current.ok);
        assert_eq!(current.issue_code, None);

        store
            .set_parser_fingerprint(
                &atlas_repo::stable_repo_id(Utf8Path::new(&repo_root)),
                "0000",
            )
            .expect("write fingerprint");
        let stale = doctor_check(&repo_root, &db_path).expect("doctor check");
        assert!(!stale.ok);
        assert_eq!(stale.issue_code, Some("parser_fingerprint_stale"));
    }
}
