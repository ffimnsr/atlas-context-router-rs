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
use atlas_repo::stable_repo_id;
use atlas_session::{SessionEventType, SessionId, SessionStore};
use atlas_store_sqlite::Store;

use crate::payload::tool_may_change_files;
use crate::{
    AgentEventRequest, AgentEventResult, AgentEventSource, HookEventParts, build_hook_event,
    execute_hook_actions, persist_hook_event, record_agent_event, resolve_hook_policy,
};

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
    let mut command = ProcessCommand::new("git");
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

fn last_hook_payload(graph_db_path: &str, repo: &str, frontend: &str) -> Value {
    let session_store = SessionStore::open(&derive_session_db_path(graph_db_path)).unwrap();
    let session_id = SessionId::derive(repo, "", frontend);
    let events = session_store.list_events(&session_id).unwrap();
    serde_json::from_str(&events.last().unwrap().payload_json).unwrap()
}

fn record_request(
    repo: &str,
    graph_db_path: &str,
    event: &str,
    payload: Value,
) -> AgentEventRequest {
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

mod payload;
mod post_tool;
mod record;
mod session;
