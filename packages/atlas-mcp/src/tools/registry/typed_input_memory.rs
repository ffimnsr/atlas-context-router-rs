//! Typed input-schema arm for the `memory` tool family.
//!
//! Dispatched by `super::typed_input_schema_for()`; schema structs
//! come from `super::schemas`.

use super::*;
use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::Value;

pub(super) fn typed_input_schema_for(name: &str) -> Option<Value> {
    match name {
        "get_session_status" => Some(
            typed_schema_with_descriptions::<GetSessionStatusArgsSchema>(&[
                (
                    "properties/session_id",
                    "Explicit session id. Omit to use the derived id for the current repo.",
                ),
                (
                    "properties/agent_id",
                    "Restrict status to one agent memory partition.",
                ),
                (
                    "properties/merge_agent_partitions",
                    "Intentionally merge status across all agent partitions.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ]),
        ),
        "record_session_event" => Some(typed_schema_with_descriptions::<
            RecordSessionEventArgsSchema,
        >(&[
            (
                "properties/event",
                "Hook-compatible event name or alias (kebab-case or PascalCase). Examples: session-start, user-prompt, post-tool-use, file-changed, stop, session-end, tool-failure, SessionStart, PostToolUse.",
            ),
            (
                "properties/payload",
                "Event payload. Include prompt text, tool name, command, changed_files, status, or summary when available.",
            ),
            (
                "properties/frontend",
                "Agent/frontend name recorded with the event. Defaults to mcp.",
            ),
            (
                "properties/session_id",
                "Explicit session id. Omit to use the derived mcp session for the current repo.",
            ),
            (
                "properties/agent_id",
                "Optional agent memory partition label echoed back in the response.",
            ),
            (
                "properties/repo_scope",
                "Repo scope object. Use { kind: 'current' } or { kind: 'repo_id', repo_id: '<id>' }. Multi-repo scopes are rejected; event capture is per-repo.",
            ),
            (
                "properties/repo_scope/properties/kind",
                "Repo scope kind: current or repo_id.",
            ),
            (
                "properties/repo_scope/properties/repo_id",
                "Required when kind='repo_id'. Must be registered and enabled.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "wake_up" => Some(typed_schema_with_descriptions::<WakeUpArgsSchema>(&[
            (
                "properties/topic",
                "Optional topic hint used to focus decision-memory search and to fill current_focus when no prior session exists.",
            ),
            (
                "properties/session_id",
                "Explicit session id. Omit to use the derived mcp session for the current repo.",
            ),
            (
                "properties/frontend",
                "Agent/frontend name recorded with the wake-up event. Defaults to mcp.",
            ),
            (
                "properties/agent_id",
                "Restrict resume-snapshot recall to one agent memory partition.",
            ),
            (
                "properties/max_items",
                "Cap for every list in the pack (default 10, hard-clamped to 25).",
            ),
            (
                "properties/repo_scope",
                "Repo scope object. Use { kind: 'current' } or { kind: 'repo_id', repo_id: '<id>' }. Multi-repo scopes are rejected; wake-up is per-repo.",
            ),
            (
                "properties/repo_scope/properties/kind",
                "Repo scope kind: current or repo_id.",
            ),
            (
                "properties/repo_scope/properties/repo_id",
                "Required when kind='repo_id'. Must be registered and enabled.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "compact_session" => Some(typed_schema_with_descriptions::<CompactSessionArgsSchema>(
            &[
                (
                    "properties/session_id",
                    "Explicit session id. Omit to use the derived id for the current repo.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "resume_session" => Some(typed_schema_with_descriptions::<ResumeSessionArgsSchema>(
            &[
                (
                    "properties/session_id",
                    "Explicit session id. Omit to use the derived id for the current repo.",
                ),
                (
                    "properties/agent_id",
                    "Restrict resume output to one agent memory partition.",
                ),
                (
                    "properties/merge_agent_partitions",
                    "Intentionally merge resume output across all agent partitions.",
                ),
                (
                    "properties/mark_consumed",
                    "Mark the snapshot consumed after reading (default true).",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "search_saved_context" => Some(typed_schema_with_descriptions::<
            SearchSavedContextArgsSchema,
        >(&[
            ("properties/query", "Search query text."),
            (
                "properties/repo_scope",
                "Repo scope object for cross-session search. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
            ),
            (
                "properties/repo_scope/properties/kind",
                "Repo scope kind: current, repo_id, or all.",
            ),
            (
                "properties/repo_scope/properties/repo_id",
                "Required when kind='repo_id'. Must be registered and enabled.",
            ),
            (
                "properties/session_id",
                "Restrict search to artifacts from this session.",
            ),
            (
                "properties/agent_id",
                "Restrict search to artifacts from one agent memory partition.",
            ),
            (
                "properties/merge_agent_partitions",
                "Intentionally merge saved-context search across all agent partitions.",
            ),
            (
                "properties/source_type",
                "Filter by source type (e.g. 'review_context', 'mcp_artifact').",
            ),
            (
                "properties/limit",
                "Maximum results to return (default 10).",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "search_decisions" => Some(typed_schema_with_descriptions::<SearchDecisionsArgsSchema>(
            &[
                ("properties/query", "Search query text."),
                (
                    "properties/session_id",
                    "Restrict search to one session. Omit for repo-wide decision recall.",
                ),
                (
                    "properties/limit",
                    "Maximum decisions to return (default 10).",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "read_saved_context" => Some(
            typed_schema_with_descriptions::<ReadSavedContextArgsSchema>(&[
                (
                    "properties/source_id",
                    "The source_id returned by save_context_artifact or search_saved_context.",
                ),
                (
                    "properties/session_id",
                    "Optional: restrict access to artifacts owned by this session. Omit to skip session scoping.",
                ),
                (
                    "properties/agent_id",
                    "Optional: restrict access to artifacts owned by this agent partition.",
                ),
                (
                    "properties/merge_agent_partitions",
                    "When true, allow reads across agent partitions intentionally after repo/session checks pass.",
                ),
                (
                    "properties/chunk_offset",
                    "0-based chunk index to start reading from (default 0). Use next_chunk_offset from a prior truncated response for paging.",
                ),
                (
                    "properties/max_bytes",
                    "Byte cap on returned content (default 65536). When content exceeds this the response sets truncated=true and includes next_chunk_offset.",
                ),
                (
                    "properties/repo_scope",
                    "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
                ),
                (
                    "properties/repo_scope/properties/kind",
                    "Repo scope kind: current, repo_id, or all.",
                ),
                (
                    "properties/repo_scope/properties/repo_id",
                    "Required when kind='repo_id'. Must be registered and enabled.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ]),
        ),
        "save_context_artifact" => Some(typed_schema_with_descriptions::<
            SaveContextArtifactArgsSchema,
        >(&[
            ("properties/content", "The content to store."),
            (
                "properties/label",
                "Human-readable label for display and retrieval.",
            ),
            (
                "properties/source_type",
                "Category tag (e.g. 'review_context', 'mcp_artifact'). Default: 'mcp_artifact'.",
            ),
            (
                "properties/session_id",
                "Associate artifact with this session. Omit to use derived session.",
            ),
            (
                "properties/agent_id",
                "Associate artifact with this agent memory partition.",
            ),
            (
                "properties/repo_scope",
                "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
            ),
            (
                "properties/repo_scope/properties/kind",
                "Repo scope kind: current, repo_id, or all.",
            ),
            (
                "properties/repo_scope/properties/repo_id",
                "Required when kind='repo_id'. Must be registered and enabled.",
            ),
            (
                "properties/content_type",
                "MIME type: 'text/plain' (default), 'text/markdown', or 'application/json'.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "purge_saved_context" => Some(
            typed_schema_with_descriptions::<PurgeSavedContextArgsSchema>(&[
                (
                    "properties/session_id",
                    "Delete all saved artifacts for this session.",
                ),
                (
                    "properties/agent_id",
                    "Restrict session deletion to one agent memory partition.",
                ),
                (
                    "properties/keep_days",
                    "For age-based cleanup: keep sources newer than this many days (default 30).",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ]),
        ),
        "cross_session_search" => Some(typed_schema_with_descriptions::<
            CrossSessionSearchArgsSchema,
        >(&[
            ("properties/query", "Full-text or semantic search query."),
            (
                "properties/source_type",
                "Optional filter: restrict to a specific source_type (e.g. 'mcp_artifact').",
            ),
            (
                "properties/agent_id",
                "Restrict cross-session search to one agent memory partition.",
            ),
            (
                "properties/merge_agent_partitions",
                "Intentionally merge cross-session search across all agent partitions.",
            ),
            (
                "properties/limit",
                "Maximum results to return (default 10).",
            ),
            (
                "properties/repo_scope",
                "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
            ),
            (
                "properties/repo_scope/properties/kind",
                "Repo scope kind: current, repo_id, or all.",
            ),
            (
                "properties/repo_scope/properties/repo_id",
                "Required when kind='repo_id'. Must be registered and enabled.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "get_global_memory" => Some(typed_schema_with_descriptions::<GetGlobalMemoryArgsSchema>(
            &[
                (
                    "properties/limit",
                    "Maximum entries to return per category (default 10).",
                ),
                (
                    "properties/focus_symbols",
                    "Symbol qualified names from the current context used to find related past sessions.",
                ),
                (
                    "properties/focus_files",
                    "File paths from the current context used to find related past sessions.",
                ),
                ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
            ],
        )),
        "memory_store" => Some(typed_schema_with_descriptions::<MemoryStoreArgsSchema>(&[
            (
                "properties/text",
                "Memory body text; stored exactly as provided.",
            ),
            ("properties/topic", "Topic label for the memory."),
            ("properties/title", "Optional title line."),
            (
                "properties/importance",
                "Importance: critical, high, normal, or low (default normal).",
            ),
            (
                "properties/scope",
                "Visibility scope: project, session, frontend, or global (default project).",
            ),
            (
                "properties/frontend",
                "Frontend identifier; required when scope is frontend. Normalized to claude, codex, copilot, cli, or mcp unless config allows custom frontends.",
            ),
            (
                "properties/source_id",
                "Source id linking this memory to a saved-context artifact.",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        "memory_recall" => Some(typed_schema_with_descriptions::<MemoryRecallArgsSchema>(&[
            (
                "properties/query",
                "Search text matched against topic, title, and body.",
            ),
            (
                "properties/topic",
                "Restrict recall to one topic (case-insensitive exact match).",
            ),
            (
                "properties/importance",
                "Restrict recall to one importance level.",
            ),
            ("properties/scope", "Restrict recall to one scope."),
            (
                "properties/shared",
                "Only return memories visible to every frontend (project + global).",
            ),
            (
                "properties/limit",
                "Maximum memories to return (default 20).",
            ),
            ("properties/output_format", DEFAULT_OUTPUT_DESCRIPTION),
        ])),
        _ => None,
    }
}
