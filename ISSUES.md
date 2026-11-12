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

- Part V. Remaining focused follow-up patches: Retrieval Follow-Up Patch remainder, Runtime Event Classification and Resume Grouping Patch, Reachability Measurement and Rust Guard Patch, Shared Parser Query Migration Patch remainder, and Minimal-Context Escalation Metadata Patch. Runtime policy surface is deferred.
- Part XII. Remaining RAG follow-up: deferred memory-operator wiring only. VM1–VM3, VR1–VR4, and VQ1 are shipped; see SHIPPED.md.

## Cross-Cutting Track Map

- Retrieval and search follow-ups: Retrieval Follow-Up Patch remainder
- Context continuity and runtime memory: Runtime Event Classification and Resume Grouping Patch
- Graph safety and workflow: Minimal-Context Escalation Metadata Patch
- Rust parser correctness: Reachability Measurement and Rust Guard Patch, Shared Parser Query Migration Patch remainder
- Agent policy and enforcement: no active work; runtime policy surface is deferred
- RAG adaptation: deferred memory-operator wiring in Part XII; all retrieval and memory tracks shipped

---

## Part IV — Context Continuity and Memory

Use this part for session persistence, saved artifacts, retrieval-backed resume, and long-lived memory work.

### Context-Mode and Continuity Roadmap

These phases cover continuity storage, session lifecycle, retrieval-backed restoration, memory quality, and longer-term cross-session intelligence.

Extend Atlas with context-mode persistence and session continuity without mixing those concerns into graph database.

This backlog covers pieces needed for:

- artifact persistence
- session continuity
- resume snapshots
- retrieval-backed restoration

Core Design Rules:

- DO NOT store saved context in graph database
- DO NOT replay raw command history into future sessions
- ALWAYS restore context through retrieval
- ALWAYS store large outputs outside model context
- KEEP graph storage, content storage, and session storage as separate systems
- KEEP continuity best-effort; never block primary CLI/MCP flow on session persistence failure
- KEEP retrieval lexical and local first; embeddings are optional later, not required for v1 context-mode completion

---

Phases CM14 (Decision Memory) and CM15 (Agent-Aware Context), ICM-0 (universal MCP session capture fallback), ICM-A (shared memory surface), and the full ICM memory follow-on roadmap are shipped. See SHIPPED.md for curation, feedback, wake-up, recall, memoir, overview memory, and shell-first surfaces.

No active Part IV items remain.

---

## Part V — Follow-Up Patches

Use these patch sections for focused improvements that cut across existing roadmap phases without rewriting phase scope.

### Retrieval Follow-Up Patch

These are the high-value retrieval/indexing improvements still missing or only partially specified after the current v3 plan.

They are meant to strengthen Atlas’s retrieval/content sidecar without changing the graph-first core.

#### Patch R7 — Context payload telemetry and metadata slimming

The earlier post-retrieval compaction experiment is dropped. Atlas context payloads are structured JSON metadata, not retrieved prose: measured review context emitted 93% node records and zero source-text bytes. Text compaction has no target. Token reduction must come from honest telemetry and payload-shape slimming, not a compactor.

##### R7.1 — Baseline context telemetry

- [ ] add `ContextTelemetry` in `packages/atlas-core/src/model/context.rs` and `ContextResult::telemetry: Option<ContextTelemetry>` with `intent`, `requested_nodes`, `emitted_nodes`, `requested_edges`, `emitted_edges`, `requested_files`, `emitted_files`, `requested_saved_sources`, `emitted_saved_sources`, `requested_content_assets`, `emitted_content_assets`, `requested_overviews`, `emitted_overviews`, `requested_bytes`, `emitted_bytes`, `requested_tokens`, `emitted_tokens`, `budget_hit`, `budget_name`, `duration_ms`, and `outcome`
- [ ] add `ContextRequest::include_telemetry` (default `false`) and populate it from the pre-trim and post-trim snapshots already captured in `apply_payload_budgets`
- [ ] classify `outcome` as `ok`, `empty`, `ambiguous`, `budget_exhausted`, or `error` at the `ContextEngine::build` boundary in `packages/atlas-review/src/context/mod.rs`
- [ ] keep `ContextResult::telemetry` optional and skipped when absent so default output stays byte-identical
- [ ] add `--telemetry` to `atlas context` and `atlas review-context`, and `include_telemetry` to MCP `get_context` and `get_review_context`
- [ ] add `atlas-metrics.workspace = true` to `packages/atlas-review/Cargo.toml`
- [ ] add `atlas-metrics` families `context_builds` (labeled by intent and outcome), `context_duration_ms`, `context_payload_bytes`, and `context_payload_tokens` (labeled by intent)
- [ ] extend `packages/atlas-review/benches/context_bench.rs` to emit telemetry JSON for a fixed corpus covering symbol, file, review, change-set, oversized-context, and provenance-sensitive requests
- [ ] add tests for requested/emitted count parity, outcome classification, metric recording, default-output byte equality, and telemetry JSON shape
- [ ] document telemetry fields, metric names, and the fixed corpus in `wiki/context-telemetry.md`

##### R7.2 — Payload metadata slimming

- [ ] add `ContextResult::repo_provenance: Option<RepoProvenance>` and hoist provenance from emitted `SelectedNode`, `SelectedFile`, and `SavedContextSource` items when every item shares the same provenance
- [ ] keep per-item `repo_provenance` when the result spans multiple repos or when provenance differs across items
- [ ] add `ContextRequest::include_ranking_evidence` (default `false`) and omit `context_ranking_evidence` from nodes, content assets, and saved-context sources when disabled
- [ ] keep `selection_reason` and `relevance_score` on every emitted item, node identity fields on graph items (`qualified_name`, `file_path`, `line_start`, `line_end`), and `source_id` on content and saved-context items
- [ ] add `--include-ranking-evidence` to `atlas context` and `atlas review-context`, and `include_ranking_evidence` to MCP `get_context` and `get_review_context`
- [ ] update MCP output schemas, `MCP_TOOLS.md`, CLI JSON docs, and goldens for the new default shape
- [ ] compare emitted bytes and tokens against the current schema on the fixed corpus, with acceptance threshold `>= 15%` token reduction at unchanged recall and unchanged required-evidence retention
- [ ] add tests for uniform-provenance hoisting, multi-repo provenance preservation, opt-in ranking evidence, and CLI/MCP parity in both metadata modes

##### Rejected experiment

- [ ] do not add `PostRetrievalCompactionMode`, provider-backed compaction, or shadow compaction unless telemetry shows a measured text-bearing share of at least 10% of emitted bytes and metadata slimming has already been applied

Why:
- measured payloads are dominated by repeated structural metadata, so hoisting and opt-in evidence removal are the real token lever
- telemetry makes selection and budget regressions visible without a lossy post-selection rewrite
- keeping provenance lossless preserves graph evidence and existing provenance-sensitive gates

#### Patch completion criteria

This patch is complete when:

- [ ] context telemetry is recorded for every build and exposed without changing default output
- [ ] the fixed corpus covers symbol, file, review, change-set, oversized-context, and provenance-sensitive requests
- [ ] hoisted provenance and opt-in ranking evidence reduce emitted tokens by at least 15% on the corpus
- [ ] default and multi-repo payloads keep all required evidence fields and pass updated MCP/CLI parity tests
- [ ] no text compactor or external provider is added to the context path

---


### Runtime Event Classification and Resume Grouping Patch

Runtime event capture, redaction, artifact routing, hook policy, and resume grouping are already shipped. The event-to-graph link table and context-engine runtime-event retrieval are dropped: they add a new session schema, heuristic symbol linking, ranking and budget paths, and noisy cross-session signal without evidence that current context misses runtime evidence.

This patch keeps only the cheap, deterministic layer that improves existing resume and wake-up surfaces:

- classify bounded runtime output with stable rules
- keep event types unchanged and store classification as metadata
- extend resume and wake-up grouping with classified failures and build/test summaries

Rules:

- keep classification deterministic, local, and non-LLM
- keep `SessionEventType` values unchanged; classification is metadata
- never store raw matched output text in classification metadata
- bound classifier input bytes and matched fields through policy constants
- keep resume and wake-up output deterministic and budget-trimmed
- do not add an event-to-graph link table, graph linking, or context-engine runtime-event retrieval in this patch
- do not write runtime data to `worldtree.db`

Implementation structure:

#### Patch X1 — Deterministic runtime classification

- [ ] add `packages/atlas-agent-events/src/classification.rs`
- [ ] define `RuntimeClassification { kind, severity, rule_id, matched_fields }` with serde `snake_case`
- [ ] define `RuntimeClassificationKind` with exact values `panic`, `exception`, `stacktrace`, `compiler_error`, `test_failure`, `test_success`, `build_success`, `deprecation_warning`, `dead_code_warning`, `permission_denied`, `command_timeout`, `graph_not_ready`, and `retrieval_failure`
- [ ] define `RuntimeClassificationSeverity` with exact values `error`, `warning`, and `info`
- [ ] add `classify_runtime_payload(payload: &Value) -> Option<RuntimeClassification>`
- [ ] scan bounded text from `stderr`, `stdout`, `output`, `error`, `message`, `result`, and `extra` fields up to `MAX_CLASSIFIER_INPUT_BYTES` (16 KiB)
- [ ] reuse `extract_hook_status`, `extract_hook_command`, and `extract_tool_name` from `packages/atlas-agent-events/src/payload.rs`
- [ ] classify `permission_denied` and `command_timeout` from status and error text before output-pattern rules
- [ ] classify `panic`, `exception`, and `stacktrace` from stable markers (`panicked at`, `RUST_BACKTRACE`, `Traceback (most recent call last)`, `Exception in thread`, and stack-frame markers)
- [ ] classify `compiler_error` from `error[E`, `error:`, `failed to compile`, and `--> file:line:col` markers
- [ ] classify `test_failure` from `test result: FAILED`, `FAILED tests/`, `failures:`, and `error: test failed`
- [ ] classify `test_success` from `test result: ok`, `passed`, and exit status `ok` on a test command
- [ ] classify `build_success` from `Finished`, `Build succeeded`, and exit status `ok` on a build command
- [ ] classify `deprecation_warning` and `dead_code_warning` from warning text and `unused` or `deprecated` markers
- [ ] classify `graph_not_ready` from `graph_stale`, `graph is stale`, `graph_unavailable`, and readiness error codes
- [ ] classify `retrieval_failure` from `retrieval_index_unavailable`, `content_store_unavailable`, and retrieval error codes
- [ ] return `None` when no rule matches; never emit an `other` fallback
- [ ] record only rule ids and matched field names in `matched_fields`; never record raw text
- [ ] add `MAX_CLASSIFIER_INPUT_BYTES` and `MAX_CLASSIFIER_MATCHED_FIELDS` to `packages/atlas-agent-events/src/policy.rs`
- [ ] add unit tests per kind, rule precedence, bounded input, no-match behavior, and absence of raw text in output

#### Patch X2 — Attach classification to hook events

- [ ] call `classify_runtime_payload` from `build_hook_event_metadata` in `packages/atlas-agent-events/src/metadata.rs`
- [ ] add `classification` to hook metadata only when classification returns `Some`
- [ ] keep `SessionEventType` and priority selection unchanged
- [ ] keep classification deterministic so identical raw input produces the identical enriched payload and event hash
- [ ] clamp `matched_fields` to `MAX_CLASSIFIER_MATCHED_FIELDS` and deduplicate field names
- [ ] add tests proving panic, compiler error, test failure, test success, permission denial, timeout, no-match, and repeated-input dedupe under `dedup_window_secs`

#### Patch X3 — Resume grouping and wake-up surfacing

- [ ] add `payload_classification(payload: &Value) -> Option<&Value>` in `packages/atlas-session/src/store/resume.rs` that reads `hook_metadata.classification` and top-level `classification`
- [ ] extend resume `unresolved_errors` entries with `classification.kind`, `classification.severity`, and `classification.rule_id` in `packages/atlas-session/src/store/resume.rs`
- [ ] extend resume `recent_commands` entries with optional classification metadata
- [ ] add `recent_build_test_summaries` to the resume snapshot with `kind`, `command`, `status`, `classification`, and `at`, capped at 5 entries
- [ ] add `recent_runtime_failures` to `WakePack` in `packages/atlas-agent-events/src/wake_pack.rs`, sourced from resume `unresolved_errors` filtered to `severity = error`
- [ ] cap `recent_runtime_failures` with the existing wake-pack budget item limit
- [ ] keep snapshots deterministic by sorting failures by `at` then rule id and deduplicating identical entries
- [ ] add `recent_build_test_summaries` to the trim list in `trim_snapshot_to_limit`
- [ ] add tests for classified errors, build/test summaries, caps, deterministic ordering, and byte-identical output when no classification exists
- [ ] update the MCP `wake_up` output schema and regenerate `MCP_TOOLS.md`
- [ ] update `wiki/session-memory-fallback.md` and `wiki/memory-architecture.md` for the new metadata and wake-up field
- [ ] update CLI and MCP snapshots for `resume_session` and `wake_up`

#### Deferred and rejected work

- [ ] do not add an `event_links` table or event-to-graph graph linking
- [ ] do not add context-engine `include_runtime_events` or runtime-event retrieval
- [ ] do not add a new `atlas-extractor` crate
- [ ] do not re-plumb `ContentStore::route_output` thresholds through config in this patch
- [ ] do not parse stacktraces into symbols; classification never creates graph links

#### Patch completion criteria

- [ ] classification is deterministic, local, and stored only as metadata
- [ ] `SessionEventType` values and repeated-input dedupe behavior are unchanged
- [ ] no raw matched output text is persisted in classification
- [ ] resume snapshot includes classified errors and build/test summaries within budget
- [ ] wake-up includes bounded recent runtime failures
- [ ] no runtime data is written to `worldtree.db`
- [ ] no event-to-graph link table or context-engine runtime retrieval is added

---

### Reachability Measurement and Rust Guard Patch

`resolve_same_package_target` in `packages/atlas-engine/src/call_resolution.rs` resolves weak cross-file calls by Cargo/npm/go owner membership plus same-directory proximity. That owner is a package, not a compiler inclusion unit, so a candidate can be in an orphan file or a separate crate target and still receive a `same_package` edge.

Measured on this repository: 6,433 `same_package` call edges (8.7% of edges), but only 3 cross-crate-root edges, all valid `fuzz/fuzz_targets` to `fuzz/src` dependencies. That is not enough evidence to build multi-language reachability indexes first.

This patch is split into two parts. Part A ships regardless and makes weak evidence visible and measurable. Part B is gated on Part A measurements and starts with Rust only.

Rules:

- measure before building a guard; do not add providers from assumption
- keep every weak-edge decision deterministic and explainable
- treat incomplete indexes as `Unknown`, never `Unreachable`
- keep direct import-backed and parser-proven edges untouched
- keep `packages/atlas-engine/src/call_resolution.rs` below 1,000 LOC; extract new logic into new modules
- store reachability provenance in `Edge.extra_json`; do not add an edge column
- preserve `atlas_repo::CanonicalRepoPath` identity for every file key
- never infer reachability by scanning directories alone

Implementation structure:

#### Part A — `cross_file_links` provenance and weak-edge measurement

- [ ] add `SemanticReferenceEdge { source_file, target_qn, edge_kind, confidence_tier, extra_json }` in `packages/atlas-store-sqlite/src/store/context.rs`
- [ ] add `Store::semantic_reference_edges(file_path, max_edges) -> Vec<SemanticReferenceEdge>` selecting `e.kind`, `e.confidence_tier`, and `e.extra_json`
- [ ] keep `Store::files_referencing_symbols_in` as a thin wrapper over the new query for existing callers
- [ ] add `confidence_tier: Option<String>` and `reachability: Option<String>` to `SemanticLink` in `packages/atlas-search/src/semantic.rs`
- [ ] add `cross_file_links_with_options(store, file_path, limit, include_weak)` and keep `cross_file_links` as the default weak-edge-excluding path
- [ ] define weak edges as `confidence_tier = 'same_package'`; exclude them by default
- [ ] keep every edge whose `confidence_tier` is not `same_package` in the default result
- [ ] add `include_weak: Option<bool>` (default `false`) to MCP `cross_file_links` in `packages/atlas-mcp/src/tools/graph.rs`
- [ ] add `confidence_tier` and `reachability` to the `cross_file_links` output schema in `packages/atlas-mcp/src/tools/registry/output_schemas/relationships.rs`
- [ ] add `reachability_measurement` to `atlas doctor --json` and MCP `doctor` with:
  - [ ] `total_same_package_edges`
  - [ ] `sampled_edges`
  - [ ] `cross_crate_root_edges`
  - [ ] `orphan_candidate_edges`
  - [ ] `unknown_edges`
  - [ ] `measurement_source` and `sample_limit`
- [ ] add `packages/atlas-engine/src/reachability_measure.rs` with `ReachabilityMeasurement` and `measure_same_package_edges(store, repo_id, sample_limit)`
- [ ] classify Rust file paths for measurement only as `lib`, `main`, `example`, `test`, `bench`, or `other` using standard Cargo layout names
- [ ] count `cross_crate_root_edges` when caller and candidate path kinds differ and neither is `other`
- [ ] count `orphan_candidate_edges` when caller and candidate path kinds differ and the candidate path kind is `other`
- [ ] add `packages/atlas-engine/benches/reachability_bench.rs` emitting the measurement as JSON
- [ ] add a fixture repo with `src/lib.rs`, `src/bin/tool.rs`, `tests/integration.rs`, and an orphan `src/orphan.rs` that receives a wrong `same_package` edge
- [ ] add tests for weak-edge filtering, default `cross_file_links` output, `include_weak = true`, provenance fields, measurement counts, and fixture stability
- [ ] update existing `cross_file_links` tests and goldens for the default weak-edge filtering behavior
- [ ] document weak-edge filtering and measurement fields in `MCP_TOOLS.md`, `wiki/reachability.md`, and the `cross_file_links` manual

#### Part B entry gate

Do not start Part B until all gates pass:

- [ ] `orphan_candidate_edges / sampled_edges >= 0.05` on at least one representative multi-crate repository
- [ ] the fixture reproduces at least one wrong `same_package` edge that `cross_file_links` would otherwise report
- [ ] Part A ships with default weak-edge filtering and provenance

#### Part B — Rust module reachability guard

- [ ] extract `callable_candidates`, `resolve_same_package_target`, `cached_owner_id`, `same_dir`, and `unique_node` into `packages/atlas-engine/src/package_resolution.rs` without behavior changes
- [ ] add `packages/atlas-engine/src/reachability.rs`
- [ ] define `ReachabilityStatus` with exact values `verified`, `unreachable`, `unknown`, and `not_applicable`
- [ ] define `ReachabilityResult` with `Reachable`, `Unreachable`, and `Unknown { reason }`
- [ ] define `CrateRootKind` with exact values `lib`, `bin`, `example`, `test`, and `bench`
- [ ] define `CrateRoot { root_file, kind, reachable_files, has_unknown_modules, has_cfg_modules }`
- [ ] define `RustReachabilityIndex { owner_id, roots, unresolved_mods, index_complete }`
- [ ] add Rust module facts to parser output in `packages/atlas-parser/src/lang/rust/emitter.rs` and `facts.rs`:
  - [ ] `inline: bool` on every module node
  - [ ] `module_file: Option<String>` for external `mod foo;` declarations resolved to `foo.rs` or `foo/mod.rs`
  - [ ] `has_cfg: bool` when the module has a `#[cfg(...)]` attribute
  - [ ] `has_path_attr: bool` and `path_attr_value` when `#[path = "..."]` is present
- [ ] store module facts in `Node.extra_json` so build and incremental update can reconstruct the tree from the store
- [ ] add parser tests for `mod foo;`, `foo/mod.rs`, inline `mod foo {}`, `#[path]`, and `#[cfg]`
- [ ] implement `RustReachabilityIndex::from_store(store, repo_id, owner_id)` over persisted module nodes and file nodes
- [ ] mark `index_complete = false` when any module lacks facts, has `has_cfg`, has `has_path_attr`, resolves outside the owner, resolves to a missing file, or depends on `include!` or macro-generated declarations
- [ ] add `ReachabilityGuard` keyed by `(language, owner_id)` with `is_reachable_from_same_root(caller, candidate) -> ReachabilityResult`
- [ ] build the guard once per `reconcile_call_targets` run from the store, not from `ParsedFile`, so incremental update works
- [ ] query the guard only for `same_package` fallback candidates after receiver-hint filtering
- [ ] retain `Reachable`; suppress `Unreachable`; retain `Unknown` as existing fallback behavior
- [ ] when `Unreachable` rejects a prior `same_package` edge, reset the edge target to the original callee name from `extra_json.callee_name`, clear `confidence_tier`, and mark `extra_json.unresolved = true`
- [ ] write `extra_json.reachability = verified | unknown` on accepted `same_package` edges and `extra_json.reachability = not_applicable` for non-Rust or provider-absent resolution
- [ ] add `[reachability]` config with `enabled` (default `true`), `budget_ms` (default `250`), and `max_owners_per_run` (default `64`)
- [ ] validate `budget_ms >= 1` and `max_owners_per_run >= 1` with config-key-named errors
- [ ] skip owners with no changed files during incremental update
- [ ] mark indexes `Unknown` with `budget_exhausted` when `budget_ms` is exceeded
- [ ] emit `reachability` summary in `atlas build --json`, `atlas update --json`, `atlas doctor --json`, and MCP `doctor` with owners indexed, owners skipped, unknown reasons, unresolved mods, and weak edges accepted or rejected
- [ ] extend `atlas db_check` only with validation of persisted `extra_json.reachability` values; do not report in-memory index stats from `db_check`
- [ ] add tests for crate-root separation, orphan files, `mod.rs`, inline modules, `#[path]`, `#[cfg]`, budget exhaustion, receiver hints, direct-import bypass, incremental deleted-file cleanup, stale-target clearing, and persisted provenance
- [ ] add `wiki/reachability.md` covering provider scope, completeness, `Unknown` behavior, and diagnostics

#### Out of scope

- [ ] do not add Go, JavaScript, TypeScript, Python, Java, Kotlin, C#, C, C++, PHP, Ruby, or Bash providers until Atlas has owner or build-metadata facts for them
- [ ] do not ingest compiler databases, Gradle/Maven models, `tsconfig` entrypoint graphs, or `.NET` project includes in this patch
- [ ] do not infer reachability from directory scans
- [ ] do not add a new edge column or a new database
- [ ] do not suppress import-backed, same-file, or parser-proven edges

#### Patch completion criteria

- [ ] Part A produces deterministic weak-edge measurements and ships independent of Part B
- [ ] `cross_file_links` hides weak `same_package` edges by default and exposes `confidence_tier` plus `reachability`
- [ ] `include_weak = true` restores the previous weak-edge output
- [ ] Part B starts only after its entry gate passes
- [ ] Rust `same_package` edges carry `verified`, `unknown`, or `not_applicable` provenance
- [ ] incomplete Rust module indexes never emit `unreachable`
- [ ] unreachable candidates cannot leave a stale `same_package` target
- [ ] `packages/atlas-engine/src/call_resolution.rs` stays below 1,000 LOC
- [ ] non-Rust languages remain `not_applicable` or `unknown`
- [ ] `doctor` reports provider coverage, unknown reasons, budget exhaustion, and unresolved modules
- [ ] `db_check` validates persisted provenance only

---

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

### Runtime Policy Surface — Deferred

Not worth implementing now. Static AGENTS/CLAUDE instructions, install bootstrap text, MCP prompts, and hook capture already carry graph-first and minimal-first workflow guidance. A runtime `agent_instructions` tool plus hook-boundary enforcement adds a policy cache, version negotiation, protected-tool blocking, and host-specific fallback without measured compliance gain.

Blockers:

- hard enforcement conflicts with the existing best-effort hook contract and must not block primary CLI/MCP flow
- hosts without native hooks cannot enforce preload, so behavior diverges by host
- protected-tool blocking can break valid sessions when policy preload is delayed or unavailable
- static instructions and VR3 escalation metadata already cover the guidance

Revisit only if:

- a host provides reliable mandatory pre-tool hooks
- measured cases show agents ignoring static guidance
- runtime policy versioning is required for compliance

If revisited, limit the first step to a read-only `agent_instructions` MCP tool returning compact policy metadata. Do not add hook-boundary blocking, policy caches, or protected-tool enforcement.

---

## Additional Backlog

- [ ] add CI-visible parser cache hit-ratio metric and fail when cache reuse drops below configured threshold
- [ ] add thin LSP shim that maps Atlas query/context/impact/reference flows onto standard LSP requests

---

## Part XI — Tokenizer-Backed Context Budget Accounting

Tokenizer-backed context budget accounting is shipped. See SHIPPED.md for details.

---

## Part XII — RAG Adaptation Roadmap (VikingMem, VikingRAG, TrieHI)

Source of truth: `docs/PLAN-RAG.md`. All VikingMem memory-operator tracks, VikingRAG + TrieHI retrieval tracks, and RAG evaluation gates are shipped; see SHIPPED.md for the typed operator algebra, structured consolidation and patch application, two-path recall, progressive overview levels and `atlas://` resources, directory-semantic query scope and TrieHI indexing, evidence sufficiency escalation, query-conditioned experience edges, and token-efficiency evaluation.

Core Design Rules:

- keep the three-database boundary: `worldtree.db` for static graph facts, `context.db` for chunked artifacts, `session.db` for events, memories, memoir, overviews, and experience edges; never add a fourth database
- keep the non-LLM default: operators, EUA patching, deterministic `TIME_COMPRESS`, DSQ scope filters, and evidence sufficiency checks run locally with no required LLM or embedding provider
- never drop merged or contradictory evidence: consolidation and `TIME_COMPRESS` preserve distinct facts from all inputs and keep `memory_supersessions` provenance
- bind overviews and experience shortcuts to `commit_sha` plus canonical file hashes; stale links never surface as fresh structural evidence
- canonicalize every path-derived scope, cache, or identity key through `atlas_repo::CanonicalRepoPath`
- sync every new flag, filter, enum value, and JSON field across CLI, MCP wrappers, and `MCP_TOOLS.md`
- respect the 1K LOC rule: add new modules instead of growing `packages/atlas-session/src/store/memory.rs`, `packages/atlas-session/src/store/memoir.rs`, `packages/atlas-session/src/store/types.rs`, and `packages/atlas-mcp/src/resources.rs`
- implement only from the published algorithm descriptions; never copy AGPL-3.0 OpenViking code

Rules apply to every checklist item in this part.

Remaining deferred memory-operator wiring:

- [ ] deferred (VM3/ICM-C): route `feedback_record` confidence penalties through `Count` plus `Set(confidence_penalty)` once an analysis-rule memory entity exists; `feedback_records` remains the authoritative table
- [ ] deferred: route `global_symbol_access` and `global_file_access` updates through `Sum` plus `Max(last_accessed)` when those counters become memory entities; they currently own dedicated tables
