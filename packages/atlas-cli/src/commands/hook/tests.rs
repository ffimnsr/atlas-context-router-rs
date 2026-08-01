use std::io::{self, Cursor, Read};
use std::path::Path;

use atlas_agent_events::{
    AgentEventRequest, AgentEventResult, AgentEventSource, record_agent_event,
};
use serde_json::{Value, json};

use crate::cli::{Cli, Command};
use crate::cli_paths::canonicalize_cli_path;

use super::hook_result_json;
use super::runtime::{read_hook_payload_from, resolve_hook_repo};

struct PanicRead;

impl Read for PanicRead {
    fn read(&mut self, _buf: &mut [u8]) -> io::Result<usize> {
        panic!("reader should not be touched when stdin is a terminal");
    }
}

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

#[test]
fn read_hook_payload_from_terminal_returns_null_without_reading() {
    let payload = read_hook_payload_from(PanicRead, true).unwrap();
    assert_eq!(payload, serde_json::Value::Null);
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
fn resolve_hook_repo_prefers_runner_script_git_root() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path();
    std::fs::create_dir_all(repo.join(".atlas/hooks")).unwrap();
    std::fs::write(repo.join(".atlas/hooks/atlas-hook"), "#!/bin/sh\n").unwrap();
    git(repo, &["init", "--quiet"]);

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

const GIT_LOCAL_ENV_VARS: &[&str] = &[
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_CONFIG",
    "GIT_CONFIG_COUNT",
    "GIT_CONFIG_KEY_0",
    "GIT_CONFIG_VALUE_0",
    "GIT_DIR",
    "GIT_GRAFT_FILE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_INTERNAL_SUPER_PREFIX",
    "GIT_NAMESPACE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_OBJECT_DIRECTORY",
    "GIT_PREFIX",
    "GIT_REPLACE_REF_BASE",
    "GIT_SHALLOW_FILE",
    "GIT_WORK_TREE",
];

const GIT_TEST_NAME: &str = "Atlas Test";
const GIT_TEST_EMAIL: &str = "test@atlas";

fn git(dir: &Path, args: &[&str]) {
    let mut command = std::process::Command::new("git");
    command
        .current_dir(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", GIT_TEST_NAME)
        .env("GIT_AUTHOR_EMAIL", GIT_TEST_EMAIL)
        .env("GIT_COMMITTER_NAME", GIT_TEST_NAME)
        .env("GIT_COMMITTER_EMAIL", GIT_TEST_EMAIL);
    for env_var in GIT_LOCAL_ENV_VARS {
        command.env_remove(env_var);
    }
    let status = command.status().expect("git command");
    assert!(status.success(), "git {args:?} failed in {}", dir.display());
}

const HOOK_JSON_CONTRACT_KEYS: &[&str] = &[
    "event",
    "frontend",
    "repo_root",
    "session_id",
    "pending_resume",
    "stored",
    "event_id",
    "source_id",
    "storage_kind",
    "snapshot",
    "actions",
];

fn hook_request(repo: &str, graph_db_path: &str, event: &str, payload: Value) -> AgentEventRequest {
    AgentEventRequest {
        repo_root: repo.to_owned(),
        graph_db_path: graph_db_path.to_owned(),
        frontend: "hook".to_owned(),
        event: event.to_owned(),
        session_id: None,
        agent_id: None,
        payload,
        source: AgentEventSource::Hook,
    }
}

/// Assert the `atlas hook --json` output contract and return the output value.
///
/// Field names and presence are part of the hook output contract; any drift
/// from the pre-refactor shape fails here.
fn assert_hook_json_contract(repo: &str, result: AgentEventResult, expected_event: &str) -> Value {
    let value = hook_result_json(repo, result);
    let mut actual_keys: Vec<&str> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    actual_keys.sort_unstable();
    let mut expected_keys = HOOK_JSON_CONTRACT_KEYS.to_vec();
    expected_keys.sort_unstable();
    assert_eq!(
        actual_keys, expected_keys,
        "hook JSON contract drifted: {}",
        value
    );
    assert_eq!(value["event"], expected_event);
    assert_eq!(value["frontend"], "hook");
    assert_eq!(value["repo_root"], repo);
    assert!(value["stored"].as_bool().is_some());
    value
}

#[test]
fn hook_json_parity_session_start() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let result = record_agent_event(hook_request(
        &repo,
        &graph_db_path,
        "session-start",
        Value::Null,
    ))
    .unwrap();
    let value = assert_hook_json_contract(&repo, result, "session-start");
    assert_eq!(value["pending_resume"], false);
    assert_eq!(value["actions"]["lifecycle"]["status"], "loaded");
}

#[test]
fn hook_json_parity_user_prompt() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let result = record_agent_event(hook_request(
        &repo,
        &graph_db_path,
        "user-prompt",
        json!({ "prompt": "review auth flow" }),
    ))
    .unwrap();
    let value = assert_hook_json_contract(&repo, result, "user-prompt");
    assert_eq!(value["actions"]["prompt_routing"]["status"], "routed");
}

#[test]
fn hook_json_parity_post_tool_use() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let result = record_agent_event(hook_request(
        &repo,
        &graph_db_path,
        "post-tool-use",
        json!({ "tool_name": "read_file" }),
    ))
    .unwrap();
    let value = assert_hook_json_contract(&repo, result, "post-tool-use");
    assert_eq!(
        value["actions"]["graph_refresh"]["reason"],
        "tool_not_graph_relevant"
    );
}

#[test]
fn hook_json_parity_pre_compact() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let result = record_agent_event(hook_request(
        &repo,
        &graph_db_path,
        "pre-compact",
        Value::Null,
    ))
    .unwrap();
    let value = assert_hook_json_contract(&repo, result, "pre-compact");
    assert_eq!(value["actions"]["lifecycle"]["status"], "persisted");
    assert!(value["snapshot"].is_object());
}

#[test]
fn hook_json_parity_post_compact() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let result = record_agent_event(hook_request(
        &repo,
        &graph_db_path,
        "post-compact",
        Value::Null,
    ))
    .unwrap();
    let value = assert_hook_json_contract(&repo, result, "post-compact");
    assert_eq!(value["actions"]["lifecycle"]["status"], "verified");
}

#[test]
fn hook_json_parity_stop() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let result =
        record_agent_event(hook_request(&repo, &graph_db_path, "stop", Value::Null)).unwrap();
    let value = assert_hook_json_contract(&repo, result, "stop");
    assert_eq!(value["actions"]["lifecycle"]["status"], "persisted");
    assert!(value["snapshot"].is_object());
}

#[test]
fn hook_json_parity_file_changed() {
    let dir = tempfile::TempDir::new().unwrap();
    let repo = dir.path().to_string_lossy().into_owned();
    let graph_db_path = format!("{repo}/.atlas/worldtree.db");

    let result = record_agent_event(hook_request(
        &repo,
        &graph_db_path,
        "file-changed",
        json!({ "changed_files": ["src/lib.rs"] }),
    ))
    .unwrap();
    let value = assert_hook_json_contract(&repo, result, "file-changed");
    assert_eq!(value["actions"]["freshness"]["status"], "stale");
}
