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

## MCP & CLI Test Defects — Run 01

Source: `AGENTS MCP Tests File Run 01.md`. 145 tests executed, ~89 PASS / 51 FAIL / 5 BLOCKED. Every item below is an unimplemented defect from that run. Tag `(code)` or `(docs)` marks the expected fix surface. `run01 L<n>` means line `<n>` in that log file; open those lines for the failing evidence before changing code.

### P0 — Context exhaustion and output bounding

- [x] `analyze_patterns`: default to compact findings only and omit per-finding `evidence` unless `verbose: true` (code) (run01 L4057; payload ends L123569; subagent overflows L3579-3582, L3604-3607)
- [x] `analyze_patterns`: add a hard serialized byte/token ceiling with `truncated`/`budget_hit` flags (code) (run01 L4057)
- [x] `analyze_patterns`: add `offset`/`limit` paging or `max_findings` so callers can page repeated-chain findings (code) (run01 L4057)
- [x] `analyze_patterns`: collapse repeated call chains into top-N summaries with member counts instead of one large finding per chain (code) (run01 L4057)
- [x] add regression test asserting `analyze_patterns {limit:5, verbose:false}` payload stays under the configured max bytes (code) (run01 L3579-3582, L3604-3607, L4057)
- [x] `assess_risk`: make `verbose: false` return a compact payload (`packages/atlas-mcp/src/tools/analysis.rs:32-40`) (code) (run01 L3549, L3561)
- [x] `analyze_architecture`: make `verbose: false` return a compact payload (`packages/atlas-mcp/src/tools/analysis.rs:32-40`) (code) (run01 L3554, L3561)
- [x] `insight_report_response`: stop discarding `_compact_payload` and `_verbose` (code) (run01 L3561)
- [x] `infer_modules`: apply `limit` to `modules`, not only `findings` (`packages/atlas-mcp/src/tools/analysis.rs:373-386`) (code) (run01 L3256, L3271)
- [x] `infer_modules`: drop `owned_symbols`/`root_paths` or gate them behind `verbose: true` (code) (run01 L3272)
- [x] `analyze_remove`: enforce `max_nodes` while enqueuing neighbors (`packages/atlas-reasoning/src/engine/removal.rs::bfs_impact`) (code) (run01 L3535, L3560)
- [x] `get_minimal_context`: apply `max_nodes` cap to `changed_symbols` (code) (run01 L3102, L3136)
- [x] `get_impact_radius`: enforce `max_nodes` on the seed symbol list (code) (run01 L3112, L3143)
- [x] `get_review_context`: propagate caller `max_nodes` into the traversal budget (code) (run01 L3140)
- [x] `get_context`: forward requested `max_nodes` instead of defaulting to 200 (`accepted_node_budget=200` with `max_nodes=15`) (code) (run01 L3135)
- [x] `explain_change`: set `truncated=true` whenever nodes/edges are dropped or `budget_hit=true` (code) (run01 L3118, L3146)
- [x] `man`/`tool_help`: add `max_bytes`/compact mode; today each call emits ~2–4k tokens (code) (run01 L3419)
- [x] `man`/`tool_help`: fill output-schema field descriptions instead of `"No schema description available."` (code) (run01 L3419)

### P0 — MCP export, schema, and introspection

- [x] export `trace_calls` from the live MCP server so `tool_list`/`tool_search`/`tool_help`/`man` see it (code) (run01 L2176, L2737, L3047, L3413)
- [x] export `embed_chunks` (code) (run01 L2176, L3413)
- [x] export `memory_ops` (code) (run01 L2176, L3413)
- [x] export `experience_inspect` (code) (run01 L2176, L3413)
- [x] export `memoir_create` (code) (run01 L2176, L3413)
- [x] export `memoir_add_concept` (code) (run01 L2176, L3413)
- [x] export `memoir_link` (code) (run01 L2176, L3413)
- [x] export `memoir_inspect` (code) (run01 L2176, L3413)
- [x] export `overview_export` (code) (run01 L2176, L3413)
- [x] export `overview_ingest` (code) (run01 L2176, L3413)
- [x] export `overview_search` (code) (run01 L2176, L3413)
- [x] export `overview_show` (code) (run01 L2176, L3413)
- [x] export `overview_refresh_plan` (code) (run01 L2176, L3413)
- [x] add drift test asserting live exported tool count equals the registry inventory (80) (code) (run01 L3384, L3412)
- [x] `tool_list`: include `inputSchema` in descriptors or remove that claim from the tool description (code) (run01 L3384, L3412)
- [x] `tool_help`: emit nearest-name suggestions for unknown tools, including edit distance 1 (code) (run01 L3396, L3415)
- [x] `man`: emit nearest-name suggestions for unknown tools (code) (run01 L3396, L3415)
- [x] `tool_help`/`man`: add top-level `name` alias alongside `requested_tool_name`/`resolved_tool_name` (code) (run01 L3394, L3418)
- [x] `tool_search`: filter stopwords from scoring (code) (run01 L3390-3391, L3416)
- [x] `tool_search`: raise the fuzzy-match floor so distance-1 matches do not surface unrelated tools (code) (run01 L3390, L3416)
- [x] `man`: emit a structured `invalid_input` code for unsupported namespaces instead of plain text (code) (run01 L3401, L3420)
- [x] `repo_registry`: align `trust_state` vocabulary with the documented value (`trusted` vs `active`) (docs) (run01 L3404, L3417)
- [x] `tool_list`: add CLI inventory/search surface or rewrite parity commands in `AGENTS_MCP_TESTS.md` (docs) (run01 L3384, L3414)

### P0 — Store schema compatibility

- [ ] define an upgrade path for `.atlas/context.db` when `user_version` is newer than binary support (code) (run01 L2861, L2879, L3575)
- [ ] define an upgrade path for `.atlas/session.db` when `user_version` is newer than binary support (code) (run01 L2867, L2880, L3576)
- [ ] CLI `status`: surface the content-store open/schema error instead of a synthetic `state=indexed` (code) (run01 L2861, L2879)
- [ ] CLI `db-check`: fail closed on newer-than-supported session schema (match MCP `schema_mismatch`) (code) (run01 L2865, L2880)
- [ ] `doctor`: add a `schema_version` check on both surfaces (code) (run01 L2864, L2884)
- [ ] `doctor`: aggregate session-store schema state from the same probe as `db_check` (code) (run01 L2864, L2884)
- [ ] `doctor`: emit a `path_canonicality` check with `noncanonical_path_rows` on both surfaces (code) (run01 L2863, L2883)
- [ ] add test for newer-than-supported `context.db` returning a stable failure category (code) (run01 L2861)
- [ ] add test for newer-than-supported `session.db` returning a stable failure category (code) (run01 L2867)
- [ ] restart or rebuild the MCP broker so its export set matches the registry (code) (run01 L2176, L2745, L3413)
- [ ] add a smoke check that fails when broker version differs from the binary version (code) (run01 L2176, L3412)

### P1 — Counting and wire-schema parity

- [ ] unify node/edge/file totals across `status`, `doctor`, `list_graph_stats`, and `debug_graph` (26210/101426/940 vs 26238/101452/966) (code) (run01 L2859, L2877-2878, L2991)
- [ ] document or remove the 28-`package`-node delta between `status` and `debug_graph` (code) (run01 L2878, L2991)
- [ ] use one dangling-edge query for `db_check` and `debug_graph` (0 vs 5 found) (code) (run01 L2867, L2881)
- [ ] use one doctor check registry for CLI and MCP (13 vs 14 checks) (code) (run01 L2862, L2885)
- [ ] unify doctor check wire fields (`name/status/details` vs `check/ok/detail`) (code) (run01 L2882)
- [ ] derive `ready` from the same predicate as `health_class`, or document that it means retrieval-ready only (code) (run01 L2860, L2893)
- [ ] `list_graph_stats`: emit per-language node counts instead of a flat language name list (code) (run01 L2932-2933, L2979)
- [ ] `list_graph_stats`: add an `edge_kinds` map sourced from the same group-by as `debug_graph` (code) (run01 L2934-2935, L2985)
- [ ] `debug_graph`: honor `limit` for `top_files` or document sample-only scope (code) (run01 L2870, L2886)
- [ ] `debug_graph`: rename top-file key `path` to `file_path` or update docs (code) (run01 L2870, L2886)
- [ ] `get_metrics`: rename fields to the documented contract or update docs (`metrics`, `parser_tree_cache_hit_ratio`, `tool_call_counts`, `query_latency`) (docs) (run01 L2871, L2887)
- [ ] `get_metrics`: add a `tool` identity field for consistency (code) (run01 L2887)
- [ ] `get_metrics`: verify the parser tree-cache reuse path (`parser_tree_reuses_total=0` across 448 parses) (code) (run01 L2871, L2888)
- [ ] `get_metrics`: add a `metrics` CLI surface or drop the CLI parity claim (docs) (run01 L2873, L2889)
- [ ] `broker_status`: rename fields or update docs (`version`/`worker_threads_configured`/`uptime_secs`) (docs) (run01 L2856, L2890)
- [ ] `broker_status`: measure and document handler-only latency; explain the observed 102–162 ms vs "<50 ms" (code) (run01 L2857, L2891)
- [ ] `db_check`: rename fields or update docs (`integrity.ok`, `summary.orphan_node_count`, `session_db.memory_schema.ok`) (docs) (run01 L2866, L2892)
- [ ] `batch_query_graph`: add `query`/`text` echo to each `results[]` entry (code) (run01 L2923, L3009)
- [ ] verify markdown heading `line_start` attribution; rebuild from canonical inputs if mismatch is systematic (code) (run01 L3015)
- [ ] `detect_changes`: unify CLI `changes` and MCP `files` serializers (code) (run01 L3121, L3150)
- [ ] `explain_change`: unify CLI `changed_by_kind` with MCP `change_kinds`/`coverage_gaps` (code) (run01 L3116, L3149)
- [ ] CLI `review-context`: use one schema for empty and `--files` paths (code) (run01 L3106, L3142)
- [ ] CLI `review-context`: add `--minimal` or remove it from the test guide (code) (run01 L3101, L3137)
- [ ] CLI `impact`: use one `analysis` shape across working-tree and `--base` sources (code) (run01 L3111, L3145)
- [ ] add CLI `symbols --resolve` with `--file`/`--kind`/`--language` (code) (run01 L2911-2915, L2962)
- [ ] add CLI `explain-query --regex` (code) (run01 L2920, L2927, L3003)

### P1 — `resolve_symbol`

- [ ] resolve exact qname, then exact name, then filtered prefix; return zero matches when none (code) (run01 L2914-2915, L2941-2945)
- [ ] gate the fuzzy/edit-distance fallback behind explicit `fuzzy=true` (code) (run01 L2914, L2944)
- [ ] apply the `file` filter before ranking and set `status=resolved` when the post-filter match is unique (code) (run01 L2912, L2947-2951)
- [ ] normalize kind aliases (`fn`→`function`, `record`→`struct`, `iface`→`interface`, `mod`→`module`, `var`→`variable`, `const`→`constant`) before ranking (code) (run01 L2913, L2953-2957)
- [ ] add regression test: nonexistent symbol name → `match_count=0`, `best_match=null` (code) (run01 L2914, L2944)
- [ ] add regression test: name+file+kind → `ambiguous=false` (code) (run01 L2912, L2950)
- [ ] add regression test: `budget_stage_rules` + `kind=fn` → single result (code) (run01 L2913, L2956)

### P1 — Query intent execution

- [ ] execute `who calls <symbol>` via caller-edge lookup and set `usage_edges_included=true` (code) (run01 L2918-2919, L2965-2969)
- [ ] implement `what breaks <symbol>` execution (code) (run01 L2928-2929, L2995-2999)
- [ ] implement `tests for <symbol>` execution (code) (run01 L2995-2999)
- [ ] reject unquoted multiword natural language with actionable grammar guidance (code) (run01 L2921-2922, L2971-2975)
- [ ] `explain_query`: add `query_intent {accepted,intent,kind,normalized_text}` (code) (run01 L2928-2929, L2995-2999)
- [ ] `explain_query`: add `intent_target_symbol` (code) (run01 L2928-2929, L2997)
- [ ] add regression test: natural-language query returns a validation error, not empty matches (code) (run01 L2921, L2974)
- [ ] reuse one intent parser between `query_graph` and `explain_query` (code) (run01 L2998)

### P1 — Graph traversal and relationship correctness

- [ ] `traverse_graph`: dedupe `(from, kind, to)` edges (code) (run01 L3041, L3077)
- [ ] `traverse_graph`: exclude expression-level edge targets or include them in `nodes[]` (code) (run01 L3039-3041, L3076)
- [ ] `traverse_graph`: set `budget_hit`/`truncated` when `accepted_node_budget` is reached (code) (run01 L3043, L3078)
- [ ] `traverse_graph`: cap depth-1 output at root adjacency or document neighbor-edge inclusion (code) (run01 L3043, L3079)
- [ ] `traverse_graph`: use deterministic balanced node selection when the budget truncates (code) (run01 L3043, L3080)
- [ ] attribute test adjacency from call sites inside `#[test]` functions for `symbol_neighbors` and `explain_change` (code) (run01 L3081, L3148)
- [ ] `concept_clusters`: restrict clustering keys to code-symbol nodes and drop expression fragments (code) (run01 L3065, L3082)
- [ ] `concept_clusters`: allow seed↔seed coupling in cluster scoring (code) (run01 L3067, L3083)
- [ ] `cross_file_links`: surface `include_weak` in the live MCP input schema (code) (run01 L3059, L3075)
- [ ] CLI `trace`: populate `path.nodes` into `nodes[]` in path mode (code) (run01 L3051, L3086)

### P1 — Context and review tools

- [ ] `get_context`: consume remaining token headroom before dropping caller/callee nodes (code) (run01 L3096, L3130)
- [ ] `get_context`: expand a file target to contained symbols (code) (run01 L3097, L3132)
- [ ] `get_context`: merge doc/content assets and warn when the retrieval index is unavailable (code) (run01 L3098, L3134)
- [ ] `get_context`: use context-specific budget names (`review_context_extraction.max_nodes` leaks) (code) (run01 L3096, L3135)
- [ ] `get_context`: unify MCP and CLI truncation at identical params (1 node/2755B vs 2 nodes/3836B) (code) (run01 L3096, L3131)
- [ ] `get_review_context`: populate `neighbors` from direct consumers (currently always `[]`) (code) (run01 L3106, L3139)
- [ ] `get_review_context`: populate `critical_edges` (currently always `[]`) (code) (run01 L3106, L3139)
- [ ] `get_review_context`: reject unknown args (`exact_dir`, `include_ranking_evidence`) or implement them (code) (run01 L3107-3108, L3141)
- [ ] align ranking-evidence default between MCP and CLI (code) (run01 L3108, L3141)
- [ ] `explain_change`: derive the summary text from emitted sections (call chains/components currently empty) (code) (run01 L3147)
- [ ] align `get_impact_radius` `affected_files`/`affected_nodes` names or update docs (docs) (run01 L3111, L3144)
- [ ] align `get_minimal_context` `impact_summary` name or update docs (docs) (run01 L3101, L3138)
- [ ] align `get_context` target field names (`path`/`paths` vs `file`/`files`) or accept aliases (docs) (run01 L3097, L3133)
- [ ] `analyze_remove`: add a bounded `impacted_files` array (code) (run01 L3534, L3569)

### P1 — Analysis engines

- [ ] `find_similar_functions`: prefilter/rank candidates before the `MAX_SIMILAR_CANDIDATES_PER_SOURCE` cap (`packages/atlas-reasoning/src/engine/code_intelligence/similar.rs:113-124`) (code) (run01 L3241, L3652, L3666-3670)
- [ ] `find_similar_functions`: recalibrate `SIMILARITY_LOW_THRESHOLD`/weights after the candidate-window fix (code) (run01 L3656, L3672-3676)
- [ ] `find_similar_functions`: emit a structured no-candidates hint instead of silently returning empty (code) (run01 L3278)
- [ ] `find_duplicates`: fix the test path filter to match `/tests/`, `tests.rs`, `_tests.rs` (`duplicates.rs:26-29`) (code) (run01 L3251, L3662, L3678-3682)
- [ ] `find_duplicates`: include test nodes when `include_tests=true` (code) (run01 L3251, L3684-3688)
- [ ] `find_duplicates`: wire `include_tests` through `DuplicateDetectionRequest` (code) (run01 L3662, L3678-3688)
- [ ] `analyze_dead_code`: align certainty vocabulary (`high`/`medium` vs `definite`/`probable`/`speculative`) (docs) (run01 L3544, L3565)
- [ ] `analyze_dead_code`: compute the summary before limit truncation and add per-tier counts (`analysis.rs:648-695`) (code) (run01 L3546, L3566)
- [ ] CLI `analyze dead-code --summary`: honor `--json` (`packages/atlas-cli/src/commands/reasoning.rs:273-275`) (code) (run01 L3546, L3567)
- [ ] align dead-code default limits (MCP 50 vs CLI 100) (code) (run01 L3546, L3568)
- [ ] align `analyze_dependency` `confidence_tier`/`blocking_callers` names or update docs (docs) (run01 L3539, L3564)
- [ ] `analyze_architecture`: emit module/layer/coupling findings instead of only file-health hotspots (code) (run01 L3554, L3562)
- [ ] `analyze_architecture`: add cycle/SCC summary counts to `ArchitectureReport` (code) (run01 L3556, L3563)
- [ ] `find_large_functions`: add `line_count` alias or update docs to `details.loc` (docs) (run01 L3626, L3635)
- [ ] `find_large_functions`: document the `large-or-complex` default mode semantics (docs) (run01 L3627, L3636)
- [ ] `find_complex_functions`: report the governing threshold (`details.threshold` leaks LOC default 80) (code) (run01 L3637)
- [ ] `find_complex_functions`: emit `complex_functions` category/id prefix (code) (run01 L3639)
- [ ] `analyze_metrics`: add global density, average degree, and diameter summary (code) (run01 L3880-4055)
- [ ] `infer_modules`: add `public_symbol_count`/`internal_symbol_count`/`export_ratio` (code) (run01 L3257, L3273)
- [ ] `infer_modules`: update `inferred_module_report_output_schema` (`packages/atlas-mcp/src/tools/registry/output_schemas/analysis.rs:232`) (code) (run01 L3273)
- [ ] `label_components`: dedupe file-level vs file-qname rows (code) (run01 L3263, L3275)
- [ ] `label_components`: honor `symbols:` filter (`labels.rs:27-46`) (code) (run01 L3262, L3274)
- [ ] `label_components`: prioritize explicit targets before truncation (`labels.rs:71-73`) (code) (run01 L3274)
- [ ] align CLI/MCP `stale_index` derivation (`insights similar-functions` warns stale while MCP reports fresh) (code) (run01 L3277)

### P1 — Content tools and CLI surfaces

- [ ] add CLI `files` command for `search_files` (code) (run01 L3292, L3328-3332)
- [ ] add CLI `grep` command for `search_content` (code) (run01 L3297, L3328-3332)
- [ ] add CLI `excerpt` command for `read_file_excerpt` (code) (run01 L3302, L3328-3332)
- [ ] add CLI match-context command for `read_file_around_match` (code) (run01 L3307, L3328-3332)
- [ ] add CLI `templates` command for `search_templates` (code) (run01 L3317, L3328-3332)
- [ ] add CLI `text-assets` command for `search_text_assets` (code) (run01 L3322, L3328-3332)
- [ ] add CLI equivalents for `symbol_neighbors`, `cross_file_links`, and `concept_clusters` (code) (run01 L3031, L3057, L3065, L3084)
- [ ] add `--depth`/`--file` aliases to `atlas query` or fix docs to `--expand-hops` (docs) (run01 L3039, L3057, L3084)
- [ ] accept `selector.ranges` alias or update docs to `line_ranges` (code) (run01 L3303, L3334-3338)
- [ ] `read_file_excerpt`: report emitted vs requested counts so truncation is unambiguous (code) (run01 L3304, L3340-3344)
- [ ] `get_docs_section`: populate near-match heading `candidates` on miss (code) (run01 L3314, L3346-3350)
- [ ] align `search_content` symbol-name warning text with the documented assertion (docs) (run01 L3299, L3352-3356)
- [ ] fix `atlas uri` accepted forms or document the grammar (code) (run01 L3366-3368)
- [ ] ignore `EPIPE` on stdout writes in the CLI (code) (run01 L3085)

### P2 — Test-guide drift (`AGENTS_MCP_TESTS.md`)

- [ ] update `broker_status` expected field names (docs) (run01 L2856, L2890)
- [ ] update `db_check` expected field names (docs) (run01 L2866, L2892)
- [ ] update `get_metrics` expected schema (docs) (run01 L2871, L2887)
- [ ] update `debug_graph` top-file key and `limit` expectation (docs) (run01 L2870, L2886)
- [ ] update `list_graph_stats` expectations for language counts and `edge_kinds` (docs) (run01 L2932-2935, L2979, L2985)
- [ ] update `get_context` target field names to `file`/`files` (docs) (run01 L3097, L3133)
- [ ] update `read_file_excerpt` selector example to `line_ranges` (docs) (run01 L3303, L3334)
- [ ] update `search_files` fixture `packages/atlas-repo/src/canonical.rs` → `path.rs` (docs) (run01 L3293, L3358-3362)
- [ ] update `search_content` warning assertion (docs) (run01 L3299, L3352)
- [ ] update `get_minimal_context` `impact_summary` name (docs) (run01 L3101, L3138)
- [ ] update `get_impact_radius` `affected_*` names (docs) (run01 L3111, L3144)
- [ ] update `analyze_dependency` `blocking_*` names (docs) (run01 L3539, L3564)
- [ ] update `analyze_dead_code` certainty tiers (docs) (run01 L3544, L3565)
- [ ] update `find_large_functions` assertion to `details.loc` (docs) (run01 L3626, L3635)
- [x] update `repo_registry` `trust_state` `active` → `trusted` (docs) (run01 L3404, L3417)
- [ ] fix parity commands pointing at the wrong symbol (26.2, 26.3, 28.2, 28.3, 30.3) (docs) (run01 L3570)
- [ ] add `--max-depth`/`--max-nodes` to 27.2/27.3 parity commands (docs) (run01 L3570)
- [ ] fix the 26.2 premise that calls public `is_schema_mismatch_error` private (docs) (run01 L3571)
- [ ] freeze heading text/numbering or make tests resolve tools by name (docs) (run01 L3451-3477, L3485-3495)
- [ ] clarify inclusive threshold wording (`>=`) in `find_complex_functions` intent (docs) (run01 L3638)

### Environment / state (not code changes)

- [ ] rebuild `.atlas/context.db` from clean canonical inputs once schema support matches (state) (run01 L2861, L3575)
- [ ] rebuild `.atlas/session.db` from clean canonical inputs once schema support matches (state) (run01 L2867, L3576)
- [ ] record HEAD drift (`7b54eac` → `a169fd5`) in run notes for reproducibility (docs) (run01 L3364, L3448)
