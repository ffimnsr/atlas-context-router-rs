use std::io::{self, Cursor, Read};
use std::path::Path;
use std::process::Command as ProcessCommand;

use camino::Utf8Path;
use serde_json::{Value, json};
use tempfile::TempDir;

use atlas_adapters::{
    ArtifactIdentity, derive_content_db_path, derive_session_db_path, generate_source_id,
    normalize_event, redact_payload,
};
use atlas_contentstore::{ContentStore, SourceMeta};
use atlas_engine::{BuildOptions, build_graph};
use atlas_session::{SessionEventType, SessionId, SessionStore};
use atlas_store_sqlite::Store;

use crate::cli::{Cli, Command};
use crate::cli_paths::canonicalize_cli_path;

use super::actions::execute_hook_actions;
use super::policy::{HookEventParts, resolve_hook_policy};
use super::runtime::{
    build_hook_event, persist_hook_event, read_hook_payload_from, resolve_hook_repo,
};

fn hook_cli_without_repo() -> Cli {
    Cli {
        repo: None,
        db: None,
        verbose: false,
        json: false,
        command: Command::Hook {
            event: "session-start".to_owned(),
        },
    }
}

struct PanicRead;

impl Read for PanicRead {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        panic!("reader should not be touched when stdin is a terminal");
    }
}

fn last_hook_payload(graph_db_path: &str, repo: &str, frontend: &str) -> Value {
    let session_store = SessionStore::open(&derive_session_db_path(graph_db_path)).unwrap();
    let session_id = SessionId::derive(repo, "", frontend);
    let events = session_store.list_events(&session_id).unwrap();
    serde_json::from_str(&events.last().unwrap().payload_json).unwrap()
}

#[test]
fn read_hook_payload_from_terminal_returns_null_without_reading() {
    let payload = read_hook_payload_from(PanicRead, true).unwrap();
    assert_eq!(payload, Value::Null);
}

#[test]
fn read_hook_payload_from_parses_json_and_redacts_secrets() {
    let payload = read_hook_payload_from(
        Cursor::new(br#"{"token":"secret-value","nested":{"raw":"keep"}}"#),
        false,
    )
    .unwrap();

    assert_eq!(payload["token"], "[REDACTED]");
    assert_eq!(payload["nested"]["raw"], "keep");
}

#[test]
fn session_start_hook_records_resume_when_snapshot_pending() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let session_id = SessionId::derive(&repo, "", "hook");

    let mut store = SessionStore::open_in_repo(dir.path()).unwrap();
    store
        .upsert_session_meta(session_id.clone(), &repo, "cli", None)
        .unwrap();
    store
        .append_event(
            normalize_event(
                SessionEventType::CommandRun,
                2,
                json!({ "command": "cargo test", "status": "ok" }),
            )
            .bind(session_id.clone()),
        )
        .unwrap();
    store.build_resume(&session_id).unwrap();

    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    persist_hook_event(&repo, &graph_db_path, "hook", "session-start", Value::Null).unwrap();

    let store = SessionStore::open_in_repo(dir.path()).unwrap();
    let events = store.list_events(&session_id).unwrap();
    let last = events.last().expect("hook should append an event");
    assert_eq!(last.event_type, SessionEventType::SessionResume);
}

#[test]
fn session_start_hook_bootstraps_frontend_scoped_session_without_session_command() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let session_id = SessionId::derive(&repo, "", "hook");
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let persisted =
        persist_hook_event(&repo, &graph_db_path, "hook", "session-start", Value::Null).unwrap();
    assert_eq!(persisted.session_id, session_id);
    assert!(!persisted.pending_resume);
    assert!(persisted.stored_event_id.is_some());

    let store = SessionStore::open_in_repo(Path::new(&repo)).unwrap();
    let sessions = store.list_sessions().unwrap();
    assert!(
        sessions
            .iter()
            .any(|session| session.session_id == session_id),
        "session-start hook should register session metadata"
    );

    let events = store.list_events(&session_id).unwrap();
    let last = events.last().expect("hook should append an event");
    assert_eq!(last.event_type, SessionEventType::SessionStart);
}

#[test]
fn resolve_hook_repo_prefers_runner_script_git_root() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path();
    std::fs::create_dir_all(repo.join(".atlas/hooks")).unwrap();
    std::fs::write(repo.join(".atlas/hooks/atlas-hook"), "#!/bin/sh\n").unwrap();
    assert!(
        ProcessCommand::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(repo)
            .status()
            .unwrap()
            .success()
    );

    let prior_script = std::env::var("ATLAS_HOOK_SCRIPT_PATH").ok();
    unsafe {
        std::env::set_var(
            "ATLAS_HOOK_SCRIPT_PATH",
            repo.join(".atlas/hooks/atlas-hook")
                .to_string_lossy()
                .into_owned(),
        );
    }

    let resolved = resolve_hook_repo(&hook_cli_without_repo()).unwrap();
    let expected = canonicalize_cli_path(repo.to_string_lossy().as_ref()).unwrap();

    if let Some(value) = prior_script {
        unsafe {
            std::env::set_var("ATLAS_HOOK_SCRIPT_PATH", value);
        }
    } else {
        unsafe {
            std::env::remove_var("ATLAS_HOOK_SCRIPT_PATH");
        }
    }

    assert_eq!(resolved, expected);
}

#[test]
fn pre_compact_hook_builds_resume_snapshot() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let session_id = SessionId::derive(&repo, "", "hook");
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    persist_hook_event(&repo, &graph_db_path, "hook", "user-prompt", Value::Null).unwrap();
    let persisted =
        persist_hook_event(&repo, &graph_db_path, "hook", "pre-compact", Value::Null).unwrap();
    assert!(persisted.snapshot.is_some());

    let store = SessionStore::open_in_repo(dir.path()).unwrap();
    let snapshot = store.get_resume_snapshot(&session_id).unwrap();
    assert!(
        snapshot.is_some(),
        "pre-compact should build a resume snapshot"
    );
}

#[test]
fn build_hook_event_redacts_secret_payload_fields() {
    let session_id = SessionId::derive("/repo", "", "hook");
    let payload = redact_payload(json!({ "token": "secret", "safe": "ok" }));
    let event = build_hook_event(
        &session_id,
        HookEventParts {
            frontend: "hook",
            event: "tool-failure",
            payload,
            hook_metadata: json!({}),
            source_id: None,
            storage_kind: None,
            pending_resume: false,
        },
    );
    assert_eq!(event.event_type, SessionEventType::CommandFail);
    assert_eq!(event.payload["frontend"], "hook");
    assert_eq!(event.payload["payload"]["token"], "[REDACTED]");
    assert_eq!(event.payload["payload"]["safe"], "ok");
}

#[test]
fn build_hook_event_maps_permission_denied_to_decision() {
    let session_id = SessionId::derive("/repo", "", "claude");
    let event = build_hook_event(
        &session_id,
        HookEventParts {
            frontend: "claude",
            event: "permission-denied",
            payload: json!({ "tool": "Bash" }),
            hook_metadata: json!({}),
            source_id: None,
            storage_kind: None,
            pending_resume: false,
        },
    );
    assert_eq!(event.event_type, SessionEventType::Decision);
    assert_eq!(event.payload["hook_event"], "permission-denied");
}

#[test]
fn build_hook_event_maps_post_tool_use_to_graph_update() {
    let session_id = SessionId::derive("/repo", "", "copilot");
    let event = build_hook_event(
        &session_id,
        HookEventParts {
            frontend: "copilot",
            event: "post-tool-use",
            payload: json!({ "tool": "Edit" }),
            hook_metadata: json!({}),
            source_id: None,
            storage_kind: None,
            pending_resume: false,
        },
    );
    assert_eq!(event.event_type, SessionEventType::GraphUpdate);
    assert_eq!(event.payload["frontend"], "copilot");
}

#[test]
fn build_hook_event_maps_aliases_through_policy_table() {
    let session_id = SessionId::derive("/repo", "", "copilot");
    let event = build_hook_event(
        &session_id,
        HookEventParts {
            frontend: "copilot",
            event: "userPromptSubmitted",
            payload: json!({ "prompt": "hi" }),
            hook_metadata: json!({}),
            source_id: None,
            storage_kind: None,
            pending_resume: false,
        },
    );
    assert_eq!(event.event_type, SessionEventType::UserIntent);
    assert_eq!(event.payload["hook_event"], "user-prompt");
}

#[test]
fn persist_hook_event_rejects_unknown_hook_name() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let error = persist_hook_event(&repo, &graph_db_path, "hook", "mystery-event", Value::Null)
        .unwrap_err();

    assert!(
        error
            .to_string()
            .contains("unknown hook event: mystery-event")
    );
}

#[test]
fn large_post_tool_use_payload_routes_to_content_store() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let payload = json!({ "output": "x".repeat(6_000) });

    let persisted =
        persist_hook_event(&repo, &graph_db_path, "claude", "post-tool-use", payload).unwrap();

    let source_id = persisted
        .source_id
        .expect("routed hook should store source id");
    assert_eq!(persisted.storage_kind, Some("pointer"));

    let mut session_store = SessionStore::open(&derive_session_db_path(&graph_db_path)).unwrap();
    let session_id = SessionId::derive(&repo, "", "claude");
    let events = session_store.list_events(&session_id).unwrap();
    let last_payload: Value = serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
    assert_eq!(last_payload["source_id"], source_id);
    assert_eq!(last_payload["payload_storage"]["kind"], "pointer");

    let mut content_store = ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
    content_store.migrate().unwrap();
    assert!(content_store.get_source(&source_id).unwrap().is_some());

    let snapshot = session_store.build_resume(&session_id).unwrap();
    let snapshot_value: Value = serde_json::from_str(&snapshot.snapshot).unwrap();
    assert!(
        snapshot_value["saved_artifact_refs"]
            .as_array()
            .unwrap()
            .contains(&json!(source_id))
    );
}

#[test]
fn large_session_only_hook_routes_to_content_store() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let payload = json!({ "output": "x".repeat(6_000) });

    let persisted =
        persist_hook_event(&repo, &graph_db_path, "codex", "pre-tool-use", payload).unwrap();

    let source_id = persisted
        .source_id
        .expect("oversized session-only hook should store source id");
    assert_eq!(persisted.storage_kind, Some("pointer"));

    let session_store = SessionStore::open(&derive_session_db_path(&graph_db_path)).unwrap();
    let session_id = SessionId::derive(&repo, "", "codex");
    let events = session_store.list_events(&session_id).unwrap();
    let last_payload: Value = serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
    assert_eq!(last_payload["source_id"], source_id);
    assert_eq!(last_payload["payload_storage"]["kind"], "pointer");

    let mut content_store = ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
    content_store.migrate().unwrap();
    assert!(content_store.get_source(&source_id).unwrap().is_some());
}

#[test]
fn session_start_hook_loads_resume_and_marks_snapshot_consumed() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let session_id = SessionId::derive(&repo, "", "hook");
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let mut store = SessionStore::open_in_repo(dir.path()).unwrap();
    store
        .upsert_session_meta(session_id.clone(), &repo, "hook", None)
        .unwrap();
    store
        .append_event(
            normalize_event(
                SessionEventType::UserIntent,
                3,
                json!({ "prompt": "review billing flow", "files": ["src/lib.rs"] }),
            )
            .bind(session_id.clone()),
        )
        .unwrap();
    store.build_resume(&session_id).unwrap();

    let persisted =
        persist_hook_event(&repo, &graph_db_path, "hook", "session-start", Value::Null).unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("session-start").unwrap(),
        &persisted,
        &Value::Null,
    );

    assert_eq!(actions["lifecycle"]["status"], "loaded");
    assert_eq!(actions["lifecycle"]["resume_loaded"], true);

    let store = SessionStore::open_in_repo(dir.path()).unwrap();
    let snapshot = store.get_resume_snapshot(&session_id).unwrap().unwrap();
    assert!(
        snapshot.consumed,
        "restore should mark pending snapshot consumed"
    );

    let events = store.list_events(&session_id).unwrap();
    let persisted_payload: Value =
        serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
    assert_eq!(
        persisted_payload["hook_metadata"]["restore_metadata"]["pending_resume"],
        true
    );
    assert_eq!(
        persisted_payload["hook_metadata"]["restore_metadata"]["has_resume_snapshot"],
        true
    );
}

#[test]
fn user_prompt_hook_routes_intent_and_finds_saved_context() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let identity = ArtifactIdentity::artifact_label("review-context");
    let source_id = generate_source_id(&identity, "BillingService review context");

    std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();

    let mut content_store = ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
    content_store.migrate().unwrap();
    content_store
        .index_artifact(
            SourceMeta {
                id: source_id.clone(),
                session_id: None,
                source_type: "review_context".to_owned(),
                label: "review context".to_owned(),
                repo_root: Some(repo.clone()),
                identity_kind: identity.kind_str().to_owned(),
                identity_value: identity.value().to_owned(),
            },
            "BillingService review context and call graph",
            "text/plain",
        )
        .unwrap();

    let payload = json!({ "prompt": "who calls BillingService" });
    let persisted = persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "user-prompt",
        payload.clone(),
    )
    .unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("user-prompt").unwrap(),
        &persisted,
        &payload,
    );

    assert_eq!(actions["prompt_routing"]["status"], "routed");
    assert_eq!(actions["prompt_routing"]["intent"], "usage_lookup");
    assert_eq!(
        actions["prompt_routing"]["saved_context_hits"][0]["source_id"],
        source_id
    );

    let session_store = SessionStore::open(&derive_session_db_path(&graph_db_path)).unwrap();
    let session_id = SessionId::derive(&repo, "", "hook");
    let events = session_store.list_events(&session_id).unwrap();
    let persisted_payload: Value =
        serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
    assert_eq!(
        persisted_payload["hook_metadata"]["saved_artifact_refs"][0],
        source_id
    );
    assert_eq!(
        persisted_payload["hook_metadata"]["source_summaries"][0]["source_id"],
        source_id
    );
    assert_eq!(
        persisted_payload["hook_metadata"]["retrieval_hints"][0]["kind"],
        "prompt_query"
    );
}

#[test]
fn stop_hook_persists_handoff_artifact() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "user-prompt",
        json!({ "prompt": "review src/lib.rs", "files": ["src/lib.rs"] }),
    )
    .unwrap();
    let persisted = persist_hook_event(&repo, &graph_db_path, "hook", "stop", Value::Null).unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("stop").unwrap(),
        &persisted,
        &Value::Null,
    );

    let source_id = actions["lifecycle"]["resume_source_id"]
        .as_str()
        .expect("stop should persist a handoff artifact");
    let mut content_store = ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
    content_store.migrate().unwrap();
    assert!(content_store.get_source(source_id).unwrap().is_some());

    assert_eq!(actions["lifecycle"]["status"], "persisted");
    assert_eq!(actions["lifecycle"]["snapshot_event_count"], 2);
    assert!(
        actions["lifecycle"]["context_hints"]["recent_files"]
            .as_array()
            .unwrap()
            .contains(&json!("src/lib.rs"))
    );
    assert!(
        actions["lifecycle"]["context_hints"]["recent_hook_events"]
            .as_array()
            .unwrap()
            .contains(&json!("user-prompt"))
    );
}

#[test]
fn session_end_hook_persists_handoff_artifact() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "user-prompt",
        json!({ "prompt": "handoff active plan", "files": ["src/lib.rs"] }),
    )
    .unwrap();
    let persisted =
        persist_hook_event(&repo, &graph_db_path, "hook", "session-end", Value::Null).unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("session-end").unwrap(),
        &persisted,
        &Value::Null,
    );

    assert_eq!(actions["lifecycle"]["status"], "persisted");
    assert_eq!(actions["lifecycle"]["snapshot_event_count"], 2);
    assert!(
        actions["lifecycle"]["context_hints"]["recent_files"]
            .as_array()
            .unwrap()
            .contains(&json!("src/lib.rs"))
    );
    assert!(
        actions["lifecycle"]["context_hints"]["recent_hook_events"]
            .as_array()
            .unwrap()
            .contains(&json!("user-prompt"))
    );
}

#[test]
fn file_changed_hook_marks_stale_and_drops_inline_content() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let changed_file = format!("{repo}/src/lib.rs");
    let payload = json!({
        "changed_files": [changed_file],
        "content": "secret inline contents",
        "diff": "@@ -1 +1 @@",
        "files": [{
            "path": "src/lib.rs",
            "before": "old body",
            "after": "new body",
            "snippet": "pub fn beta() {}"
        }]
    });

    let persisted = persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "file-changed",
        payload.clone(),
    )
    .unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("file-changed").unwrap(),
        &persisted,
        &payload,
    );
    let persisted_payload = last_hook_payload(&graph_db_path, &repo, "hook");

    assert_eq!(actions["freshness"]["status"], "stale");
    assert_eq!(actions["freshness"]["inline_content_persisted"], false);
    assert!(
        actions["freshness"]["changed_files"]
            .as_array()
            .unwrap()
            .contains(&json!("src/lib.rs"))
    );
    assert_eq!(
        persisted_payload["hook_metadata"]["freshness"]["status"],
        "stale"
    );
    assert_eq!(
        persisted_payload["hook_metadata"]["freshness"]["inline_content_persisted"],
        false
    );
    assert!(persisted_payload["payload"]["content"].is_null());
    assert!(persisted_payload["payload"]["diff"].is_null());
    assert!(persisted_payload["payload"]["files"][0]["before"].is_null());
    assert!(persisted_payload["payload"]["files"][0]["after"].is_null());
    if let Some(source_id) = persisted.source_id {
        assert!(persisted.storage_kind.is_some());
        let mut content_store =
            ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
        content_store.migrate().unwrap();
        assert!(content_store.get_source(&source_id).unwrap().is_some());
    } else {
        assert!(persisted.storage_kind.is_none());
    }
}

#[test]
fn large_user_prompt_payload_routes_to_content_store() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let payload = json!({ "prompt": format!("review {}", "x".repeat(6_000)) });

    let persisted = persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "user-prompt",
        payload.clone(),
    )
    .unwrap();
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("user-prompt").unwrap(),
        &persisted,
        &payload,
    );
    let persisted_payload = last_hook_payload(&graph_db_path, &repo, "hook");

    assert_eq!(persisted.storage_kind, Some("pointer"));
    assert!(persisted.source_id.is_some());
    assert_eq!(actions["prompt_routing"]["status"], "routed");
    assert_eq!(persisted_payload["payload_storage"]["kind"], "pointer");
}

#[test]
fn large_stop_payload_routes_to_content_store() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");
    let payload = json!({ "summary": "x".repeat(6_000) });

    let persisted = persist_hook_event(&repo, &graph_db_path, "hook", "stop", payload).unwrap();
    let persisted_payload = last_hook_payload(&graph_db_path, &repo, "hook");

    assert_eq!(persisted.storage_kind, Some("pointer"));
    assert!(persisted.source_id.is_some());
    assert!(persisted.snapshot.is_some());
    assert_eq!(persisted_payload["payload_storage"]["kind"], "pointer");
    assert_eq!(persisted_payload["hook_event"], "stop");
}

#[test]
fn pre_and_post_compact_hooks_round_trip_resume_snapshot() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    persist_hook_event(
        &repo,
        &graph_db_path,
        "hook",
        "user-prompt",
        json!({ "prompt": "compact this session" }),
    )
    .unwrap();
    let pre_compact =
        persist_hook_event(&repo, &graph_db_path, "hook", "pre-compact", Value::Null).unwrap();
    assert_eq!(pre_compact.snapshot.as_ref().unwrap()["event_count"], 2);

    let post_compact =
        persist_hook_event(&repo, &graph_db_path, "hook", "post-compact", Value::Null).unwrap();
    let post_actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        "hook",
        resolve_hook_policy("post-compact").unwrap(),
        &post_compact,
        &Value::Null,
    );

    assert_eq!(post_actions["lifecycle"]["status"], "verified");
    assert_eq!(post_actions["lifecycle"]["has_resume_snapshot"], true);
}

#[test]
fn post_tool_use_hook_refreshes_graph_for_changed_files() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"hook-refresh\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::create_dir_all(repo.join(".atlas")).unwrap();
    assert!(
        ProcessCommand::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(repo)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        ProcessCommand::new("git")
            .args(["add", "Cargo.toml", "src/lib.rs"])
            .current_dir(repo)
            .status()
            .unwrap()
            .success()
    );

    let repo_str = repo.to_string_lossy().into_owned();
    let graph_db_path = format!("{repo_str}/.atlas/worldtree.db");
    Store::open(&graph_db_path).unwrap();
    build_graph(
        Utf8Path::new(&repo_str),
        &graph_db_path,
        &BuildOptions::default(),
    )
    .unwrap();

    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() {}\n",
    )
    .unwrap();

    let payload = json!({
        "tool_name": "Write",
        "changed_files": [repo.join("src/lib.rs").to_string_lossy().into_owned()],
    });
    let persisted = persist_hook_event(
        &repo_str,
        &graph_db_path,
        "hook",
        "post-tool-use",
        payload.clone(),
    )
    .unwrap();
    let actions = execute_hook_actions(
        &repo_str,
        &graph_db_path,
        "hook",
        resolve_hook_policy("post-tool-use").unwrap(),
        &persisted,
        &payload,
    );

    assert_eq!(actions["graph_refresh"]["status"], "updated");
    let store = Store::open(&graph_db_path).unwrap();
    let nodes = store.nodes_by_file("src/lib.rs").unwrap();
    assert!(
        nodes
            .iter()
            .any(|node| node.qualified_name.ends_with("::fn::beta"))
    );
}

#[test]
fn post_tool_use_build_test_flow_persists_review_refresh_artifacts() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path();
    std::fs::create_dir_all(repo.join("src")).unwrap();
    std::fs::write(
        repo.join("Cargo.toml"),
        "[package]\nname = \"hook-review-refresh\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(repo.join("src/lib.rs"), "pub fn alpha() {}\n").unwrap();
    std::fs::create_dir_all(repo.join(".atlas")).unwrap();
    assert!(
        ProcessCommand::new("git")
            .arg("init")
            .arg("--quiet")
            .current_dir(repo)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        ProcessCommand::new("git")
            .args(["add", "Cargo.toml", "src/lib.rs"])
            .current_dir(repo)
            .status()
            .unwrap()
            .success()
    );

    let repo_str = repo.to_string_lossy().into_owned();
    let graph_db_path = format!("{repo_str}/.atlas/worldtree.db");
    Store::open(&graph_db_path).unwrap();
    build_graph(
        Utf8Path::new(&repo_str),
        &graph_db_path,
        &BuildOptions::default(),
    )
    .unwrap();

    std::fs::write(
        repo.join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() {}\n",
    )
    .unwrap();

    let payload = json!({
        "tool_name": "Bash",
        "status": "ok",
        "command": "cargo test",
        "changed_files": [repo.join("src/lib.rs").to_string_lossy().into_owned()],
    });
    let persisted = persist_hook_event(
        &repo_str,
        &graph_db_path,
        "hook",
        "post-tool-use",
        payload.clone(),
    )
    .unwrap();
    let actions = execute_hook_actions(
        &repo_str,
        &graph_db_path,
        "hook",
        resolve_hook_policy("post-tool-use").unwrap(),
        &persisted,
        &payload,
    );

    assert_eq!(actions["graph_refresh"]["status"], "updated");
    assert_eq!(
        actions["review_refresh"]["status"], "refreshed",
        "review_refresh={}",
        actions["review_refresh"]
    );
    assert_eq!(actions["review_refresh"]["trigger"], "test");
    assert!(
        actions["review_refresh"]["changed_files"]
            .as_array()
            .unwrap()
            .contains(&json!("src/lib.rs"))
    );

    let artifacts = actions["review_refresh"]["artifacts"].as_array().unwrap();
    assert_eq!(artifacts.len(), 3);

    let mut content_store = ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
    content_store.migrate().unwrap();
    for artifact in artifacts {
        let source_id = artifact["source_id"].as_str().unwrap();
        let source = content_store.get_source(source_id).unwrap().unwrap();
        assert!(matches!(
            source.source_type.as_str(),
            "review_context" | "explain_change" | "impact_result"
        ));
    }
}
