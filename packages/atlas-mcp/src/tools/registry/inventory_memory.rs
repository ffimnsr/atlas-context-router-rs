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
                "description": "Bounded session-start recall for hookless agents. Assembles a compact context pack from the resume snapshot, decision memory, saved-context hints, global memory, changed files, bounded classified runtime failures, and graph readiness, then records the session-start event through the shared event service shared with native hooks. Large saved artifacts are referenced by source_id only, never inlined.",
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
                "description": "Recall memories for this repo with the same lexical ranking, defaults, and visibility rules as `atlas memory recall`. Exact topic matches rank first; the viewer is the derived mcp session. Use shared=true for project + global memories only, and hybrid=true to fuse opt-in memory embeddings with the lexical path.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":        { "type": "string",  "description": "Search text matched against topic, title, and body." },
                        "topic":        { "type": "string",  "description": "Restrict recall to one topic (case-insensitive exact match)." },
                        "importance":   { "type": "string",  "description": "Restrict recall to one importance level." },
                        "scope":        { "type": "string",  "description": "Restrict recall to one scope." },
                        "shared":       { "type": "boolean", "description": "Only return memories visible to every frontend (project + global)." },
                        "hybrid":       { "type": "boolean", "description": "Fuse optional memory embeddings with lexical recall (requires memory.embedding.enabled). Lexical hits always outrank vector-only matches." },
                        "limit":        { "type": "integer", "description": "Maximum memories to return (default 20)." },
                        "output_format":{ "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
        }),
        json!({
                "name": "memory_ops",
                "description": "Return the bounded typed-operator history for one memory row (VM1). Read-only parity surface for `atlas memory ops`; unknown ids return found=false instead of an error.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "memory_id":     { "type": "string", "description": "Exact memory id to inspect." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["memory_id"]
                }
        }),
        json!({
                "name": "feedback_record",
                "description": "Record a correction when an analysis prediction was wrong (same fields, defaults, and validation as `atlas feedback record`). Stored as first-class deterministic correction memory; records where actual differs from predicted can lower matching analysis confidence (ICM-C3).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "predicted":    { "type": "string",  "description": "What the analysis predicted; required." },
                        "actual":       { "type": "string",  "description": "What actually happened; required. Differs from predicted to mark the record as false-positive evidence." },
                        "correction":   { "type": "string",  "description": "Correction text explaining the right answer." },
                        "tool":         { "type": "string",  "description": "Tool that produced the prediction (default mcp)." },
                        "analysis_kind":{ "type": "string",  "description": "Analysis kind: dead_code, remove, safety, or remove_dead." },
                        "symbol":       { "type": "string",  "description": "Exact qualified name of the symbol the prediction was about." },
                        "file":         { "type": "string",  "description": "Repo-relative path of the file the prediction was about." },
                        "source_id":    { "type": "string",  "description": "Source id linking this record to a saved-context artifact." },
                        "output_format":{ "type": "string",  "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["predicted", "actual"]
                }
        }),
        json!({
                "name": "memoir_create",
                "description": "Create a memoir concept graph in the continuity database. Memoir graphs are a separate knowledge layer: they never merge into code graph nodes or edges. Duplicate graph names fail deterministically within the same repo and scope (ICM-F).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "name":          { "type": "string", "description": "Graph name; unique per repo and scope (case-insensitive)." },
                        "description":   { "type": "string", "description": "Optional graph description." },
                        "scope":         { "type": "string", "description": "Visibility scope: project, session, frontend, or global (default project)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["name"]
                }
        }),
        json!({
                "name": "memoir_add_concept",
                "description": "Add a concept to an existing memoir graph. Missing graphs are never auto-created. Concept names are unique per graph (case-insensitive) (ICM-F).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "graph":         { "type": "string", "description": "Graph name." },
                        "name":          { "type": "string", "description": "Concept name; unique inside the graph (case-insensitive)." },
                        "description":   { "type": "string", "description": "Concept description or summary." },
                        "kind":          { "type": "string", "description": "Concept kind label (default concept)." },
                        "source_id":     { "type": "string", "description": "Source id linking this concept to saved-context evidence." },
                        "scope":         { "type": "string", "description": "Graph scope: project, session, frontend, or global (default project)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["graph", "name", "description"]
                }
        }),
        json!({
                "name": "memoir_link",
                "description": "Link two memoir concepts with a controlled relation type. Unknown relation types are rejected; aliases replaced_by (direction reversed) and separate_from (tagged related_to) are normalized. Missing endpoints require the explicit create_missing opt-in (ICM-F).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "graph":         { "type": "string", "description": "Graph name." },
                        "source":        { "type": "string", "description": "Source concept name." },
                        "target":        { "type": "string", "description": "Target concept name." },
                        "relation":      { "type": "string", "description": "Relation type: depends_on, part_of, contradicts, refines, replaces, caused_by, fixed_by, blocked_by, decided_by, or related_to. Aliases replaced_by and separate_from are normalized." },
                        "confidence":    { "type": "number", "description": "Relation confidence between 0.0 and 1.0 (default 1.0)." },
                        "source_id":     { "type": "string", "description": "Source id linking this relation to saved-context evidence." },
                        "scope":         { "type": "string", "description": "Graph scope: project, session, frontend, or global (default project)." },
                        "create_missing":{ "type": "boolean", "description": "Explicitly opt in to creating missing concept endpoints (default false)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["graph", "source", "target", "relation"]
                }
        }),
        json!({
                "name": "memoir_inspect",
                "description": "Inspect a concept's bounded memoir neighbourhood. Every edge includes canonical relation direction (outgoing/incoming) and source evidence ids. Traversal depth is clamped to 1..=5 and node/edge counts are capped (ICM-F).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "concept":       { "type": "string", "description": "Concept name to inspect." },
                        "graph":         { "type": "string", "description": "Graph name; omit to resolve across graphs (ambiguous names fail)." },
                        "depth":         { "type": "integer", "description": "Traversal depth, clamped to 1..=5 (default 1)." },
                        "relation":      { "type": "string", "description": "Restrict traversal to one relation type." },
                        "scope":         { "type": "string", "description": "Graph scope used when graph is provided (default project)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["concept"]
                }
        }),
        json!({
                "name": "memoir_delete",
                "description": "Delete a memoir concept graph and all of its concepts and relations. Deletion is scoped to the repo and graph scope; a missing graph reports found=false. Use dry_run=true to inspect counts without writing (ICM-F).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "graph":         { "type": "string", "description": "Graph name." },
                        "scope":         { "type": "string", "description": "Graph scope: project, session, frontend, or global (default project)." },
                        "dry_run":       { "type": "boolean", "description": "Report counts without deleting (default false)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["graph"]
                }
        }),
        json!({
                "name": "experience_inspect",
                "description": "Inspect query-conditioned experience edges touching one atlas:// URI. Report-only: edges live in session.db and are never code-graph truth (VR4). The ledger stays empty until a retrieval/update workflow records edges; an empty `edges` list is a valid report, not an error.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "uri":           { "type": "string", "description": "Canonical atlas:// source or target URI." },
                        "direction":     { "type": "string", "description": "both, outgoing, or incoming (default both)." },
                        "limit":         { "type": "integer", "description": "Maximum edges (default 20, max 100)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["uri"]
                }
        }),
        json!({
                "name": "overview_export",
                "description": "Export bounded overview analysis packets for external LLM or human analysis. Atlas never calls a model: packets include canonical subject identity, commit SHA, graph freshness, bounded evidence, and budget metadata. Same request fields and defaults as `atlas overview export` (ICM-G).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "scope":              { "type": "string", "description": "project, packages, modules, files, symbols, functions, methods, or changed (default project)." },
                        "subjects":           { "type": "array", "items": { "type": "string" }, "description": "Specific subjects: qualified name, file path, package, or module." },
                        "subject":            { "type": "string", "description": "Single subject shorthand." },
                        "changed":            { "type": "boolean", "description": "Export subjects affected by changed files only." },
                        "changed_only":       { "type": "boolean", "description": "Alias for changed." },
                        "since":              { "type": "string", "description": "Git ref to diff against for changed export." },
                        "limit":              { "type": "integer", "description": "Maximum packets (default 50, max 200)." },
                        "include_code_spans": { "type": "boolean", "description": "Include bounded source excerpts." },
                        "include_callers":    { "type": "boolean", "description": "Include caller qualified names." },
                        "include_callees":    { "type": "boolean", "description": "Include callee qualified names." },
                        "include_tests":      { "type": "boolean", "description": "Include test-adjacency qualified names." },
                        "max_tokens":         { "type": "integer", "description": "Approximate token budget per packet." },
                        "output_format":      { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    }
                }
        }),
        json!({
                "name": "overview_ingest",
                "description": "Ingest externally produced overview analysis JSON with the same validation as `atlas overview ingest`. Packet ids, canonical subject identity, commit SHA, line spans, redaction, and large-body routing are validated before persistence (ICM-G).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "analysis":      { "type": "object", "description": "Overview analysis object matching the documented ingest schema." },
                        "path":          { "type": "string", "description": "Path to analysis JSON when analysis object is omitted." },
                        "input":         { "type": "string", "description": "Alias for path." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    }
                }
        }),
        json!({
                "name": "overview_search",
                "description": "Search overview memory with FTS plus explicit ranking (exact subject/title/tag, freshness, confidence, recency, subject specificity). Same filters and ranking evidence as `atlas overview search` (ICM-G).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "query":        { "type": "string", "description": "Search query." },
                        "subject_kind": { "type": "string", "description": "Restrict to one subject kind." },
                        "freshness":    { "type": "string", "description": "Restrict to one freshness state." },
                        "package":      { "type": "string", "description": "Restrict to one package name." },
                        "module":       { "type": "string", "description": "Restrict to one module path." },
                        "file":         { "type": "string", "description": "Restrict to one canonical file path." },
                        "source_kind":  { "type": "string", "description": "external_llm, manual, or imported." },
                        "limit":        { "type": "integer", "description": "Maximum results (default 20)." },
                        "level":        { "type": "string", "description": "Optional disclosure level: l0, l1, or l2. Adds a rendered view per hit." },
                        "output_format":{ "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    },
                    "required": ["query"]
                }
        }),
        json!({
                "name": "overview_show",
                "description": "Show one overview record by subject id, qualified name, canonical file path, package, or module. Ambiguous matches fail with candidate list, matching `atlas overview show` (ICM-G).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "subject":       { "type": "string", "description": "Subject id, qualified name, canonical path, package, or module." },
                        "target":        { "type": "string", "description": "Alias for subject." },
                        "level":         { "type": "string", "description": "Optional disclosure level: l0, l1, or l2. Adds a rendered view." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    }
                }
        }),
        json!({
                "name": "overview_refresh_plan",
                "description": "Plan overview freshness refresh after changes. Read-only unless apply=true. Reports directly stale, possibly stale, and orphaned subjects with suggested export commands, matching `atlas overview refresh-plan` (ICM-G).",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "since":         { "type": "string", "description": "Git ref to diff against (default working tree)." },
                        "apply":         { "type": "boolean", "description": "Apply freshness marking (default false)." },
                        "output_format": { "type": "string", "description": DEFAULT_OUTPUT_DESCRIPTION }
                    }
                }
        }),
    ]
}
