//! Base `tools/list` registry entries for the `memory` tool family.
//!
//! Assembled by `super::base_tool_list_json()`; entry order is
//! irrelevant because descriptors are sorted by name afterwards.

use crate::tools::shared::DEFAULT_OUTPUT_DESCRIPTION;
use serde_json::{Value, json};

/// Base registry entry JSON for the memory tools.
pub(super) fn tools() -> Vec<Value> {
    vec![
        json!({
                "name": "get_session_status",
                "description": "Return the status of the current MCP session: identity, event count, last compaction time, and whether a resume snapshot exists.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id":    { "type": "string",  "description": "Explicit session id. Omit to use the derived id for the current repo." },
                        "agent_id":      { "type": "string",  "description": "Restrict status to one agent memory partition." },
                        "merge_agent_partitions": { "type": "boolean", "description": "Intentionally merge status across all agent partitions." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "record_session_event",
                "description": "Session capture for hosts without native LLM hooks, mirroring `atlas hook <event>` exactly. Records the same agent events through the shared event service: session-start, user-prompt, pre-tool-use, post-tool-use, pre-compact, post-compact, stop, session-end, file-changed, tool-failure, error, and the other supported hook events. Returns stored event identity, storage routing, resume snapshot state, and executed actions (lifecycle, prompt routing, graph refresh, freshness, review refresh).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "event": { "type": "string",  "description": "Hook-compatible event name or alias (kebab-case or PascalCase). Examples: session-start, user-prompt, post-tool-use, file-changed, stop, session-end, tool-failure, SessionStart, PostToolUse." },
                        "payload": { "type": "object", "description": "Event payload. Include prompt text, tool name, command, changed_files, status, or summary when available." },
                        "frontend": { "type": "string",  "description": "Agent/frontend name recorded with the event. Defaults to mcp." },
                        "session_id": { "type": "string",  "description": "Explicit session id. Omit to use the derived mcp session for the current repo." },
                        "agent_id": { "type": "string",  "description": "Optional agent memory partition label echoed back in the response." },
                        "repo_scope": {
                            "type": "object",
                            "description": "Repo scope object. Use { kind: 'current' } or { kind: 'repo_id', repo_id: '<id>' }. Multi-repo scopes are rejected; event capture is per-repo.",
                            "properties": {
                                "kind": { "type": "string", "description": "Repo scope kind: current or repo_id." },
                                "repo_id": { "type": "string", "description": "Required when kind='repo_id'. Must be registered and enabled." }
                            },
                            "required": ["kind"]
                        },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["event"]
                }
        }),
        json!({
                "name": "wake_up",
                "description": "Bounded session-start recall for hookless agents. Assembles a compact context pack from the resume snapshot, decision memory, saved-context hints, global memory, changed files, and graph readiness, then records the session-start event through the shared event service shared with native hooks. Large saved artifacts are referenced by source_id only, never inlined.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "topic": { "type": "string",  "description": "Optional topic hint used to focus decision-memory search and to fill current_focus when no prior session exists." },
                        "session_id": { "type": "string",  "description": "Explicit session id. Omit to use the derived mcp session for the current repo." },
                        "frontend": { "type": "string",  "description": "Agent/frontend name recorded with the wake-up event. Defaults to mcp." },
                        "agent_id": { "type": "string",  "description": "Restrict resume-snapshot recall to one agent memory partition." },
                        "max_items": { "type": "integer",  "description": "Cap for every list in the pack (default 10, hard-clamped to 25)." },
                        "repo_scope": {
                            "type": "object",
                            "description": "Repo scope object. Use { kind: 'current' } or { kind: 'repo_id', repo_id: '<id>' }. Multi-repo scopes are rejected; wake-up is per-repo.",
                            "properties": {
                                "kind": { "type": "string", "description": "Repo scope kind: current or repo_id." },
                                "repo_id": { "type": "string", "description": "Required when kind='repo_id'. Must be registered and enabled." }
                            },
                            "required": ["kind"]
                        },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "compact_session",
                "description": "Compact and curate the session event ledger. Removes stale low-value events, merges repeated actions, deduplicates reasoning outputs, and promotes high-value events to survive future eviction. Returns curation stats. Safe to call repeatedly.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id":    { "type": "string",  "description": "Explicit session id. Omit to use the derived id for the current repo." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "resume_session",
                "description": "Retrieve and optionally consume the resume snapshot for the current (or specified) session. Builds a snapshot on demand if one does not exist.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id":     { "type": "string",  "description": "Explicit session id. Omit to use the derived id for the current repo." },
                        "agent_id":       { "type": "string",  "description": "Restrict resume output to one agent memory partition." },
                        "merge_agent_partitions": { "type": "boolean", "description": "Intentionally merge resume output across all agent partitions." },
                        "mark_consumed":  { "type": "boolean", "description": "Mark the snapshot consumed after reading (default true)." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "search_saved_context",
                "description": "Search previously saved artifacts in the content store using BM25 + trigram fallback. Returns stable object `structuredContent` with `query`, preview `matches`, `summary`, `truncated`, and `warnings` for follow-up retrieval.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":        { "type": "string",  "description": "Search query text." },
                        "repo_scope": {
                            "type": "object",
                            "description": "Repo scope object for cross-session search. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Repo scope kind: current, repo_id, or all." },
                                "repo_id": { "type": "string", "description": "Required when kind='repo_id'. Must be registered and enabled." }
                            },
                            "required": ["kind"]
                        },
                        "session_id":   { "type": "string",  "description": "Restrict search to artifacts from this session." },
                        "agent_id":     { "type": "string",  "description": "Restrict search to artifacts from one agent memory partition." },
                        "merge_agent_partitions": { "type": "boolean", "description": "Intentionally merge saved-context search across all agent partitions." },
                        "source_type":  { "type": "string",  "description": "Filter by source type (e.g. 'review_context', 'mcp_artifact')." },
                        "limit":        { "type": "integer", "description": "Maximum results to return (default 10)." },
                        "output_format":{ "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
        }),
        json!({
                "name": "search_decisions",
                "description": "Search persisted decision memory for prior conclusions, linked evidence, and artifact references. Returns stable object `structuredContent` with `query`, `matches`, `summary`, `truncated`, and `warnings`.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":        { "type": "string",  "description": "Search query text." },
                        "session_id":   { "type": "string",  "description": "Restrict search to one session. Omit for repo-wide decision recall." },
                        "limit":        { "type": "integer", "description": "Maximum decisions to return (default 10)." },
                        "output_format":{ "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
        }),
        json!({
                "name": "read_saved_context",
                "description": "Retrieve the full content of a saved artifact by source_id. Supports paging via chunk_offset and max_bytes for large artifacts. Enforces session and repository scoping so cross-session reads are blocked.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "source_id":     { "type": "string",  "description": "The source_id returned by save_context_artifact or search_saved_context." },
                        "session_id":    { "type": "string",  "description": "Optional: restrict access to artifacts owned by this session. Omit to skip session scoping." },
                        "agent_id":      { "type": "string",  "description": "Optional: restrict access to artifacts owned by this agent partition." },
                        "merge_agent_partitions": { "type": "boolean", "description": "When true, allow reads across agent partitions intentionally after repo/session checks pass." },
                        "chunk_offset":  { "type": "integer", "description": "0-based chunk index to start reading from (default 0). Use next_chunk_offset from a prior truncated response for paging." },
                        "max_bytes":     { "type": "integer", "description": "Byte cap on returned content (default 65536). When content exceeds this the response sets truncated=true and includes next_chunk_offset." },
                        "repo_scope": {
                            "type": "object",
                            "description": "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Repo scope kind: current, repo_id, or all." },
                                "repo_id": { "type": "string", "description": "Required when kind='repo_id'. Must be registered and enabled." }
                            },
                            "required": ["kind"]
                        },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["source_id"]
                }
        }),
        json!({
                "name": "save_context_artifact",
                "description": "Index and store a large tool output or context payload. Returns a pointer (source_id) for large content, a preview for medium content, or the raw string for small content.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "content":      { "type": "string",  "description": "The content to store." },
                        "label":        { "type": "string",  "description": "Human-readable label for display and retrieval." },
                        "source_type":  { "type": "string",  "description": "Category tag (e.g. 'review_context', 'mcp_artifact'). Default: 'mcp_artifact'." },
                        "session_id":   { "type": "string",  "description": "Associate artifact with this session. Omit to use derived session." },
                        "agent_id":     { "type": "string",  "description": "Associate artifact with this agent memory partition." },
                        "repo_scope": {
                            "type": "object",
                            "description": "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Repo scope kind: current, repo_id, or all." },
                                "repo_id": { "type": "string", "description": "Required when kind='repo_id'. Must be registered and enabled." }
                            },
                            "required": ["kind"]
                        },
                        "content_type": { "type": "string",  "description": "MIME type: 'text/plain' (default), 'text/markdown', or 'application/json'." },
                        "output_format":{ "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["content", "label"]
                }
        }),
        json!({
                "name": "get_context_stats",
                "description": "Return storage statistics for the current (or specified) session: event count, saved source count, chunk count, and DB paths.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id":    { "type": "string",  "description": "Explicit session id. Omit to use the derived id for the current repo." },
                        "agent_id":      { "type": "string",  "description": "Restrict storage statistics to one agent memory partition." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "purge_saved_context",
                "description": "Delete saved artifacts. Provide session_id to delete all artifacts for that session, or omit to apply age-based cleanup (default: keep last 30 days).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "session_id":    { "type": "string",  "description": "Delete all saved artifacts for this session." },
                        "agent_id":      { "type": "string",  "description": "Restrict session deletion to one agent memory partition." },
                        "keep_days":     { "type": "integer", "description": "For age-based cleanup: keep sources newer than this many days (default 30)." },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "cross_session_search",
                "description": "CM11: Search saved context artifacts across all sessions for this repo. Returns stable object `structuredContent` with `query`, distinct `sessions`, preview `matches`, `summary`, `truncated`, and `warnings`. Use this for cross-session recall when the relevant content may have been saved in a prior session.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":         { "type": "string",  "description": "Full-text or semantic search query." },
                        "source_type":   { "type": "string",  "description": "Optional filter: restrict to a specific source_type (e.g. 'mcp_artifact')." },
                        "agent_id":      { "type": "string",  "description": "Restrict cross-session search to one agent memory partition." },
                        "merge_agent_partitions": { "type": "boolean", "description": "Intentionally merge cross-session search across all agent partitions." },
                        "limit":         { "type": "integer", "description": "Maximum results to return (default 10)." },
                        "repo_scope": {
                            "type": "object",
                            "description": "Repo scope object. Use { kind: 'current' }, { kind: 'repo_id', repo_id: '<id>' }, or { kind: 'all' }.",
                            "properties": {
                                "kind": { "type": "string", "description": "Repo scope kind: current, repo_id, or all." },
                                "repo_id": { "type": "string", "description": "Required when kind='repo_id'. Must be registered and enabled." }
                            },
                            "required": ["kind"]
                        },
                        "output_format": { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
        }),
        json!({
                "name": "get_global_memory",
                "description": "CM11: Return the cross-session global memory summary for this repo: frequently-accessed symbols and files, and recurring workflow patterns. Optionally provide focus_symbols and focus_files to also find past sessions most relevant to the current work context.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "limit":          { "type": "integer", "description": "Maximum entries to return per category (default 10)." },
                        "focus_symbols":  { "type": "array", "items": { "type": "string" }, "description": "Symbol qualified names from the current context used to find related past sessions." },
                        "focus_files":    { "type": "array", "items": { "type": "string" }, "description": "File paths from the current context used to find related past sessions." },
                        "output_format":  { "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": []
                }
        }),
        json!({
                "name": "memory_store",
                "description": "Store a memory record for this repo through the shared memory service (same fields, defaults, and validation as `atlas memory store`). Returns the persisted memory record including its id, scope, importance, and source_id.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "text":        { "type": "string",  "description": "Memory body text; stored exactly as provided." },
                        "topic":       { "type": "string",  "description": "Topic label for the memory." },
                        "title":       { "type": "string",  "description": "Optional title line." },
                        "importance":  { "type": "string",  "description": "Importance: critical, high, normal, or low (default normal)." },
                        "scope":       { "type": "string",  "description": "Visibility scope: project, session, frontend, or global (default project)." },
                        "frontend":    { "type": "string",  "description": "Frontend identifier; required when scope is frontend. Normalized to claude, codex, copilot, cli, or mcp unless config allows custom frontends." },
                        "source_id":   { "type": "string",  "description": "Source id linking this memory to a saved-context artifact." },
                        "output_format":{ "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["text"]
                }
        }),
        json!({
                "name": "memory_recall",
                "description": "Recall memories for this repo with the same lexical ranking, defaults, and visibility rules as `atlas memory recall`. Exact topic matches rank first; the viewer is the derived mcp session. Use shared=true for project + global memories only.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":        { "type": "string",  "description": "Search text matched against topic, title, and body." },
                        "topic":        { "type": "string",  "description": "Restrict recall to one topic (case-insensitive exact match)." },
                        "importance":   { "type": "string",  "description": "Restrict recall to one importance level." },
                        "scope":        { "type": "string",  "description": "Restrict recall to one scope." },
                        "shared":       { "type": "boolean", "description": "Only return memories visible to every frontend (project + global)." },
                        "limit":        { "type": "integer", "description": "Maximum memories to return (default 20)." },
                        "output_format":{ "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
        }),
    ]
}
