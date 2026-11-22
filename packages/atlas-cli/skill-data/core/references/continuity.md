# Session, memory, and history state

Atlas keeps continuity truth as Markdown files under `.atlas/` with derived SQLite indexes. Never write any Atlas database directly; only call Atlas MCP tools or CLI commands.

## Session memory fallback for hosts without hooks

When your coding tool is wired to the generic `.atlas/hooks/atlas-hook` runner, Atlas captures session events automatically. When this client does not support Atlas hooks (or hook availability is unknown), use Atlas MCP tools as the fallback lifecycle protocol.

### Recall before work

- At session start, call `wake_up` when available; otherwise `resume_session` before substantive work.
- Call `search_decisions` for prior conclusions and `search_saved_context` for prior artifacts relevant to the task.
- Call `memory_recall` for durable project notes and plan pointers, then `read_saved_context` with the stored `source_id` when the full body is needed.
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
12. Multi-step plan created or revised → save the full plan via `save_context_artifact` with `source_type: "plan"` and a durable pointer via `memory_store` with `scope: "project"` and the artifact `source_id`; recall it next session with `memory_recall` then `read_saved_context`.
13. Upgrade, dependency bump, feature, or behavior fix completed → save a `decision` (or `handoff`) artifact with what changed, why, and how it was verified, then store a `memory_store` `scope: "project"` pointer row with recall tags (e.g. `upgrade`) and the artifact `source_id`, so any later session can trace back the change and its purpose.

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
| Artifact | Full plan bodies, reviews, investigations, error fixes, and handoffs | `atlas session save-artifact` |
| Memory | Durable repo facts: decisions, preferences, gotchas, and plan pointers | `atlas memory`, `atlas feedback` |
| Memoir | Concept graphs with typed relations, separate from the code graph | `atlas memoir` |
| Overview | Evidence-linked code overview memory and external analysis handoff | `atlas overview` |
| History | Graph snapshots per git commit and churn trends | `atlas history` |

Write and update by layer:

- Artifact: use for full plan bodies, reviews, investigations, error fixes, and handoffs that a future session should read verbatim. Artifacts are committed under `.atlas/artifacts/` like project memories under `.atlas/memories/`, so they survive sessions and machines without a pointer row. A changed body creates a new `source_id`; update the memory pointer that references it and keep the old artifact only when its history matters.
- Memory: use for durable facts, constraints, preferences, gotchas, and plan pointers. Project memories are committed under `.atlas/memories/`; session-scoped memories are scratch and frontend-scoped memories belong to one host only. Add searchable `tags`, link evidence with `source_id`, and connect related rows with `links` or `memory_link`; the `supersedes` relation marks the target row as replaced so default recall stops surfacing it. Update an existing row with `patch_id` plus `patch_text` or `patch_file`, then verify with `memory_recall`; do not add a duplicate row for a changed fact.
- Memoir: use for design, dependency, decision, and incident graphs that the code graph cannot express. Memoir is committed under `.atlas/memoir/` and never merges into graph nodes or edges. Concept names are unique per graph, so revise with a versioned concept linked by `replaces` or `refines`; do not re-add the same name or silently reverse relation direction.
- Overview: use for evidence-linked code-area documentation that an external analyzer or human produces from bounded packets. Overviews are committed under `.atlas/overview/`, and a newer ingest supersedes the prior active record instead of editing it in place. Run `overview_refresh_plan` after code changes and never present stale, possibly stale, or orphaned records as fresh.
- Decision memory: use `search_decisions` for conclusions captured from session events and surfaced by `wake_up`. It is a read surface, not a replacement for durable notes; for decisions that must survive, also store a `decision` artifact and a project memory.

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

Resource URIs are read-only addresses for cross-tool and cross-session references. Use them instead of pasting the same body into multiple tools.

| URI | Use |
| --- | --- |
| `atlas://repo/{repo_id}/tree/{dir}`, `file/{path}`, `symbol/{qualified_name}` | Graph-scoped tree, file, and symbol reads |
| `atlas://docs/index`, `atlas://docs/{file}#{heading}` | Documentation index and one Markdown section |
| `atlas://graph/provenance`, `atlas://health/status` | Trust and readiness metadata for the current graph |
| `atlas://memories/topic/{topic}`, `atlas://memories/id/{memory_id}` | One durable memory by topic or exact id |
| `atlas://memoir/{graph}/{concept}` | One memoir concept and its bounded neighbourhood |
| `atlas://artifacts/{source_id}/{chunk_index}`, `atlas://saved-context/{source_id}` | Saved-context chunk or full artifact by `source_id` |
| `atlas://tool-docs/{name}`, `atlas://prompt-docs/{name}` | Runtime tool manual and prompt-facing docs |

Use `atlas uri` to read, list, grep, or search these addresses. Do not invent a new URI form when a canonical one exists.
