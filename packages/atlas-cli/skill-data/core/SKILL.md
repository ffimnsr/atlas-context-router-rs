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

- `record_session_event` is the session ledger. Call it with `user-prompt` when a substantial task starts, `post-tool-use` or `file-changed` after files change, `tool-failure` when an error is resolved, `pre-compact` before compression, and `stop` or `session-end` before the final response. Include `payload.prompt`, `payload.tool_name`, and repo-relative `changed_files` when available so the next session can reconstruct what happened without rereading the transcript.
- `save_context_artifact` is the verbatim evidence layer. Use it for a full plan, review, investigation, handoff, error fix, or preference summary that a future session should not have to regenerate. Set a stable `label`, a specific `source_type` (`plan`, `review`, `investigation`, `handoff`, `error_resolved`, `decision`, `preference`), and `content_type`; keep the returned `source_id` and reference symbols or files instead of pasting raw transcripts. Artifacts at or below 512 B return inline and are not persisted, so store short content in `memory_store` instead. When the body changes, save a new artifact and re-point the memory row; do not leave the old `source_id` as current.
- `memory_store` is the durable fact and pointer layer. Use it for decisions, constraints, preferences, gotchas, and `source_id` pointers that must survive the session. Choose `scope: "project"` for repo facts, `session` for scratch, and `frontend` only for host-specific behavior; set `topic`, honest `importance`, evidence `source_id`, and searchable `tags`. Use `links` for related memory ids or `memory_link` for one directed edge after the fact; relations accept the memoir vocabulary plus `supersedes`, which marks the target row as replaced. Update a row with `patch_id` plus `patch_text` or `patch_file` instead of creating a duplicate, and verify the result with `memory_recall`.
- `memoir_*` is the design knowledge layer. Use `memoir_create` for a bounded topic graph, `memoir_add_concept` for components, decisions, risks, plan steps, or incidents, and `memoir_link` for directed relations such as `depends_on`, `part_of`, `replaces`, `caused_by`, and `fixed_by`. Re-adding a concept name fails, so revise by adding a versioned concept and linking it with `replaces` or `refines`. Inspect with `memoir_inspect` before and after changes, and delete only after a dry run.
- `overview_*` is the code documentation layer. Export bounded packets with `overview_export`, have an external analyzer or human return the documented `schema_version: 1` JSON, and persist it with `overview_ingest`. After code changes, run `overview_refresh_plan` and apply only after reviewing stale, possibly stale, and orphaned subjects. Read with `overview_search` or `overview_show`, starting at `l1` and escalating to `l2` only when the full body is needed.
- `feedback_record` corrects future analysis. Pass the wrong `predicted` verdict, the real `actual` outcome, and a `correction` whenever dead-code, removal, or safety analysis was wrong. Search past corrections with `atlas feedback search` before repeating a questionable verdict; do not use feedback for user preference notes.
- `compact_session` curates a long ledger before context compression. Record `pre-compact` first, then compact; inspect size with `get_context_stats` and remove old or session-scoped artifacts with `purge_saved_context` when storage pressure is real.
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

Use the shortest chain that answers the question, then stop. Each pattern below says when to start, what to call, and what to update before you finish. Load `--ref workflows` for expanded recipes.

### Planning and continuity

- **Multi-step plan.** Start with `wake_up` and the task topic, then read `recent_decisions`, `critical_memories`, `changed_files`, and `graph_readiness`. Reconfirm code facts with `query_graph` or `resolve_symbol`, `get_context`, and `cross_file_links` or `concept_clusters` before writing steps. Save the full plan and a project memory pointer as described in `## Plan and handoff`, then execute one step at a time and `update_graph` after edits. If the plan changes, save a new artifact and re-point the memory row; do not leave the old `source_id` as current.
- **Durable decision, preference, or gotcha.** Record the fact with `memory_store` at `scope: "project"`, link the artifact or symbol with `source_id`, and choose `importance` honestly. Later sessions read it with `memory_recall` and `search_decisions` for hook-captured conclusions; update the same row with `patch_id` plus a patch body instead of adding a duplicate. When one memory replaces another, link the newer row with `memory_link` using `supersedes` so default recall stops surfacing the stale one. Use `save_context_artifact` when the rationale or evidence must be preserved verbatim.
- **Concept or design map.** Create one bounded memoir graph for the topic, add concepts with stable `kind` values and evidence `source_id`, then link them with the controlled relation vocabulary. Inspect the neighbourhood before adding more so you do not duplicate concepts or reverse relation direction. Revise with versioned concepts plus `replaces` or `refines`, and use `create_missing: true` only when the new endpoints are intentional.
- **Code-area overview.** Export packets for the target subjects with callers, callees, tests, and code spans only when they fit the token budget. Have an external analyzer or human produce the documented ingest JSON, then persist it with `overview_ingest`. Search or show the record before relying on it, and run `overview_refresh_plan` after related changes so stale or orphaned subjects are not presented as fresh.
- **Prior-session recall.** Search the current session with `search_saved_context`, search all sessions with `cross_session_search`, and fetch the full body with `read_saved_context` by `source_id`. Use `get_global_memory` for frequently accessed symbols, files, and workflow patterns. If the result is a plan pointer, follow the `source_id` instead of reconstructing the plan from chat history.
- **Long session or storage pressure.** Check `get_context_stats`, record `pre-compact`, and `compact_session` before compression. Purge only old or session-scoped artifacts with `purge_saved_context`; keep project memories and committed artifact-backed decisions intact.

### Discovery and review

- **Unfamiliar area.** Start with `query_graph` or `resolve_symbol` to land on canonical names, then use `get_context` for bounded ranked context. Add `cross_file_links` or `concept_clusters` to find boundary files and co-dependent groups, and `symbol_neighbors` for direct callers, callees, and tests. If the graph is stale after edits, call `update_graph` before drawing conclusions.
- **Change review.** Start with `detect_changes` when the changed files are unknown, then `get_minimal_context` for first triage. Escalate to `get_review_context` for changed symbols, neighbors, and risk detail, `explain_change` for deterministic risk and test gaps, and `get_impact_radius` for explicit blast radius. Record the reviewed files with `record_session_event` and save a `review` artifact when the conclusion should survive the session.
- **Non-code evidence.** Use `search_files` or `search_content` after graph tools identify the relevant area, then read exact evidence with `read_file_excerpt`, `get_docs_section`, or `read_file_around_match`. Narrow templates with `search_templates` and SQL, config, environment, or prompt files with `search_text_assets`. These tools complement the graph; they do not replace symbol and relationship lookup.
- **Bulk symbol lookup or inventory.** Use `batch_query_graph` for several short symbol-name searches in one round trip, and `list_symbols` when you need deterministic paginated enumeration rather than ranked text search. Use `explain_query` when `query_graph` returns empty or noisy matches so you can inspect tokenization and regex behavior before retrying.

### Refactor and analysis

- **Refactor or remove safely.** Score the symbol with `assess_risk` and `analyze_safety`, then confirm removal with `analyze_dependency` or `analyze_remove`. Check `symbol_neighbors` for call sites and tests, keep refactors in dry-run until the plan is reviewed, and run the project tests after applying. If a verdict was wrong, correct it with `feedback_record`.
- **Unused code.** Run `analyze_dead_code` for candidates, then confirm each candidate with `analyze_dependency` or `analyze_remove` before deleting anything. Treat dead-code output as a candidate list, not proof of runtime disuse. Record false positives with `feedback_record` so later confidence is adjusted.
- **Duplicate or complex code.** Use `find_duplicates` or `find_similar_functions` before extracting shared code, and `find_complex_functions` or `find_large_functions` before splitting a function. Confirm the hotspots with `analyze_metrics`, then review callers and tests before editing. These reports are deterministic and do not call a model, so cite the specific finding in the plan.
- **Architecture map.** Use `analyze_architecture` for cycles, layer violations, and coupling hotspots, then `infer_modules` and `label_components` to learn package and component boundaries. `analyze_patterns` adds repeated call chains, hubs, bottlenecks, and deep paths, while `concept_clusters` groups co-dependent files around seed paths. Persist durable conclusions in memoir or project memory rather than only in the chat.

### Health and maintenance

- **Stale or unhealthy graph.** Check `status`; use `doctor` or `db_check` for integrity and `list_graph_stats` or `debug_graph` for graph shape. Refresh with `update_graph` from the working tree or base ref, and use `build_graph` for a missing or corrupt database. Run `postprocess_graph` when flows, communities, architecture metrics, query hints, or large-function summaries are stale, and `embed_chunks` with a dry run first only when an embedding backend is configured.
- **MCP or multi-repo diagnostics.** Use `broker_status` or `get_metrics` to separate broker liveness from graph health, and `repo_registry` to inspect repo identity and trust states in multi-repo graphs. Use `tool_list`, `tool_search`, and `tool_help` when a tool name, argument, or output contract is uncertain. Report the exact failing tool and retry guidance instead of guessing.

## Plan and handoff

- Start from `wake_up` with the task topic; read `recent_decisions`, `critical_memories`, `changed_files`, and `graph_readiness` before replanning. If `wake_up` is unavailable, fall back to `resume_session` and `search_decisions`.
- Recon before writing steps with `query_graph` or `resolve_symbol`, `get_context`, and `cross_file_links` or `concept_clusters` for area boundaries. Do not write steps against symbols you have not resolved or files you have not located.
- Persist the full plan with `save_context_artifact` (`source_type: "plan"`, `content_type: "text/markdown"`) using a stable label. Keep the returned `source_id` and save the rationale, open questions, and verification steps, not only the task list.
- Persist a durable pointer with `memory_store`: `scope: "project"`, task `topic`, the artifact `source_id`, and honest `importance`. Keep the memory body short; the artifact carries the full plan.
- Artifacts live under `.atlas/artifacts/` and are gitignored; project memories live under `.atlas/memories/` and are committed. Use artifacts for verbatim plan bodies, memories for the cross-session pointer and summary.
- Without an explicit `session_id`, artifacts attach to the derived MCP session. Recall them from another session with `cross_session_search` or through the memory pointer.
- After the plan changes, save a new artifact and update the memory pointer. If the plan is abandoned or superseded, record that outcome in a new `decision` or `handoff` artifact so the next session does not follow the stale plan.
- Before the final response, `record_session_event` `stop` (or `session-end`) and `compact_session` when the session is long.
- Next session: `wake_up` → `memory_recall` → `read_saved_context` with the stored `source_id`.
- Use continuity layers deliberately: artifacts for verbatim evidence, project memories for cross-session pointers and facts, memoir for design and incident graphs, overview for evidence-linked code-area docs. Update these rows instead of duplicating them.

### Update checklist before finishing substantive work

1. Artifact: save or replace the plan, review, investigation, or handoff body; capture the new `source_id`.
2. Memory: patch the project pointer or durable fact if it changed, and confirm with `memory_recall`.
3. Memoir: add a versioned concept and `replaces` or `refines` relation if design, dependency, decision, or incident knowledge changed.
4. Overview: ingest fresh analysis or run `overview_refresh_plan` if code-area documentation changed or became stale.
5. Session: record `stop` or `session-end`; compact the ledger if the session is long.

## Tool discovery

Runtime docs are canonical for current arguments and output contracts. Do not treat this instruction block as exhaustive inventory.

- `tool_list` lists the visible exported tools; call it when the registry may have changed or you need the exact tool name.
- `tool_search` finds a tool by name or capability when the name is uncertain, and `tool_help` returns the exact input contract, output shape, and minimal example for one tool.
- `man` is the legacy namespace-aware alias; prefer `tool_help` for current MCP tools.

Call `tool_help` before first use of a tool with non-obvious arguments and after a structured error. Do not guess field names, and do not invent a tool when `tool_search` returns no match.

## Common tool map

- continuity: `wake_up`, `get_session_status`, `resume_session`, `save_context_artifact`, `record_session_event`, `compact_session`, `get_context_stats`, `purge_saved_context`, `search_saved_context`, `cross_session_search`, `read_saved_context`, `search_decisions`, `experience_inspect`
- shared memory and notes: `memory_store`, `memory_recall`, `memory_link`, `memory_ops`, `feedback_record`
- memoir: `memoir_create`, `memoir_add_concept`, `memoir_link`, `memoir_inspect`
- overview: `overview_export`, `overview_ingest`, `overview_search`, `overview_show`, `overview_refresh_plan`
- architecture and code health: `analyze_architecture`, `infer_modules`, `label_components`, `analyze_metrics`, `analyze_patterns`, `find_large_functions`, `find_complex_functions`, `find_similar_functions`, `find_duplicates`, `analyze_dead_code`, `analyze_dependency`, `analyze_remove`
- maintenance: `postprocess_graph`, `embed_chunks`, `repo_registry`, `broker_status`, `get_metrics`
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
| `workflows` | Plan and continuity layers, review, symbol inspection, safety, analysis, and historical graph trends |
| `setup` | Install and init modes, skill commands, build/update, MCP server setup, health and config commands |
