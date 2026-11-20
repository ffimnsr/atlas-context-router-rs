# Atlas core workflow

Atlas is a code knowledge graph for this repository: a Rust CLI plus a stdio MCP server (`atlas serve`). This guide ships inside the installed binary, so it always matches your version. Re-read it after an upgrade with `atlas skills get core`; run `atlas skills list` to see every guide this version serves.

Work order is fixed: continuity first, then graph lookup, then local reads. Continuity is not optional background context; it is the first call of every session.

## Continuity first: wake up every session

Call `wake_up` once at the start of every session, before any code question and before the first graph call. It returns a bounded recall pack and records the session-start event through the shared event service, so it also serves as the session-start capture.

- Pass `{ "topic": "<task>" }` when the first user message already names the task; the topic focuses decision and memory recall.
- Read `current_focus`, `recent_decisions`, `critical_memories`, `recent_feedback`, `active_memoir_concepts`, `changed_files`, `recent_runtime_failures`, and `graph_readiness` before asking the user to repeat context.
- Reuse `wake_up`'s `graph_readiness` when present instead of spending a separate call on `status`.
- Call `wake_up` once per session, not once per message; it writes session state, so never call it speculatively.
- When hooks are wired, the `.atlas/hooks/atlas-hook` SessionStart hook records the event and stores wake metadata, but it does not inject the pack body; still call `wake_up` to read it.
- If `wake_up` is unavailable, fall back in this order: `resume_session`, `search_decisions`, `search_saved_context`, `get_global_memory`.
- Before your final response on substantive work, close the loop with `record_session_event` `stop` or `session-end` and a handoff artifact. Hosts without hooks must follow the full fallback protocol.

Continuity layers are supporting state, not the code graph: session ledger, saved artifacts, durable memory, memoir graphs, overview memory, and history snapshots. Never write any Atlas database directly; only call Atlas MCP tools or CLI commands.

## Record, save, and feed back in every session

Continuity is not only a session-start read. Keep writing durable state while you work; a session that calls `wake_up` and records nothing leaves the next session with an empty pack.

- `record_session_event` on meaningful transitions: `user-prompt` with `payload.prompt`, `post-tool-use` with `changed_files`, `file-changed` when you know the files but not the originating tool payload, `tool-failure` when an error is resolved, and `stop` or `session-end` before the final response.
- `save_context_artifact` for anything a future session would want verbatim: `decision`, `review`, `investigation`, `handoff`, `error_resolved`, or `preference`. Reference `source_id` and symbols instead of pasting raw transcripts.
- `memory_store` for durable repo notes that outlive one session: decisions, constraints, preferences, and gotchas. Choose `scope` deliberately (`project` for repo facts, `session` for scratch) and `importance` honestly; `memory_recall` reads them back.
- Multi-step plan: save the full plan with `save_context_artifact` (`source_type: "plan"`) and a durable pointer with `memory_store` (`scope: "project"`, artifact `source_id`). Artifacts at or below 512 B return inline and are not persisted, so put short plans in `memory_store` only.
- `feedback_record` when an Atlas analysis prediction was wrong: pass `predicted`, `actual`, and `correction` so later analyses carry the correction. Use it for wrong dead-code, removal, or safety verdicts, not for user preference notes.
- `compact_session` before context compression is imminent or detected, and when a long session needs curating.
- Never store secrets, credentials, trivial logs, duplicate facts, or raw unbounded transcripts.

Load `atlas skills get core --ref continuity` for the exact triggers, artifact types, memory and memoir layers, history snapshots, and CLI mirrors.

## Atlas first

Use Atlas before `grep`, file search, broad reads, or exploratory terminal commands for code questions. Atlas is cheaper, faster, and returns structure text search cannot: symbols, callers, callees, tests, dependents, impact radius, and review context.

Priority rule: if instructions conflict, prefer Atlas for discovery and relationship lookup. Exceptions: read or edit exact files already identified, run validation commands (`cargo test --quiet`, clippy, format), or inspect non-code assets after Atlas points to them.

Use default JSON output. Trust `structuredContent` as source of truth when present.

## Atlas-first requirements

- First code-exploration step: call `status` unless graph freshness is already known from the current turn, including from the `wake_up` pack's `graph_readiness`.
- Symbol or usage question: start with `query_graph` or `resolve_symbol`.
- Caller/callee/test question: use `symbol_neighbors` for one hop; use `trace_calls` for directional multi-hop chains or a path between two symbols.
- Broader context needed: use `get_context` before `traverse_graph`.
- Last resort for relationships: use `traverse_graph` with bounded `max_depth` and `max_nodes`.
- File reads/edits are OK after Atlas identifies exact files, or when user already named exact files.
- Content search tools (`search_files`, `search_content`, `search_templates`, `search_text_assets`) are companion tools for docs, config, SQL, templates, prompts, or exact non-code evidence. Do not use them first for symbol questions.

## Minimal-first escalation order

Every task starts and ends the same way:
0. `wake_up` once at session start.
1. Record substantive work as it happens (`record_session_event`, `save_context_artifact`, `memory_store`, `feedback_record`) and close with `stop`.

Review/change tasks:
1. `detect_changes` when changed files are unknown.
2. `get_minimal_context` for first bounded triage.
3. `get_review_context` when changed-symbol, neighbor, or risk detail is needed.
4. `explain_change` when deterministic risk/test-gap explanation is needed.
5. `get_impact_radius` when explicit blast radius is needed.

Symbol/usage tasks:
1. `query_graph` or `resolve_symbol`.
2. `symbol_neighbors` for direct callers, callees, and tests.
3. `get_context` for bounded ranked context.
4. `trace_calls` when you need a directional call chain or a path to a target.
5. `traverse_graph` only when unfiltered multi-hop reachability is needed.

Allowed reasons to escalate:
- ambiguous symbol resolution
- truncated result
- missing caller/callee/test evidence
- cross-file or cross-package risk
- explicit user request for broader context
- safety-critical uncertainty

Anti-patterns:
- starting with `grep`, file search, or broad reads for structural code questions
- skipping `wake_up` because the task looks small or self-contained
- finishing a substantive session without recording events, artifacts, or durable notes
- using `query_graph` as caller/callee search instead of following with relationship tools
- using `traverse_graph` before symbol resolution
- broad traversal without bounded `max_depth` and `max_nodes`
- using full review context when minimal context is enough
- skipping Atlas because a terminal command feels faster

## Workflow patterns

Use the shortest chain that answers the question, then stop. Load `--ref workflows` for expanded recipes.

| Task | Chain |
| --- | --- |
| Multi-step plan | `wake_up` with topic → recon (`query_graph`, `get_context`) → `save_context_artifact` full plan → `memory_store` project pointer → `record_session_event` `stop` |
| Unfamiliar area | `query_graph` or `resolve_symbol` → `get_context` → `cross_file_links` or `concept_clusters` → `symbol_neighbors` |
| Change review | `detect_changes` → `get_minimal_context` → `get_review_context` → `explain_change` → `get_impact_radius` |
| Refactor or remove safely | `assess_risk` + `analyze_safety` → `analyze_dependency` or `analyze_remove` → `symbol_neighbors` tests → dry-run refactor → tests |
| Duplicate or complex code | `find_duplicates` or `find_similar_functions` → `find_complex_functions` or `find_large_functions` → `analyze_metrics` |
| Architecture map | `analyze_architecture` → `infer_modules` → `label_components` → `analyze_patterns` |
| Unused code | `analyze_dead_code` → `analyze_dependency` or `analyze_remove` → `feedback_record` when the verdict is wrong |
| Non-code evidence | `search_files` or `search_content` → `read_file_excerpt`, `get_docs_section`, or `read_file_around_match` → `search_templates` or `search_text_assets` |
| Durable decision, preference, or gotcha | `memory_store` (`scope: "project"`) → `memory_recall` or `search_decisions` later; `feedback_record` for wrong Atlas analysis |
| Concept or design map | `memoir_create` → `memoir_add_concept` → `memoir_link` → `memoir_inspect` |
| Code-area overview | `overview_search` or `overview_show` → `overview_export` → `overview_ingest`; `overview_refresh_plan` after changes |
| Prior-session recall | `cross_session_search` or `search_saved_context` → `read_saved_context`; `get_global_memory` for repo-wide patterns |
| Stale or unhealthy graph | `status`, `doctor`, or `db_check` → `update_graph` working tree → `build_graph`; `postprocess_graph` for derived analytics; `embed_chunks` only when embeddings are configured |
| MCP or multi-repo diagnostics | `broker_status` or `get_metrics` for broker liveness and counters; `repo_registry` for repo trust; `list_graph_stats` or `debug_graph` for graph shape |
| Long session or storage pressure | `get_context_stats` → `compact_session` → `purge_saved_context` for old or session-scoped artifacts |

Use `batch_query_graph` for several symbol lookups in one call, `list_symbols` for deterministic paginated inventory, and `explain_query` when `query_graph` returns empty or noisy matches.

## Plan and handoff

- Start from `wake_up` with the task topic; read `recent_decisions`, `critical_memories`, `changed_files`, and `graph_readiness` before replanning.
- Recon before writing steps: `query_graph` or `resolve_symbol`, `get_context`, and `cross_file_links` or `concept_clusters` for area boundaries.
- Persist the full plan with `save_context_artifact` (`source_type: "plan"`, `content_type: "text/markdown"`).
- Persist a durable pointer with `memory_store`: `scope: "project"`, task `topic`, the artifact `source_id`, and honest `importance`.
- Artifacts live under `.atlas/artifacts/` and are gitignored; project memories live under `.atlas/memories/` and are committed. Use artifacts for verbatim plan bodies, memories for the cross-session pointer and summary.
- Without an explicit `session_id`, artifacts attach to the derived MCP session. Recall them from another session with `cross_session_search` or through the memory pointer.
- After the plan changes, update the memory pointer. Before closing, `record_session_event` `stop` (or `session-end`) and `compact_session` when the session is long.
- Next session: `wake_up` → `memory_recall` → `read_saved_context` with the stored `source_id`.

## Tool discovery

Runtime docs are canonical for current arguments and output contracts. Do not treat this instruction block as exhaustive inventory.

- `tool_list`: list visible Atlas MCP tools
- `tool_search`: find tools by name or capability
- `tool_help`: read exact runtime docs for one tool, including input contracts and minimal examples
- `man`: legacy namespace-aware manual alias

## Common tool map

- continuity: `wake_up`, `get_session_status`, `resume_session`, `save_context_artifact`, `record_session_event`, `compact_session`, `search_saved_context`, `search_decisions`
- shared memory and notes: `memory_store`, `memory_recall`
- analysis feedback: `feedback_record`
- symbol lookup: `query_graph`, `resolve_symbol`
- relationships: `symbol_neighbors`, `trace_calls`, `traverse_graph`, `get_context`
- review/change context: `detect_changes`, `get_minimal_context`, `get_review_context`
- impact/risk: `get_impact_radius`, `explain_change`, `assess_risk`, `analyze_safety`
- graph build: `build_graph` with `{}` for missing/corrupt graph DB or full reindex; `update_graph` with direct `change_source` for incremental refresh
- repo health: `status`, `doctor`, `db_check`, `debug_graph`, `list_graph_stats`
- companion non-code lookup: `search_files`, `search_content`, `read_file_excerpt`, `get_docs_section`, `read_file_around_match`, `search_templates`, `search_text_assets`

## Trust metadata

- Every MCP tool response includes `atlas_provenance` with `repo_root`, `db_path`, `indexed_file_count`, and `last_indexed_at`.
- Some graph-backed tools also include `atlas_freshness` when working-tree edits may make graph results stale.
- If `atlas_provenance.repo_root` does not match current workspace, stop and verify session wiring.
- If `atlas_provenance.db_path` points at unexpected database, stop and call `status` or `doctor` before trusting results.
- If graph DB is missing, corrupt, or requires a full reindex, call `build_graph` with `{}`.
- If `atlas_freshness.stale` is true after edits, call `update_graph` with `{ "change_source": { "kind": "working_tree" } }` before making claims about current code.
- Treat missing optional `atlas_*` fields as "not applicable", not as tool failure.

## Path identity invariant

- Canonicalize repo paths before hashing, persistence, dedupe, cache keys, source IDs, chunk IDs, snapshot file references, or qualified-name prefixes.
- Apply same rule to future sidecar/cache/index keys, including in-memory parser caches and retrieval sidecars keyed by repo files.
- Use `atlas_repo::CanonicalRepoPath` or helper APIs built on it. Do not add local path-normalization helpers for path-derived identity.
- If `doctor` or `db_check` reports `noncanonical_path_rows`, prefer rebuild from clean canonical inputs instead of in-place row rewrites.

## CLI fallback

Every MCP tool has a CLI mirror, so the same workflow works when MCP is not wired up: `atlas wake-up`, `atlas session`, `atlas memory`, `atlas feedback`, `atlas status`, `atlas query <text>`, `atlas symbols`, `atlas trace`, `atlas context`, `atlas impact`, `atlas review-context`, `atlas build`, `atlas update`, `atlas serve`. Run `atlas --help` for the full list.

## Load more when you need it

This guide covers the session start, the loop, the tool map, escalation, and trust checks. Load one reference with `--ref`:

```bash
atlas skills get core --ref continuity
atlas skills get core --ref cli
atlas skills get core --ref mcp-tools
```

`atlas skills get core --full` prints this guide plus every reference at once, several times the size.

| Load | When you need |
| --- | --- |
| `continuity` | Session event protocol, saved artifacts, decisions, memories, memoir, overview, history, atlas:// URIs |
| `cli` | Any subcommand, argument, or flag not shown here; full CLI surface generated from the installed binary |
| `mcp-tools` | Full MCP tool inventory with input contracts, generated from the server |
| `workflows` | Review a change-set, inspect a symbol, removal and rename safety, repo-wide analysis reports |
| `setup` | Install and init modes, skill commands, build/update, MCP server setup, health and config commands |
