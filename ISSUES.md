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

- Part V. Remaining focused follow-up patches: Retrieval Follow-Up Patch remainder, Runtime Event Enrichment and Graph Linking Patch, Rust Reachability Guard Patch, Shared Parser Query Migration Patch remainder, Context Escalation Contract Patch, Dynamic Agent Policy and Hook Enforcement Patch.
- Part XII. Remaining RAG follow-up: deferred memory-operator wiring only. VM1–VM3, VR1–VR4, and VQ1 are shipped; see SHIPPED.md.

## Cross-Cutting Track Map

- Retrieval and search follow-ups: Retrieval Follow-Up Patch remainder
- Context continuity and runtime memory: Runtime Event Enrichment and Graph Linking Patch
- Graph safety and workflow: Context Escalation Contract Patch
- Rust parser correctness: Rust Reachability Guard Patch, Shared Parser Query Migration Patch remainder
- Agent policy and enforcement: Dynamic Agent Policy and Hook Enforcement Patch
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


### Runtime Event Enrichment and Graph Linking Patch

Atlas already has session events, adapter extraction helpers, content-store artifact routing, resume snapshots, saved-context retrieval, and context-engine saved-context merge. Do not replace that foundation with a parallel extractor system. Extend it with deterministic enrichment that turns runtime activity into bounded, graph-aware memory while preserving the existing storage boundaries: graph facts stay in `worldtree.db`, large/runtime artifacts stay in `context.db`, and session timelines stay in `session.db`.

#### Patch X1 — Scope and crate boundary

- [ ] define this as enrichment over existing `atlas-session`, `atlas-contentstore`, and `atlas-adapters`
- [ ] avoid creating `packages/atlas-extractor` unless extraction logic grows large enough to justify a separate crate
- [ ] if a new crate is created later, require it to depend on service APIs, not write SQLite directly
- [ ] keep extractor pipeline deterministic, local, and non-LLM
- [ ] keep extractor best-effort; extraction failure must not block primary CLI/MCP tool output
- [ ] keep raw runtime output out of graph DB
- [ ] document storage ownership:
  - [ ] `worldtree.db` stores static code graph facts only
  - [ ] `session.db` stores bounded event metadata and references
  - [ ] `context.db` stores large artifacts, chunks, previews, and searchable runtime text

Why:
- existing continuity architecture already solved session/content boundaries
- a parallel extractor crate or DB path would duplicate behavior and increase drift

#### Patch X2 — Raw input envelope and deterministic event enrichment

- [ ] define a `RuntimeInput` / `RawActivityInput` envelope for enrichment:
  - [ ] `frontend` (`cli`, `mcp`, adapter host)
  - [ ] `session_id`
  - [ ] `repo_root`
  - [ ] `input_kind`
  - [ ] `tool_or_command`
  - [ ] `status`
  - [ ] `stdout_preview`
  - [ ] `stderr_preview`
  - [ ] `artifact_source_id`
  - [ ] `files`
  - [ ] `metadata`
  - [ ] `created_at`
- [ ] define enriched output that maps onto existing `NewSessionEvent` payloads:
  - [ ] `event_type`
  - [ ] `summary`
  - [ ] `symbols`
  - [ ] `file_paths`
  - [ ] `source_ids`
  - [ ] `classification`
  - [ ] `confidence`
  - [ ] `metadata`
- [ ] enrich existing event constructors rather than bypassing them:
  - [ ] `extract_cli_event`
  - [ ] `extract_graph_event`
  - [ ] `extract_context_event`
  - [ ] `extract_reasoning_event`
  - [ ] `extract_user_event`
  - [ ] `extract_tool_event`
  - [ ] `normalize_event`
- [ ] keep outputs canonical JSON so existing event hashing and dedupe remain stable
- [ ] add tests proving same input produces same enriched event and hash

Why:
- enrichment should preserve existing event persistence and dedupe semantics
- deterministic input/output keeps resume snapshots stable

#### Patch X3 — Rule-based classification

- [ ] add bounded rule-based classifiers for runtime activity:
  - [ ] panic
  - [ ] exception
  - [ ] stacktrace
  - [ ] compiler error
  - [ ] test failure
  - [ ] test success
  - [ ] build success
  - [ ] deprecation warning
  - [ ] unused/dead-code warning
  - [ ] permission denied
  - [ ] command timeout
  - [ ] graph stale/readiness warning
  - [ ] retrieval/content-store failure
- [ ] map classifications to existing `SessionEventType` values where possible:
  - [ ] `ERROR`
  - [ ] `COMMAND_RUN`
  - [ ] `COMMAND_FAIL`
  - [ ] `CONTEXT_REQUEST`
  - [ ] `REASONING_RESULT`
  - [ ] `FILE_READ`
  - [ ] `FILE_WRITE`
  - [ ] `GRAPH_BUILD`
  - [ ] `GRAPH_UPDATE`
- [ ] add new event types only when existing types cannot represent the event safely
- [ ] include classification metadata instead of exploding event-type count:
  - [ ] `classification.kind`
  - [ ] `classification.severity`
  - [ ] `classification.rule_id`
  - [ ] `classification.matched_fields`
- [ ] add tests for error parsing, warning parsing, test summary parsing, and no-match behavior

Why:
- event type should stay stable; detailed meaning belongs in structured metadata
- deterministic classifiers provide useful memory without LLM inference

#### Patch X4 — Artifact routing before session insertion

- [ ] run all raw stdout/stderr/tool-result blobs through existing content-store routing before session insertion
- [ ] define routing thresholds through the central budget policy:
  - [ ] `small_output_bytes`
  - [ ] `preview_output_bytes`
  - [ ] `large_output_bytes`
  - [ ] `max_runtime_artifact_bytes`
- [ ] keep session event payloads bounded:
  - [ ] small output may be stored inline only when safe and redacted
  - [ ] medium output stores preview plus `source_id`
  - [ ] large output stores pointer only
- [ ] use `ContentStore::route_output` / saved-context artifact routing instead of a new artifact path
- [ ] index routed artifacts with metadata:
  - [ ] `session_id`
  - [ ] `source_type`

#### Patch X5 — Graph linking without storing runtime data in graph DB

- [ ] link enriched events to graph facts by stable identifiers, not raw node IDs alone
- [ ] store links in session/content side tables, not `worldtree.db`
- [ ] define link records:
  - [ ] `event_id`
  - [ ] `session_id`
  - [ ] `repo_root`
  - [ ] `qualified_name`
  - [ ] `canonical_file_path`
  - [ ] optional `node_id`
  - [ ] optional `file_id`
  - [ ] `link_kind`
  - [ ] `confidence`
  - [ ] `graph_last_indexed_at`
- [ ] prefer canonical identifiers:
  - [ ] canonical repo path
  - [ ] qualified name
  - [ ] kind
  - [ ] line span when available
- [ ] treat `node_id` and `file_id` as cache hints only because graph rebuilds can change row IDs
- [ ] make graph linking best-effort:
  - [ ] events with no graph target remain valid runtime memory
  - [ ] ambiguous symbols store candidate list and ambiguity metadata
  - [ ] stale graph state records `safe_to_answer=false` for link-derived claims when needed
- [ ] add tests for exact symbol, file path, ambiguous symbol, stale graph, and graph-missing cases

Why:
- runtime memory should be graph-aware without mutating graph facts
- stable identifiers survive rebuilds better than SQLite row IDs

#### Patch X6 — Readiness, identity, and budget integration

- [ ] run graph linking only through canonical graph readiness state
- [ ] define behavior by execution state:
  - [ ] `fresh` -> resolve and link normally
  - [ ] `stale` -> link with freshness warning and stale metadata
  - [ ] `partial` -> link only when completeness requirements are met
  - [ ] `corrupt` -> skip graph linking and store runtime event without graph links
- [ ] require canonical path identity before any event/file/artifact key hashing
- [ ] apply central budget policy to:
  - [ ] classifier input bytes
  - [ ] number of symbols extracted
  - [ ] number of file paths extracted
  - [ ] number of graph lookup candidates
  - [ ] number of links stored
  - [ ] artifact preview bytes
- [ ] emit enrichment budget metadata:
  - [ ] `budget_hit`
  - [ ] `partial`
  - [ ] `safe_to_answer`
  - [ ] omitted symbol/file/link counts
- [ ] add tests for stale/partial/corrupt graph behavior and budget truncation

Why:
- runtime enrichment must follow the same safety rules as graph-backed tools
- extraction can otherwise become another unbounded path

#### Patch X7 — Context-engine integration

- [ ] extend context engine to include enriched runtime events only when requested or relevant
- [ ] add request controls:
  - [ ] `include_runtime_events`
  - [ ] `runtime_event_limit`
  - [ ] `runtime_artifact_limit`
  - [ ] `runtime_since`
  - [ ] `runtime_session_id`
- [ ] retrieve runtime memory by:
  - [ ] linked symbol
  - [ ] canonical file path
  - [ ] session id
  - [ ] classification kind
  - [ ] artifact source id
- [ ] merge runtime memory under graph/content companion policy
- [ ] expose source kind:
  - [ ] `runtime_event`
  - [ ] `runtime_artifact`
  - [ ] `saved_context`
- [ ] include selection reason and ranking evidence:
  - [ ] same symbol
  - [ ] same file
  - [ ] recent error
  - [ ] same session
  - [ ] direct artifact reference
- [ ] keep runtime context bounded and preview-only by default
- [ ] add tests for context with graph-only, saved-context-only, runtime-event-only, and mixed graph/runtime inputs

Why:
- runtime memory is useful only when it participates in context selection
- it must not bypass existing context budgets or ranking rules

#### Patch X8 — CLI, MCP, and hook integration

- [ ] integrate enrichment with existing CLI adapter event flow
- [ ] integrate enrichment with MCP tool handler boundaries
- [ ] keep MCP session event persistence best-effort and non-blocking
- [ ] avoid duplicating `save_context_artifact`; reuse existing tool and content routing
- [ ] update hook integration roadmap so host hooks emit enriched inputs through service APIs
- [ ] ensure generated hooks never write SQLite directly
- [ ] add command/tool metadata for:
  - [ ] command start
  - [ ] command finish
  - [ ] tool result
  - [ ] permission decision
  - [ ] compaction boundary
  - [ ] session end
  - [ ] error/failure
- [ ] add integration tests for CLI, MCP, and bridge-file fallback event enrichment

Why:
- runtime memory should come from existing adapters and hooks
- host-specific capture gaps must reduce enrichment quality, not break commands

#### Patch X9 — Resume snapshot enrichment

- [ ] include enriched runtime signals in resume snapshots:
  - [ ] recent errors
  - [ ] recent failed commands
  - [ ] recent successful build/test summaries
  - [ ] linked symbols
  - [ ] linked files
  - [ ] artifact references
  - [ ] active unresolved runtime issues
- [ ] group by category and severity
- [ ] include retrieval hints instead of raw artifact content
- [ ] cap snapshot contribution by budget policy
- [ ] make snapshot rendering deterministic
- [ ] add snapshot tests for enriched errors, artifact references, and linked symbols

Why:
- resume should recover useful runtime state without replaying history
- enriched events make snapshots more useful while staying compact

#### Patch X completion criteria

- [ ] runtime enrichment extends existing session/content/adapters architecture without replacing it
- [ ] no runtime data is stored in graph DB
- [ ] large runtime outputs route through content store before session insertion
- [ ] redaction policy supports runtime-loaded external rule files with validation
- [ ] event-to-graph links use stable identifiers and treat row IDs as optional cache hints
- [ ] graph linking obeys readiness state and budget policy
- [ ] context engine can merge runtime events/artifacts with graph and saved context under one bounded ranking policy
- [ ] CLI, MCP, and hook flows feed enrichment best-effort
- [ ] resume snapshots include compact enriched runtime signals
- [ ] tests cover classification, artifact routing, graph linking, context integration, and resume enrichment

---


### Code Reachability Guard Patch — All Languages

Atlas cross-file call resolution can over-report references for orphan, excluded, generated, or stale files because `same_package` heuristics use owner membership, same-directory proximity, and simple-name matching without proving that a language build can include both caller and candidate. `cross_file_links` then treats these weak heuristic edges as evidence that files are connected.

`resolve_same_package_target` in `packages/atlas-engine/src/call_resolution.rs` is language-independent. It resolves direct import-backed edges first, then applies its weak `same_package` fallback across supported languages. This patch adds a shared, capability-driven reachability guard only to that fallback. Exact, import-backed, and parser-proven edges remain valid without a reachability provider.

Design overview:

1. **`CodeReachabilityIndex`** — language-provider output, built once per owner/build unit during parse/build/update. It records canonical roots and files included from each root. It is in-memory build state, never persisted to `worldtree.db`.

2. **`ReachabilityProvider`** — language-specific builder/query implementation. Provider semantics follow compiler/build-system inclusion rules; no generic filesystem scan may infer source reachability.

3. **`ReachabilityGuard`** — shared wrapper keyed by `(language, owner_id)`, passed into `resolve_same_package_target`. It answers whether caller and candidate occur under at least one common inclusion root.

4. **Provenance** — add `reachability: ReachabilityStatus` edge metadata: `Verified`, `Unreachable`, `Unknown`, or `NotApplicable`. Do not use a boolean: it cannot distinguish legacy/provider-absent fallback from a deliberate check.

`ReachabilityResult` variants: `Reachable`, `Unreachable`, and `Unknown { reason }`. `Unknown` preserves current fallback behavior but records unverified provenance; `Unreachable` suppresses only weak `same_package` edges. This avoids both false positives and regressions for languages/configurations without providers.

#### Patch R1 — Rust `CrateReachabilityIndex` provider and builder

- [ ] define `CrateReachabilityIndex` struct in `atlas-engine` (or `atlas-parser` if mod-walk lives there):
  - [ ] `owner_id: String` — Cargo manifest key, matches existing `owner_id` field
  - [ ] `crate_roots: Vec<CrateRoot>` — one entry per compiled crate target
  - [ ] each `CrateRoot`:
    - [ ] `root_file: CanonicalRepoPath` — e.g. `src/lib.rs`, `src/main.rs`, `examples/foo.rs`
    - [ ] `crate_kind: CrateKind` — `Lib`, `Bin`, `Example`, `Test`, `Bench`
    - [ ] `reachable_files: HashSet<CanonicalRepoPath>` — all files reachable via `mod` from this root
- [ ] implement `CrateReachabilityIndex::build(owner_id, manifest_path, parsed_files)`:
  - [ ] identify crate roots by standard Cargo layout heuristics: `src/lib.rs`, `src/main.rs`, `examples/*.rs`, `tests/*.rs`, `benches/*.rs`
  - [ ] respect `[[bin]]`, `[[example]]`, `[[test]]`, `[[bench]]` `path` overrides from `Cargo.toml` when parsed
  - [ ] walk `mod <name>;` declarations in each root file using already-parsed AST nodes (no re-parse)
  - [ ] resolve sibling `mod` paths relative to declaring file using Rust module path rules (`mod foo;` → `foo.rs` or `foo/mod.rs`)
  - [ ] recursively follow `mod` declarations up to a configurable depth cap (default: 64 levels)
  - [ ] treat `mod foo { ... }` inline modules as transparent (they do not add a new file, all their declarations remain in the declaring file)
  - [ ] treat unresolvable `mod` targets as absent rather than erroring out; record them in `unresolved_mods` for diagnostics
  - [ ] all file paths stored as `CanonicalRepoPath` via `atlas_repo::CanonicalRepoPath`
- [ ] expose `is_file_reachable(file: &CanonicalRepoPath) -> bool` helper that checks across all `CrateRoot` entries in the index
- [ ] expose `reachable_from_same_root(caller: &CanonicalRepoPath, candidate: &CanonicalRepoPath) -> bool` — returns `true` only when both files appear in the same `CrateRoot.reachable_files` set
- [ ] add unit tests:
  - [ ] standard `src/lib.rs` layout with one level of `mod`
  - [ ] nested `mod foo { mod bar; }` inline with sibling file
  - [ ] multi-target package: lib + bin + example each have separate reachable sets
  - [ ] orphan `.rs` file in same package directory not reachable from any crate root
  - [ ] unresolvable `mod` target is recorded but does not panic or block other mods
  - [ ] path identity: same file via different path strings produces one entry

Why:
- `owner_id` covers Cargo package membership, not Rust module-tree membership
- index must be built from AST, not filesystem, to stay consistent with parsed graph facts

#### Patch R2 — shared `ReachabilityProvider`, `CodeReachabilityIndex`, and `ReachabilityGuard` integration

- [ ] define `ReachabilityGuard` in `atlas-engine`:
  - [ ] wraps language providers and `HashMap<(String, String), CodeReachabilityIndex>` keyed by `(language, owner_id)`
  - [ ] `is_reachable_from_same_root(language, owner_id, caller, candidate) -> ReachabilityResult`
  - [ ] `ReachabilityResult` variants: `Reachable`, `Unreachable`, `Unknown { reason }`
  - [ ] `Unknown` falls back to existing heuristic behavior and records `reachability: Unknown`; only `Unreachable` blocks weak edge emission
- [ ] build `ReachabilityGuard` once per engine build/update run, before resolution pass
- [ ] thread `ReachabilityGuard` into `resolve_same_package_target` alongside existing `owner_cache`
- [ ] update `resolve_same_package_target` resolution order:
  1. filter candidates by `owner_id` (existing step — coarse package filter)
  2. apply receiver-hint filtering (existing step — keep)
  3. **new**: query `ReachabilityGuard` for each `same_owner_matches` candidate
  4. retain `Reachable`; suppress `Unreachable`; retain `Unknown` only as existing fallback candidates
  5. apply existing same-dir tie-break, preferring `Reachable` candidates over `Unknown`
  6. write `reachability: Verified` for reachable fallback edges and `reachability: Unknown` for provider-absent/inconclusive fallback edges
- [ ] add `reachability: ReachabilityStatus` to edge metadata or edge extra fields (stored in existing `metadata` JSON or new column)
- [ ] add regression tests:
  - [ ] orphan file in same Cargo package is rejected as same-package target after reachability filtering
  - [ ] live file reachable via `mod` chain is accepted as same-package target
  - [ ] receiver-hint still narrows candidates correctly after reachability filtering
  - [ ] absent index falls back gracefully and does not panic

Why:
- package membership alone is too broad; reachability narrows to files the compiler actually sees
- `IndexAbsent` fallback prevents breaking existing resolution for languages or layouts where index is not built

#### Patch R3 — Edge provenance and `cross_file_links` filtering for all languages

- [ ] audit `cross_file_links` query for heuristic-edge false positives in every provider-supported language:
  - [ ] identify whether `cross_file_links` joins only on edge existence or also on confidence tier
  - [ ] determine whether filtering at read time or write time is safer given incremental update semantics
- [ ] decide and document filter strategy:
  - [ ] **preferred**: filter at write time — do not persist `same_package` edges for unreachable candidates; `cross_file_links` naturally sees correct graph
  - [ ] **acceptable fallback**: filter at read time — exclude `reachability: Unreachable` heuristic edges; preserve `Unknown` edges while exposing unverified provenance
  - [ ] document chosen strategy in a code comment near the `cross_file_links` query
- [ ] ensure incremental update removes stale node rows and their inbound `same_package` edges when a file is deleted
  - [ ] verify existing node deletion cascade covers edge rows; add explicit edge cleanup if missing
- [ ] expose edge provenance in `cross_file_links` output:
  - [ ] add `confidence_tier` to `CrossFileLink` result struct if not already present
  - [ ] add `reachability: ReachabilityStatus` to `CrossFileLink` when available
- [ ] add tests:
  - [ ] orphan/excluded file shows zero heuristic `cross_file_links` inbound edges after a reachability-gated build
  - [ ] deleted file shows zero `cross_file_links` results after incremental refresh removes its nodes
  - [ ] import-backed edge still appears in `cross_file_links` regardless of reachability guard
  - [ ] provider-absent language preserves fallback edge with `reachability: Unknown`

Why:
- `cross_file_links` is the user-visible surface; false-positive heuristic edges here mislead dead-code and impact analysis
- write-time filtering is cleaner than read-time masking

#### Patch R4 — Diagnostics and observability for all providers

- [ ] expose reachability index stats in `atlas doctor` / `atlas db_check` output:
  - [ ] number of owners/build units with an index, grouped by language/provider
  - [ ] number of provider builds failed, skipped, or `Unknown`, grouped by reason
  - [ ] number of unresolved inclusion references, grouped by language
  - [ ] number of `same_package` edges grouped by `ReachabilityStatus`
- [ ] expose reachability status per file in `atlas status --json` or a dedicated debug command:
  - [ ] file is reachable from which inclusion root(s), with provider and language
  - [ ] file has no reachable root (orphan/excluded) or has `Unknown` reachability with reason
- [ ] log reachability provider build failures at `warn` level with owner/build-unit path; do not fail the build
- [ ] add MCP `doctor` response fields for reachability index health when data is available

Why:
- operators need to see whether the guard is active and which packages lack an index
- silent guard absence produces the same false positives as before, so visibility is required

#### Patch R5 — Language-provider rollout

- [ ] define provider capability and coverage contract for every language in `ParserRegistry`; unsupported providers return `Unknown`, never fabricate reachability
- [ ] implement providers in priority order:
  - [ ] Rust: Cargo targets plus parsed-AST `mod` traversal (R1)
  - [ ] Go: package directories, build constraints/tags, generated files, and distinct external/internal test packages
  - [ ] JavaScript/TypeScript: configured entrypoints plus static import/export graph, `tsconfig` paths, package exports, and extension rules
  - [ ] Python: configured import roots plus package/module imports; configured script and test roots remain valid execution roots
  - [ ] Java/Kotlin: Gradle/Maven source sets, module descriptors, and build dependency boundaries
  - [ ] C#: project compile includes, source generators where declared, and project/assembly boundaries
  - [ ] C/C++: compilation database translation units plus include graph; headers are reachable only through a translation unit
  - [ ] PHP/Ruby/Bash and remaining languages: provider only after build/framework entrypoint semantics are modeled; otherwise report `Unknown`
- [ ] use existing parsed AST/import facts where available; configuration/build metadata may be parsed as companion assets, but never infer reachability by scanning directories alone
- [ ] add per-provider fixtures covering reachable candidate, excluded/orphan candidate, multi-root project, provider failure/absence, and direct import-backed edge bypass
- [ ] expose provider name, status, root count, reachable file count, unresolved inclusion references, and `Unknown` reason in diagnostics

Why:
- language inclusion semantics differ: Rust module trees, Go packages, JS/TS entrypoint dependency graphs, Python import roots, JVM/.NET build source sets, and C/C++ translation units are not interchangeable
- phased capability avoids silently degrading valid resolution where Atlas has not yet modeled build semantics

#### Patch R completion criteria

- [ ] `CodeReachabilityIndex`, `ReachabilityProvider`, and tri-state `ReachabilityGuard` exist; Rust `CrateReachabilityIndex` is first provider
- [ ] `resolve_same_package_target` in `packages/atlas-engine/src/call_resolution.rs` filters only weak fallback candidates through `ReachabilityGuard`; direct/import-backed resolution remains unaffected
- [ ] `same_package` edges carry `ReachabilityStatus` provenance, with `Unknown` fallback explicit
- [ ] `cross_file_links` does not claim provider-confirmed orphan/excluded files are connected after a reachability-gated build
- [ ] incremental refresh removes deleted-file nodes and clears inbound edges
- [ ] `atlas doctor` reports provider coverage, unknown/failure reasons, and unresolved inclusion references
- [ ] tests cover every shipped provider: orphan/excluded rejection, live acceptance, receiver-hint interaction, `Unknown` fallback, deleted-file cleanup, direct import bypass, and `cross_file_links` false-positive regression

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

### Context Escalation Contract Patch

Atlas has compact context tools, review context, symbol lookup, neighbor tools, and wider traversal tools, but the preferred order is currently only hinted in prompts and installed instructions. Make the core agent workflow explicit: start with the smallest bounded graph context that can answer the question, then escalate only when evidence says broader context is needed.

#### Patch E1 — Define minimal-context-first workflow

- [ ] document canonical escalation order for review/change tasks:
  - [ ] `detect_changes` when files are unknown
  - [ ] `get_minimal_context` for first bounded triage
  - [ ] `get_review_context` only when changed-symbol, neighbor, or risk detail is needed
  - [ ] `explain_change` when deterministic risk/test-gap explanation is needed
  - [ ] `get_impact_radius` when explicit blast radius is needed
- [ ] document canonical escalation order for symbol/usage tasks:
  - [ ] `query_graph` / `resolve_symbol` first
  - [ ] `symbol_neighbors` for direct callers/callees/tests
  - [ ] `get_context` for bounded ranked context
  - [ ] `traverse_graph` only when one-hop context is insufficient
- [ ] define allowed reasons to escalate:
  - [ ] ambiguous symbol resolution
  - [ ] truncated result
  - [ ] missing caller/callee/test evidence
  - [ ] cross-file or cross-package risk
  - [ ] explicit user request for broader context
  - [ ] safety-critical uncertainty
- [ ] define anti-patterns:
  - [ ] starting review with full review context when minimal context is enough
  - [ ] using traversal before symbol resolution
  - [ ] using file search before graph tools answer structural questions
  - [ ] broad traversal without a bounded max depth and max nodes

Why:
- reduces token load and noisy context
- keeps graph workflows deterministic and cheap by default

#### Patch E2 — Surface contract in MCP, prompts, and installed instructions

- [ ] update MCP tool descriptions to mention minimal-first escalation where relevant
- [ ] update `review_change` prompt to make minimal context first a requirement, not just a recommendation
- [ ] update `inspect_symbol` prompt to require direct-neighbor context before wider traversal
- [ ] update README MCP workflow section to match same order
- [ ] ensure wording is consistent across CLI install block, MCP prompts, and README

Why:
- agents follow tool descriptions and prompts more reliably than implicit design intent
- one workflow description prevents drift across docs and MCP metadata

#### Patch E2.5 — Enforce minimal-context-first inside higher-level tools

- [ ] require higher-level tools to start from minimal bounded context internally unless explicitly bypassed:
  - [ ] `get_review_context`
  - [ ] `explain_change`
  - [ ] `get_impact_radius`
  - [ ] `analyze_safety`
  - [ ] `analyze_remove`
  - [ ] `analyze_dead_code`
  - [ ] `analyze_dependency`
  - [ ] refactor planning tools
- [ ] define explicit bypass reasons:
  - [ ] user requested full context
  - [ ] minimal context is truncated
  - [ ] minimal context reports ambiguity
  - [ ] tool requires full impact graph by contract
  - [ ] configured safety policy requires broader context
- [ ] include metadata showing whether minimal context was used, bypassed, or escalated:
  - [ ] `minimal_context_used`
  - [ ] `minimal_context_bypassed`
  - [ ] `escalation_reason`
  - [ ] `next_tools`
- [ ] add tests proving review/analyze/impact tools do not over-fetch when minimal context is sufficient

Why:
- workflow guidance is weaker than internal enforcement
- higher-level tools should not silently bypass bounded triage

#### Patch E3 — Add escalation metadata and tests where practical

- [ ] include response metadata that helps decide whether to escalate:
  - [ ] `truncated`
  - [ ] `omitted_count`
  - [ ] `ambiguity`
  - [ ] `next_tools`
  - [ ] `recommended_escalation_reason`
- [ ] ensure `get_minimal_context` reports when review context would add useful detail
- [ ] ensure `symbol_neighbors` reports when traversal may be needed because caps were hit
- [ ] add prompt/registry snapshot tests for minimal-first contract wording
- [ ] add MCP response tests for escalation metadata on truncated/ambiguous outputs

Why:
- tools should tell agents when more context is justified
- escalation should be evidence-driven, not habit-driven

#### Patch E completion criteria

- [ ] minimal-context-first contract is documented as required workflow
- [ ] higher-level tools internally start from minimal context or emit explicit bypass metadata
- [ ] MCP prompts, tool descriptions, README, and installed AGENTS instructions agree
- [ ] graph/context responses expose enough metadata to justify escalation
- [ ] tests protect contract wording and escalation metadata

---

### Dynamic Agent Policy and Hook Enforcement Patch

Atlas already installs static AGENTS/CLAUDE instructions and platform hook files, but current workflow policy still lives mostly in static text. Add one runtime policy surface plus hard hook enforcement so agents can load fresh Atlas workflow guidance at session start without trying to make markdown executable.

#### Patch A1 — Canonical runtime policy contract

- [ ] define compact `AgentInstructionsPolicy` model in shared service code with fields:
  - [ ] `policy_version`
  - [ ] `generated_at`
  - [ ] `frontend`
  - [ ] `policy_mode`
  - [ ] `required_first_step`
  - [ ] `required_tool_order`
  - [ ] `protected_tools`
  - [ ] `forbidden_patterns`
  - [ ] `fallback_behavior`
  - [ ] `trust_notes`
  - [ ] `source`
- [ ] keep policy payload deterministic and compact enough for hook/session injection
- [ ] make one shared Rust service produce policy for both MCP tool calls and `atlas hook`
- [ ] version policy explicitly so hooks can detect stale cached payloads
- [ ] add serde round-trip tests for policy schema stability

Why:
- runtime workflow policy should have one source of truth
- MCP tool output, hook preload, prompts, and installed instructions must not drift

#### Patch A2 — MCP `agent_instructions` tool surface

- [ ] add `agent_instructions` to MCP tool registry in `packages/atlas-mcp/src/tools/registry.rs`
- [ ] add dispatch arm in `packages/atlas-mcp/src/tools/dispatch.rs`
- [ ] implement handler that returns current `AgentInstructionsPolicy`
- [ ] accept explicit inputs:
  - [ ] `frontend`
  - [ ] `policy_mode`
  - [ ] `include_fallback_static_rules`
  - [ ] `output_format`
- [ ] default output to compact agent-facing payload suitable for session preload
- [ ] include TOON and JSON parity tests for tool output
- [ ] add registry snapshot test so installed instructions and MCP registry stay aligned

Why:
- agent needs runtime policy as normal Atlas surface, not hidden ad hoc hook text
- hook runner should reuse same policy source returned by MCP

#### Patch A3 — Installed instruction bootstrap text

- [ ] update install-generated instruction block in `packages/atlas-cli/src/install/instructions.rs`
- [ ] replace duplicated workflow detail with explicit bootstrap rule:
  - [ ] call `agent_instructions` before substantive repo exploration
  - [ ] use static AGENTS rules only when runtime policy is unavailable
  - [ ] keep graph-first and minimal-context-first invariants in static text
- [ ] keep injected section idempotent under existing instruction markers
- [ ] add install test proving stale injected section is replaced with new bootstrap wording
- [ ] add install test proving user-authored content before and after injected section is preserved

Why:
- static markdown should bootstrap runtime policy, not duplicate mutable operational rules
- install flow already owns AGENTS.md injected guidance and should remain source for static bootstrap text

#### Patch A4 — Generic hook runner preload integration

- [ ] extend the generic `.atlas/hooks/atlas-hook` runner so preload path calls shared Rust policy service instead of duplicating JSON assembly in shell
- [ ] preload runtime policy on session-start and prompt-submit at the runner/CLI boundary
- [ ] keep preload host-agnostic; Atlas does not generate per-tool hook config files
- [ ] cache last successful compact policy payload under `.atlas/hooks/lib/` with version/hash metadata
- [ ] define bounded cache TTL or invalidation rule so long sessions can refresh policy safely
- [ ] add tests for runner preload output after install

Why:
- existing generic hook runner already exists and should carry runtime policy preload
- session-start and prompt-submit are strongest points for loading fresh policy before work begins

#### Patch A5 — Hard enforcement at hook boundary

- [ ] make hook enforcement check whether current session has loaded valid policy version before protected tool execution
- [ ] define initial protected tool set:
  - [ ] `query_graph`
  - [ ] `get_context`
  - [ ] `get_review_context`
  - [ ] `get_minimal_context`
  - [ ] `get_impact_radius`
  - [ ] `explain_change`
  - [ ] graph-backed analysis tools
  - [ ] refactor planning tools
- [ ] define exempt diagnostic/repair tools that remain fail-open when policy preload fails:
  - [ ] `status`
  - [ ] `doctor`
  - [ ] `db_check`
  - [ ] `debug_graph`
  - [ ] `build_or_update_graph`
- [ ] return explicit enforcement decision metadata:
  - [ ] `policy_loaded`
  - [ ] `policy_version`
  - [ ] `enforcement_mode`
  - [ ] `blocked_reason`
  - [ ] `fallback_active`
- [ ] record enforcement events through existing adapter/session APIs; do not let hooks write SQLite directly
- [ ] add integration test proving protected tool is blocked before preload and allowed after preload

Why:
- AGENTS text alone cannot guarantee runtime behavior
- hook boundary is correct deterministic enforcement point for required policy preload

#### Patch A6 — Fallback and degraded-mode behavior

- [ ] define explicit fallback path when runtime policy fetch fails:
  - [ ] static AGENTS/install rules remain active
  - [ ] protected tools use configured fail-open or fail-closed behavior by class
  - [ ] fallback state is surfaced in metadata instead of silent skip
- [ ] ensure fallback does not bypass graph-readiness checks or existing safety gates
- [ ] ensure fallback path remains deterministic when cache exists but live fetch fails
- [ ] add tests for:
  - [ ] live fetch failure with valid cache
  - [ ] live fetch failure without cache
  - [ ] stale cache version rejection
  - [ ] explicit degraded metadata in hook/session output

Why:
- runtime policy fetch can fail and behavior must stay explicit, bounded, and safe
- degraded mode should not silently weaken existing Atlas safety contracts

#### Patch A7 — Prompt and documentation consistency

- [ ] update MCP prompts in `packages/atlas-mcp/src/prompts.rs` to mention `agent_instructions` as first runtime step where relevant
- [ ] update installed AGENTS instructions to reference runtime-policy bootstrap and fallback rules
- [ ] update README and wiki MCP workflow docs to match same wording
- [ ] ensure graph/content companion wording and minimal-context-first wording stay consistent with runtime-policy contract
- [ ] add snapshot tests protecting prompt/install/doc wording from drift

Why:
- prompts and installed instructions are agent-facing control surfaces and must agree
- runtime policy is only useful if every workflow surface points to same first-step contract

#### Patch A completion criteria

- [ ] `agent_instructions` exists as MCP tool with stable compact output
- [ ] installed AGENTS/bootstrap text tells agents to call `agent_instructions` first and defines fallback clearly
- [ ] the generic `.atlas/hooks/atlas-hook` runner preloads runtime policy on session/prompt start
- [ ] protected Atlas tools are blocked when required policy preload has not happened
- [ ] fallback mode is explicit, deterministic, and covered by tests
- [ ] prompts, installed instructions, README, and wiki workflow docs agree on runtime-policy-first behavior
- [ ] adapter/session event flow records policy preload and enforcement decisions without direct hook SQLite writes

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
