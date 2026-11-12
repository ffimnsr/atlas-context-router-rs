# Atlas — Shipped Work

This file summarizes shipped technical capabilities (still detailed).

For active backlog, see ISSUES.md.

## Technical Scope Implemented

- Repository scan, parse, graph persistence, incremental update, search, impact traversal, review context, context assembly, reasoning, and deterministic refactor flows are implemented.
- CLI and MCP surfaces are implemented over shared service-layer logic rather than separate feature stacks.
- Session continuity, saved-context storage, hook integration, and agent-facing transport support are implemented.
- Core operational controls such as canonical path identity, lifecycle state, ranking/trimming reuse, and budget enforcement are implemented.

## Repository and Graph Pipeline

- Rust workspace, crate boundaries, CI, and quality gates are in place.
- SQLite-backed graph persistence is implemented with file, node, edge, metadata, and FTS-backed lookup support.
- Repository scanning is implemented with git-root detection, tracked-file collection, change detection, rename/delete handling, and package/workspace-aware ownership.
- Parser abstraction is implemented with per-language handlers behind shared extraction contracts.
- Implemented language coverage includes Rust, Go, Python, JavaScript, TypeScript, Java, C#, PHP, C, C++, Scala, Ruby, JSON, TOML, HTML, CSS, Bash, and Markdown.
- Full build pipeline is implemented: collect files, hash, parse, replace file graph slices, and summarize results.
- Incremental update pipeline is implemented: detect changed files, invalidate dependents, remove deleted slices, and update only affected graph regions.
- Graph lifecycle diagnostics are implemented through status, doctor, db-check, and debug-graph style workflows.
- Watch mode and operational diagnostics are implemented for local development refresh loops.

## Historical Graphs

- Commit-linked graph snapshot storage is implemented with schema for commits, graph_snapshots, and snapshot_files tables.
- Deterministic git metadata ingestion is implemented with git rev-parse, log, show, ls-tree, diff-tree, and cat-file operations.
- Checkout-free file reconstruction is implemented using `git show <sha>:<path>` with binary detection and path canonicalization.
- File graph reuse and content-addressed storage is implemented to avoid duplicating unchanged file graphs across commits.
- Snapshot membership tracking is implemented to record which file hashes, nodes, and edges are active at each commit.
- Incremental historical indexing is implemented with missing-commit detection, force-push safeguards, and explicit repair mode for rewritten history.
- Lifecycle tables `node_history` and `edge_history` are implemented to track first/last/introduction/removal across snapshots and commits.
- Graph diffing across commits is implemented with file, node, edge, module, and architecture diff scopes.
- Symbol, file, module, and dependency history query commands are implemented with evidence-backed outputs including commit SHAs and qualified names.
- Churn metrics, stability indicators, and trend analysis are implemented for per-symbol, per-file, and per-module analysis.
- Snapshot reconstruction is implemented to restore graph state for any indexed commit with partial completeness tracking.
- Retention controls and pruning are implemented with keep-latest-N, keep-by-age, and storage diagnostics.
- CLI commands `atlas history status`, `atlas history build`, `atlas history update`, `atlas history diff`, and pruning workflows are implemented.
- All historical operations preserve exact commit SHA evidence, are deterministic and reproducible, and never rely on branch names for identity.

## Query, Search, and Impact Surfaces

- Symbol lookup is implemented through graph query surfaces with ranked exact-match, qualified-name, filtered, fuzzy, regex, and hybrid retrieval modes.
- Result-level ranking evidence is implemented for graph retrieval with compact stable evidence for match fields, exact and prefix hits, fuzzy repairs, fired boosts, graph-expansion hops, and hybrid/RRF contributions.
- Ranking evidence is propagated through `query_graph`, `batch_query_graph`, `explain_query`, CLI JSON query output, and bounded context/review relevance scoring where payload budgets allow.
- Impact traversal is implemented with bounded graph walking, changed-node seeding, impacted-node/file selection, and structured output.
- Review-context and minimal-context flows are implemented for changed files and bounded downstream context assembly.
- Explain-change and change classification flows are implemented with compact structured summaries.
- Content and file discovery surfaces are implemented for non-graph assets such as docs, prompts, templates, SQL, config, and other text assets.

## Context Engine and Analysis Engines

- Context engine is implemented with structured request types for symbol, file, review, and impact flows.
- Target resolution is implemented for qualified names, exact symbol names, exact file paths, and ambiguity-aware candidate fallback.
- Bounded context packaging is implemented with node, edge, file, and code-span selection plus truncation metadata.
- Reasoning engine is implemented with removal impact analysis, dead-code detection, refactor safety scoring, dependency-removal checks, rename blast radius, and change-risk classification.
- Refactor engine is implemented with deterministic plan/apply flows for rename, dead-code removal, and import cleanup, including dry-run output and validation.

## Context Payload Telemetry and Metadata Slimming

- `ContextTelemetry` is implemented in `packages/atlas-core/src/model/context.rs` as optional `ContextResult::telemetry`, covering requested and emitted nodes, edges, files, saved sources, content assets, overviews, bytes, and tokens plus `budget_hit`, `budget_name`, `duration_ms`, and `outcome` (`ok`, `empty`, `ambiguous`, `budget_exhausted`, `error`) classified at the `ContextEngine::build` boundary.
- Telemetry is opt-in through `ContextRequest::include_telemetry` (default `false`), `--telemetry` on `atlas context` and `atlas review-context`, and `include_telemetry` on MCP `get_context` and `get_review_context`; absent telemetry is skipped so default output stays byte-identical.
- Metrics families `context_builds` (labeled by intent and outcome), `context_duration_ms`, `context_payload_bytes`, and `context_payload_tokens` are implemented, and the context bench emits telemetry JSON for a fixed corpus covering symbol, file, review, change-set, oversized-context, and provenance-sensitive requests.
- Uniform repo provenance is hoisted to `ContextResult::repo_provenance`, while per-item provenance is preserved when the result spans multiple repos or provenance differs across items.
- `ContextRequest::include_ranking_evidence` (default `false`) omits `context_ranking_evidence` from nodes, content assets, and saved-context sources while keeping `selection_reason`, `relevance_score`, graph identity fields, and `source_id`; `--include-ranking-evidence` and the matching MCP option enable it.
- Payload slimming reaches the `>= 15%` token-reduction threshold at unchanged recall and unchanged required-evidence retention through provenance hoisting and opt-in ranking evidence.

## Insights Engine

- Deterministic insights engine is implemented with shared report primitives (`InsightSummary`, `InsightFinding`, `InsightEvidence`), stable severity values, deterministic sort order, and reuse of existing ranking/truncation and freshness/provenance metadata contracts; no LLM dependency and no writes to SQLite.
- Code health metrics engine is implemented for node-level fan-in/fan-out, dependency depth with cycle guard, reference counts, test adjacency, LOC, cyclomatic and cognitive complexity, branch counts, and max nesting depth (with per-language `not_available` instead of text guessing), file-level node/edge/import/test-coverage metrics, module-level coupling and cohesion approximations, and min/max/avg/p50/p90/p95 distribution statistics with outlier detection.
- Large and complex function finder is implemented with repo-wide and file-scoped modes, LOC/complexity threshold overrides, `large`/`complex`/`large-or-complex` modes, changed-file and fan-in/fan-out ranking boosts, complete finding payloads, and CLI `atlas insights large-functions` plus MCP `find_large_functions` with parity tests.
- Architecture analysis is implemented with module-level graph aggregation, SCC cycle detection with local/cross-module classification and deterministic cycle paths, inline layer-rule enforcement with violation findings, coupling scores, tightly-coupled cluster detection, and high-connectivity file flags.
- Risk assessment is implemented as explainable weighted `0-100` scores with public-API, fan-in/out, cross-module dependency, test adjacency, depth, unresolved-edge, size/complexity, and cycle-participation factors, each with factor contribution and evidence, plus configurable low/medium/high thresholds.
- Pattern detection is implemented for repeated call chains, unused/isolated structures with removal blockers, high-centrality hubs and bottlenecks, and deep chains with traversal caps and cycle guards.
- Insights surfaces are implemented across CLI (`atlas insights architecture|metrics|risk|patterns|large-functions|complex-functions` with `--json` and `--limit`) and MCP (`analyze_architecture`, `analyze_metrics`, `assess_risk`, `analyze_patterns`, `find_large_functions`, `find_complex_functions`) with freshness/provenance metadata and CLI/MCP parity coverage.
- Insight thresholds and inline layer rules are config-driven under `.atlas/config.toml` with positive-value validation, ignore lists for files/modules/node kinds, and actionable config errors.
- Similar-function detection is implemented with deterministic callable fingerprints from canonical file paths, symbol kind/name tokens, normalized signature tokens, call/import/reference neighborhood, module bucket, source-body shingles, and duplicate-normalized shingles; candidates are scored with weighted name/signature/body/neighborhood/module/size buckets and return feature score breakdowns, matched and differing features, and similarity band, with language/kind/arity/same-file bounds and stable JSON through CLI and MCP surfaces.
- Duplicate detection is implemented beyond exact structural patterns by normalizing callable source tokens (keywords and control flow preserved, identifiers and literals replaced), detecting `exact_normalized` and `near_duplicate` groups with normalized token shingles, ranking groups by confidence, duplicated token/line count, member count, and stable deterministic group ID, and exposing members, files, normalized pattern summary, suggested extraction target, and source-span records through CLI/MCP.
- Inferred module analysis is implemented with stable module IDs, display names, root paths, owned symbols, inbound/outbound dependencies, confidence and evidence, explicit-owner flags, package-owner preference, path-bucket inference (`packages/<name>`, `src/<segment>`, tests, docs/wiki/markdown), graph-community clustering before path fallback when no explicit owner exists, and deterministic module dependency edges.
- Component labeling is implemented with the Atlas component taxonomy (repo scan, parse, persist graph, incremental update, search/traverse, review context, context memory, session continuity, CLI, MCP, config, diagnostics, tests, docs) and deterministic multi-label file/symbol assignment from path and symbol-name rules with confidence and evidence, plus component-label propagation into review/context workflow impacted components.
- Similarity and duplicate band thresholds are configurable via `insights.*_threshold` config, duplicate suppressions are supported from config and CLI/MCP request filters, and a persisted fingerprint cache with incremental invalidation backs repeated analysis.

## Docs Generation and Export

- `atlas docs generate` CLI command is implemented for deterministic Markdown documentation: `index.md`, `files.md`, `symbols.md`, `modules.md`, and `components.md` with repo summary, graph stats, generated timestamp, indexed file count, symbol counts by kind, inferred modules, component labels, and top-level dependency summaries; per-file sections carry canonical path, language, package/module/component labels, owned symbols, inbound/outbound dependency counts, and notable duplicates; per-symbol sections carry qualified name, kind, file/span, documentation snippet, callers/callees, tests, and owning module/component labels.
- Docs output is stable for tests through deterministic sorting of paths/symbols/modules/components and timestamp override in fixtures; missing, stale, or unreadable graph state fails with actionable errors and never silently generates partial docs unless `--allow-partial` is passed.
- `atlas docs export` is implemented with `--format mermaid` and `--format dot` for module, component, and selected symbol-neighborhood dependency diagrams, `--scope repo|module|component|file|symbol` plus `--name` for focused exports, stable node IDs, human-readable labels, edge kinds and counts with collapsed high-volume edges, `--include-diagrams` integration into generated Markdown, and `--max-nodes`/`--max-edges` caps with deterministic omission reporting.

## CLI and MCP Interfaces

- Product baseline is implemented with binary name `atlas`, hidden work dir `.atlas/`, graph DB path `.atlas/worldtree.db`, and config path `.atlas/config.toml`.
- CLI command surfaces are implemented for init, build, update, detect-changes, status, query, impact, review-context, context, doctor, db-check, debug-style diagnostics, reasoning, refactor, install, and serve workflows.
- MCP tool registry is implemented for graph queries, traversal, review/impact/context flows, health/debug tools, saved-context tools, content/file search tools, and reasoning analysis tools.
- Docs-section lookup parity is implemented through shared CLI and MCP surfaces with heading-path or line-based resolution, bounded section excerpts, truncation metadata, and parity-tested not-found behavior.
- Explicit postprocess parity is implemented through shared CLI and MCP surfaces for derived graph analytics refresh, including full and changed-only modes, stage selection, dry-run preview, and parity-tested error handling.
- MCP transport support is implemented with stdio-compatible serving and repo-scoped backend brokering.
- CLI and MCP parity is implemented across major shared service surfaces rather than maintained as unrelated code paths.

## Session, Saved Context, and Continuity

- Separate session and content stores are implemented instead of mixing runtime/session data into the graph database.
- Session event persistence is implemented with bounded event payload handling, artifact references, and session metadata.
- Saved-context artifact storage, previewing, retrieval, search, and purge flows are implemented.
- Resume snapshots and retrieval-backed restoration are implemented for session continuity.
- Context storage compaction and budget-aware retention behavior are implemented.
- Decision memory with persistent decision events, artifact linking, reasoning storage, and decision retrieval is implemented.
- Agent-scoped context and session management is implemented with agent_id partitioning, agent memory summaries, delegated task tracking, and agent responsibility summaries.
- Agent-aware context isolation, intentional merged views, and agent-scoped session continuity are implemented.
- Shared memory surface is implemented over continuity-owned storage with `MemoryImportance` (`critical`, `high`, `normal`, `low`) and `MemoryScope` (`project`, `session`, `frontend`, `global`) validation at CLI, MCP, and storage boundaries, a `memories` table with topic/title/body/importance/decay/source metadata plus indexes for `topic`, `importance`, `scope`, `session_id`, and `last_accessed_at`, and `atlas db check` schema validation.
- `atlas memory store|recall|list|delete` CLI commands are implemented with `--topic`, `--title`, `--importance`, `--scope`, `--frontend`, `--source-id`, `--shared`, `--older-than`/`--newer-than` filters, and `--json`; recall uses lexical search with exact topic matches ranked first, list sorts by `updated_at DESC`, delete requires exact memory id with `--dry-run`, and stored text is kept exactly as provided except central redaction policy.
- Frontend-aware visibility rules are implemented: `global` visible everywhere, `project` visible to all frontends in the repo, `session` visible only to the same session, `frontend` visible only to the same repo plus same frontend; `--shared` recall returns only `global` and `project` memories, project-scoped writes work without an active session, and frontend identities normalize to `claude`, `codex`, `copilot`, `cli`, and `mcp` with unknown names rejected unless explicitly allowed.
- MCP `memory_store` and `memory_recall` are implemented with the same fields, validation, visibility rules, and bounded default output as the CLI, with parity tests proving stored record shapes, errors, and defaults match.
- Memory decay policy is implemented in `[memory.decay]` config with `enabled`, `low_days` (30), `normal_days` (90), `high_days` (365), and `critical_never_prune` (true) defaults; retention days are validated as positive integers so `atlas doctor` fails clearly on invalid config, the config template documents the section, and `atlas debug config` surfaces the effective values.
- `atlas memory decay` is implemented with `--dry-run`, `--topic`, and `--json`: it computes updated `decay_score` values from `updated_at` age vs the importance retention window, reports protected critical memories at score 0.0 with `retention_days: null`, and never deletes rows; dry-run reports without writing.
- `atlas memory stale` and `atlas memory prune` are implemented with `--dry-run`, `--topic`, `--importance`, `--older-than`, and `--json`: stale lists only rows past their retention window and never reports critical memories as auto-prune candidates; prune deletes only policy-pruneable rows, counts protected critical rows as skipped, and fails without an explicit `--allow-critical` override before any critical-memory prune path exists.
- `atlas memory health` is implemented with `--topic`, `--scope`, and `--json`: deterministic categories `healthy`, `stale`, `noisy`, `duplicated`, `orphaned`, and `oversized`, detecting low-importance old memories, repeated memories, dangling saved-context `source_id` references (checked against the content store), noisy topics above 10 entries, oversized bodies above 2000 characters, and topics without a critical decision memory, with actionable suggestions and exact follow-up commands in human output.
- `atlas memory consolidate` is implemented with `--topic`, `--scope`, `--dry-run`, and `--json`: a deterministic planner groups memories by topic plus normalized title, normalized body, same `source_id`, or same metadata category, dry-run reports kept ids, merged ids, and preserved source ids read-only, and apply mode creates a consolidated memory carrying `merged_ids`/`merged_source_ids` metadata, marks merged rows as superseded (`superseded_by`), and stores `memory_supersessions` link rows with `old_memory_id`, `new_memory_id`, `reason`, and `created_at`.
- Supersession semantics are implemented in the storage layer: recall ranks active rows above superseded rows, list hides superseded rows by default with explicit inspection via the `include_superseded` filter, memory schema validation covers the new column/table/indexes, and the MCP memory record schema includes `superseded_by`.
- Feedback records are implemented as first-class deterministic correction memory in the continuity-owned session database: a `feedback_records` table with `id`, `repo_root`, `session_id`, `tool_name`, `analysis_kind`, `predicted`, `actual`, `correction`, `related_symbol`, `related_file`, `source_id`, `created_at`, and `metadata_json`, an FTS5 index over predicted/actual/correction/related_symbol/related_file kept in sync by triggers, exact-match search filters, deterministic stats, and `atlas db check` schema validation (CLI and MCP).
- `atlas feedback record|search|stats` CLI commands are implemented with required `--predicted`/`--actual`, optional `--correction`, `--tool`, `--analysis-kind`, `--symbol`, `--file`, `--source-id`, search filters `--tool`, `--analysis-kind`, `--symbol`, `--file`, `--limit`, and `--json`; missing predicted or actual fails at both clap and service-layer validation, search returns predicted/actual/correction/related symbol and file/relevance score/created time, and empty feedback DBs return stable zero-count stats.
- MCP `feedback_record` is implemented through the same service layer and validation contract as the CLI, with parity tests proving record shapes and error messages match.
- Confidence adjustment is implemented for `atlas analyze dead-code`, `atlas analyze remove`, `atlas analyze safety`, and `atlas refactor remove-dead --dry-run`: matching false-positive feedback records (same symbol, file, or analysis kind, where `actual != predicted` or metadata marks `false_positive`) lower confidence by one deterministic step (certainty tier, impact class, safety score + band recompute, or safety band), `feedback_evidence` is exposed in JSON output whenever scoring changes, adjustment never mutates state or graph data, apply-mode refactors stay untouched, and the `analysis.feedback_adjustment.enabled` config flag (default true) can disable the whole path; empty feedback keeps every output byte-identical to the deterministic baseline.

## Hook and Agent Host Integration

- Install flows are implemented for Copilot, Claude, and Codex integration.
- Generated hook and MCP configuration support repo-local integration flows instead of requiring manual setup.
- Hook lifecycle coverage is implemented for session start, prompt submission, tool execution, compaction, stop, session end, and related host events.
- Thin hook runner architecture is implemented so shell launchers remain stable while Rust code owns normalization, routing, storage, and policy behavior.
- Shared agent event service is implemented in `packages/atlas-agent-events` so native hooks and future MCP fallback capture share one event policy, persistence, and action pipeline (`AgentEventRequest` / `AgentEventSource` / `AgentEventResult` / `record_agent_event`).
- MCP `record_session_event` fallback tool is implemented in `packages/atlas-mcp/src/session_events.rs` so hosts without native LLM hooks get hook-equivalent capture with the same event aliases, redaction, storage routing, lifecycle actions, graph refresh, and review refresh behavior as `atlas hook`.
- Installed AGENTS/CLAUDE instruction blocks now include a mandatory session memory fallback protocol for hookless hosts: wake-up/resume recall at start, `record_session_event` triggers for user-prompt/post-tool-use/file-changed/pre-compact/stop/session-end, `save_context_artifact` for decisions/errors/preferences/handoffs, and explicit do-not-store rules.
- Agent event capture is implemented with deterministic, bounded, redacted, and deduplicated enriched event payloads across native hooks and MCP fallback.
- Installed AGENTS instructions state the minimal-context-first escalation order explicitly: `detect_changes`, `get_minimal_context`, `get_review_context`, `explain_change`, and `get_impact_radius` for review and change tasks, and `query_graph` or `resolve_symbol`, `symbol_neighbors`, `get_context`, then bounded `traverse_graph` for symbol and usage tasks.
- `atlas install --mode` is implemented with `mcp` (MCP config only), `hook` (git + agent hooks only), `cli` (instruction fallback text only), and `all` (default) modes that compose with the explicit `--no-*` flags.
- `atlas install --dry-run` output identifies every instruction fallback file it would create, append, refresh, or replace, and the install summary is dry-run aware.
- Hookless-host continuity is documented in `wiki/session-memory-fallback.md` (recall, capture triggers, artifact labels, compaction, do-not-store rules) with a native-hooks-vs-MCP-fallback compatibility matrix (`surface`, `works_with`, `automatic`, `token_cost`, `best_for`) also mirrored in README; Claude/Codex hook docs state that native hooks and the MCP fallback share one event service.
- Event-name parity is enforced by tests: installed instruction blocks, MCP registry descriptions, and wiki docs may only reference event names supported by `record_session_event` / native hooks (`atlas_mcp::supported_event_names` / `is_supported_event_name`).

## Runtime Event Classification and Resume Grouping

- Deterministic runtime classification is implemented in `packages/atlas-agent-events/src/classification.rs` with `RuntimeClassification { kind, severity, rule_id, matched_fields }`, kinds `panic`, `exception`, `stacktrace`, `compiler_error`, `test_failure`, `test_success`, `build_success`, `deprecation_warning`, `dead_code_warning`, `permission_denied`, `command_timeout`, `graph_not_ready`, and `retrieval_failure`, and severities `error`, `warning`, and `info`.
- `classify_runtime_payload` scans bounded payload text (`stderr`, `stdout`, `output`, `error`, `message`, `result`, `extra`) up to `MAX_CLASSIFIER_INPUT_BYTES` (16 KiB), applies status and error-text rules before output-pattern rules, reuses existing hook status, command, and tool extractors, returns `None` when no rule matches, and records only rule ids plus matched field names through `MAX_CLASSIFIER_MATCHED_FIELDS`, never raw matched text.
- Classification is attached to hook metadata by `build_hook_event_metadata` only when it returns `Some`, leaving `SessionEventType` and priority selection unchanged; classification stays deterministic so identical raw input produces an identical enriched payload and event hash.
- Resume snapshots extend `unresolved_errors` and `recent_commands` with classification metadata and add `recent_build_test_summaries` (`kind`, `command`, `status`, `classification`, `at`) capped at five entries and included in `trim_snapshot_to_limit`.
- `WakePack` adds `recent_runtime_failures` sourced from resume errors with `severity = error`, capped by the existing wake-pack item limit and ordered deterministically by time then rule id with duplicate entries removed.
- MCP `wake_up` output schema, `MCP_TOOLS.md`, session-memory wiki docs, and CLI/MCP snapshots are updated for the new classification metadata and wake-up field.

## Cross-Cutting Infrastructure

- Shared ranking and trimming primitives are implemented across query, context, review, impact, and analysis surfaces.
- Graph build lifecycle state is implemented and surfaced through status, doctor, and MCP responses.
- Canonical graph readiness is implemented as single source of truth for built/queryable/current/integrity state, including explicit `fresh`, `stale`, `partial`, and `corrupt` execution states.
- CLI, MCP, adapters, and graph-backed analysis flows consume the same readiness contract and surface consistent safe-to-answer, freshness, and degraded-mode metadata.
- Canonical repo path identity is implemented across graph, content, session, adapter, and saved-context keys.
- Central budget policy and shared budget metadata are implemented across public surfaces.
- Repo-scoped MCP backend brokering is implemented without breaking stdio compatibility.
- Hook policy ownership, bounded payload routing, freshness handling, and review-refresh artifact flows are implemented.

## Multi-Repo Federation

- Repo registry is implemented with stable `repo_id` hashes over canonical roots, human-editable versioned registry storage under `.atlas/`, and registration entries carrying canonical absolute root, display alias, VCS metadata, relationship kind (`root`, `submodule`, `workspace_member`, `manual`), trust state, include/exclude globs, and inter-repo dependency metadata.
- Discovery and bootstrap are implemented: root repo auto-registers on `atlas init`, initialized git submodules auto-register as first-class repos with parent linkage preserved, sibling repos register manually via `atlas repo add <path>`, and `atlas repo remove`/`atlas repo sync` manage entries without touching unrelated graph data.
- Multi-repo identity is implemented as `(repo_id, canonical_repo_relative_path)` with qualified-name collision prevention, synthetic repo/owner nodes, and membership edges (`repo contains package`, `repo contains workspace`, `registry contains repo`, `repo depends_on repo`, `repo submodule_of repo`), with nodes carrying `repo_id` provenance metadata.
- Build and update flows are implemented per registered repo as independent parse/update units with per-repo git diff state, submodule-safe git invocation, targeted `--repo-id`/`--all-repos`/`--affected-repos` updates, per-repo cached status, and partial-success reporting.
- Cross-repo resolution is implemented only when registry relationship or dependency evidence exists, with submodule boundaries treated as repo boundaries first, cross-repo edge metadata (source repo, target repo, relationship reason, confidence tier), explicit unresolved cross-repo references, and cross-repo impact/removal analysis that exposes repo hops.
- CLI surface is implemented with `atlas repo list|add|remove|sync`, `--repo-id`/`--all-repos` flags on build/update/detect-changes/query/impact, repo labels in human output wherever the same symbol exists in multiple repos, and stable JSON repo metadata fields.
- MCP surface is implemented with a `repo_registry` inspection tool, explicit `repo_root` and registry-backed `repo_id` scoping on graph/context tools, per-root broker sessions from explicit repo selectors, and repo identity in ambiguity candidates and provenance payloads.
- Review/context/saved-artifact integration is implemented with changed-repo summaries before changed files, cross-repo boundary violations in impact/review summaries, cross-repo caller/callee following when enabled, repo-set session artifact ownership, and saved-context reads blocked when session repo scope does not overlap requested scope.
- Safety and rollout behavior is implemented: single-repo default path stays zero-config and unchanged, federation is gated behind registry presence or `--all-repos`, fan-out is bounded, per-repo and aggregate budgets are reported, and unavailable/corrupted repos degrade cleanly.

## Retrieval and Content Sidecar Hardening

- Retrieval/content indexing lifecycle state is implemented with explicit indexing/indexed/index-failed states, persisted status metadata, searchable-now source of truth, CLI/MCP status surfaces, and interrupted-index recovery behavior.
- Retrieval indexing guardrails are implemented with configurable retrieval and embedding batch sizes, hard caps for chunks per run and per file, oversized-run policy, indexing metrics, and regression coverage for chunk explosion and partial recovery.
- Embedding dimension registry and freeze rules are implemented with provider/model/dimension metadata, frozen active index dimensions, insert/search mismatch rejection, cached dimension discovery, diagnostics, and provider-switch tests.
- Stable content-derived chunk identity is implemented through `chunk_id` use for dedupe, chunk reuse, retrieval cache keys, and saved-context references, with tests for stable and changed content identity.
- Retrieval/token-efficiency evaluation is implemented with recall/MRR/exact-hit metrics, retrieved/emitted token tracking, tool-call counts, graph-only versus retrieval benchmarks, fixed-budget evaluation, and acceptance thresholds before hybrid retrieval defaults.
- Embedding configuration is implemented in `.atlas/config.toml` for `atlas-search` URL and model settings instead of environment variable getters.
- Graph/content companion lookup is implemented as a coordinated retrieval contract: graph for structural code facts, content for non-code and context-adjacent assets, saved context for prior Atlas outputs, and context engine merging under one bounded selection, ranking, evidence, and truncation policy.
- Mixed graph/content ranking evidence is implemented with source-kind envelopes, normalized signals across graph/content/session assets, selection reasons, truncation metadata, and prompt/MCP/installed-instruction wording that requires graph-first companion lookup.
- `search_content` invalid-regex handling is strict and agent-friendly, returning clear errors with escaped-regex or literal-search guidance instead of silently falling back.
- Retrieval backend capability flags are implemented with lexical FTS, dense vector, hybrid lexical+vector, sparse/BM25-native, and metadata-filtering flags derived from configuration, mode-request validation against backend capabilities before query/index, automatic unsupported-hybrid degradation with explicit warning, active-capability reporting on MCP/CLI surfaces, and tests for lexical-only, dense-only, hybrid-capable, and unsupported-mode backends.

## Parser Fuzz and Validation Hardening

- Stateful `TreeCache` fuzz coverage is implemented for parse, reparse-with-old-tree, insert, remove, evict, rename-key, delete/rename transitions, and old-tree reuse with changed bytes.
- Engine update-flow fuzz coverage is implemented against temp git repos and temp SQLite databases for add/modify/delete/rename sequences, supported and unsupported paths, working-tree diff mode, explicit file-list mode, old-tree reuse, and deleted-file cleanup.
- Parser output invariant fuzzing is implemented across all built-in language handlers, asserting file-node presence, path consistency, non-empty node/edge identities, valid line spans, and size consistency.
- AST helper fuzzing is implemented across built-in grammars, walking arbitrary parse trees and exercising `node_text`, line helpers, ancestor checks, common field lookups, and `find_all` without panics on malformed or invalid UTF-8 input.
- Refactor validation parser-reuse fuzzing is implemented for supported/unsupported paths, empty files, malformed supported-language content, and UTF-8-safe validation diagnostics.
- Parser fuzz corpora and dictionaries are seeded from parser fixtures and regex samples, with README/toolchain documentation and corpus refresh commands.

## Parser Query-Backed Extraction

- Rust parser extraction is query-backed: syntax facts are captured from Atlas-owned tree-sitter `.scm` query files (`packages/atlas-parser/queries/rust.scm`), while Rust code retains Atlas graph semantics for qualified names, parent scopes, `Contains`/`Calls`/`References`/`Implements` edges, confidence tiers, and language-specific metadata.
- Shared query helpers are implemented for loading one static query per language via `include_str!`, compiling queries, grouping captures by query match without losing capture order, reading capture text through existing `ast_helpers::node_text`, and surfacing parse/query errors as test-visible failures instead of silent empty captures.
- The Rust definition-extraction migration is behavior-preserving: parser public APIs, `ParsedFile` schema, qualified-name strings, `NodeKind` choices, `Contains`/`Implements` edge behavior, same-file call/reference resolvers, and test-module/test-function detection stayed unchanged, and golden fixture outputs for core and bad-syntax Rust fixtures are unchanged.
- Rust semantic extraction fixes are implemented on the query foundation: trait method declarations are captured from `trait_item` bodies and contained by the trait, `#[test]`/`#[cfg(test)]` detection replaces substring matching without misclassifying `#[cfg(not(test))]` or custom attributes, impl target handling normalizes local type/trait names and emits same-file `Implements` edges only when targets resolve uniquely, and call/reference syntax extraction uses query captures while preserving resolver semantics and confidence tiers.
- Scope semantics are preserved through query migration: root scope starts at the repo-relative file path, inline `mod` blocks and `impl` blocks push module/impl qualified names, and nested module suffixes stay compatible with the existing `qualified_suffix` behavior.
- Query-backed extraction is implemented for C, C++, C#, Java, Scala, JavaScript, TypeScript, Python, Ruby, PHP, Bash, HTML, and CSS with Atlas-owned `.scm` files under `packages/atlas-parser/queries/`, shared `@atlas.*` capture names, and parser code still owning qualified names, parent scopes, edge kinds, confidence tiers, and language heuristics.
- Shared query helper coverage includes capture lookup by exact name, optional and required captures with clear failures, byte-range sorting for deterministic output, and source-order preservation for overlapping matches.
- JSON, TOML, and Markdown remain intentionally manual with documented reasons (recursive object/key path ownership, stable table/key-path semantics, and heading/slug/fence/malformed-input safeguards), while every migrated parser keeps public APIs, `ParsedFile`, `NodeKind`, edge behavior, and existing golden outputs unchanged.

## Reachability Measurement and Rust Reachability Guard

- `cross_file_links` exposes `SemanticReferenceEdge { source_file, target_qn, edge_kind, confidence_tier, extra_json }` through `Store::semantic_reference_edges`, keeps `files_referencing_symbols_in` as a thin wrapper, and adds `confidence_tier` and `reachability` to `SemanticLink`.
- Weak-edge filtering is implemented with `cross_file_links_with_options(..., include_weak)`: `confidence_tier = 'same_package'` edges are hidden by default, every other tier is kept, and MCP `cross_file_links` exposes `include_weak` (default `false`) with schema fields for tier and reachability.
- `atlas doctor --json` and MCP `doctor` report `reachability_measurement` with `total_same_package_edges`, `sampled_edges`, `cross_crate_root_edges`, `orphan_candidate_edges`, `unknown_edges`, `measurement_source`, and `sample_limit`, backed by `packages/atlas-engine/src/reachability_measure.rs` and Cargo layout path-kind classification (`lib`, `main`, `example`, `test`, `bench`, `other`), with a bench and a fixture reproducing a wrong `same_package` edge.
- Rust module reachability is implemented in `packages/atlas-engine/src/reachability.rs` with `ReachabilityStatus` (`verified`, `unreachable`, `unknown`, `not_applicable`), `ReachabilityResult`, `CrateRootKind`, `CrateRoot`, and `RustReachabilityIndex` built from persisted module and file nodes through `from_store`; shared resolution helpers moved to `package_resolution.rs` without behavior changes.
- Rust parser module facts are stored in `Node.extra_json` (`inline`, `module_file`, `has_cfg`, `has_path_attr`, `path_attr_value`) with parser tests for `mod foo;`, `foo/mod.rs`, inline modules, `#[path]`, and `#[cfg]`; indexes become incomplete for missing facts, `#[cfg]`, `#[path]`, out-of-owner or missing files, and `include!` or macro-generated declarations.
- `ReachabilityGuard` is keyed by `(language, owner_id)` and built once per `reconcile_call_targets` run from the store, applied only to `same_package` fallback candidates after receiver-hint filtering: reachable edges are retained, unreachable edges are suppressed and reset to the original callee name with cleared confidence tier and `unresolved = true`, and unknown edges keep existing fallback behavior; accepted edges carry `reachability = verified | unknown` and non-Rust or provider-absent resolution carries `not_applicable`.
- `[reachability]` config is implemented with `enabled` (default `true`), `budget_ms` (default `250`), and `max_owners_per_run` (default `64`), positive-value validation, unchanged-owner skipping, budget-exhausted indexes reported as `Unknown`, and build/update/doctor summaries covering owners indexed and skipped, unknown reasons, unresolved mods, and accepted or rejected weak edges.
- `atlas db_check` validates persisted `extra_json.reachability` values only, `packages/atlas-engine/src/call_resolution.rs` stays below 1,000 LOC, and non-Rust languages remain `not_applicable` or `unknown`; provider scope, completeness, and diagnostics are documented in `wiki/reachability.md`.

## SQLite Store Concurrency Contract

- Canonical SQLite ownership policy is documented and implemented across Atlas stores: each store owns one `rusqlite::Connection`, store structs are thread-confined, concurrent DB access uses separate connections, and current write behavior remains single-owner per store instance.
- Store, content-store, session-store, and DB utility docs consistently describe single-connection-per-store ownership, WAL behavior across separate connections, and why graph DB opens with `SQLITE_OPEN_NO_MUTEX` under thread confinement.
- Engine build/update boundaries keep Rayon parse work separate from sequential SQLite write/update phases, with regression coverage and trait-bound checks preventing store types from becoming `Send` or `Sync`.
- Shared-connection wrappers such as `Arc<Mutex<Connection>>` or `RwLock<Connection>` are explicitly rejected for Atlas store concurrency.
- Future read concurrency is documented as separate checked-out connections only; read pooling remains a measured, default-off follow-up rather than current behavior.

## MCP Protocol and Transport

- MCP `2026-07-28` protocol compliance is implemented with one canonical protocol-version constant and typed per-request `_meta` parsing for protocol version, client info, client capabilities, and per-request log level.
- Stateless request model is implemented: `initialize`/`notifications/initialized` are no longer part of the normal path, unsupported versions fail with deterministic `UnsupportedProtocolVersionError`, and success results carry `resultType: "complete"` plus serverInfo metadata.
- `server/discover` is implemented on stdio and HTTP as required discovery, returning supported versions, capabilities, serverInfo, instructions, `ttlMs`, and `cacheScope` before any handshake state exists.
- Streamable HTTP transport is implemented as a single `POST /mcp` endpoint plus `GET /health` and protected-resource metadata, with required `Accept` negotiation, `MCP-Protocol-Version`/`Mcp-Method`/`Mcp-Name` header validation, Base64 sentinel decoding for `Mcp-Name` and `Mcp-Param-*` headers, `HeaderMismatch` errors, and `404` plus `-32601` for unknown methods.
- Protocol-level HTTP sessions are removed: no `Mcp-Session-Id`, `GET /mcp`, `DELETE /mcp`, `Last-Event-ID`, resumable SSE, or per-session outbound queues.
- Stdio runs modern-first: per-request `_meta`, `server/discover` or `tools/list` as first request, no initialize dependency, diagnostics on stderr only, and protocol-clean stdout with isolated legacy fallback.
- Cacheable result metadata is implemented for `tools/list`, `prompts/list`, `resources/list`, `resources/templates/list`, `resources/read`, and `server/discover` with `ttlMs`, `cacheScope`, and deterministic list ordering.
- `subscriptions/listen` is implemented as a long-lived POST response stream with opted-in notification types, `subscriptionId` tagging, and request-scoped notification isolation.
- Handrolled MCP JSON-RPC parser, dispatcher, response builders, TOON output mode, and legacy protocol modules are removed; MCP output is JSON-only with object `structuredContent` as the source of truth, and generated docs, prompts, and installed instructions describe JSON-only behavior.

## MCP Server Features

- Typed descriptor layer is implemented for tools, prompts, resources, resource templates, and completions with `name`, `title`, `description`, `inputSchema`, `outputSchema`, `annotations`, `icons`, and `_meta`, serialized from typed structs rather than hand-built JSON.
- All advertised schemas are valid JSON Schema 2020-12 with standalone per-method schema builders shared between registry output and request validation.
- Resource model is implemented with deterministic `resources/list` pagination, `resources/read` for health/status, graph provenance, saved-context, and docs-section resources, plus `resources/templates/list` for dynamic resource families.
- Completion handlers are implemented for structured inputs including output format, review/context intent, known tool names, and docs-section template variables, with context-sensitive and deterministic ordering.
- Tool results use object `structuredContent` with per-tool `outputSchema`, human-readable `content` summary, and resource-link content items for saved artifacts and docs sections.
- HTTP authorization is implemented with config-driven protected resources: issuer/JWKS/OIDC discovery, audience and scope validation, `/.well-known/oauth-protected-resource` metadata, `WWW-Authenticate` with resource and scope hints, and distinct `401` unauthenticated versus `403` forbidden responses.
- MCP logging capability was removed per `2026-07-28`: no `logging/setLevel`, no logging capability advertisement, and request-scoped log level parsed from `_meta` only.

## MCP Tool Contracts and Errors

- Tool descriptor compliance is implemented: no custom icon shapes, `outputSchema` describes only `structuredContent` for stable-object tools, non-object `structuredContent` is never emitted, and Atlas-specific diagnostics live under `_meta`.
- Protocol errors versus tool execution errors are classified at the dispatcher boundary: unknown tools, malformed `tools/call`, and decode failures return JSON-RPC errors, while input validation, business-rule, and downstream failures return `CallToolResult` with `isError: true`.
- Shared `ToolErrorPayload` is implemented with stable error codes (`invalid_input`, `file_not_found`, `symbol_not_found`, `graph_stale`, `timeout`, `dependency_failed`, `internal_tool_error`), concise human text in `content`, and machine-readable `details`, rendered through one shared result helper across all migrated tools.
- Tool-input hardening is implemented from transcript-derived failure fixtures: omitted or empty `subpath` normalizes to repo root, `read_file_excerpt` selector parsing tolerates wrapper-emitted absent-equivalent fields, path-validation errors expose repo root and canonical path guidance, and conflicting mode/selector inputs return structured self-correcting retry errors.
- Mixed result contracts are normalized so every agent-facing tool emits deterministic object `structuredContent` with exact `outputSchema`; no tool returns bare strings, bare arrays, or mode-dependent alternate envelopes, and generated `MCP_TOOLS.md` lists only `stable-object` or `text-only` contracts.

## MCP Tool Input Ergonomics

- Discriminated input objects are implemented for previously ambiguous families: `get_context.target`, `read_file_excerpt.selector`, `get_docs_section.selector`, shared `repo_scope`, and shared `change_source`, each with required `kind` enum values and per-kind companion fields. The former `build_or_update_graph.operation` selector was later replaced by direct `build_graph` and `update_graph` tools.
- Legacy ambiguous fields were removed after a compatibility window: no `get_context` `query`/`file`/`files` precedence, no `batch_query_graph` `text`/`queries` override behavior, no flat `repo_id`/`all_repos` scope fields, and no change-source boolean/mode duplication.
- `batch_query_graph` uses an `items` array with the same query object shape as `query_graph`, max-length and empty-array enforcement, and fail-closed conflict rejection.
- Query-intent grammar is explicit for `query_graph` and `get_context` query targets: plain identifiers, exact qualified names, `who calls <symbol>`, `what breaks <symbol>`, and `tests for <symbol>`, with retry guidance for prose-only queries.
- Stable object results are implemented for `query_graph`, `batch_query_graph`, `search_saved_context`, `search_decisions`, `cross_session_search`, and the file/content discovery family, each with exact `outputSchema` validating emitted `structuredContent`.
- Runtime `tool_help`/`man` output includes an `input_contract` section per tool with discriminant field, accepted enum values, required companion fields, mutually exclusive legacy fields, and one minimal valid example per variant; generated `MCP_TOOLS.md` and installed AGENTS instructions reference runtime docs as canonical.
- Registry and snapshot tests enforce the contracts: hidden precedence wording, legacy ambiguous field groups, missing object `structuredContent` schemas, and stale manual docs fail CI.

## MCP Tool Manual and Schema Introspection

- Shared manual-documentation service is implemented from the live MCP tool registry so per-tool docs are never duplicated in hardcoded tables; lookup requires exact case-sensitive tool names, supports namespace `mcp` only with clear validation errors for unknown namespaces, returns deterministic document payloads without executing the target tool, and stays read-only and safe in restricted environments.
- `man` response shape is implemented with requested and resolved tool identity, description, tool structure, input-args section (field name, type, required/optional state, default, accepted enum values, per-field description), output-response section with optionality and metadata/error payload shapes, usage section in `man mcp <tool>` form with invocation examples, error section for unknown/deprecated/hidden/schema-unavailable tools, and deterministic field ordering for CLI, MCP, snapshots, and generated docs.
- MCP `man` tool and CLI `atlas man mcp <tool>` are implemented with parity: compact default output for agents, `--json` matching the structured payload, visible registered tools only, nearest-name suggestions on unknown targets using deterministic ranking, bounded truncation of oversized examples and descriptions without dropping required sections, and freshness/provenance metadata when manual output depends on generated registry state.

## MCP Elicitation, Durable Tasks, and MRTR

- Elicitation is implemented with form and URL modes, single- and multi-select enums, titled and untitled enum values, default values on primitive fields, and typed response validation; `purge_saved_context` requires elicited confirmation before running without a `session_id`.
- Durable tasks are implemented with continuity-owned SQLite persistence: task id, originating method, lifecycle timestamps, status, progress snapshot, final result or error, polling, deferred result retrieval, and cancellation where the underlying job is cancellable.
- MRTR (Multi Round-Trip Requests) is implemented for `2026-07-28`: server-initiated JSON-RPC requests are replaced with `InputRequiredResult`/`inputRequests`/`inputResponses`, `requestState` is integrity-protected with method/tool/args digest, expiry, and authenticated principal where available, and destructive confirmations return `resultType: "input_required"`.
- Tasks follow the `2026-07-28` extension shape: advertised under `extensions`, old `tasks/list`/`tasks/result`/`tasks/cancel` handlers removed, `tasks/get` and `tasks/update` implemented, and long-running tool calls return task handles without per-request opt-in.
- The previous reverse-request broker and URL elicitation id flow are retired in favor of MRTR.

## MCP Repo Selection

- `atlas serve --direct-stdio` no longer binds repo from inherited process cwd when `--repo` and `--db` are absent; explicit `--repo`/`--db` remain fixed-mode overrides that always win.
- Repo identity is resolved from explicit inputs only: `repo_root`/`repo_id` tool arguments, repo-scoped resource URIs, or server configuration, with canonical path identity preserved for DB paths, session keys, and provenance.
- Ambiguous repo selection fails closed with actionable tool-execution errors naming candidate roots and accepted selectors; selection source (`explicit_cli`, `explicit_request`, `cached_active_root`) is tracked for debugging.
- Deprecated Roots machinery is fully removed: no `roots/list` parsing, no dynamic-roots APIs, and repo-scope installs pass `--repo <path> serve` while user-scope installs do not rely on Roots.

## Tokenizer-Backed Context Budget Accounting

- Shared token-counting crate `atlas-token-count` is implemented as the only crate that depends on external tokenizer machinery, wrapping local tokenizer JSON files and the byte heuristic (`bytes.div_ceil(bytes_per_token)`) behind one deterministic `TokenCounter` API (`TokenCounter::heuristic`, `TokenCounter::from_file`, `count_text`, `count_json_bytes`) with count-method metadata (`TokenCountMethod`, `TokenCount`, `fallback_reason`); tokenizer errors carry path/config context without payload content.
- Configurable token accounting is implemented under `[context.tokenizer]` in `.atlas/config.toml` with provider (`heuristic` default or `tokenizers`), optional model metadata, relative `tokenizer_file` resolution from `.atlas/`, `fallback` mode (`heuristic` default or `fail_closed`), and `bytes_per_token` (default 4), rendered in generated config templates (commented example values never activate missing files) and validated at load with config-key-named errors for blank values and zero byte-per-token.
- Runtime token counter loading is implemented through `Config::token_counter` returning a ready counter plus load-fallback metadata (`fallback_used`, `fallback_reason`): missing or invalid tokenizer files fall back to the byte heuristic by default, and `fallback = "fail_closed"` errors before payload truncation with an actionable message naming `context.tokenizer.tokenizer_file` and the resolved path.
- Review/context payload budgets are enforced through the configured counter: `PayloadMeasurement` measures the same serialized JSON payload CLI/MCP emit (payload metadata stripped), byte caps stay independent transport/memory safety limits, token budgets use real tokenizer counts, deterministic trimming order and direct-target retention are preserved, `omitted_byte_count` semantics are unchanged, and `tokens_estimated` keeps its compatibility meaning as the counted token value.
- Token accounting metadata is surfaced in JSON outputs through `truncation.payload.token_accounting` (`provider`, optional `model`, `fallback_used`, `fallback_reason`, `bytes_per_token`) without exposing tokenizer file contents, with additive MCP output schemas for `get_context`/`get_review_context` structured content and concise CLI text payload lines; metadata appears whenever payload truncation metadata exists, including token-budget overrides with no dropped units.
- Deterministic tokenizer quality gates are implemented with a committed character-level WordPiece fixture whose known counts differ from `bytes.div_ceil(4)`: review truncation-order regression tests (saved context before files, files before direct target nodes, direct targets retained), CLI and MCP integration/parity tests for tokenizer, heuristic-fallback, and fail-closed modes, and exact `tokens_estimated` equality with fixture-backed counts so regressions to byte-only accounting fail CI.
- Documentation is implemented for byte versus tokenizer-backed caps, default heuristic mode, local tokenizer file mode (local JSON inputs only, never downloaded), fallback metadata fields, and fail-closed behavior in README, with config template snapshot tests and a docs drift guard.

## Runtime Metrics

- Process-local metrics registry `atlas-metrics` is implemented with lock-free `AtomicU64` counters, fixed-bucket histograms (count/sum/min/max/mean/p50/p90/p99), labeled metric families, a derived parser cache reuse ratio, and a serializable `MetricsSnapshot`; recording never panics on poisoned label locks.
- Build and update runs are instrumented at the public engine boundary with run counters, failure counters, end-to-end duration histograms, and parsed-files-per-run histograms, recording both successful and failed runs.
- Parser instrumentation is implemented in `ParserRegistry::parse`, counting parse attempts and tree-cache reuse when a cached old tree is supplied to tree-sitter for incremental re-parsing.
- Query instrumentation is implemented in `execute_query_with_embedding`, recording call counts and latency histograms by execution mode; `explain_query` uses the unrecorded inner path so an MCP `query_graph` call that then explains itself does not double-count.
- MCP tool dispatch is instrumented with per-tool call counters split by `ok`/`error` outcome and per-tool duration histograms.
- Operator surfaces are implemented through the MCP `get_metrics` tool and an additive `metrics` block in `atlas build --json` / `atlas update --json` for non-dry-run runs, so dry-run golden output stays deterministic while CI can read build duration and parser cache reuse ratio.

## Wake-Up Packs and Session Start Recall

- `WakePack` is implemented with `repo_root`, `session_id`, `frontend`, `current_focus`, `recent_decisions`, `critical_memories`, `recent_feedback`, `active_memoir_concepts`, `changed_files`, `graph_readiness`, `retrieval_hints`, and `generated_at`, bounded through config and central budget policy and serialized to stable JSON.
- `atlas wake-up` is implemented with `--topic`, `--session`, `--frontend`, `--max-items`, and `--json`, pulling content from memory, feedback, session resume, and graph readiness services; topic-relevant memories and feedback are prioritized and large artifacts are referenced by `source_id` only.
- MCP `wake_up` is implemented as the bounded session-start recall surface for hookless hosts: it assembles `current_focus`, `recent_decisions`, `critical_memories`, `recent_feedback`, `active_memoir_concepts`, `changed_files`, `graph_readiness`, and `retrieval_hints` from the resume snapshot, decision memory, saved-context hints, and global memory; large artifacts are referenced by `source_id` only; and the wake-up is recorded through the shared agent event service as a `session-start` event with resume-snapshot consumption and LoadRestore parity.
- Wake-up generation is called from `SessionStart` hook paths where the host supports it, attaches to session resume only through bounded injection paths, records success or failure metadata in session events, and keeps hook failures non-blocking and best-effort.
- Snapshot coverage is implemented for empty, normal, and large sessions, and hook failures never stop host command flow.

## Cross-Session Recall and Hybrid Recall

- Cross-session memory recall is implemented across prior repo sessions without mixing raw session history into future context, while preserving agent and frontend visibility boundaries.
- Recall ranking is implemented over topic match, importance, recency, scope visibility, and source-backed evidence so the highest-signal memories surface first.
- Optional memory embeddings are implemented under `[memory.embedding]` (`enabled`, `provider`, `model`, `dimension`) and `[memory.search]` (`hybrid_weight_fts`, `hybrid_weight_vector`, `min_vector_similarity`), disabled by default and requiring explicit opt-in; the configured provider is limited to the supported `search` backend.
- A `memory_embeddings` table is implemented with `memory_id`, `embedding_model`, `dimension`, `vector_blob`, and `created_at`, with a model index and cascade delete; inserts are rejected when the configured dimension or model differs from stored metadata (`memory_embedding_dimension_mismatch`).
- `atlas memory embed [--dry-run]` is implemented to backfill embeddings and report coverage, and `atlas memory recall <query> --hybrid` is implemented with reciprocal-rank fusion over lexical hits plus vector similarity.
- Hybrid ranking keeps graph-backed and exact lexical evidence stronger than vector-only matches by default: rows are ordered by lexical relevance tier first, then hybrid score, and vector-only hits are appended after lexical rows with explicit `vector_score` and `hybrid_score` fields.
- Enabling embeddings without a provider or with an invalid dimension fails clearly before recall, and baseline lexical recall continues to work with no embedding provider configured.

## Memoir Concept Graph

- Separate memoir storage is implemented with `memoir_graphs`, `memoir_concepts`, and `memoir_relations` tables outside the code graph schema, keeping semantic memory from leaking into graph `nodes` and `edges`.
- Relations persist `graph_id`, `source_concept_id`, `target_concept_id`, `relation_type`, `confidence`, `source_id`, `created_at`, and `metadata_json`.
- A controlled relation vocabulary is implemented with `depends_on`, `part_of`, `contradicts`, `refines`, `replaces`, `caused_by`, `fixed_by`, `blocked_by`, `decided_by`, and `related_to`; aliases such as `replaced_by` and `separate_from` normalize to canonical direction or tagging rules, and unknown relation types are rejected unless custom relations are later enabled.
- `atlas memoir create`, `atlas memoir add-concept`, `atlas memoir link`, and `atlas memoir inspect` are implemented with `--description`, `--scope`, `--kind`, `--source-id`, `--confidence`, `--graph`, `--depth`, `--relation`, and `--json`, with MCP `memoir_create`, `memoir_add_concept`, `memoir_link`, and `memoir_inspect` as thin wrappers over the same service layer.
- Duplicate graph names fail deterministically in the same repo and scope, bounded inspect output includes relation direction and source evidence ids, and code graph queries remain unaware of memoir tables unless the explicit memoir surface is invoked.

## Typed Memory Operator Algebra

- A typed `MemoryOp` operator enum is implemented with `Set`, `Sum`, `Count`, `Max`, `Avg`, `AppendUnique`, `EuaPatch`, and `TimeCompress`, serialized with `#[serde(tag = "op", rename_all = "snake_case")]` so operator history is stable JSON.
- `apply_memory_op(entity, op, now)` is implemented as a pure in-memory transition returning `OpOutcome { changed_fields, numeric_value, history_entry }`; `Set` overwrites top-level fields and `metadata.<key>`, `Sum` uses `i64::checked_add`, `Count` increments metadata plus `last_seen`, `Max` supports numeric and RFC 3339 values, `Avg` keeps a running sample count, and `AppendUnique` dedupes exact values under `max_items` with zero rejected.
- Operator execution is deterministic and local with no LLM, embedding, or network call; one operator applies per transaction, `op_history_json` keeps the newest 32 bounded `{op, at, fields}` entries, operators on superseded rows are rejected, and overflow, NaN/infinite averages, and unknown enum variants fail instead of clamping.
- The `memories` table gained `op_history_json` and `numeric_value`, plus a `memories_fts` FTS5 table over topic, title, and body with insert/update/delete triggers, rebuild backfill, and `atlas db check` checks for FTS drift and malformed operator history.
- `SessionStore::apply_memory_op(memory_id, &MemoryOp)` is implemented to load a row, apply one operator, and write body, metadata, numeric value, operator history, and `updated_at` in one transaction; enriched command-failure events route through `Count`, `Max(last_failed_at)`, and `AppendUnique(error_signature)` instead of ad-hoc SQL.
- Read-only `atlas memory ops <memory_id> --json` and MCP `memory_ops` are implemented with identical request fields, defaults, history entry shapes, and unknown-id handling.

## Structured Memory Consolidation and EUA Patch Application

- Deterministic structured consolidation is implemented in `memory_consolidate.rs`: members are sorted by `created_at ASC`, bodies split into normalized sentence and bullet units on newline, `". "`, `"! "`, `"? "`, and `"。"`, and units dedupe by lowercase, whitespace-collapsed, punctuation-stripped keys while keeping the first chronological original text.
- Consolidated output preserves every distinct fact up to `OVERSIZED_BODY_CHARS`, routes the full synthesized body through `context.db` with a `source_id` reference on overflow, carries `merged_source_ids` in `metadata_json`, takes the highest member importance, the group-key topic, and the first member title.
- `consolidation_plan()` returns `body`, `metadata_json`, and `merged_source_ids` in dry-run output; apply mode writes one consolidated row, `memory_supersessions` rows with `reason = 'consolidated'`, and `superseded_by` markers, with dry-run leaving storage unchanged.
- The EUA patch parser is implemented in `memory_patch.rs` for `<<<< SEARCH` / `====` / `>>>> REPLACE` blocks, multiple blocks per patch in document order, and typed `memory_patch_invalid` errors for unbalanced or nested markers.
- Patching applies exact substring replacement first and requires a unique match (`memory_patch_ambiguous` otherwise), then falls back to bounded Levenshtein over candidate windows of `search.chars().count()`; `max_edit_distance` defaults to `max(1, ceil(0.15 * search chars))` and is rejected above half the search length.
- Each patch records one `MemoryOp::EuaPatch` history entry with `search_hash`, `replace_len`, `edit_distance`, and `matched_exact`; superseded rows and patches producing an empty body are rejected.
- `atlas memory store --patch-id <memory_id> --patch-file <path>` and `--patch-text <text>` are implemented with matching MCP `memory_store` fields `patch_id`, `patch_file`, and `patch_text` and identical validation.
- `TIME_COMPRESS` is implemented in `curation.rs` through `compress_event_group`, aggregating evicted rows per topic, file, and command into a `SESSION_SUMMARY` metadata record with `total_reads`, `command_runs`, `failure_count`, `first_seen`, `last_seen`, and `source_ids`.
- A `SESSION_SUMMARY` event is written before any `DELETE FROM session_events`; `DECISION` and `USER_INTENT` events plus `critical` memories and their supersession chains are kept out of compression and deletion, compression and deletion share one transaction that rolls back when the summary write fails, `MemoryOp::TimeCompress` is called from `compact_session_events()`, and repeat compaction is idempotent.

## Two-Path Quota-Based Memory Recall

- Memory recall uses `memories_fts MATCH` instead of `LIKE '%query%'`, with the existing FTS escaping helper, multi-token AND matching first and OR fallback only when AND returns zero rows, while exact-topic matches keep their higher lexical tier.
- Recall scoring is implemented as `(1 - weight_time - weight_business) * S_origin + weight_time * S_time + weight_business * S_busi` clamped to `0..=1`, where `S_origin` is normalized BM25 plus importance, scope, and source-backed weights, `S_time` is `1.0` inside the freshness window and `1 / (1 + age_days / 30)` after it, and `S_busi` maps `critical = 1.0`, `high = 0.75`, `normal = 0.5`, `low = 0.25`.
- Recall config is implemented under `memory.recall` with `freshness_window_days` (default 1), `weight_time` (0.25), `weight_business` (0.15), `primary_quota_ratio` (0.75), and `aux_max_hops` (default 2, max 2), validated for non-negative freshness, weights in `0..=1`, and `weight_time + weight_business < 1`.
- An auxiliary memoir path is implemented in `memory_recall_aux.rs`: query tokens match `memoir_concepts.name` and `description` case-insensitively, `memoir_relations` traverse from one to `aux_max_hops` with the bounded BFS used by `memoir::inspect`, concept `source_id`s map to `memories.source_id`, and candidates not already returned by the primary path are ranked independently with depth and relation-type penalties.
- Quota merge is implemented as `Q_primary = ceil(primary_quota_ratio * limit)` and `Q_aux = limit - Q_primary`, with auxiliary filling leftover capacity when primary underfills; primary rows come first, then auxiliary rows, each tagged `recall_path = "primary"` or `"auxiliary"`.
- Recall JSON includes `origin_score`, `time_score`, `importance_score`, and `recall_path`; superseded rows and rows outside caller visibility scopes are excluded from both paths, the memoir path is read-only and never auto-creates concepts, and recall works with no embedding provider configured.

## Code Overview Memory

- Overview domain types are implemented for `OverviewSubjectKind` (`project`, `package`, `module`, `file`, `symbol`, `function`, `method`), `OverviewFreshness` (`fresh`, `possibly_stale`, `stale`, `orphaned`, `unverified`), and `OverviewSourceKind` (`external_llm`, `manual`, `imported`).
- `OverviewSubject` validation is implemented per kind: `project` requires `repo_root` and `commit_sha`; `package` requires `package_name`; `module` requires `module_path`; `file` requires `canonical_file_path`; `symbol`, `function`, and `method` require `qualified_name`, `canonical_file_path`, `line_start`, and `line_end`.
- Subject identity is deterministic and canonical: every path is canonicalized through `atlas_repo::CanonicalRepoPath` before hashing, persistence, dedupe, lookup, or stale matching, and `subject_id` is a versioned hash over `repo_root`, kind, canonical subject fields, and schema version, so `./src/lib.rs` and `src/lib.rs` resolve to the same subject.
- `OverviewRecord` is implemented with subject, title, summary, description, responsibilities, flow, inputs, outputs, invariants, gotchas, examples, tags, source metadata, freshness, confidence, source ids, and timestamps; list fields are typed vectors at the service boundary and JSON arrays at storage.
- Overview text is bounded through central budget policy before storage, and oversized bodies route through content-store artifacts with only a preview plus `source_id` kept in the overview row.
- Continuity-owned `overview_records` storage is implemented with the full record column set, `overview_record_fts` over title, summary, description, responsibilities, flow, invariants, gotchas, and tags, indexes for repo, subject, kind, path, qualified name, commit, freshness, source kind, and updated time, and `supersedes_id` supersession.
- Overview uniqueness is enforced for active records by `repo_root`, `subject_id`, `commit_sha`, and `source_kind` unless `supersedes_id` is set; old records are preserved for audit.
- Storage APIs are implemented for `insert_overview_record`, `get_overview_record`, `search_overview_records`, `list_stale_overview_records`, `supersede_overview_record`, and `mark_overview_freshness`, with invalid enum strings and malformed JSON arrays rejected before writing.
- `atlas db check` validates overview schema, FTS integrity, orphan `source_id` references, invalid enum values, and noncanonical path rows; migration golden tests, in-memory storage tests, FTS search tests, and db-check failure fixtures cover the layer.

## Overview Analysis Packets and External Ingest

- `OverviewExportRequest` is implemented with `scope`, `subjects`, `changed_only`, `since`, `limit`, `include_code_spans`, `include_callers`, `include_callees`, `include_tests`, `max_tokens`, and `json`.
- `OverviewAnalysisPacket` is implemented with `schema_version`, `repo_root`, `commit_sha`, graph provenance, freshness metadata, subject identity, concise graph evidence, bounded code excerpt, callers, callees, related files, test adjacency, and retrieval hints; packet id is a stable hash over subject id, commit SHA, selected evidence ids, and export schema version.
- Export scopes `project`, `packages`, `modules`, `files`, `symbols`, `functions`, `methods`, and `changed` are implemented through `atlas overview export --scope`, `--subject`, and `--changed --since`, using graph-backed context resolution first and companion content lookup only for graph or context-evidenced assets.
- Every packet carries canonical subject identity, `safe_to_answer`, graph freshness, omitted counts, and budget-hit metadata; corrupt graph readiness fails clearly, while stale export is allowed only with explicit stale metadata.
- `OverviewAnalysisInput` is implemented with `schema_version`, `packet_id`, subject identity, title, summary, description, responsibilities, flow, inputs, outputs, invariants, gotchas, examples, tags, confidence, analyzer metadata, and optional source artifact ids.
- `atlas overview ingest <path>` and `atlas overview ingest -` are implemented with exact `schema_version` validation first, optional `packet_id` match validation, subject existence checks against the current or indexed graph, and line-range overlap checks before marking an ingested record `fresh`.
- Ingest freshness is deterministic: mismatched commit SHA with a resolvable subject marks `possibly_stale`, unresolvable canonical path or qualified name marks `orphaned`, and manual or imported records without a packet id mark `unverified`; confidence is enforced in `0.0..=1.0`, `summary` is required, and either `description` or a non-empty detail array is required.
- Unknown top-level fields are rejected unless `metadata_json.extra` captures them through a controlled importer path; overview text passes central redaction before persistence, and large descriptions or examples route through the content store.
- `atlas overview run-external --dry-run --json` prints exact configured external command invocations without executing them, and apply mode requires JSON output, validates ingest, and reports per-subject success or failure; external analyzer handoff remains packet generation plus a documented command contract, not built-in model execution.

## Overview Freshness, Read Surfaces, and Context Integration

- `OverviewFreshnessService` is implemented to compare overview records against current graph readiness, current commit SHA, changed files, content hashes, and line-span overlap.
- Direct staleness is marked when a record's canonical path changed since the stored commit, its content hash differs from the current file hash, or its qualified name no longer resolves; `possibly_stale` is marked for callers, containing modules, containing packages, and project records when dependent files change.
- Orphan marking is implemented for deleted files, removed symbols, and renamed subjects without a confident canonical target, using `detect_changes` or history data with existing repo and change services as the git fallback.
- `atlas overview stale` is implemented with `--subject-kind`, `--file`, `--package`, `--module`, `--since`, `--limit`, and `--json`, and `atlas overview refresh-plan --changed --since` reports subject id, stale reason, suggested export command, affected dependents, previous record id, and priority.
- Refresh priority is deterministic: direct change before dependent change, subject-kind specificity, current context relevance, confidence, and updated-at age; refresh planning is read-only unless an explicit apply flag marks freshness.
- `atlas overview show <subject>` resolves exact subject id, qualified name, canonical file path, package name, or module path, and ambiguous results fail with a candidate list; `atlas overview search` supports subject-kind, freshness, package, module, file, source-kind, limit, and JSON filters with exact-match, title or tag, FTS, freshness, confidence, recency, and specificity ranking.
- `atlas overview list`, `atlas overview delete <overview_id> --dry-run --json`, and `atlas overview export-docs --format markdown --output <path>` are implemented; delete requires an exact overview id and keeps routed artifacts, while exported docs group by project, package, module, file, then symbol and visibly label stale records.
- Context requests are implemented with `include_overviews`, `overview_limit`, `overview_freshness`, `overview_subject_kinds`, and `overview_max_bytes`, and fresh overviews join `atlas context`, `atlas review-context`, MCP `get_context`, and MCP `get_review_context` under the existing graph, content, and session budget policy; stale records appear only when requested and always carry a stale reason.
- Overview context ranking and selection reasons are implemented (`same_symbol`, `same_file`, `containing_module`, `package_summary`, `project_summary`, `changed_dependency`), payloads stay preview-only by default with `source_id` for full retrieval, and overview text cannot override graph facts in risk, impact, dead-code, or refactor analysis.
- MCP `overview_export`, `overview_ingest`, `overview_search`, `overview_show`, and `overview_refresh_plan` are implemented with the same request fields, defaults, validation, ambiguity behavior, and JSON shapes as the CLI, with compact defaults carrying `source_id`, freshness, selection reason, and next export command where relevant.
- `[overview]` config is implemented with `enabled` (default true), `auto_export_on_commit` (false), `auto_mark_stale_on_commit` (true), `external_analyzer_command`, `ingest_after_external_command` (false), `max_subjects_per_run`, and `max_packet_bytes`; external analyzer commands are explicit and never inferred from environment variables.
- Hook integration can run `overview refresh-plan` after a commit or manual trigger, failures stay best-effort and non-blocking, oversized packets route to the content store and are referenced by `source_id`, and hook outcomes are recorded as session events with command, status, packet count, ingested count, stale count, and source ids.
- Release gate `ICM Overview Memory Complete` is defined and satisfied by storage schema, export and ingest contracts, freshness planner, CLI read surfaces, context integration, MCP parity, hook and manual trigger path, docs, fixtures, and JSON snapshots, with no overview body written to `worldtree.db`, no built-in LLM provider, no stale overview emitted as fresh, and no path-derived identity without `CanonicalRepoPath`.

## Progressive Overview Levels and `atlas://` Resources

- `OverviewLevel` is implemented with exact `l0`, `l1`, and `l2` values, lowercase serde, and `FromStr`; `render_level(record, level)` returns `OverviewLevelView` where L0 is title plus summary only, L1 adds description preview, responsibilities, flow, inputs, outputs, invariants, and gotchas, and L2 adds `source_id`, canonical file path, line span, and a `read_file_excerpt` hint.
- Every level view carries freshness so `stale` and `orphaned` remain explicit, and `OVERVIEW_L0_MAX_CHARS`, `OVERVIEW_L1_MAX_CHARS`, and `OVERVIEW_L2_MAX_CHARS` truncate with `truncated: true`.
- The MCP resource layer is split into `resources/mod.rs`, `resources/docs.rs`, and `resources/registry.rs` with behavior preserved, and `resources/uris.rs` implements `atlas://repo/{repo_id}/tree/{dir}`, `atlas://repo/{repo_id}/file/{path}`, `atlas://repo/{repo_id}/symbol/{qualified_name}`, `atlas://memories/topic/{topic}`, `atlas://memories/id/{memory_id}`, `atlas://memoir/{graph}/{concept}`, and `atlas://artifacts/{source_id}/{chunk_index}`.
- URI parsing and formatting percent-encodes path segments, rejects `..`, absolute paths, empty segments, and encoded traversal, canonicalizes repo paths through `atlas_repo::CanonicalRepoPath` before lookup, and accepts `?level=l0|l1|l2` only on tree, file, and symbol URIs.
- Every URI form appears in `resources/templates/list` with handler-identical defaults; `Read` defaults tree, file, and symbol to L1 and artifacts to L2, `List` returns immediate children only for tree URIs with child directories, files, and L0 abstracts, `Grep` delegates to `search_content` with the URI subtree as `subpath`, and `Search` delegates to `query_graph`, `overview_search`, or `memory_recall` with the subtree as scope.
- `atlas uri read <uri>` (`--level`, `--json`) and `atlas uri list <uri>` (`--json`) are implemented, `--level` is implemented on `atlas overview show` and `atlas overview search`, and the same `level` field exists on MCP `overview_show` and `overview_search` with identical defaults.
- `SelectionReason::OverviewLevel` attaches `uri`, `level`, and `freshness` to overview context entries; CLI and MCP URI read and list parity tests enforce identical shapes, L0 read bytes stay at or below L1 and L2 bytes for the same subject, and a check enforces every shipped `atlas://` URI appears in `resources/templates/list`.

## Shell-First Install Modes and Memory TUI

- `atlas init --mode` is implemented with `mcp`, `hook`, `cli`, and `all` values; each mode is idempotent, `--dry-run` reports every file to be created, and `--mode all` installs only MCP config, hooks, and CLI config relevant to shell-first memory workflows. Plain `atlas init` defaults to non-invasive MCP config.
- `atlas memory tui` is implemented as a read-only surface for memories, topics, feedback, memoir concepts, overview records, health findings, and saved artifacts, with memory filters for topic, scope, importance, frontend, subject kind, freshness, package, module, file, source kind, updated time, and limit.
- The first TUI version is non-mutating, smoke-testable without panic, and emits a single deterministic frame for non-TTY or `--json` use while the interactive loop stays navigation-only.
- Reusable memory fixtures are implemented for critical decision memory, low-priority stale memory, dead-code false-positive feedback, a memoir dependency graph, overview records, a wake-up pack with saved-artifact references, and a frontend-private memory.
- JSON snapshots are implemented for `atlas memory store`, `atlas memory recall`, `atlas memory health`, `atlas feedback record`, `atlas feedback search`, `atlas memoir inspect`, `atlas overview show`, `atlas overview search`, and `atlas wake-up`, with deterministic normalization for wall-clock-derived fields.
- Release gate `ICM Memory Layer Complete` is defined and satisfied by CLI and MCP memory parity, importance and decay policies, feedback-adjusted analysis, memoir typed relations, overview export and ingest context parity, wake-up packs without raw large content, health audit coverage, shared and private visibility rules, docs, and JSON snapshots.

## Directory-Semantic Query Scope and TrieHI Indexing

- Content-store directory hierarchy is implemented with `sources.parent_dir` and `sources.relative_path`, backfilled from canonical path identity with deterministic parent derivation, indexed by `(repo_root, parent_dir)` and `(repo_root, relative_path)`, and rejected when `relative_path` is not canonical.
- `SearchFilters` is implemented with `subpath_prefix`, `exact_dir`, `exclude_prefixes`, and `limit` (default 50, capped at 500); recursive scope compiles to `relative_path >= ? AND relative_path < ?`, non-recursive scope to `parent_dir = ?`, exclusions to `NOT (relative_path >= ? AND relative_path < ?)`, and both hardcoded `LIMIT 50` clauses are replaced by the bound limit.
- Vector scope pre-filtering is implemented with `VectorScope { subpath_prefix, exact_dir, exclude_prefixes, repo_ids, kinds, languages }`; `nodes_by_vector_similarity` joins `retrieval_chunks` to `nodes` and `files` so scope, kind, and language predicates apply in SQL before cosine scoring, missing `repo_root` fails closed instead of scanning every repo, and `atlas-search` and `atlas-review` callers pass explicit scopes including hybrid retrieval.
- `--exact-dir` and repeatable `--exclude-subpath` are implemented on `atlas query`, `atlas symbols`, `atlas context`, and `atlas review-context`, with the same fields on MCP `query_graph`, `list_symbols`, `get_context`, `get_review_context`, and `search_content`; parity tests and a candidate-count comparison cover the scoped path.
- An optional in-memory TrieHI index is implemented in `directory_trie.rs` with `DirectoryTrie` and `TrieNode { local, inclusive, children }`, maintaining `inclusive(v) = local(v) union of child inclusive(child)`, built lazily when `retrieval_chunks` exceeds `retrieval.triehi_min_chunks` (default 10000, validated `>= 1`).
- Recursive, non-recursive, and exclusion queries resolve to roaring bitmaps without SQL and apply before cosine scoring with `scope_candidates` debug metrics; git rename and move detection maps to `DSM MOVE` with `O(depth)` ancestor-set updates, falling back to a `retrieval_chunks` rebuild when a move cannot be relinked, and tests cover insert, move, merge, and delete invariants plus bitmap-versus-SQL equality.

## Evidence Sufficiency and Adaptive Escalation

- `EvidenceSufficiencyReport` is implemented with `target_resolved`, `callers_complete`, `tests_covered`, `dispatch_complete`, and `overview_fresh`, and `EvidenceGap` with `target_unresolved`, `callers_truncated`, `callees_truncated`, `tests_missing`, `dispatch_unresolved`, and `overview_not_fresh`.
- `sufficiency_score` is the weighted fraction of satisfied required constraints derived from intent: `Review`, `Remove`, and `Safety` require `target_resolved`, `callers_complete`, and `tests_covered`; `Trace` requires `callees_complete`; `Overview` requires `overview_fresh`, with `escalation_reason`, `next_tools`, and `sufficiency_score` exposed.
- Round 1 signal collection records caller and callee bucket truncation, unresolved target ambiguity, direct or mapped test presence with explicit `tests_verified_absent`, unresolved `Implements` and `ImplementedBy` edges plus dynamic registration targets, and `overview_fresh` from matched `OverviewRecord.freshness` with `unverified` treated as not fresh, all recorded in `ContextResult` metadata.
- Bounded escalation is implemented under `context.escalation` with `enabled` (default true) and `max_rounds` (default 1, capped at 3); Round 2 runs only for unsatisfied required constraints and fetches only missing evidence kinds.
- Escalation stops when the `TokenCounter` budget is exhausted and records `skipped_reason = "budget_exhausted"`; `evidence_gaps`, `sufficiency_score`, `escalation_reason`, `next_tools`, and `rounds_used` are emitted in CLI `--json` and MCP context and review outputs, and the decision trace is reconstructible from metadata without any LLM call.

## Query-Conditioned Experience Edges

- `experience_edges` storage is implemented in `session.db` with canonical source and target URIs, query text and tokens, optional query embedding, trace summary, commit SHA, source and target file hashes, `use_count`, `confidence`, timestamps, and `freshness` constrained to `fresh`, `possibly_stale`, or `invalid`, with confidence `0..=1`, `use_count > 0`, a uniqueness rule over `(repo_root, source_uri, target_uri, query_text)`, and repo-source and repo-target indexes.
- `NewExperienceEdge` validates canonical URIs, caps `trace_summary` at 512 characters, tokenizes query text into normalized lowercase tokens capped at 64, stores `query_embedding` only when an embedding provider is configured, and rejects dimension mismatches.
- Redundancy suppression is implemented as `targets \ existing_edge_targets` per `(repo_root, source_uri, query_text)` group; conflicting upserts increment `use_count`, refresh `last_validated_at`, and keep the higher confidence, and `invalidate_by_paths` marks one-endpoint edges `possibly_stale` and deleted-endpoint edges `invalid`, wired to changed file hashes from `atlas update`.
- `activate_experience_edges(repo_root, query_text, intent, limit)` returns only fresh edges, activating by cosine `>= 0.80` when embeddings exist or by token Jaccard `>= 0.65` plus intent match otherwise, ranking by `confidence * ln(1 + use_count)` and capping at `context.experience.max_edges` (default 3).
- `ContextEngine` Round 1 is seeded with activated targets using `SelectionReason::ExperienceShortcut`, each carrying `edge_id`, `confidence`, `trace_summary`, and `commit_sha`; shortcuts are never emitted for non-fresh edges or unverified hash and commit mismatches.
- Read-only `atlas experience inspect <uri>` (`--direction`, `--limit`, `--json`), `atlas experience prune --older-than <days> --dry-run`, and MCP `experience_inspect` are implemented with matching output, and `atlas db check` validates orphan URIs, invalid freshness, and confidence range.
- Tests cover redundancy suppression, hash invalidation, deleted targets, upsert counting, prune dry-run, stale-edge suppression, fresh-edge round reduction, and provenance presence; no experience edge is written to `worldtree.db`.

## Retrieval Evaluation and Token-Efficiency Gates

- `RetrievalMetrics` is implemented with `l0_tokens`, `l1_tokens`, `l2_tokens`, `scope_candidate_count`, `unscoped_candidate_count`, `aux_recall_at_k`, `primary_recall_at_k`, and `rounds_used`.
- Token efficiency is reported as L0/L1/L2 token ratios against the L2 baseline, DSQ pruning as candidate reduction for recursive, non-recursive, and exclusion predicates, auxiliary lift as the recall delta between primary-only and two-path recall, and escalation round reduction as rounds saved by experience edges.
- The RAG evaluation bench emits all four measurement families as JSON from a deterministic fixture corpus with fixed code, memory, memoir, and overview rows.
- Acceptance thresholds are enforced: L0 tokens at or below 10% of L2, DSQ candidate reduction at or above 50% for scoped queries, auxiliary lift above zero at `k = 10`, and escalation rounds at or below 1.
- Baseline JSON is stored under `packages/atlas-search/benches/baselines/`, regressions beyond 10% fail the bench gate, and a CI job runs the RAG eval bench and uploads the JSON artifact without gating merge.
- Methodology and thresholds are documented in `wiki/retrieval-evaluation.md`; the fixture corpus and baseline comparison are stable across consecutive runs, and release gate `RAG Adaptation Complete` is defined.

## Graph Store Corruption Recovery

- Shared `GraphStoreHealthClass` classification is implemented over existing `IntegrityState`, `GraphExecutionState`, and build lifecycle state with `healthy`, `stale`, `interrupted_build`, `failed_build`, `sqlite_corrupt`, `schema_mismatch`, and `logical_inconsistency` values.
- Physical SQLite corruption and logical graph inconsistency are split into stable machine-readable classes and codes while evidence sources stay consistent across `Store::open` errors, `PRAGMA integrity_check`, `PRAGMA foreign_key_check`, orphan-node scans, dangling-edge scans, `graph_build_state`, and freshness checks.
- CLI and MCP share the same classification, `health_class`, `error_code`, message, and suggestions for `status`, `doctor`, `db_check`, and `build_or_update_graph`.
- Recovery modes are implemented as `manual_rebuild_required`, `auto_quarantine_and_rebuild`, and `block_only`, with per-entry-point defaults: diagnostics are block-only, explicit `build` and `update` auto-quarantine and rebuild, and graph-backed query, context, and analyze tools block with a rebuild command.
- Graph-store quarantine is implemented for `.atlas/worldtree.db` with deterministic collision-safe paths such as `.atlas/worldtree.db.quarantine.<UTC>.<n>`, preserving the quarantined DB and WAL/SHM sidecars for inspection, creating a fresh migrated DB, and running a full rebuild from repository source rather than an incremental update.
- Quarantine and rebuild outcomes are recorded in graph build lifecycle state with failure reasons, and `health_class`, `recovery_mode`, `quarantine_path`, rebuild result, and failure reason surface in CLI JSON and relevant MCP responses.
- Graph-backed query, context, traversal, and analyze tools are wired through the existing `GraphReadiness::check_tool` policy and fail closed on `sqlite_corrupt`, `schema_mismatch`, or `logical_inconsistency` with `error_code`, `health_class`, `execution_state`, `db_path`, optional `quarantine_path`, and `recommended_rebuild_command`; diagnostics tools keep working, and stale-but-queryable is distinguished from corrupt-and-blocked.
- Tests cover physical corruption, logical inconsistency, rebuild success, rebuild failure, diagnostics-no-mutation, and fail-closed query behavior.

## Portable CLI Repository Paths

- Explicit `--repo` values expand leading `~`, `$HOME`, and `${HOME}` without shell evaluation, resolve relative values against the startup cwd, and reject missing or non-directory paths.
- Explicit and inferred repository roots are canonicalized through `atlas_repo` path APIs before identity use, and tests cover home expansion, relative paths, validation failures, symlink identity, and MCP instance identity equivalence.
- Portable fixed-repository editor configuration is documented without deprecated MCP Roots.

## Repository-Wide Symbol Inventory

- `atlas symbols` and MCP `list_symbols` are implemented for paginated repo symbol inventory with kind, language, subpath, and repo_id filters, shared kind-alias normalization in `atlas-core`, totals, and `next_offset` paging.
- A `/symbols` shell command is implemented over the same service surface.

## Developer Tooling and Reporting

- A canonical `docs/error_codes.md` catalog is implemented and referenced by README, MCP responses, and tests as the single error-code source.
- `MCP_TOOLS.md` is generated from the live tool registry, and a test plus docs check fails on drift from hand-maintained tool tables.
- Informational `cargo-llvm-cov` coverage reporting runs in a GitHub workflow job and uploads artifacts without gating merge.
- Per-crate criterion benches cover build, incremental update, query modes, context engine, and history reconstruction workloads, with a CI regression harness that parses `cargo bench --message-format=json` and stores comparable artifacts.
- Central `budget_policy` mapping is implemented in config with validation and documentation of byte-heuristic versus tokenizer-backed limits, tokenizer budget fields, tokenizer-backed accounting on context, review, and export `max_tokens` paths, deterministic byte and character fallback with fallback metadata, and tests across both modes.
- `proptest` coverage is implemented for ranking and trimming, canonical-path normalization, and FTS query escaping.

## Still Open

- Shared parser query migration remainder (Go migration and final validation pass) and minimal-context escalation metadata remain in ISSUES.md.
- Remaining Additional Backlog items (parser-cache CI gate, LSP shim) remain in ISSUES.md.

