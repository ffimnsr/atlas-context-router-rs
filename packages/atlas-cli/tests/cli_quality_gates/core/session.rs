use super::*;
use atlas_session::NewSessionEvent;

#[test]
fn session_decisions_json_returns_artifact_refs_for_current_session() {
    let repo = setup_repo(&[
        (
            "Cargo.toml",
            "[package]\nname = \"session-decisions\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
        ("src/lib.rs", "pub fn verify_token() -> bool { true }\n"),
    ]);

    let repo_root = canonical_path(repo.path());
    let repo_root_str = repo_root.to_string_lossy().into_owned();
    let cli_session = SessionId::derive(&repo_root_str, "", "cli");
    let other_session = SessionId::derive(&repo_root_str, "", "codex");

    let mut store = SessionStore::open_in_repo(repo.path()).expect("open session store");
    store
        .upsert_session_meta(cli_session.clone(), &repo_root_str, "cli", None)
        .expect("upsert cli session meta");
    store
        .upsert_session_meta(other_session.clone(), &repo_root_str, "codex", None)
        .expect("upsert codex session meta");
    store
        .append_event(NewSessionEvent {
            session_id: other_session,
            event_type: SessionEventType::Decision,
            priority: 4,
            payload: json!({
                "summary": "stale external decision",
                "query": "verify_token",
                "source_id": "artifact-other",
                "evidence": [{"kind": "saved_context", "source_id": "artifact-other"}],
            }),
            created_at: None,
        })
        .expect("append codex decision event");
    store
        .append_event(NewSessionEvent {
            session_id: cli_session.clone(),
            event_type: SessionEventType::Decision,
            priority: 4,
            payload: json!({
                "summary": "reuse verify_token rollout decision",
                "rationale": "same auth path touched again",
                "conclusion": "reuse current-session decision",
                "query": "verify_token",
                "source_id": "artifact-cli",
                "evidence": [{"kind": "saved_context", "source_id": "artifact-cli"}],
            }),
            created_at: None,
        })
        .expect("append cli decision event");

    let data = read_json_data_output(
        "session.decisions",
        run_atlas(
            repo.path(),
            &[
                "--json",
                "session",
                "decisions",
                "verify_token",
                "--current-session",
            ],
        ),
    );

    assert_eq!(data["query"], json!("verify_token"));
    assert_eq!(data["session_id"], json!(cli_session.as_str()));
    assert_eq!(data["total"], json!(1));
    assert_eq!(data["results"].as_array().expect("results array").len(), 1);
    assert_eq!(
        data["results"][0]["decision"]["summary"],
        json!("reuse verify_token rollout decision")
    );
    assert_eq!(
        data["results"][0]["decision"]["source_ids"][0],
        json!("artifact-cli")
    );
    assert_eq!(
        data["results"][0]["decision"]["evidence"][0]["source_id"],
        json!("artifact-cli")
    );
}
