use std::collections::BTreeSet;
use std::io::Read;
use std::path::Path;

use anyhow::{Context, Result, bail};
use camino::Utf8Path;
use serde_json::{Map, Value, json};

use atlas_adapters::{
    derive_content_db_path, derive_session_db_path, generate_source_id, normalize_event,
    redact_payload,
};
use atlas_contentstore::{ContentStore, OutputRouting, SearchFilters, SourceMeta};
use atlas_core::model::{ChangeType, ContextIntent, ContextRequest, ContextTarget};
use atlas_engine::{Config, UpdateOptions, UpdateTarget, update_graph};
use atlas_impact::analyze as advanced_impact;
use atlas_repo::find_repo_root;
use atlas_review::{ContextEngine, query_parser};
use atlas_session::{NewSessionEvent, ResumeSnapshot, SessionEventType, SessionId, SessionStore};
use atlas_store_sqlite::{BuildFinishStats, Store};

use crate::cli::{Cli, Command};
use crate::commands::changes::build_explain_change_summary;

use super::{db_path, print_json, resolve_repo};

const MAX_HOOK_STDIN_BYTES: u64 = 64 * 1024;
const MAX_HOOK_EVENT_SCAN: usize = 20;
const MAX_HOOK_SOURCE_HINTS: usize = 3;
const MAX_HOOK_PROMPT_HITS: usize = 3;
const MAX_HOOK_REVIEW_REFRESH_FILES: usize = 8;
const MAX_HOOK_REVIEW_REFRESH_DEPTH: u32 = 3;
const MAX_HOOK_REVIEW_REFRESH_NODES: usize = 64;
const FILE_CHANGED_INLINE_CONTENT_KEYS: &[&str] = &[
    "after",
    "before",
    "content",
    "contents",
    "diff",
    "new_content",
    "old_content",
    "patch",
    "raw",
    "snippet",
    "text",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HookStorage {
    SessionOnly,
    SessionAndContent,
}

#[derive(Clone, Debug)]
struct HookPayloadRouting {
    event_payload: Value,
    source_id: Option<String>,
    storage_kind: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HookLifecycleAction {
    None,
    LoadRestore,
    PersistHandoff,
    VerifyRestore,
}

struct HookPolicy {
    canonical_event: &'static str,
    aliases: &'static [&'static str],
    event_type: SessionEventType,
    priority: i32,
    storage: HookStorage,
    lifecycle: HookLifecycleAction,
    prompt_routing: bool,
    freshness: bool,
    graph_refresh: bool,
    review_refresh: bool,
    build_resume_snapshot: bool,
    session_start: bool,
}

const SESSION_START_ALIASES: &[&str] = &["session-start", "SessionStart", "sessionStart"];
const USER_PROMPT_ALIASES: &[&str] = &["user-prompt", "UserPromptSubmit", "userPromptSubmitted"];
const USER_PROMPT_EXPANSION_ALIASES: &[&str] = &["user-prompt-expansion", "UserPromptExpansion"];
const PRE_TOOL_USE_ALIASES: &[&str] = &["pre-tool-use", "PreToolUse", "preToolUse"];
const POST_TOOL_USE_ALIASES: &[&str] = &["post-tool-use", "PostToolUse", "postToolUse"];
const PRE_COMPACT_ALIASES: &[&str] = &["pre-compact", "PreCompact"];
const POST_COMPACT_ALIASES: &[&str] = &["post-compact", "PostCompact"];
const STOP_ALIASES: &[&str] = &["stop", "Stop"];
const SESSION_END_ALIASES: &[&str] = &["session-end", "SessionEnd", "sessionEnd"];
const PERMISSION_REQUEST_ALIASES: &[&str] = &["permission-request", "PermissionRequest"];
const PERMISSION_DENIED_ALIASES: &[&str] = &["permission-denied", "PermissionDenied"];
const TOOL_FAILURE_ALIASES: &[&str] = &["tool-failure", "PostToolUseFailure"];
const STOP_FAILURE_ALIASES: &[&str] = &["stop-failure", "StopFailure"];
const ERROR_ALIASES: &[&str] = &["error", "errorOccurred"];
const ELICITATION_ALIASES: &[&str] = &["elicitation", "Elicitation"];
const ELICITATION_RESULT_ALIASES: &[&str] = &["elicitation-result", "ElicitationResult"];
const INSTRUCTIONS_LOADED_ALIASES: &[&str] = &["instructions-loaded", "InstructionsLoaded"];
const NOTIFICATION_ALIASES: &[&str] = &["notification", "Notification"];
const SUBAGENT_START_ALIASES: &[&str] = &["subagent-start", "SubagentStart"];
const SUBAGENT_STOP_ALIASES: &[&str] = &["subagent-stop", "SubagentStop"];
const TASK_CREATED_ALIASES: &[&str] = &["task-created", "TaskCreated"];
const TASK_COMPLETED_ALIASES: &[&str] = &["task-completed", "TaskCompleted"];
const CONFIG_CHANGE_ALIASES: &[&str] = &["config-change", "ConfigChange"];
const CWD_CHANGED_ALIASES: &[&str] = &["cwd-changed", "CwdChanged"];
const FILE_CHANGED_ALIASES: &[&str] = &["file-changed", "FileChanged"];
const WORKTREE_CREATE_ALIASES: &[&str] = &["worktree-create", "WorktreeCreate"];
const WORKTREE_REMOVE_ALIASES: &[&str] = &["worktree-remove", "WorktreeRemove"];

const HOOK_POLICIES: &[HookPolicy] = &[
    HookPolicy {
        canonical_event: "session-start",
        aliases: SESSION_START_ALIASES,
        event_type: SessionEventType::SessionStart,
        priority: 5,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::LoadRestore,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: true,
    },
    HookPolicy {
        canonical_event: "user-prompt",
        aliases: USER_PROMPT_ALIASES,
        event_type: SessionEventType::UserIntent,
        priority: 3,
        storage: HookStorage::SessionAndContent,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: true,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "user-prompt-expansion",
        aliases: USER_PROMPT_EXPANSION_ALIASES,
        event_type: SessionEventType::UserIntent,
        priority: 3,
        storage: HookStorage::SessionAndContent,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: true,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "pre-tool-use",
        aliases: PRE_TOOL_USE_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "post-tool-use",
        aliases: POST_TOOL_USE_ALIASES,
        event_type: SessionEventType::GraphUpdate,
        priority: 3,
        storage: HookStorage::SessionAndContent,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: true,
        review_refresh: true,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "pre-compact",
        aliases: PRE_COMPACT_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::PersistHandoff,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: true,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "post-compact",
        aliases: POST_COMPACT_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::VerifyRestore,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "stop",
        aliases: STOP_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionAndContent,
        lifecycle: HookLifecycleAction::PersistHandoff,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: true,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "session-end",
        aliases: SESSION_END_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionAndContent,
        lifecycle: HookLifecycleAction::PersistHandoff,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: true,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "permission-request",
        aliases: PERMISSION_REQUEST_ALIASES,
        event_type: SessionEventType::Decision,
        priority: 3,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "permission-denied",
        aliases: PERMISSION_DENIED_ALIASES,
        event_type: SessionEventType::Decision,
        priority: 3,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "tool-failure",
        aliases: TOOL_FAILURE_ALIASES,
        event_type: SessionEventType::CommandFail,
        priority: 3,
        storage: HookStorage::SessionAndContent,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "stop-failure",
        aliases: STOP_FAILURE_ALIASES,
        event_type: SessionEventType::CommandFail,
        priority: 3,
        storage: HookStorage::SessionAndContent,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "error",
        aliases: ERROR_ALIASES,
        event_type: SessionEventType::Error,
        priority: 4,
        storage: HookStorage::SessionAndContent,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "elicitation",
        aliases: ELICITATION_ALIASES,
        event_type: SessionEventType::UserIntent,
        priority: 3,
        storage: HookStorage::SessionAndContent,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: true,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "elicitation-result",
        aliases: ELICITATION_RESULT_ALIASES,
        event_type: SessionEventType::UserIntent,
        priority: 3,
        storage: HookStorage::SessionAndContent,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: true,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "instructions-loaded",
        aliases: INSTRUCTIONS_LOADED_ALIASES,
        event_type: SessionEventType::RuleInstruction,
        priority: 2,
        storage: HookStorage::SessionAndContent,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "notification",
        aliases: NOTIFICATION_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "subagent-start",
        aliases: SUBAGENT_START_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "subagent-stop",
        aliases: SUBAGENT_STOP_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "task-created",
        aliases: TASK_CREATED_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "task-completed",
        aliases: TASK_COMPLETED_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "config-change",
        aliases: CONFIG_CHANGE_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: true,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "cwd-changed",
        aliases: CWD_CHANGED_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "file-changed",
        aliases: FILE_CHANGED_ALIASES,
        event_type: SessionEventType::FileWrite,
        priority: 3,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: true,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "worktree-create",
        aliases: WORKTREE_CREATE_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
    HookPolicy {
        canonical_event: "worktree-remove",
        aliases: WORKTREE_REMOVE_ALIASES,
        event_type: SessionEventType::CommandRun,
        priority: 2,
        storage: HookStorage::SessionOnly,
        lifecycle: HookLifecycleAction::None,
        prompt_routing: false,
        freshness: false,
        graph_refresh: false,
        review_refresh: false,
        build_resume_snapshot: false,
        session_start: false,
    },
];

#[derive(Debug)]
struct HookPersistence {
    session_id: SessionId,
    pending_resume: bool,
    stored_event_id: Option<i64>,
    snapshot: Option<Value>,
    source_id: Option<String>,
    storage_kind: Option<&'static str>,
}

struct PromptRoutingMetadata {
    prompt_excerpt: String,
    query: String,
    intent: Value,
    target: Value,
    hits: Vec<Value>,
}

struct HookMetadataContext<'a> {
    repo: &'a str,
    graph_db_path: &'a str,
    store: &'a SessionStore,
    session_id: &'a SessionId,
    policy: &'a HookPolicy,
    payload: &'a Value,
    routed: &'a HookPayloadRouting,
    pending_resume: bool,
}

struct HookEventParts<'a> {
    frontend: &'a str,
    event: &'a str,
    payload: Value,
    hook_metadata: Value,
    source_id: Option<&'a str>,
    storage_kind: Option<&'a str>,
    pending_resume: bool,
}

struct ReviewRefreshArtifact {
    kind: &'static str,
    source_id: String,
}

struct ReviewRefreshResult {
    trigger: &'static str,
    changed_files: Vec<String>,
    artifacts: Vec<ReviewRefreshArtifact>,
}

pub fn run_hook(cli: &Cli) -> Result<()> {
    let event = match &cli.command {
        Command::Hook { event } => event.as_str(),
        _ => unreachable!(),
    };

    let repo = resolve_hook_repo(cli)?;
    let graph_db_path = db_path(cli, &repo);
    let payload = read_hook_payload()?;
    let frontend = hook_frontend();
    let policy = resolve_hook_policy(event)?;
    let persisted = persist_hook_event(&repo, &graph_db_path, &frontend, event, payload.clone())?;
    let actions = execute_hook_actions(
        &repo,
        &graph_db_path,
        &frontend,
        policy,
        &persisted,
        &payload,
    );

    if cli.json {
        print_json(
            "hook",
            json!({
                "event": event,
                "frontend": frontend,
                "repo_root": repo,
                "session_id": persisted.session_id.as_str(),
                "pending_resume": persisted.pending_resume,
                "stored": persisted.stored_event_id.is_some(),
                "event_id": persisted.stored_event_id,
                "source_id": persisted.source_id,
                "storage_kind": persisted.storage_kind,
                "snapshot": persisted.snapshot,
                "actions": actions,
            }),
        )?;
    }

    Ok(())
}

fn resolve_hook_repo(cli: &Cli) -> Result<String> {
    if cli.repo.is_some() {
        return resolve_repo(cli);
    }

    if let Ok(script_path) = std::env::var("ATLAS_HOOK_SCRIPT_PATH") {
        let script_path = script_path.trim();
        if !script_path.is_empty() {
            let script_path = Utf8Path::new(script_path);
            let start = if script_path.is_file() {
                script_path.parent().unwrap_or(script_path)
            } else {
                script_path
            };
            if let Ok(root) = find_repo_root(start) {
                return Ok(root.into_string());
            }
        }
    }

    let cwd = resolve_repo(cli)?;
    if let Ok(root) = find_repo_root(Utf8Path::new(&cwd)) {
        return Ok(root.into_string());
    }

    Ok(cwd)
}

fn execute_hook_actions(
    repo: &str,
    graph_db_path: &str,
    frontend: &str,
    policy: &HookPolicy,
    persisted: &HookPersistence,
    payload: &Value,
) -> Value {
    let mut actions = Map::new();

    let lifecycle =
        execute_lifecycle_action(repo, graph_db_path, frontend, policy, &persisted.session_id);
    if !lifecycle.is_null() {
        actions.insert("lifecycle".to_owned(), lifecycle);
    }

    let prompt_routing = execute_prompt_routing_action(repo, graph_db_path, policy, payload);
    if !prompt_routing.is_null() {
        actions.insert("prompt_routing".to_owned(), prompt_routing);
    }

    let graph_refresh = execute_graph_refresh_action(repo, graph_db_path, policy, payload);
    if !graph_refresh.is_null() {
        actions.insert("graph_refresh".to_owned(), graph_refresh);
    }

    let freshness = build_freshness_metadata(policy, repo, payload);
    if !freshness.is_null() {
        actions.insert("freshness".to_owned(), freshness);
    }

    let review_refresh = execute_review_refresh_action(
        repo,
        graph_db_path,
        policy,
        actions.get("graph_refresh"),
        payload,
        &persisted.session_id,
    );
    if !review_refresh.is_null() {
        actions.insert("review_refresh".to_owned(), review_refresh);
    }

    if actions.is_empty() {
        Value::Null
    } else {
        Value::Object(actions)
    }
}

fn execute_lifecycle_action(
    repo: &str,
    graph_db_path: &str,
    frontend: &str,
    policy: &HookPolicy,
    session_id: &SessionId,
) -> Value {
    match policy.lifecycle {
        HookLifecycleAction::None => Value::Null,
        HookLifecycleAction::LoadRestore => load_restore_state(repo, graph_db_path, session_id),
        HookLifecycleAction::PersistHandoff => persist_handoff_artifact(
            repo,
            graph_db_path,
            frontend,
            session_id,
            policy.canonical_event,
        ),
        HookLifecycleAction::VerifyRestore => verify_restore_state(graph_db_path, session_id),
    }
}

fn build_restore_metadata(
    store: &SessionStore,
    session_id: &SessionId,
    pending_resume: bool,
) -> Value {
    match store.get_resume_snapshot(session_id) {
        Ok(snapshot) => json!({
            "pending_resume": pending_resume,
            "has_resume_snapshot": snapshot.is_some(),
            "snapshot_consumed": snapshot.as_ref().map(|row| row.consumed),
            "snapshot_event_count": snapshot.as_ref().map(|row| row.event_count),
        }),
        Err(error) => json!({
            "pending_resume": pending_resume,
            "error": error.to_string(),
        }),
    }
}

fn compute_prompt_routing(
    repo: &str,
    graph_db_path: &str,
    payload: &Value,
) -> Option<PromptRoutingMetadata> {
    let prompt_text = extract_prompt_text(payload)?;
    let request = query_parser::parse_query(&prompt_text);
    let query = prompt_lookup_query(&request.target, &prompt_text);
    let hits = search_saved_context_previews(repo, graph_db_path, &query);

    Some(PromptRoutingMetadata {
        prompt_excerpt: excerpt(&prompt_text, 160),
        query,
        intent: serde_json::to_value(request.intent).unwrap_or_else(|_| json!("symbol")),
        target: serde_json::to_value(request.target).unwrap_or(Value::Null),
        hits,
    })
}

fn push_unique_string(values: &mut Vec<String>, candidate: String, limit: usize) {
    if values.iter().any(|existing| existing == &candidate) {
        return;
    }
    values.push(candidate);
    if values.len() > limit {
        values.truncate(limit);
    }
}

fn push_unique_value(values: &mut Vec<Value>, candidate: Value, limit: usize) {
    if values.iter().any(|existing| existing == &candidate) {
        return;
    }
    values.push(candidate);
    if values.len() > limit {
        values.truncate(limit);
    }
}

fn build_hook_event_metadata(context: HookMetadataContext<'_>) -> Value {
    let mut retrieval_hints = Vec::new();
    let mut saved_artifact_refs = Vec::new();
    let storage_mode = match context.policy.storage {
        HookStorage::SessionOnly => "session_only",
        HookStorage::SessionAndContent => "session_and_content",
    };

    if let Some(source_id) = context.routed.source_id.as_ref() {
        push_unique_string(
            &mut saved_artifact_refs,
            source_id.clone(),
            MAX_HOOK_SOURCE_HINTS,
        );
        push_unique_value(
            &mut retrieval_hints,
            json!({
                "kind": "hook_payload",
                "event": context.policy.canonical_event,
                "source_id": source_id,
                "storage_kind": context.routed.storage_kind,
            }),
            MAX_HOOK_PROMPT_HITS,
        );
    }

    if context.policy.prompt_routing
        && let Some(prompt_routing) =
            compute_prompt_routing(context.repo, context.graph_db_path, context.payload)
    {
        let prompt_query = prompt_routing.query.clone();
        push_unique_value(
            &mut retrieval_hints,
            json!({
                "kind": "prompt_query",
                "query": prompt_query,
                "intent": prompt_routing.intent.clone(),
                "target": prompt_routing.target.clone(),
            }),
            MAX_HOOK_PROMPT_HITS,
        );

        for hit in prompt_routing.hits.iter().take(MAX_HOOK_SOURCE_HINTS) {
            if let Some(source_id) = hit.get("source_id").and_then(Value::as_str) {
                push_unique_string(
                    &mut saved_artifact_refs,
                    source_id.to_owned(),
                    MAX_HOOK_SOURCE_HINTS,
                );
                push_unique_value(
                    &mut retrieval_hints,
                    json!({
                        "kind": "saved_context_hit",
                        "query": prompt_routing.query,
                        "source_id": source_id,
                        "label": hit.get("label").cloned().unwrap_or(Value::Null),
                        "source_type": hit.get("source_type").cloned().unwrap_or(Value::Null),
                    }),
                    MAX_HOOK_PROMPT_HITS,
                );
            }
        }
    }

    let source_summaries = read_source_summaries(context.graph_db_path, &saved_artifact_refs);
    let freshness = build_freshness_metadata(context.policy, context.repo, context.payload);

    json!({
        "storage_mode": storage_mode,
        "restore_metadata": build_restore_metadata(
            context.store,
            context.session_id,
            context.pending_resume,
        ),
        "retrieval_hints": retrieval_hints,
        "saved_artifact_refs": saved_artifact_refs,
        "source_summaries": source_summaries,
        "freshness": freshness,
    })
}

fn build_freshness_metadata(policy: &HookPolicy, repo: &str, payload: &Value) -> Value {
    if !policy.freshness {
        return Value::Null;
    }

    json!({
        "status": "stale",
        "event": policy.canonical_event,
        "stale": true,
        "changed_files": extract_changed_files(repo, payload),
        "inline_content_persisted": false,
    })
}

fn load_restore_state(repo: &str, graph_db_path: &str, session_id: &SessionId) -> Value {
    let session_db_path = derive_session_db_path(graph_db_path);
    let mut store = match SessionStore::open(&session_db_path) {
        Ok(store) => store,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "session_store_unavailable",
                "error": error.to_string(),
            });
        }
    };

    let snapshot = match store.get_resume_snapshot(session_id) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "resume_snapshot_unavailable",
                "error": error.to_string(),
            });
        }
    };
    let pending_snapshot = snapshot.as_ref().is_some_and(|row| !row.consumed);
    let context_hints = collect_context_hints(repo, graph_db_path, &store, session_id);

    if pending_snapshot {
        let _ = store.mark_resume_consumed(session_id, true);
    }

    json!({
        "status": "loaded",
        "resume_loaded": pending_snapshot,
        "snapshot": snapshot.as_ref().map(snapshot_to_value),
        "context_hints": context_hints,
    })
}

fn persist_handoff_artifact(
    repo: &str,
    graph_db_path: &str,
    frontend: &str,
    session_id: &SessionId,
    trigger: &str,
) -> Value {
    let session_db_path = derive_session_db_path(graph_db_path);
    let mut store = match SessionStore::open(&session_db_path) {
        Ok(store) => store,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "session_store_unavailable",
                "error": error.to_string(),
            });
        }
    };

    let snapshot = match store.get_resume_snapshot(session_id) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => match store.build_resume(session_id) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return json!({
                    "status": "error",
                    "reason": "resume_build_failed",
                    "error": error.to_string(),
                });
            }
        },
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "resume_snapshot_unavailable",
                "error": error.to_string(),
            });
        }
    };

    let context_hints = collect_context_hints(repo, graph_db_path, &store, session_id);
    let artifact = json!({
        "hook_event": trigger,
        "frontend": frontend,
        "session_id": session_id.as_str(),
        "resume_snapshot": snapshot_to_value(&snapshot),
        "context_hints": context_hints,
    });
    let artifact_json = match serde_json::to_string(&artifact) {
        Ok(text) => text,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "handoff_serialize_failed",
                "error": error.to_string(),
            });
        }
    };

    let source_id = generate_source_id(
        &format!("{repo}:{frontend}:{trigger}:handoff"),
        &artifact_json,
    );
    let mut content_store = match ContentStore::open(&derive_content_db_path(graph_db_path)) {
        Ok(store) => store,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "content_store_unavailable",
                "error": error.to_string(),
            });
        }
    };
    if let Err(error) = content_store.migrate() {
        return json!({
            "status": "error",
            "reason": "content_store_migrate_failed",
            "error": error.to_string(),
        });
    }

    let meta = SourceMeta {
        id: source_id.clone(),
        session_id: Some(session_id.as_str().to_owned()),
        source_type: "hook_handoff".to_owned(),
        label: format!("hook:{frontend}:{trigger}:handoff"),
        repo_root: Some(repo.to_owned()),
    };

    if let Err(error) = content_store.index_artifact(meta, &artifact_json, "application/json") {
        return json!({
            "status": "error",
            "reason": "handoff_persist_failed",
            "error": error.to_string(),
        });
    }

    json!({
        "status": "persisted",
        "resume_source_id": source_id,
        "snapshot_event_count": snapshot.event_count,
        "context_hints": context_hints,
    })
}

fn verify_restore_state(graph_db_path: &str, session_id: &SessionId) -> Value {
    let session_db_path = derive_session_db_path(graph_db_path);
    let store = match SessionStore::open(&session_db_path) {
        Ok(store) => store,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "session_store_unavailable",
                "error": error.to_string(),
            });
        }
    };

    let snapshot = match store.get_resume_snapshot(session_id) {
        Ok(snapshot) => snapshot,
        Err(error) => {
            return json!({
                "status": "error",
                "reason": "resume_snapshot_unavailable",
                "error": error.to_string(),
            });
        }
    };
    let meta = store.get_session_meta(session_id).ok().flatten();

    json!({
        "status": "verified",
        "has_resume_snapshot": snapshot.is_some(),
        "snapshot_consumed": snapshot.as_ref().map(|row| row.consumed),
        "snapshot_event_count": snapshot.as_ref().map(|row| row.event_count),
        "last_resume_at": meta.as_ref().and_then(|row| row.last_resume_at.clone()),
        "last_compaction_at": meta.and_then(|row| row.last_compaction_at),
    })
}

fn execute_prompt_routing_action(
    repo: &str,
    graph_db_path: &str,
    policy: &HookPolicy,
    payload: &Value,
) -> Value {
    if !policy.prompt_routing {
        return Value::Null;
    }

    let Some(prompt_routing) = compute_prompt_routing(repo, graph_db_path, payload) else {
        return json!({
            "status": "skipped",
            "reason": "no_prompt_text",
        });
    };

    json!({
        "status": "routed",
        "prompt_excerpt": prompt_routing.prompt_excerpt,
        "query": prompt_routing.query,
        "intent": prompt_routing.intent,
        "target": prompt_routing.target,
        "saved_context_hits": prompt_routing.hits,
    })
}

fn search_saved_context_previews(repo: &str, graph_db_path: &str, query: &str) -> Vec<Value> {
    if query.trim().len() < 3 {
        return Vec::new();
    }

    let mut content_store = match ContentStore::open(&derive_content_db_path(graph_db_path)) {
        Ok(store) => store,
        Err(_) => return Vec::new(),
    };
    if content_store.migrate().is_err() {
        return Vec::new();
    }

    let filters = SearchFilters {
        repo_root: Some(repo.to_owned()),
        ..SearchFilters::default()
    };
    let chunks = match content_store.search_with_fallback(query, &filters) {
        Ok(chunks) => chunks,
        Err(_) => return Vec::new(),
    };

    let mut seen = BTreeSet::new();
    let mut previews = Vec::new();
    for chunk in chunks {
        if !seen.insert(chunk.source_id.clone()) {
            continue;
        }
        let source = content_store.get_source(&chunk.source_id).ok().flatten();
        previews.push(json!({
            "source_id": chunk.source_id,
            "label": source.as_ref().map(|row| row.label.clone()),
            "source_type": source.as_ref().map(|row| row.source_type.clone()),
            "preview": excerpt(&chunk.content, 256),
            "content_type": chunk.content_type,
        }));
        if previews.len() >= MAX_HOOK_PROMPT_HITS {
            break;
        }
    }

    previews
}

fn execute_graph_refresh_action(
    repo: &str,
    graph_db_path: &str,
    policy: &HookPolicy,
    payload: &Value,
) -> Value {
    if !policy.graph_refresh {
        return Value::Null;
    }

    let tool_name = extract_tool_name(payload);
    let changed_files = extract_changed_files(repo, payload);
    if let Some(tool_name) = tool_name.as_deref()
        && !tool_may_change_files(tool_name)
        && changed_files.is_empty()
    {
        return json!({
            "status": "skipped",
            "reason": "tool_not_graph_relevant",
            "tool_name": tool_name,
        });
    }

    let target = if changed_files.is_empty() {
        UpdateTarget::WorkingTree
    } else {
        UpdateTarget::Files(changed_files.clone())
    };
    let config = Config::load(&atlas_engine::paths::atlas_dir(repo)).unwrap_or_default();
    if let Ok(store) = Store::open(graph_db_path) {
        let _ = store.begin_build(repo);
    }

    let result = update_graph(
        Utf8Path::new(repo),
        graph_db_path,
        &UpdateOptions {
            fail_fast: false,
            batch_size: config.parse_batch_size(),
            target,
        },
    );

    match result {
        Ok(summary) => {
            if let Ok(store) = Store::open(graph_db_path) {
                let _ = store.finish_build(
                    repo,
                    BuildFinishStats {
                        files_discovered: (summary.parsed + summary.deleted + summary.renamed)
                            as i64,
                        files_processed: summary.parsed as i64,
                        files_failed: summary.parse_errors as i64,
                        nodes_written: summary.nodes_updated as i64,
                        edges_written: summary.edges_updated as i64,
                    },
                );
            }
            json!({
                "status": "updated",
                "tool_name": tool_name,
                "changed_files": changed_files,
                "deleted": summary.deleted,
                "renamed": summary.renamed,
                "parsed": summary.parsed,
                "nodes_updated": summary.nodes_updated,
                "edges_updated": summary.edges_updated,
            })
        }
        Err(error) => {
            if let Ok(store) = Store::open(graph_db_path) {
                let _ = store.fail_build(repo, &error.to_string());
            }
            json!({
                "status": "error",
                "tool_name": tool_name,
                "changed_files": changed_files,
                "error": error.to_string(),
            })
        }
    }
}

fn execute_review_refresh_action(
    repo: &str,
    graph_db_path: &str,
    policy: &HookPolicy,
    graph_refresh: Option<&Value>,
    payload: &Value,
    session_id: &SessionId,
) -> Value {
    if !policy.review_refresh {
        return Value::Null;
    }

    let Some(refresh) = graph_refresh else {
        return json!({
            "status": "skipped",
            "reason": "graph_refresh_missing",
        });
    };

    if refresh.get("status") != Some(&json!("updated")) {
        return json!({
            "status": "skipped",
            "reason": "graph_refresh_not_updated",
        });
    }

    let Some(trigger) = classify_review_refresh_trigger(payload) else {
        return json!({
            "status": "skipped",
            "reason": "tool_not_review_relevant",
        });
    };

    let changed_files = extract_changed_files(repo, payload);
    if changed_files.is_empty() {
        return json!({
            "status": "skipped",
            "reason": "no_changed_files",
        });
    }

    match build_review_refresh_artifacts(repo, graph_db_path, session_id, trigger, &changed_files) {
        Ok(result) => json!({
            "status": "refreshed",
            "trigger": result.trigger,
            "changed_files": result.changed_files,
            "artifacts": result.artifacts.iter().map(|artifact| json!({
                "kind": artifact.kind,
                "source_id": artifact.source_id,
            })).collect::<Vec<_>>(),
            "max_depth": MAX_HOOK_REVIEW_REFRESH_DEPTH,
            "max_nodes": MAX_HOOK_REVIEW_REFRESH_NODES,
        }),
        Err(error) => json!({
            "status": "error",
            "reason": "review_refresh_failed",
            "error": format!("{error:#}"),
        }),
    }
}

fn build_review_refresh_artifacts(
    repo: &str,
    graph_db_path: &str,
    session_id: &SessionId,
    trigger: &'static str,
    changed_files: &[String],
) -> Result<ReviewRefreshResult> {
    let bounded_files: Vec<String> = changed_files
        .iter()
        .take(MAX_HOOK_REVIEW_REFRESH_FILES)
        .cloned()
        .collect();
    let changed: Vec<atlas_core::model::ChangedFile> = bounded_files
        .iter()
        .cloned()
        .map(|path| atlas_core::model::ChangedFile {
            path,
            change_type: ChangeType::Modified,
            old_path: None,
        })
        .collect();

    let store = Store::open(graph_db_path)
        .with_context(|| format!("cannot open database at {graph_db_path}"))?;
    let review_request = ContextRequest {
        intent: ContextIntent::Review,
        target: ContextTarget::ChangedFiles {
            paths: bounded_files.clone(),
        },
        max_nodes: Some(MAX_HOOK_REVIEW_REFRESH_NODES),
        depth: Some(MAX_HOOK_REVIEW_REFRESH_DEPTH),
        ..ContextRequest::default()
    };
    let review_context = ContextEngine::new(&store)
        .build(&review_request)
        .context("review context generation failed")?;
    let explain_change = build_explain_change_summary(
        &store,
        &changed,
        &bounded_files,
        MAX_HOOK_REVIEW_REFRESH_DEPTH,
        MAX_HOOK_REVIEW_REFRESH_NODES,
    )
    .context("explain change generation failed")?;
    let impact_result = advanced_impact(
        store
            .impact_radius(
                &bounded_files.iter().map(String::as_str).collect::<Vec<_>>(),
                MAX_HOOK_REVIEW_REFRESH_DEPTH,
                MAX_HOOK_REVIEW_REFRESH_NODES,
            )
            .context("impact radius generation failed")?,
    );

    let review_source_id = persist_named_hook_artifact(
        repo,
        graph_db_path,
        session_id,
        trigger,
        "review_context",
        &serde_json::to_value(&review_context).context("cannot serialize review context")?,
    )?;
    let explain_source_id = persist_named_hook_artifact(
        repo,
        graph_db_path,
        session_id,
        trigger,
        "explain_change",
        &serde_json::to_value(&explain_change).context("cannot serialize explain change")?,
    )?;
    let impact_source_id = persist_named_hook_artifact(
        repo,
        graph_db_path,
        session_id,
        trigger,
        "impact_result",
        &serde_json::to_value(&impact_result).context("cannot serialize impact result")?,
    )?;

    Ok(ReviewRefreshResult {
        trigger,
        changed_files: bounded_files,
        artifacts: vec![
            ReviewRefreshArtifact {
                kind: "review_context",
                source_id: review_source_id,
            },
            ReviewRefreshArtifact {
                kind: "explain_change",
                source_id: explain_source_id,
            },
            ReviewRefreshArtifact {
                kind: "impact_result",
                source_id: impact_source_id,
            },
        ],
    })
}

fn persist_named_hook_artifact(
    repo: &str,
    graph_db_path: &str,
    session_id: &SessionId,
    trigger: &str,
    kind: &str,
    value: &Value,
) -> Result<String> {
    let artifact_json =
        serde_json::to_string_pretty(value).context("cannot serialize hook artifact")?;
    let artifact_text = format!(
        "kind: {kind}\n{}",
        artifact_json
            .lines()
            .enumerate()
            .map(|(index, line)| format!("{:06}| {line}", index + 1))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let source_id = generate_source_id(&format!("{repo}:{trigger}:{kind}"), &artifact_text);
    let mut content_store = ContentStore::open(&derive_content_db_path(graph_db_path))
        .context("cannot open hook artifact content store")?;
    content_store
        .migrate()
        .context("cannot migrate hook artifact content store")?;
    content_store
        .index_artifact(
            SourceMeta {
                id: source_id.clone(),
                session_id: Some(session_id.as_str().to_owned()),
                source_type: kind.to_owned(),
                label: format!("hook:{trigger}:{kind}"),
                repo_root: Some(repo.to_owned()),
            },
            &artifact_text,
            "text/plain",
        )
        .context("cannot persist hook artifact")?;
    Ok(source_id)
}

fn classify_review_refresh_trigger(payload: &Value) -> Option<&'static str> {
    let status = extract_hook_status(payload)?;
    if !matches!(status.as_str(), "ok" | "success" | "completed" | "passed") {
        return None;
    }

    let tool_name = extract_tool_name(payload)?;
    if !tool_name.eq_ignore_ascii_case("bash") {
        return None;
    }

    let command = extract_hook_command(payload)?;
    classify_build_test_command(&command)
}

fn extract_hook_status(payload: &Value) -> Option<String> {
    find_first_string_by_key(payload, &["status", "result"])
        .map(|status| status.to_ascii_lowercase())
}

fn extract_hook_command(payload: &Value) -> Option<String> {
    find_first_string_by_key(
        payload,
        &["command", "cmd", "shell_command", "shellCommand"],
    )
}

fn classify_build_test_command(command: &str) -> Option<&'static str> {
    let command = command.to_ascii_lowercase();
    let build_markers = [
        "cargo build",
        "cargo check",
        "go build",
        "npm run build",
        "pnpm build",
        "yarn build",
        "bun build",
    ];
    if build_markers.iter().any(|marker| command.contains(marker)) {
        return Some("build");
    }

    let test_markers = [
        "cargo test",
        "cargo nextest",
        "pytest",
        "go test",
        "npm test",
        "pnpm test",
        "yarn test",
        "bun test",
        "mvn test",
        "gradle test",
    ];
    test_markers
        .iter()
        .any(|marker| command.contains(marker))
        .then_some("test")
}

fn collect_context_hints(
    repo: &str,
    graph_db_path: &str,
    store: &SessionStore,
    session_id: &SessionId,
) -> Value {
    let events = match store.list_events(session_id) {
        Ok(events) => events,
        Err(_) => {
            return json!({
                "recent_files": Vec::<String>::new(),
                "recent_source_ids": Vec::<String>::new(),
                "recent_saved_context": Vec::<Value>::new(),
                "recent_prompts": Vec::<String>::new(),
            });
        }
    };

    let mut files = BTreeSet::new();
    let mut source_ids = BTreeSet::new();
    let mut prompts = Vec::new();
    let mut hook_events = Vec::new();

    for event in events.iter().rev().take(MAX_HOOK_EVENT_SCAN) {
        let Ok(payload) = serde_json::from_str::<Value>(&event.payload_json) else {
            continue;
        };

        if let Some(name) = payload.get("hook_event").and_then(Value::as_str)
            && !hook_events.iter().any(|existing| existing == name)
        {
            hook_events.push(name.to_owned());
        }
        if prompts.len() < MAX_HOOK_SOURCE_HINTS
            && let Some(prompt) = extract_prompt_text(&payload)
        {
            let sample = excerpt(&prompt, 120);
            if !prompts.iter().any(|existing| existing == &sample) {
                prompts.push(sample);
            }
        }

        collect_source_ids(&payload, &mut source_ids);
        for path in extract_changed_files(repo, &payload) {
            files.insert(path);
        }
    }

    let recent_source_ids: Vec<String> = source_ids
        .iter()
        .take(MAX_HOOK_SOURCE_HINTS)
        .cloned()
        .collect();
    let recent_saved_context = read_source_summaries(graph_db_path, &recent_source_ids);

    json!({
        "recent_files": files.into_iter().take(MAX_HOOK_EVENT_SCAN).collect::<Vec<_>>(),
        "recent_source_ids": recent_source_ids,
        "recent_saved_context": recent_saved_context,
        "recent_prompts": prompts,
        "recent_hook_events": hook_events,
    })
}

fn read_source_summaries(graph_db_path: &str, source_ids: &[String]) -> Vec<Value> {
    if source_ids.is_empty() {
        return Vec::new();
    }

    let mut content_store = match ContentStore::open(&derive_content_db_path(graph_db_path)) {
        Ok(store) => store,
        Err(_) => return Vec::new(),
    };
    if content_store.migrate().is_err() {
        return Vec::new();
    }

    source_ids
        .iter()
        .filter_map(|source_id| {
            let source = content_store.get_source(source_id).ok().flatten()?;
            Some(json!({
                "source_id": source.id,
                "label": source.label,
                "source_type": source.source_type,
                "created_at": source.created_at,
            }))
        })
        .collect()
}

fn snapshot_to_value(snapshot: &ResumeSnapshot) -> Value {
    json!({
        "event_count": snapshot.event_count,
        "consumed": snapshot.consumed,
        "created_at": snapshot.created_at,
        "updated_at": snapshot.updated_at,
        "snapshot": parse_json_or_string(&snapshot.snapshot),
    })
}

fn parse_json_or_string(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_owned()))
}

fn prompt_lookup_query(target: &ContextTarget, prompt_text: &str) -> String {
    match target {
        ContextTarget::QualifiedName { qname } => qname.clone(),
        ContextTarget::SymbolName { name } => name.clone(),
        ContextTarget::FilePath { path } => path.clone(),
        ContextTarget::ChangedFiles { paths } => paths
            .first()
            .cloned()
            .unwrap_or_else(|| prompt_text.to_owned()),
        ContextTarget::ChangedSymbols { qnames } => qnames
            .first()
            .cloned()
            .unwrap_or_else(|| prompt_text.to_owned()),
        ContextTarget::EdgeQuerySeed { source_qname, .. } => source_qname.clone(),
    }
}

fn extract_prompt_text(payload: &Value) -> Option<String> {
    find_first_string_by_key(
        payload,
        &["prompt", "query", "message", "text", "content", "raw"],
    )
}

fn extract_tool_name(payload: &Value) -> Option<String> {
    find_first_string_by_key(payload, &["tool_name", "toolName", "tool"])
}

fn tool_may_change_files(tool_name: &str) -> bool {
    matches!(
        tool_name.to_ascii_lowercase().as_str(),
        "edit" | "write" | "multiedit" | "bash" | "patch"
    )
}

fn extract_changed_files(repo: &str, payload: &Value) -> Vec<String> {
    let mut candidates = Vec::new();
    collect_strings_for_keys(
        payload,
        &[
            "files",
            "paths",
            "changed_files",
            "changedFiles",
            "file",
            "file_path",
            "filePath",
            "path",
            "target_file",
            "targetPath",
        ],
        &mut candidates,
    );

    let mut normalized = BTreeSet::new();
    for candidate in candidates {
        if let Some(path) = normalize_hook_path(repo, &candidate) {
            normalized.insert(path);
        }
    }
    normalized.into_iter().collect()
}

fn collect_source_ids(payload: &Value, source_ids: &mut BTreeSet<String>) {
    let mut candidates = Vec::new();
    collect_strings_for_keys(payload, &["source_id"], &mut candidates);
    for source_id in candidates {
        if !source_id.trim().is_empty() {
            source_ids.insert(source_id);
        }
    }
}

fn find_first_string_by_key(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Value::Array(values) => values
            .iter()
            .find_map(|nested| find_first_string_by_key(nested, keys)),
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map.get(*key).and_then(extract_string_value) {
                    return Some(found);
                }
            }
            map.values()
                .find_map(|nested| find_first_string_by_key(nested, keys))
        }
        _ => None,
    }
}

fn collect_strings_for_keys(value: &Value, keys: &[&str], out: &mut Vec<String>) {
    match value {
        Value::Array(values) => {
            for nested in values {
                collect_strings_for_keys(nested, keys, out);
            }
        }
        Value::Object(map) => {
            for key in keys {
                if let Some(value) = map.get(*key) {
                    collect_all_strings(value, out);
                }
            }
            for nested in map.values() {
                collect_strings_for_keys(nested, keys, out);
            }
        }
        _ => {}
    }
}

fn collect_all_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                out.push(trimmed.to_owned());
            }
        }
        Value::Array(values) => {
            for nested in values {
                collect_all_strings(nested, out);
            }
        }
        Value::Object(map) => {
            for nested in map.values() {
                collect_all_strings(nested, out);
            }
        }
        _ => {}
    }
}

fn extract_string_value(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_owned())
        }
        Value::Array(values) => values.iter().find_map(extract_string_value),
        Value::Object(map) => [
            "text", "content", "value", "prompt", "query", "message", "path", "file",
        ]
        .iter()
        .find_map(|key| map.get(*key).and_then(extract_string_value)),
        _ => None,
    }
}

fn normalize_hook_path(repo: &str, candidate: &str) -> Option<String> {
    let trimmed = candidate.trim();
    if trimmed.is_empty() || trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return None;
    }

    let repo_path = Path::new(repo);
    let candidate_path = Path::new(trimmed);
    let normalized = if candidate_path.is_absolute() {
        candidate_path
            .strip_prefix(repo_path)
            .ok()?
            .to_string_lossy()
            .into_owned()
    } else {
        trimmed.trim_start_matches("./").replace('\\', "/")
    };

    let normalized = normalized.trim_matches('/').to_owned();
    if normalized.is_empty() || normalized.starts_with(".atlas/") {
        return None;
    }
    Some(normalized)
}

fn excerpt(text: &str, max_chars: usize) -> String {
    let truncated: String = text.chars().take(max_chars).collect();
    if truncated.chars().count() == text.chars().count() {
        truncated
    } else {
        format!("{truncated}...")
    }
}

fn strip_inline_file_content(value: Value) -> Value {
    match value {
        Value::Array(values) => {
            Value::Array(values.into_iter().map(strip_inline_file_content).collect())
        }
        Value::Object(map) => {
            let mut sanitized = Map::new();
            for (key, value) in map {
                if FILE_CHANGED_INLINE_CONTENT_KEYS.contains(&key.as_str()) {
                    continue;
                }
                sanitized.insert(key, strip_inline_file_content(value));
            }
            Value::Object(sanitized)
        }
        other => other,
    }
}

fn sanitize_payload_for_storage(policy: &HookPolicy, payload: Value) -> Value {
    if policy.canonical_event == "file-changed" {
        strip_inline_file_content(payload)
    } else {
        payload
    }
}

fn persist_hook_event(
    repo: &str,
    graph_db_path: &str,
    frontend: &str,
    event: &str,
    payload: Value,
) -> Result<HookPersistence> {
    let session_id = SessionId::derive(repo, "", frontend);
    let session_db_path = derive_session_db_path(graph_db_path);
    let mut store = SessionStore::open(&session_db_path)
        .with_context(|| format!("cannot open session store at {session_db_path}"))?;
    store
        .upsert_session_meta(session_id.clone(), repo, frontend, None)
        .context("cannot register hook session")?;

    let pending_resume = store
        .get_resume_snapshot(&session_id)?
        .as_ref()
        .is_some_and(|snapshot| !snapshot.consumed);

    let policy = resolve_hook_policy(event)?;
    let sanitized_payload = sanitize_payload_for_storage(policy, payload);
    let routed = route_hook_payload(
        repo,
        graph_db_path,
        &session_id,
        frontend,
        policy,
        sanitized_payload.clone(),
    )?;
    let hook_metadata = build_hook_event_metadata(HookMetadataContext {
        repo,
        graph_db_path,
        store: &store,
        session_id: &session_id,
        policy,
        payload: &sanitized_payload,
        routed: &routed,
        pending_resume,
    });
    let event_row = build_hook_event(
        &session_id,
        HookEventParts {
            frontend,
            event,
            payload: routed.event_payload,
            hook_metadata,
            source_id: routed.source_id.as_deref(),
            storage_kind: routed.storage_kind,
            pending_resume,
        },
    );
    let stored_event_id = store.append_event(event_row)?.map(|row| row.id);

    let snapshot = if policy.build_resume_snapshot {
        let built = store.build_resume(&session_id)?;
        Some(json!({
            "event_count": built.event_count,
            "consumed": built.consumed,
            "updated_at": built.updated_at,
        }))
    } else {
        None
    };

    Ok(HookPersistence {
        session_id,
        pending_resume,
        stored_event_id,
        snapshot,
        source_id: routed.source_id,
        storage_kind: routed.storage_kind,
    })
}

fn resolve_hook_policy(event: &str) -> Result<&'static HookPolicy> {
    let Some(policy) = HOOK_POLICIES
        .iter()
        .find(|policy| policy.aliases.contains(&event))
    else {
        bail!("unknown hook event: {event}");
    };

    Ok(policy)
}

fn route_hook_payload(
    repo: &str,
    graph_db_path: &str,
    session_id: &SessionId,
    frontend: &str,
    policy: &HookPolicy,
    payload: Value,
) -> Result<HookPayloadRouting> {
    if payload.is_null() {
        return Ok(HookPayloadRouting {
            event_payload: payload,
            source_id: None,
            storage_kind: None,
        });
    }

    let raw_payload = serde_json::to_string(&payload).context("cannot serialize hook payload")?;
    let label = format!("hook:{frontend}:{}", policy.canonical_event);
    let mut content_store = ContentStore::open(&derive_content_db_path(graph_db_path))
        .context("cannot open hook content store")?;
    content_store
        .migrate()
        .context("cannot migrate hook content store")?;

    let meta = SourceMeta {
        id: generate_source_id(&format!("{repo}:{label}"), &raw_payload),
        session_id: Some(session_id.as_str().to_owned()),
        source_type: "hook_event".to_owned(),
        label,
        repo_root: Some(repo.to_owned()),
    };

    match content_store.route_output(meta, &raw_payload, "application/json")? {
        OutputRouting::Raw(_) => Ok(HookPayloadRouting {
            event_payload: payload,
            source_id: None,
            storage_kind: None,
        }),
        OutputRouting::Preview { source_id, preview } => Ok(HookPayloadRouting {
            event_payload: json!({ "preview": preview }),
            source_id: Some(source_id),
            storage_kind: Some("preview"),
        }),
        OutputRouting::Pointer { source_id } => Ok(HookPayloadRouting {
            event_payload: Value::Null,
            source_id: Some(source_id),
            storage_kind: Some("pointer"),
        }),
    }
}

fn hook_frontend() -> String {
    std::env::var("ATLAS_HOOK_FRONTEND")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "hook".to_owned())
}

fn read_hook_payload() -> Result<Value> {
    let mut raw = String::new();
    std::io::stdin()
        .take(MAX_HOOK_STDIN_BYTES)
        .read_to_string(&mut raw)
        .context("cannot read hook payload from stdin")?;

    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(Value::Null);
    }

    let parsed =
        serde_json::from_str::<Value>(trimmed).unwrap_or_else(|_| json!({ "raw": trimmed }));
    Ok(redact_payload(parsed))
}

fn build_hook_event(session_id: &SessionId, parts: HookEventParts<'_>) -> NewSessionEvent {
    let policy = resolve_hook_policy(parts.event).expect("recognized hook event");
    let mut payload = json!({
        "frontend": parts.frontend,
        "hook_event": policy.canonical_event,
        "payload": parts.payload,
        "hook_metadata": parts.hook_metadata,
    });

    if let Some(obj) = payload.as_object_mut() {
        if let Some(source_id) = parts.source_id {
            obj.insert("source_id".to_owned(), Value::String(source_id.to_owned()));
        }
        if let Some(storage_kind) = parts.storage_kind {
            obj.insert(
                "payload_storage".to_owned(),
                json!({ "kind": storage_kind, "content_type": "application/json" }),
            );
        }
    }

    if policy.session_start {
        NewSessionEvent {
            session_id: session_id.clone(),
            event_type: if parts.pending_resume {
                SessionEventType::SessionResume
            } else {
                SessionEventType::SessionStart
            },
            priority: policy.priority,
            payload: json!({
                "frontend": parts.frontend,
                "hook_event": policy.canonical_event,
                "pending_resume": parts.pending_resume,
                "payload": payload["payload"].clone(),
                "hook_metadata": payload["hook_metadata"].clone(),
            }),
            created_at: None,
        }
    } else {
        normalize_event(policy.event_type.clone(), policy.priority, payload)
            .bind(session_id.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::process::Command as ProcessCommand;

    use atlas_contentstore::ContentStore;
    use atlas_engine::{BuildOptions, build_graph};
    use atlas_store_sqlite::Store;
    use tempfile::TempDir;

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

    fn last_hook_payload(graph_db_path: &str, repo: &str, frontend: &str) -> Value {
        let session_store = SessionStore::open(&derive_session_db_path(graph_db_path)).unwrap();
        let session_id = SessionId::derive(repo, "", frontend);
        let events = session_store.list_events(&session_id).unwrap();
        serde_json::from_str(&events.last().unwrap().payload_json).unwrap()
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
            persist_hook_event(&repo, &graph_db_path, "hook", "session-start", Value::Null)
                .unwrap();
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

        if let Some(value) = prior_script {
            unsafe {
                std::env::set_var("ATLAS_HOOK_SCRIPT_PATH", value);
            }
        } else {
            unsafe {
                std::env::remove_var("ATLAS_HOOK_SCRIPT_PATH");
            }
        }

        assert_eq!(resolved, repo.to_string_lossy());
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

        let mut session_store =
            SessionStore::open(&derive_session_db_path(&graph_db_path)).unwrap();
        let session_id = SessionId::derive(&repo, "", "claude");
        let events = session_store.list_events(&session_id).unwrap();
        let last_payload: Value =
            serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
        assert_eq!(last_payload["source_id"], source_id);
        assert_eq!(last_payload["payload_storage"]["kind"], "pointer");

        let mut content_store =
            ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
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
        let last_payload: Value =
            serde_json::from_str(&events.last().unwrap().payload_json).unwrap();
        assert_eq!(last_payload["source_id"], source_id);
        assert_eq!(last_payload["payload_storage"]["kind"], "pointer");

        let mut content_store =
            ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
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
            persist_hook_event(&repo, &graph_db_path, "hook", "session-start", Value::Null)
                .unwrap();
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
        let source_id = generate_source_id("review-context", "BillingService review context");

        std::fs::create_dir_all(dir.path().join(".atlas")).unwrap();

        let mut content_store =
            ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
        content_store.migrate().unwrap();
        content_store
            .index_artifact(
                SourceMeta {
                    id: source_id.clone(),
                    session_id: None,
                    source_type: "review_context".to_owned(),
                    label: "review context".to_owned(),
                    repo_root: Some(repo.clone()),
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
        let persisted =
            persist_hook_event(&repo, &graph_db_path, "hook", "stop", Value::Null).unwrap();
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
        let mut content_store =
            ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
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

        let mut content_store =
            ContentStore::open(&derive_content_db_path(&graph_db_path)).unwrap();
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
}
