use anyhow::{Result, bail};
use serde_json::Value;

use atlas_session::{SessionEventType, SessionId, SessionStore};

pub const MAX_HOOK_STDIN_BYTES: u64 = 64 * 1024;
pub const MAX_HOOK_EVENT_SCAN: usize = 20;
pub const MAX_HOOK_SOURCE_HINTS: usize = 3;
pub const MAX_HOOK_PROMPT_HITS: usize = 3;
pub const MAX_HOOK_REVIEW_REFRESH_FILES: usize = 8;
pub const MAX_HOOK_REVIEW_REFRESH_DEPTH: u32 = 3;
pub const MAX_HOOK_REVIEW_REFRESH_NODES: usize = 64;
pub const FILE_CHANGED_INLINE_CONTENT_KEYS: &[&str] = &[
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
pub enum HookStorage {
    SessionOnly,
    SessionAndContent,
}

#[derive(Clone, Debug)]
pub struct HookPayloadRouting {
    pub event_payload: Value,
    pub source_id: Option<String>,
    pub storage_kind: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookLifecycleAction {
    None,
    LoadRestore,
    PersistHandoff,
    VerifyRestore,
}

pub struct HookPolicy {
    pub canonical_event: &'static str,
    pub aliases: &'static [&'static str],
    pub event_type: SessionEventType,
    pub priority: i32,
    pub storage: HookStorage,
    pub lifecycle: HookLifecycleAction,
    pub prompt_routing: bool,
    pub freshness: bool,
    pub graph_refresh: bool,
    pub review_refresh: bool,
    pub build_resume_snapshot: bool,
    pub session_start: bool,
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
pub struct HookPersistence {
    pub session_id: SessionId,
    pub pending_resume: bool,
    pub stored_event_id: Option<i64>,
    pub snapshot: Option<Value>,
    pub source_id: Option<String>,
    pub storage_kind: Option<&'static str>,
}

pub struct PromptRoutingMetadata {
    pub prompt_excerpt: String,
    pub query: String,
    pub intent: Value,
    pub target: Value,
    pub hits: Vec<Value>,
}

pub struct HookMetadataContext<'a> {
    pub repo: &'a str,
    pub graph_db_path: &'a str,
    pub store: &'a SessionStore,
    pub session_id: &'a SessionId,
    pub policy: &'a HookPolicy,
    pub payload: &'a Value,
    pub routed: &'a HookPayloadRouting,
    pub pending_resume: bool,
    pub event_source: &'a str,
    pub agent_id: Option<&'a str>,
}

pub struct HookEventParts<'a> {
    pub frontend: &'a str,
    pub event: &'a str,
    pub payload: Value,
    pub hook_metadata: Value,
    pub source_id: Option<&'a str>,
    pub storage_kind: Option<&'a str>,
    pub pending_resume: bool,
    pub event_source: &'a str,
    pub agent_id: Option<&'a str>,
}

pub struct ReviewRefreshArtifact {
    pub kind: &'static str,
    pub source_id: String,
}

pub struct ReviewRefreshResult {
    pub trigger: &'static str,
    pub changed_files: Vec<String>,
    pub artifacts: Vec<ReviewRefreshArtifact>,
}

pub fn resolve_hook_policy(event: &str) -> Result<&'static HookPolicy> {
    let Some(policy) = HOOK_POLICIES
        .iter()
        .find(|policy| policy.aliases.contains(&event))
    else {
        bail!("unknown hook event: {event}");
    };

    Ok(policy)
}
