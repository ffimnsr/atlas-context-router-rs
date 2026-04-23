use anyhow::{Result, bail};
use serde_json::Value;

use atlas_session::{SessionEventType, SessionId, SessionStore};

pub(crate) const MAX_HOOK_STDIN_BYTES: u64 = 64 * 1024;
pub(crate) const MAX_HOOK_EVENT_SCAN: usize = 20;
pub(crate) const MAX_HOOK_SOURCE_HINTS: usize = 3;
pub(crate) const MAX_HOOK_PROMPT_HITS: usize = 3;
pub(crate) const MAX_HOOK_REVIEW_REFRESH_FILES: usize = 8;
pub(crate) const MAX_HOOK_REVIEW_REFRESH_DEPTH: u32 = 3;
pub(crate) const MAX_HOOK_REVIEW_REFRESH_NODES: usize = 64;
pub(crate) const FILE_CHANGED_INLINE_CONTENT_KEYS: &[&str] = &[
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
pub(crate) enum HookStorage {
    SessionOnly,
    SessionAndContent,
}

#[derive(Clone, Debug)]
pub(crate) struct HookPayloadRouting {
    pub(crate) event_payload: Value,
    pub(crate) source_id: Option<String>,
    pub(crate) storage_kind: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HookLifecycleAction {
    None,
    LoadRestore,
    PersistHandoff,
    VerifyRestore,
}

pub(crate) struct HookPolicy {
    pub(crate) canonical_event: &'static str,
    pub(crate) aliases: &'static [&'static str],
    pub(crate) event_type: SessionEventType,
    pub(crate) priority: i32,
    pub(crate) storage: HookStorage,
    pub(crate) lifecycle: HookLifecycleAction,
    pub(crate) prompt_routing: bool,
    pub(crate) freshness: bool,
    pub(crate) graph_refresh: bool,
    pub(crate) review_refresh: bool,
    pub(crate) build_resume_snapshot: bool,
    pub(crate) session_start: bool,
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
pub(crate) struct HookPersistence {
    pub(crate) session_id: SessionId,
    pub(crate) pending_resume: bool,
    pub(crate) stored_event_id: Option<i64>,
    pub(crate) snapshot: Option<Value>,
    pub(crate) source_id: Option<String>,
    pub(crate) storage_kind: Option<&'static str>,
}

pub(crate) struct PromptRoutingMetadata {
    pub(crate) prompt_excerpt: String,
    pub(crate) query: String,
    pub(crate) intent: Value,
    pub(crate) target: Value,
    pub(crate) hits: Vec<Value>,
}

pub(crate) struct HookMetadataContext<'a> {
    pub(crate) repo: &'a str,
    pub(crate) graph_db_path: &'a str,
    pub(crate) store: &'a SessionStore,
    pub(crate) session_id: &'a SessionId,
    pub(crate) policy: &'a HookPolicy,
    pub(crate) payload: &'a Value,
    pub(crate) routed: &'a HookPayloadRouting,
    pub(crate) pending_resume: bool,
}

pub(crate) struct HookEventParts<'a> {
    pub(crate) frontend: &'a str,
    pub(crate) event: &'a str,
    pub(crate) payload: Value,
    pub(crate) hook_metadata: Value,
    pub(crate) source_id: Option<&'a str>,
    pub(crate) storage_kind: Option<&'a str>,
    pub(crate) pending_resume: bool,
}

pub(crate) struct ReviewRefreshArtifact {
    pub(crate) kind: &'static str,
    pub(crate) source_id: String,
}

pub(crate) struct ReviewRefreshResult {
    pub(crate) trigger: &'static str,
    pub(crate) changed_files: Vec<String>,
    pub(crate) artifacts: Vec<ReviewRefreshArtifact>,
}

pub(crate) fn resolve_hook_policy(event: &str) -> Result<&'static HookPolicy> {
    let Some(policy) = HOOK_POLICIES
        .iter()
        .find(|policy| policy.aliases.contains(&event))
    else {
        bail!("unknown hook event: {event}");
    };

    Ok(policy)
}
