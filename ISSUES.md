# Atlas — Stateful Coding Agent Backend

Instruction for all the items in this file:
- Keep each checklist item scoped to one small workable chunk.
- Describe exact code, command, schema field, validation rule, or test to add/change.
- Do not combine multiple implementation steps into one checklist item if they can be merged separately.
- Prefer additive wording like "add", "replace", "update", "remove", "validate", "test".
- Avoid broad goals without concrete implementation detail.

## Goal

Create a cli for stateful coding agent backend.

The primary behavior to preserve is:

- build a repository code graph
- incrementally update it from git diffs
- persist graph data in SQLite
- query graph structure and impact radius
- assemble review context from changed files and neighboring nodes
- expose a CLI first, with MCP later
- make sure to ALWAYS sync CLI tool and MCP tooling (including its flags)

For terms that are easy to misread in this document:

- `flow`: named ordered path or scenario over existing graph nodes, for example `http request -> handler -> service -> repository`, `changed symbol -> direct callers -> affected tests`, or `review path for this PR`. This is metadata over graph, not new edge kind and not runtime tracing requirement in v1.
- `flow membership`: join row in `flow_memberships` that says one node participates in one flow, with optional `position`, `role`, and metadata. `membership` here never means user/team/account membership.
- `community`: unordered cluster of related nodes/modules/files found by some graph algorithm or heuristic, for example SCC/cycle cluster, package cluster, or architecture slice. Community says "these belong together"; flow says "these form ordered path".
- `embeddings`: optional vector search data for retrieval/ranking only. Not required for core build/update/query path.

### Core Design Rule

- Avoid feature growth without signal quality gains
- Prioritize better ranking
- Prioritize better context
- Prioritize better signals

---

## Roadmap Layout

- Remaining focused follow-up patches: Shared Parser Query Migration Patch remainder and Minimal-Context Escalation Metadata Patch. The retrieval follow-up, runtime classification, and reachability patches are shipped; see SHIPPED.md. Runtime policy surface is deferred.

---

## Follow-Up Patches

Use these patch sections for focused improvements that cut across existing roadmap phases without rewriting phase scope.

### Shared Parser Query Migration Patch

Query-backed extraction shipped for C, C++, C#, Java, Scala, JavaScript, TypeScript, Python, Ruby, PHP, Bash, HTML, and CSS. See SHIPPED.md for the shared query contract and language coverage. JSON, TOML, and Markdown are documented manual exceptions.

Remaining:

- [ ] migrate the Go parser to `packages/atlas-parser/queries/go.scm` with shared `@atlas.*` captures
- [ ] document why Go remains manual if query migration cannot preserve current qualified names, receiver/call behavior, or golden outputs
- [ ] confirm every non-Rust parser has either an Atlas-owned query file or a documented reason to remain manual
- [ ] run `cargo test -p atlas-parser` after the final migration batch
- [ ] run `cargo clippy --workspace --all-targets --all-features -- -D warnings` after the final migration batch
- [ ] run `cargo fmt --all` after the final migration batch

---

### Minimal-Context Escalation Metadata Patch

The canonical minimal-first workflow and its agent-facing surfaces are shipped: installed AGENTS instructions, `review_change` and `inspect_symbol` prompts, README, and MCP tool guidance all document the same order. VR3 also emits evidence-sufficiency and escalation metadata for context and review results.

Do not add internal minimal-context enforcement to higher-level tools. It duplicates retrieval already performed by those tools, adds latency and tokens, and conflicts with explicit tool contracts such as review, impact, and refactor planning that need their full target set.

Rules:

- keep escalation evidence-driven and deterministic
- do not add a second retrieval pass inside tools that already resolve their targets
- keep default output stable when no escalation is justified
- keep prompt, README, installed instructions, and tool descriptions aligned

Implementation structure:

#### Patch E3 — Escalation hints on minimal context and neighbors

- [ ] add `next_tools: Vec<String>` and `recommended_escalation_reason: Option<String>` to `get_minimal_context` output in `packages/atlas-mcp/src/tools/context_ops/minimal.rs`, skipped from JSON when empty or absent
- [ ] map `summary.truncated = true` to `next_tools = ["get_review_context"]` and reason `truncated`
- [ ] map `deleted_files_present` to `next_tools` including `get_impact_radius` and reason `deleted_files`
- [ ] map `test_impact` to `next_tools` including `get_review_context` and reason `missing_tests`
- [ ] leave both fields empty when the result is complete and unambiguous
- [ ] add the same fields to `symbol_neighbors` output in `packages/atlas-mcp/src/tools/graph.rs` when budget metadata reports truncation
- [ ] map neighbor truncation to `next_tools = ["get_context", "traverse_graph"]` and reason `truncated`
- [ ] update MCP output schemas for `get_minimal_context` and `symbol_neighbors` with optional `next_tools` and `recommended_escalation_reason`
- [ ] add tests for complete output, truncated output, deleted-file output, and test-impact output
- [ ] audit MCP tool descriptions for `get_review_context`, `get_impact_radius`, `explain_change`, `get_context`, and `traverse_graph`; add minimal-first wording where missing
- [ ] add a drift test that asserts the same escalation order appears in `packages/atlas-cli/src/install/instructions.rs`, `packages/atlas-mcp/src/prompts.rs`, and README

#### Rejected enforcement

- [ ] do not require `get_review_context`, `explain_change`, `get_impact_radius`, `analyze_*`, or refactor tools to run `get_minimal_context` internally; VR3 sufficiency already escalates when evidence is missing

#### Patch completion criteria

- [ ] `get_minimal_context` and `symbol_neighbors` emit escalation hints only when justified
- [ ] complete results keep default output byte-stable
- [ ] context and review tools keep VR3 sufficiency metadata
- [ ] prompt, installed instructions, README, and tool descriptions agree on escalation order
- [ ] no tool performs a hidden second retrieval pass

---

## Additional Backlog

- [ ] add CI-visible parser cache hit-ratio metric and fail when cache reuse drops below configured threshold
- [ ] add thin LSP shim that maps Atlas query/context/impact/reference flows onto standard LSP requests

---

## MCP Guide Smoke — Run 02

Source: live smoke of `AGENTS_MCP_TESTS.md` against broker `1.7.7` (fresh pid, 80/80 tools exported) at commit `10e982e`. 62 bounded checks across 5 read-only slices. `analyze_patterns`, `build_graph`, `update_graph`, `purge_saved_context`, `compact_session`, `record_session_event`, and `feedback_record` were deliberately skipped (context blowup / state mutation). Each item records observed vs expected plus the code anchor; re-run the named call to reproduce.

### P0 — Structured error contract (bare-string errors)

- [x] `overview_show` miss returns bare text `overview subject '...' not found`; return an object with `found=false`, empty `matches`, and stable `code` (code) (`packages/atlas-mcp/src/session_tools/overview.rs`) — miss now returns `found=false`, `code=subject_not_found`, `record=null`, `matches=[]`; schema gained `found`/`matches`/`code`/`subject`
- [x] `overview_ingest` invalid payload returns bare text `invalid overview analysis object`; return `isError` + `code=invalid_input` + `message` (code) (`packages/atlas-mcp/src/session_tools/overview.rs:142`) — `overview_ingest_invalid_input` emits a `ToolErrorPayload` envelope; classifier maps the message to `invalid_input`
- [x] `memoir_link` missing endpoint returns bare text `memoir concept '...' not found`; return structured error with code and `create_missing` retry guidance (code) (`packages/atlas-mcp/src/session_tools/memoir.rs`) — classifier now maps `create_missing` to `invalid_input`; retry guidance names the opt-in flag
- [x] `tool_help` unknown tool returns text-only suggestions; emit `{code:"invalid_input", message, suggestions[]}` object (code) (`packages/atlas-mcp/src/tools/manual.rs:608,651,688`) — suggestions carried in `structuredContent.details.suggestions`
- [x] `man` unknown-tool path shares the same bare-text behavior; route both through one structured builder (code) (`packages/atlas-mcp/src/tools/manual.rs:691`) — one `manual_error_payload(tool, error)` backs both surfaces
- [x] `search_files`/`search_text_assets` invalid `subpath` returns bare text `invalid subpath '...'`; map to structured `invalid_input` (code) (`packages/atlas-mcp/src/discovery/shared.rs:77`) — already classified; now covered by the envelope test
- [x] add a regression test asserting every exported tool's error path emits an object with `isError`, `code`, `message` (code) — `rmcp_server::tests::call_tool::structured_error_envelope_covers_validation_failures`

### P0 — `get_context` budget accounting

- [x] refill spends past the caller budget: observed `tokens_estimated=880` with `token_budget_applied=800` for `{kind:"file"}` `max_nodes=5`; clamp refill to remaining headroom (code) (`packages/atlas-review/src/context/payload.rs:501,518`) — refill was already clamped; the overshoot came from the irreducible floor (one seed node), so `trim_floor_sections` now degrades optional sections (`telemetry`, `seed_budgets`, `traversal_budget`, `repo_provenance`, `workflow`) until the budget holds
- [x] `budget_hit` is derived only from `truncation.truncated`, so `review_context_extraction.max_edges` reports `budget_hit=true` at `budget_observed=18` / `budget_limit=100`; derive each budget's flag from its own usage and keep the rollup separate (code) (`packages/atlas-review/src/context/payload.rs:179,205`) — `record_usage` is now only called with `partial = observed > limit`, so collateral edge drops no longer surface as a phantom `max_edges` hit
- [x] file target barely expands: observed `node_count=1`, `nodes_dropped=12`, `files_dropped=1`, and `seed_kind=changed_symbols` accepted 20 seeds for a pure file target; expand contained symbols before payload trimming and stop seeding `changed_symbols` for file-only targets (code) (`packages/atlas-review/src/context/mod.rs:500,508`) — expansion already worked; the synthetic seeds are relabelled `file_symbols` and mapped to `graph_traversal.max_seed_nodes`
- [x] `budget_name` for a file target surfaces `review_context_extraction.max_edges`; use `file_context`-prefixed budget keys for file targets (code) (`packages/atlas-mcp/src/tools/context_ops/get_context.rs:459`) — node/edge/file reports now use `extraction_budget_prefix(intent)`
- [x] add a regression test asserting `tokens_estimated <= token_budget_applied` for file and symbol targets at `max_nodes` 5 and 15 (code) — `context::tests::budget::caller_token_budget_holds_for_file_and_symbol_targets` (also covers `ChangedSymbols`, budgets 400 and 800)

### P1 — Analysis report shape

- [ ] `infer_modules` still emits `root_paths_preview`/`root_paths_omitted` in compact output; drop them or gate behind `verbose=true` (code) (`packages/atlas-mcp/src/tools/analysis_bounds.rs:111`)
- [ ] `find_complex_functions` compact hides the governing threshold because `governing_threshold`/`threshold_kind` are only inserted into `details`, and `details_omitted=true`; hoist both to the candidate level (code) (`packages/atlas-reasoning/src/engine/large_functions.rs:387-395`)
- [ ] `find_complex_functions` reports `tool=complex_function_report` while category is `complex_functions`; align the tool identity or document the deliberate split (code) (`packages/atlas-mcp/src/tools/analysis.rs:351`)
- [ ] `assess_risk` with `verbose=true` is lossy and smaller than compact (~2.3 KB vs ~2.4 KB, drops top-level `score`/`classification`/`target`/`tool`); make verbose a strict superset (code) (`packages/atlas-mcp/src/tools/analysis.rs` `insight_report_response`)
- [ ] `analyze_architecture` emits only `architecture_file_health` findings (3/3) despite `module_count=29` and `high_coupling_module_count=1`; emit module/layer/coupling findings in the ranked output (code) (`packages/atlas-reasoning/src/engine/architecture.rs:564`)
- [ ] `label_components` emits two rows for the same `.atlas/synthetic/...cargo_fuzz_Cargo_toml.atlas` file because dedupe only covers `NodeKind::File`; dedupe by `file_path` across node kinds (code) (`packages/atlas-reasoning/src/engine/code_intelligence/labels.rs:64`)

### P1 — Overview budget metadata

- [ ] `overview_export` silently clamps `max_tokens: 200` to `256`; honor the request or report the clamp in `warnings` (code) (`packages/atlas-mcp/src/session_tools/overview.rs:103`)
- [ ] `overview_export` packet reports `budget_hit=true` (omitted 2101 bytes) while the summary reports `budget_hit=false`; roll up one consistent flag (code) (`packages/atlas-mcp/src/session_tools/overview.rs`)

### P1 — Counting parity residual

- [ ] `status`/`list_graph_stats` report 26730 nodes while `list_symbols.total_matches` reports 26758; publish one symbol-inventory total and explain the delta (code) (`packages/atlas-mcp/src/tools/inventory.rs:187`)
- [ ] `doctor.graph_build_state` reports 26825 nodes vs live `node_count` 26730; separate build telemetry from live counts in the field names (code)
- [ ] `postprocess_graph` plans 982 files vs `status.indexed_file_count` 956; reconcile the file scope (code)
- [ ] add a drift test asserting `status`, `list_graph_stats`, `debug_graph`, `list_symbols`, and `doctor` agree on node/file totals or each declares its scope (code)

### Run 02 notes (no action)

- Ruled out as stale guide expectations: `analyze_dead_code` `high`/`medium` certainty vocabulary (documented), `find_large_functions` `details.loc` (documented; `line_count` alias lives in `details`), singular `complex_function:` id prefix (intentional per `large_functions.rs:371`).
- `analyze_patterns` intentionally not exercised; Run 01 already tracks its output bounding.
- A throwaway memoir graph `run01_smoke_20261113` (project scope, 2 concepts, 1 `related_to` edge) was created during the smoke. No memoir delete surface exists.

---
