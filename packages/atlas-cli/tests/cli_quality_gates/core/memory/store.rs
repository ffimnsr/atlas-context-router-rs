use super::*;

#[test]
fn memory_store_creates_record_with_defaults() {
    let repo = setup_fixture_repo();

    let payload = store_memory(repo.path(), &["remember to run the hook tests"]);

    let memory = &payload["memory"];
    let id = memory_id(&payload);
    assert_eq!(id.len(), 64, "id must be a stable sha256 hex id");
    assert_eq!(memory["body"], json!("remember to run the hook tests"));
    assert_eq!(memory["importance"], json!("normal"));
    assert_eq!(memory["scope"], json!("project"));
    assert_eq!(memory["topic"], json!(""));
    assert_eq!(memory["decay_score"], json!(0.0));
    assert_eq!(memory["session_id"], json!(null));
    assert_eq!(memory["created_at"], memory["updated_at"]);
    assert_eq!(memory["updated_at"], memory["last_accessed_at"]);

    // Row must exist in the continuity-side session database.
    let session_db = repo.path().join(".atlas").join("session.db");
    let conn = Connection::open(&session_db).expect("open session db");
    let (body, importance, scope): (String, String, String) = conn
        .query_row(
            "SELECT body, importance, scope FROM memories WHERE id = ?1",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .expect("stored row");
    assert_eq!(body, "remember to run the hook tests");
    assert_eq!(importance, "normal");
    assert_eq!(scope, "project");
}

#[test]
fn memory_store_persists_all_flags() {
    let repo = setup_fixture_repo();

    let payload = store_memory(
        repo.path(),
        &[
            "frontend-private deploy note",
            "--topic",
            "deploy",
            "--title",
            "Deploy note",
            "--importance",
            "critical",
            "--scope",
            "frontend",
            "--frontend",
            "codex",
            "--source-id",
            "artifact-9",
        ],
    );

    let memory = &payload["memory"];
    assert_eq!(memory["importance"], json!("critical"));
    assert_eq!(memory["scope"], json!("frontend"));
    assert_eq!(memory["frontend"], json!("codex"));
    assert_eq!(memory["topic"], json!("deploy"));
    assert_eq!(memory["title"], json!("Deploy note"));
    assert_eq!(memory["source_id"], json!("artifact-9"));
    assert_eq!(memory["body"], json!("frontend-private deploy note"));

    // Global scope needs no active session either.
    let global = store_memory(
        repo.path(),
        &["global note", "--scope", "global", "--importance", "high"],
    );
    assert_eq!(global["memory"]["scope"], json!("global"));
    assert_eq!(global["memory"]["session_id"], json!(null));
}

#[test]
fn memory_store_rejects_invalid_values() {
    let repo = setup_fixture_repo();

    let cases: &[(&[&str], &str)] = &[
        (
            &["x", "--importance", "urgent"],
            "unknown memory importance",
        ),
        (&["x", "--scope", "org"], "unknown memory scope"),
        (
            &["x", "--scope", "frontend"],
            "requires a frontend identifier",
        ),
        (&["   "], "memory body must not be empty"),
    ];

    for (args, expected) in cases {
        let mut full = vec!["memory", "store"];
        full.extend_from_slice(args);
        let output = run_atlas_capture(repo.path(), &full);
        assert!(!output.status.success(), "store {:?} must fail", args);
        let stderr = String::from_utf8(output.stderr).expect("stderr utf-8");
        assert!(
            stderr.contains(expected),
            "stderr must mention {expected:?}\nstderr:\n{stderr}"
        );
    }
}
