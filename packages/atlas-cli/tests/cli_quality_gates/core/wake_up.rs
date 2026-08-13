//! ICM-D — wake-up pack E2E coverage: stable empty/normal/large packs, topic
//! prioritization, source-id-only artifact references, and SessionStart hook
//! metadata recording.

use super::*;
use atlas_adapters::derive_session_db_path;
use atlas_session::{SessionId, SessionStore};

fn wake_up(repo_root: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json", "wake-up"];
    full.extend_from_slice(args);
    read_json_data_output("wake-up", run_atlas(repo_root, &full))
}

fn store_memory(repo_root: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json", "memory", "store"];
    full.extend_from_slice(args);
    read_json_data_output("memory.store", run_atlas(repo_root, &full))
}

fn record_feedback(repo_root: &Path, args: &[&str]) -> Value {
    let mut full = vec!["--json", "feedback", "record"];
    full.extend_from_slice(args);
    read_json_data_output("feedback.record", run_atlas(repo_root, &full))
}

/// Calls an MCP tool over `atlas serve` and returns the parsed tool body.
fn mcp_call(repo_root: &Path, id: u64, name: &str, arguments: &str) -> Value {
    let request = serde_json::to_string(&json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": name,
            "arguments": serde_json::from_str::<Value>(arguments).expect("arguments json"),
        },
    }))
    .expect("serialize request");
    let requests = format!("{}{}\n", initialized_session_prelude(1), request);
    let output = run_serve_jsonrpc_session(repo_root, &["serve"], requests);
    read_json_tool_result(&output, id)
}

fn session_event_payloads(repo_root: &Path, session_id: &str) -> Vec<Value> {
    let graph_db = repo_root.join(".atlas").join("worldtree.db");
    let session_db = derive_session_db_path(graph_db.to_str().expect("graph db path"));
    let store = SessionStore::open(&session_db).expect("open session store");
    store
        .list_events(&SessionId(session_id.to_owned()))
        .expect("list events")
        .into_iter()
        .map(|event| serde_json::from_str(&event.payload_json).expect("event payload json"))
        .collect()
}

/// Empty session: stable normalized pack with no_session status and a
/// graph-not-built warning (snapshot coverage: empty).
#[test]
fn wake_up_empty_session_returns_stable_pack() {
    let repo = setup_fixture_repo();

    let pack = wake_up(repo.path(), &[]);
    assert_eq!(pack["frontend"], "cli");
    assert_eq!(pack["summary"]["status"], "no_session");
    assert_eq!(pack["summary"]["pending_resume"], false);
    assert_eq!(pack["summary"]["decision_count"], 0);
    assert_eq!(pack["summary"]["critical_memory_count"], 0);
    assert_eq!(pack["summary"]["feedback_count"], 0);
    assert!(pack["current_focus"]["intent"].is_null());
    assert!(pack["recent_decisions"].as_array().unwrap().is_empty());
    assert!(pack["critical_memories"].as_array().unwrap().is_empty());
    assert!(pack["recent_feedback"].as_array().unwrap().is_empty());
    assert_eq!(pack["graph_readiness"]["graph_built"], false);
    assert_eq!(pack["graph_readiness"]["execution_state"], "missing");
    assert!(pack["generated_at"].as_str().is_some());
    assert!(
        pack["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w.as_str().unwrap().contains("graph has not been built"))
    );
}

/// Normal session: `--topic hooks` prioritizes topic-relevant memories and
/// feedback records (snapshot coverage: normal).
#[test]
fn wake_up_topic_prioritizes_memories_and_feedback() {
    let repo = setup_fixture_repo();

    store_memory(
        repo.path(),
        &[
            "hooks must stay non-blocking",
            "--topic",
            "hooks",
            "--title",
            "hook policy",
            "--importance",
            "critical",
        ],
    );
    let feedback = record_feedback(
        repo.path(),
        &[
            "--predicted",
            "hooks are dead code",
            "--actual",
            "hooks fire on session start",
            "--correction",
            "session-start hooks are installed",
            "--analysis-kind",
            "dead_code",
            "--symbol",
            "src/lib.rs::fn::helper",
            "--file",
            "src/lib.rs",
        ],
    );
    let feedback_id = feedback["record"]["id"].as_str().expect("feedback id");

    let pack = wake_up(repo.path(), &["--topic", "hooks"]);
    assert_eq!(pack["current_focus"]["intent"], "hooks");
    let memories = pack["critical_memories"].as_array().unwrap();
    assert!(
        memories
            .iter()
            .any(|m| m["kind"] == "memory" && m["topic"] == "hooks"),
        "topic must surface the topic memory: {memories:?}"
    );
    let feedback_hits = pack["recent_feedback"].as_array().unwrap();
    assert!(
        feedback_hits
            .iter()
            .any(|f| f["record_id"] == feedback_id && f["predicted"] == "hooks are dead code"),
        "topic must surface the topic feedback: {feedback_hits:?}"
    );
    assert!(pack["summary"]["feedback_count"].as_i64().unwrap() >= 1);

    // No topic: the critical memory still appears (critical rows rank first).
    let pack = wake_up(repo.path(), &[]);
    assert!(
        pack["critical_memories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["kind"] == "memory" && m["importance"] == "critical"),
        "critical memories must surface without a topic"
    );
}

/// Large session: saved artifacts appear only as `source_id` references and
/// never inline bodies (snapshot coverage: large).
#[test]
fn wake_up_references_large_artifacts_by_source_id_only() {
    let repo = setup_fixture_repo();

    let payload = std::iter::repeat_n("safe oversized handoff payload", 180)
        .collect::<Vec<_>>()
        .join(" ");
    let saved = mcp_call(
        repo.path(),
        2,
        "save_context_artifact",
        &format!(
            r#"{{"label":"oversized-handoff","content":"{}","content_type":"text/plain"}}"#,
            payload
        ),
    );
    let source_id = saved["source_id"].as_str().expect("source id");

    let pack = wake_up(repo.path(), &[]);
    let serialized = serde_json::to_string(&pack).unwrap();
    assert!(
        !serialized.contains("safe oversized handoff payload"),
        "artifact body must never be inlined in wake-up output"
    );
    assert!(
        pack["critical_memories"]
            .as_array()
            .unwrap()
            .iter()
            .any(|m| m["kind"] == "artifact" && m["source_id"] == source_id),
        "artifact must be referenced by source_id"
    );
}

/// SessionStart hook path records wake-up success metadata in the session
/// event (ICM-D3) without failing the hook.
#[test]
fn session_start_hook_records_wake_up_metadata() {
    let repo = setup_repo(&[
        (
            "Cargo.toml",
            "[package]\nname = \"hook-wake\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        ),
        ("src/lib.rs", "pub fn alpha() {}\n"),
    ]);

    run_atlas(repo.path(), &["install", "--platform", "codex"]);
    run_installed_hook(repo.path(), "codex", "session-start", "{}");

    let session_id = SessionId::derive(repo.path().to_str().unwrap(), "", "codex");
    let payloads = session_event_payloads(repo.path(), session_id.as_str());
    let session_start = payloads
        .iter()
        .find(|payload| payload["hook_event"] == "session-start")
        .expect("session-start event must be recorded");
    let wake_up_meta = &session_start["payload"]["wake_up"];
    assert_eq!(wake_up_meta["status"], "ok");
    assert_eq!(wake_up_meta["max_items"], 10);
    assert!(wake_up_meta["generated_at"].as_str().is_some());
    assert!(wake_up_meta["error"].is_null());
    assert!(wake_up_meta["counts"]["memories"].as_i64().is_some());

    // The hook command itself still succeeds (non-blocking best-effort).
    run_installed_hook(repo.path(), "codex", "session-start", "{}");
}
