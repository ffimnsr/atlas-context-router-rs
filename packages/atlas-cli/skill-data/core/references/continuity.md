# Session, memory, and history state

Atlas keeps session continuity in per-repo SQLite stores under `.atlas/`. Never write any Atlas database directly; only call Atlas MCP tools or CLI commands.

## Session memory fallback for hosts without hooks

When your coding tool is wired to the generic `.atlas/hooks/atlas-hook` runner, Atlas captures session events automatically. When this client does not support Atlas hooks (or hook availability is unknown), use Atlas MCP tools as the fallback lifecycle protocol.

### Recall before work

- At session start, call `wake_up` when available; otherwise `resume_session` before substantive work.
- Call `search_decisions` for prior conclusions and `search_saved_context` for prior artifacts relevant to the task.
- Call `get_global_memory` when you need repo-level cross-session context.

CLI mirrors:

```bash
atlas wake-up --topic "payment refactor"
atlas session resume
atlas session decisions "retry policy"
atlas session search "graph build"
atlas memory global
```

### Mandatory capture triggers

Call `record_session_event` when any of these happens:

1. User gives a substantial task → `record_session_event` with `event: "user-prompt"` and `payload.prompt`.
2. A tool call creates, edits, deletes, renames, or generates files → `record_session_event` with `event: "post-tool-use"` and payload containing `tool_name` and `changed_files` (repo-relative paths).
3. You know exact changed files but not the originating tool payload → `record_session_event` with `event: "file-changed"`.
4. Error resolved → `record_session_event` with `event: "tool-failure"` (or `error`) AND save the fix via `save_context_artifact` with `source_type: "error_resolved"`.
5. Architecture/design decision made → save via `save_context_artifact` with `source_type: "decision"`, linking symbols/files/source ids when possible.
6. User preference discovered → save via `save_context_artifact` with `source_type: "preference"`.
7. Major investigation, review, or handoff summary produced → save via `save_context_artifact` with `source_type: "review"` / `"investigation"` / `"handoff"`.
8. Context compression is imminent or detected → call `compact_session` and `record_session_event` with `event: "pre-compact"` first.
9. Durable repo fact, constraint, preference, or gotcha that should outlive the session → `memory_store` with an explicit `scope` (`project` or `session`) and honest `importance`.
10. Atlas analysis prediction wrong (dead code, removal, safety verdict) → `feedback_record` with `predicted`, `actual`, and `correction`.
11. Major work completed before your final response → `record_session_event` with `event: "stop"` (or `event: "session-end"`) and save a concise handoff artifact.

Do NOT store: trivial logs, raw secrets or credentials, duplicate facts, raw unbounded transcripts, or facts already present in repository instruction files. Prefer compact summaries and `source_id` references.

### Store and curate

```bash
atlas session start
atlas session status
atlas session save-artifact
atlas session compact
atlas session stats
```

Run `atlas session <command> --help` for flags; the skill `cli` reference has every one.

## Pick the right knowledge layer

| Layer | Use it for | Commands |
| --- | --- | --- |
| Session | Event ledger, resume snapshot, saved artifacts for one session | `atlas session` |
| Memory | Durable repo facts: decisions, preferences, gotchas | `atlas memory`, `atlas feedback` |
| Memoir | Concept graphs with typed relations, separate from the code graph | `atlas memoir` |
| Overview | Evidence-linked code overview memory and external analysis handoff | `atlas overview` |
| History | Graph snapshots per git commit and churn trends | `atlas history` |

Common commands:

```bash
atlas memory store
atlas memory recall "token expiry"
atlas memory list
atlas memory health
atlas memoir create
atlas memoir add-concept
atlas memoir link
atlas memoir inspect
atlas overview export
atlas overview search
atlas overview refresh-plan
atlas history status
atlas history diff
atlas history churn
```

Subcommands that need arguments are shown without them; run `atlas <group> <command> --help` for the exact required flags.

## Canonical URIs

Use `atlas uri` when an address must cross tools or sessions:

```bash
atlas uri atlas://repo/{repo_id}/file/src/lib.rs
atlas uri grep atlas://repo/{repo_id}/tree/src "handle_request"
atlas uri search atlas://repo/{repo_id}/tree/src "retry policy"
```

Use the canonical forms only; bare `atlas://file/<path>` is not part of the grammar.
