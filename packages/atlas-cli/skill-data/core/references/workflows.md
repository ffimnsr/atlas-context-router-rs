# Atlas workflows

Task recipes beyond the core loop. Each MCP call has a CLI mirror; run `atlas skills get core --ref cli` for every subcommand, argument, and flag.

## Plan and persist multi-step work

1. `wake_up` with the task topic; read `recent_decisions`, `critical_memories`, `changed_files`, and `graph_readiness` before replanning.
2. Recon: `query_graph` or `resolve_symbol`, `get_context`, `cross_file_links` or `concept_clusters` for area boundaries, and `analyze_architecture` for unfamiliar modules.
3. Write the plan, then persist the full body with `save_context_artifact` (`source_type: "plan"`, `content_type: "text/markdown"`).
4. Persist a durable pointer with `memory_store` (`scope: "project"`, task `topic`, the artifact `source_id`, and honest `importance`).
5. Execute one step at a time; after edits run `update_graph` with the working tree, then `detect_changes` and `get_minimal_context`.
6. Close with `record_session_event` `stop` and `compact_session` when the session is long.

Artifacts are committed under `.atlas/artifacts/`; project memories are committed under `.atlas/memories/`. Artifacts at or below 512 B return inline and are not persisted. Without an explicit `session_id`, artifacts attach to the derived MCP session; use `cross_session_search` or the memory pointer across sessions.

```bash
atlas session save-artifact --label refactor-plan --source-type plan --content-type text/markdown "plan body"
atlas memory store --topic refactor-plan --title "Refactor plan" --text "summary" --scope project --source-id <source_id>
atlas memory recall "refactor-plan"
```

## Map an unfamiliar area

1. `analyze_architecture` for module cycles, layer violations, and coupling hotspots.
2. `infer_modules` and `label_components` to learn package and component boundaries.
3. `concept_clusters` with seed files to group co-dependent files.
4. `cross_file_links` on one file to rank the files that reference its symbols.
5. `query_graph` or `resolve_symbol` and `get_context` once the boundary files are known.
6. Persist interpretive findings before moving on: what the area is for and how its key functions work goes through `overview_export` then `overview_ingest`, durable gotchas go to `memory_store`, and design conclusions go to a memoir graph. Keep callers, callees, and other structural facts in the graph; it re-derives them.

```bash
atlas insights architecture
atlas insights infer-modules
atlas insights label-components --files src/lib.rs
atlas content clusters src/lib.rs
atlas content links src/lib.rs
```

## Build and update memoir knowledge

Use memoir for design, dependency, decision, and incident knowledge the code graph cannot express. Do not use it for callers, callees, impact, or current code shape; keep those in the code graph.

1. Reconfirm code facts with `query_graph`, `get_context`, or `trace_calls`.
2. Save supporting evidence with `save_context_artifact`; keep the returned `source_id`.
3. `memoir_create` one bounded topic graph (`architecture`, `incident-1234`, `plan-auth-refresh`) with `scope: "project"`.
4. `memoir_add_concept` with a stable `kind` (`component`, `decision`, `risk`, `plan-step`, `incident`) and `source_id` when evidence exists.
5. `memoir_link` with the controlled vocabulary: `depends_on`, `part_of`, `contradicts`, `refines`, `replaces`, `caused_by`, `fixed_by`, `blocked_by`, `decided_by`, `related_to`.
6. `memoir_inspect` before adding more; depth 1-2 is usually enough.

Update rules:

- Concept names are case-insensitive and unique per graph. Re-adding the same name fails; add a versioned concept and link it with `replaces` or `refines`.
- Link revised decisions as `decision-v2 --replaces--> decision-v1`; link incidents as `incident --caused_by--> component` and `incident --fixed_by--> fix`.
- Missing endpoints are not created automatically. Pass `create_missing: true` only when intentional.
- Delete a graph only after `memoir_delete` with `dry_run: true`; deletion removes concepts and relations.
- `.atlas/memoir/` is committed Markdown. Do not store secrets or raw transcripts.

Before finishing, inspect the changed concept and confirm every relation direction. If this changed an active plan, patch the plan memory pointer so the next session sees the revised design rather than the old one.

```bash
atlas memoir create Architecture --description "service and dependency map"
atlas memoir add-concept Architecture parser "Source parser" --kind component --source-id <source_id>
atlas memoir add-concept Architecture parser-v2 "Parser with cached lexer" --kind component
atlas memoir link Architecture parser-v2 parser --relation replaces
atlas memoir link Architecture parser lexer --relation depends_on --confidence 0.9
atlas memoir inspect parser --graph Architecture --depth 2
```

## Maintain overview memory

Use overview for evidence-linked descriptions of projects, packages, modules, files, symbols, functions, and methods. It is not a decision log and not a substitute for freshness-checked graph facts.

1. Refresh the graph first with `update_graph`.
2. `overview_export` with `scope`, `subjects` or `subject`, optional `changed`/`since`, and optional `include_callers`, `include_callees`, `include_tests`, `include_code_spans`, `max_tokens`.
3. An external analyzer or human returns `OverviewAnalysisInput`; echo `packet_id`, `evidence_ids`, and the packet `subject` unchanged.
4. `overview_ingest` with the `analysis` object, or `path`/`input` for JSON on disk; the CLI reads a file or `-` for stdin.
5. `overview_search` by query, subject kind, freshness, package, module, file, or source kind. `overview_show` one subject; start at `l1` and escalate to `l2` only when the full body is needed.
6. After changes, run `overview_refresh_plan` read-only, then `apply: true` only after reviewing the plan.
7. `get_context` and `get_review_context` include overviews through `include_overviews` and `overview_freshness`.

Minimal ingest shape:

```json
{
  "schema_version": 1,
  "packet_id": "<from export>",
  "evidence_ids": ["node:12"],
  "subject": {
    "kind": "function",
    "repo_root": "/repo",
    "commit_sha": "abc123",
    "qualified_name": "src/parser.rs::fn::parse",
    "canonical_file_path": "src/parser.rs",
    "line_start": 10,
    "line_end": 24
  },
  "title": "compute",
  "summary": "computes values",
  "description": "optional bounded body",
  "responsibilities": [],
  "flow": [],
  "inputs": [],
  "outputs": [],
  "invariants": [],
  "gotchas": [],
  "examples": [],
  "tags": ["parser"],
  "confidence": 0.8,
  "source_kind": "external_llm",
  "analyzer_name": "external-tool",
  "analyzer_version": "1",
  "source_ids": [],
  "metadata": {}
}
```

Rules:

- `schema_version` must be `1`; `summary` plus `description` or one non-empty detail array is required; unknown top-level fields are rejected.
- `source_kind` is `external_llm`, `manual`, or `imported`.
- Ingest re-validates canonical path, qualified name, line span, commit SHA, redaction, and body budget.
- A newer ingest supersedes the prior active record. Stale, possibly stale, and orphaned records are excluded from context unless explicitly requested.
- Large bodies route through the content store; the overview row keeps a bounded preview and `source_id`.
- Use `atlas overview list` and `atlas overview stale` for maintenance, and `atlas overview run-external --dry-run` before enabling an external analyzer command.

Before finishing, run `overview_refresh_plan` and report any stale, possibly stale, or orphaned subjects you did not refresh. If an overview supersedes an assumption in an active plan, update the plan artifact and its memory pointer as well.

```bash
atlas overview export --scope functions --include-callers --include-tests --max-tokens 2048 --json
atlas overview ingest analysis.json --json
atlas overview search "parser" --subject-kind function --freshness fresh --limit 10
atlas overview show src/parser.rs --level l1
atlas overview refresh-plan --since origin/main
atlas overview refresh-plan --since origin/main --apply
```

## Keep durable memory current

Use `memory_store` for facts that outlive a session: decisions, constraints, preferences, gotchas, and plan pointers. Use artifacts for verbatim evidence.

Every upgrade, dependency bump, feature, or behavior fix lands as a traceable pair: a `decision` or `handoff` artifact stating what changed, why, and how it was verified, plus a project-scope memory row with recall tags (e.g. `upgrade`) and the artifact `source_id`. Both families are committed, so `memory_recall` followed by `read_saved_context` answers what changed and why across sessions, clones, and machines. Skip the pair only when an edit changes no behavior.

- Choose `scope` deliberately: `project` for repo facts, `session` for scratch, and `frontend` only for host-specific behavior.
- Keep `topic` stable; set `importance` honestly; add searchable `tags`; link evidence with `source_id`.
- Use `links` at store time or `memory_link` afterward for directed memory-map edges. Relations accept the memoir vocabulary plus `supersedes`; a newer row linked with `supersedes` marks the target as replaced, so default recall stops surfacing the stale row. The target must already exist, duplicate links collapse, and self-links are rejected.
- Update an existing row with `patch_id` plus `patch_text` or `patch_file` instead of storing a duplicate.
- Recall with `memory_recall`; use `shared: true` for project and global memories, `tags` to require every listed label, and `hybrid: true` only when embeddings are configured.
- Audit one row with `memory_ops`; it returns the typed operator history and reports `found=false` for unknown ids. Inspect storage with `get_context_stats`.
- `search_decisions` reads conclusions captured by session hooks and ranked by the query; restrict to one session when needed. It does not write, so if no decision row exists, save a `decision` artifact and a project memory instead of assuming it was stored.
- When an Atlas analysis prediction was wrong, call `feedback_record` with `predicted`, `actual`, and `correction`; search past mistakes with `atlas feedback search`.

Before finishing, recall the new or patched topic and confirm the row is visible at the intended scope. If the memory is only a pointer, verify that `read_saved_context` still resolves its `source_id`.

```bash
atlas memory store "Parser cache must be invalidated per commit" --topic parser --scope project --importance high --source-id <source_id> --tag parser --tag cache
atlas memory store "rmcp 3.11: pre-init EOF is clean shutdown" --topic mcp-transport --scope project --importance high --source-id <source_id> --tag upgrade
atlas memory link <memory_id> <target_id> --relation supersedes
atlas memory recall "parser cache" --shared --tag parser
atlas memory ops <memory_id>
atlas feedback record --predicted "unused" --actual "called via plugin registry" --correction "keep parser hook" --analysis-kind dead_code --symbol src/plugin.rs::fn::register
```

## Recall and curate continuity

- Start every session with `wake_up`; use `resume_session` only when `wake_up` is unavailable. `resume_session` returns or consumes the resume snapshot, so set `mark_consumed` deliberately instead of replaying the same snapshot twice.
- Use `get_session_status` before deciding to compact or resume. It reports session identity, event count, last compaction time, and whether a resume snapshot exists, and it is read-only.
- `search_saved_context` searches the current session; `cross_session_search` searches all sessions for the repo; `read_saved_context` returns the full body by `source_id`. If a search hit is a pointer, follow the `source_id` instead of guessing at the body.
- `get_global_memory` surfaces frequently accessed symbols, files, and workflow patterns across sessions. Use it to orient on a repo you have not touched recently, not as a substitute for current graph queries.
- `compact_session` when the ledger grows; `purge_saved_context` removes session artifacts or artifacts older than `keep_days`. Confirm destructive scope before purging.
- `experience_inspect` reports query-conditioned experience edges for one `atlas://` URI, including direction, confidence, use count, and freshness. Only fresh edges activate in context assembly; an empty ledger is a valid result, not an error. Use it to audit a shortcut before trusting it, never as a write tool.

At the close of substantive work, record `stop` or `session-end` and compact only when the ledger is long. Do not finish with an updated artifact but a stale memory pointer, or with a revised design but no memoir or overview update.

## Review a change-set

1. `detect_changes` when changed files are unknown.
2. `get_minimal_context` for first bounded triage.
3. `get_review_context` when changed symbols, neighbors, or risk detail is needed.
4. `explain_change` when deterministic risk and test-gap explanation is needed.
5. `get_impact_radius` when explicit blast radius is needed.

```bash
atlas detect-changes --base origin/main
atlas review-context --base origin/main --format markdown --max-depth 3 --max-nodes 200
atlas explain-change --base origin/main --max-depth 5 --max-nodes 200
atlas impact --base origin/main --max-depth 5 --max-nodes 200
atlas review-context --files src/lib.rs src/session.rs
```

Add `--staged` to `detect-changes` or `explain-change` for staged diffs, or pass `--files` explicitly. Pass `--allow-stale` or `--allow-partial` only when a freshness warning is acceptable; otherwise refresh first.

## Debug a failure

Fix the root cause, not the symptom. Reproduce before changing anything.

- Run the failing command or test and capture the exact error text and exit status; do not theorize before reproducing.
- Locate the code path: `query_graph` or `resolve_symbol` on the failing symbol, `symbol_neighbors` for callers and test adjacency, then `get_context` for a bounded view. Read exact files only after Atlas names them.
- Check prior incidents before debugging blind: `wake_up` surfaces recent runtime failures, and `memory_recall` plus `search_decisions` may already hold the same-symbol fix.
- Isolate by evidence: bisect with git when the failure appeared after a range of edits, and use `detect_changes` to see what actually moved in the working tree.
- Fix the root cause, add a regression test that fails without the fix, and rerun the narrowest suite, then the wider one.
- Record `record_session_event` with `event: "tool-failure"` (or `error`) and save the fix with `save_context_artifact` `source_type: "error_resolved"`. A lesson that applies repo-wide goes to `memory_store` at `scope: "project"`.

## Test alongside a change

A behavior change is not done until a test pins it.

- Find existing coverage before writing new tests: `symbol_neighbors` returns adjacent test nodes, `query_graph` accepts `tests for <symbol>`, and `get_context` with `tests: true` includes test nodes.
- Write the regression test together with the fix, in the project's existing test layout; prefer per-test owned state over shared globals and fixed sleeps.
- Escalate outward: the narrowest suite covering the change first, then the package, then the workspace only when the narrow runs pass.
- A flaky test is a bug: root-cause scheduling, leaked state, or a real race. Never hide it behind retries or longer timeouts.
- When the failure exposed a wrong Atlas verdict, correct future analysis with `feedback_record`.

## Perform an upgrade or dependency bump

Recon first, then bump, then prove nothing broke.

- Map dependents before touching the manifest: `symbol_neighbors` or `get_impact_radius` on the code that consumes the dependency, plus `search_content` for version pins in config files.
- Apply the bump, refresh the lockfile, and read the changelog or migration notes of the upgraded component for breaking edges.
- Run the project's dependency audit when one exists (advisory scanner, license gate) and treat new advisories as blockers.
- Sweep for breakage with `explain_change` or `get_review_context` on the changed files, then run fmt, clippy, and the test gates.
- Close with the traceability pair: a `decision` or `handoff` artifact (what, why, verification) plus a project memory row tagged `upgrade`.

## Prepare a release

Atlas covers verification and the durable record; repository files stay repo policy.

- Confirm the gates: fmt, clippy, tests, and any project-specific release checks are green in a clean working tree.
- Confirm the paper trail: `memory_recall` by the change tags (`upgrade`, `feature`, `fix`) shows a traceability record for every behavior change in release scope.
- Save one `decision` artifact for the release itself — scope, verified gates, notable changes, known caveats — with a project memory row pointing at it.
- Changelog files, agent instruction files, and version constants belong to the repository's own policy; follow the repo's conventions when it has them, but this workflow neither writes nor mandates them.

## Scaffold a new component

Place it in the architecture before writing the first line.

- Find the right layer with `analyze_architecture` and `infer_modules`, and confirm nothing already does the job with `find_duplicates` or `find_similar_functions` before creating new code.
- Keep the surface minimal: prefer crate-internal exposure over public, composition over type hierarchies, and one responsibility per module.
- New code goes in new files or modules rather than growing an already-large file; add unit tests alongside and integration tests for the end-to-end path.
- After creating files, `update_graph` with the explicit file list so graph answers about the new area are current; export an `overview_export` packet if the area deserves durable docs.
- Close with the traceability pair tagged `feature`.

## Coordinate parallel agents

Delegate only what can run alone; keep write scopes disjoint.

- Delegate self-contained subtasks: each sub-agent gets full context (files, requirements, constraints) because it cannot see this conversation.
- Split work so write scopes do not overlap; the parent coordinates and synthesizes rather than duplicating sub-agent work.
- Each sub-agent runs the same Atlas loop — `wake_up` through `stop` — and records into its own `agent_id` partition when the host supports one.
- Merge partitions only at deliberate merge points (`merge_agent_partitions` flags); read cross-partition state only then.
- The parent closes with one combined `handoff` artifact plus a memory pointer so the next session sees the merged outcome.

## Run periodic hygiene

Tie upkeep to events, not to wall-clock schedules.

- After a big landing or release: `overview_refresh_plan`, and apply only after reviewing stale, possibly stale, and orphaned subjects.
- Under storage pressure: `get_context_stats` first, then `compact_session` for a long ledger or `purge_saved_context` for old or session-scoped artifacts.
- On suspected corruption or after a hard crash: `doctor`, then `db_check` for orphan and dangling rows.
- On memory drift: `atlas memory health` for noisy, duplicated, orphaned, or oversized rows, and `memory_recall` to confirm important topics stay visible.
- When history snapshots grow: `atlas history prune` with an explicit retention policy.
- Before repeating a questionable analysis verdict: `atlas feedback search` so a known false positive lowers confidence instead of being rediscovered.

## Inspect one symbol

1. `query_graph` or `resolve_symbol` to land on the canonical qualified name.
2. `symbol_neighbors` for direct callers, callees, and tests.
3. `get_context` for bounded ranked context.
4. `trace_calls` for a directional chain or a path to a target.
5. `traverse_graph` only for unfiltered multi-hop reachability.

```bash
atlas symbols --resolve handle_request
atlas query handle_request --kind function
atlas content neighbors src/lib.rs::fn::handle_request
atlas context "who calls handle_request"
atlas trace handle_request --direction up --depth 2
atlas trace build_graph --to reconcile_call_targets
```

`atlas content` mirrors the companion content tools: `files`, `grep`, `excerpt`, `match-context`, `templates`, `text-assets`, `neighbors`, `links`, and `clusters`. Use them after graph tools point at code, not instead of them.

## Decide whether code is safe to remove or rename

```bash
atlas analyze remove src/lib.rs::fn::legacy_parse --max-depth 3 --max-nodes 200
atlas analyze safety src/lib.rs::fn::legacy_parse
atlas analyze dependency src/lib.rs::fn::legacy_parse
atlas analyze dead-code
atlas insights risk src/lib.rs::fn::legacy_parse
atlas refactor rename --symbol src/lib.rs::fn::legacy_parse --to parse_legacy --dry-run
```

Keep `--dry-run` on refactors until the planned edits are reviewed, and run the project's tests after applying. `analyze dead-code` lists candidates; it does not prove a symbol is unused at runtime.

## Repo-wide analysis reports

Use the MCP mirrors for structured output: `analyze_architecture`, `analyze_metrics`, `analyze_patterns`, `find_large_functions`, `find_complex_functions`, `find_similar_functions`, `find_duplicates`, `infer_modules`, and `label_components`. Start with `find_duplicates` or `find_similar_functions` before extracting shared code, and `find_complex_functions` or `find_large_functions` before splitting a function.

```bash
atlas insights architecture
atlas insights metrics --limit 20
atlas insights patterns --limit 20
atlas insights large-functions --threshold 80 --mode large-or-complex
atlas insights complex-functions --complexity-threshold 15
atlas insights similar-functions src/lib.rs::fn::handle_request --limit 10
atlas insights duplicates --min-score 0.7
atlas insights infer-modules
atlas insights label-components --files src/lib.rs
```

Reports are deterministic and never call a model. They accept `--allow-stale` and `--allow-partial`; prefer refreshing the graph instead.

## Historical and derived graph surfaces

Historical and multi-repo surfaces are CLI-only. Use them after the core graph workflow identifies the area, not as the first call, and run `atlas <group> <command> --help` for required flags.

- `atlas history status` reports snapshot coverage; `atlas history build` and `atlas history update` populate bounded commit ranges, while `diff`, `symbol`, `file`, and `churn` answer regression and churn questions. Use history for trends across commits, not for current-state facts; refresh or rebuild the current graph first.
- `atlas repo list` and `atlas repo sync` manage the multi-repo registry. Inspect `repo_registry` before trusting cross-repo answers, and sync refs and trust state after adding or moving a repo.

```bash
atlas history status
atlas history churn
atlas repo list
atlas repo sync
```

## Communities and flows

Communities and flows are named graph groupings with different truth sources. Both are readable from MCP and the CLI, so they join the normal workflow instead of being CLI-only side features.

- **Communities (derived).** Louvain clusters written by `postprocess_graph` with `{ "stage": "communities" }` or `atlas communities detect`. Read them with `list_communities` (set `include_nodes` for member names, capped at 100 with a `nodes_truncated` flag) or `atlas communities list`. They are consumed as fallback module buckets by `infer_modules` and `label_components`, so stale communities degrade those guesses silently.
- **Communities freshness rule.** After `build_graph` or `update_graph`, check the `status` payload: `postprocess.communities_refreshed_after_last_build` is `true` (fresh), `false` (stale — run `postprocess_graph` with `{ "stage": "communities" }` before trusting community-derived buckets), or `null` (the latest postprocess run skipped the communities stage; rerun it before relying on clusters). Treat a `false` value as a blocking precondition for `infer_modules` and `label_components` conclusions, not a warning to ignore.
- **Flows (curated).** Named ordered symbol sequences humans create via `atlas flows create` / `add-member`. Read them with `list_flows` (filter by `name` for one flow with ordered members, or `node_qn` for every flow containing a symbol; members are capped at 100 with `members_truncated`) or `atlas flows list` / `members <name>`. Flows survive rebuilds but are not graph-derived: after refactors, update their members with `atlas flows add-member` / `remove-member` so the sequence does not drift, and never invent a flow the user did not curate.
- When the user asks for architecture boundaries, prefer live analysis (`analyze_architecture`, `concept_clusters`) first; use communities and flows as named cross-references, not as authoritative architecture.

```bash
atlas flows list
atlas flows members checkout
atlas communities detect
atlas communities list
```

## Keep the graph fresh

`atlas_provenance` and `atlas_freshness` describe the graph the answer came from. If freshness says stale after edits, refresh before making claims:

```bash
atlas update --working-tree
atlas update --base origin/main
```

A missing or corrupt database needs a full rebuild with `build_graph` or `atlas build`.
