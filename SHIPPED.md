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

## Insights Engine

- Deterministic insights engine is implemented with shared report primitives (`InsightSummary`, `InsightFinding`, `InsightEvidence`), stable severity values, deterministic sort order, and reuse of existing ranking/truncation and freshness/provenance metadata contracts; no LLM dependency and no writes to SQLite.
- Code health metrics engine is implemented for node-level fan-in/fan-out, dependency depth with cycle guard, reference counts, test adjacency, LOC, cyclomatic and cognitive complexity, branch counts, and max nesting depth (with per-language `not_available` instead of text guessing), file-level node/edge/import/test-coverage metrics, module-level coupling and cohesion approximations, and min/max/avg/p50/p90/p95 distribution statistics with outlier detection.
- Large and complex function finder is implemented with repo-wide and file-scoped modes, LOC/complexity threshold overrides, `large`/`complex`/`large-or-complex` modes, changed-file and fan-in/fan-out ranking boosts, complete finding payloads, and CLI `atlas insights large-functions` plus MCP `find_large_functions` with parity tests.
- Architecture analysis is implemented with module-level graph aggregation, SCC cycle detection with local/cross-module classification and deterministic cycle paths, inline layer-rule enforcement with violation findings, coupling scores, tightly-coupled cluster detection, and high-connectivity file flags.
- Risk assessment is implemented as explainable weighted `0-100` scores with public-API, fan-in/out, cross-module dependency, test adjacency, depth, unresolved-edge, size/complexity, and cycle-participation factors, each with factor contribution and evidence, plus configurable low/medium/high thresholds.
- Pattern detection is implemented for repeated call chains, unused/isolated structures with removal blockers, high-centrality hubs and bottlenecks, and deep chains with traversal caps and cycle guards.
- Insights surfaces are implemented across CLI (`atlas insights architecture|metrics|risk|patterns|large-functions|complex-functions` with `--json` and `--limit`) and MCP (`analyze_architecture`, `analyze_metrics`, `assess_risk`, `analyze_patterns`, `find_large_functions`, `find_complex_functions`) with freshness/provenance metadata and CLI/MCP parity coverage.
- Insight thresholds and inline layer rules are config-driven under `.atlas/config.toml` with positive-value validation, ignore lists for files/modules/node kinds, and actionable config errors.

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

## Hook and Agent Host Integration

- Install flows are implemented for Copilot, Claude, and Codex integration.
- Generated hook and MCP configuration support repo-local integration flows instead of requiring manual setup.
- Hook lifecycle coverage is implemented for session start, prompt submission, tool execution, compaction, stop, session end, and related host events.
- Thin hook runner architecture is implemented so shell launchers remain stable while Rust code owns normalization, routing, storage, and policy behavior.
- Shared agent event service is implemented in `packages/atlas-agent-events` so native hooks and future MCP fallback capture share one event policy, persistence, and action pipeline (`AgentEventRequest` / `AgentEventSource` / `AgentEventResult` / `record_agent_event`).
- MCP `record_session_event` fallback tool is implemented in `packages/atlas-mcp/src/session_events.rs` so hosts without native LLM hooks get hook-equivalent capture with the same event aliases, redaction, storage routing, lifecycle actions, graph refresh, and review refresh behavior as `atlas hook`.
- Installed AGENTS/CLAUDE instruction blocks now include a mandatory session memory fallback protocol for hookless hosts: wake-up/resume recall at start, `record_session_event` triggers for user-prompt/post-tool-use/file-changed/pre-compact/stop/session-end, `save_context_artifact` for decisions/errors/preferences/handoffs, and explicit do-not-store rules.
- MCP `wake_up` tool is implemented in `packages/atlas-mcp/src/wake_up.rs` as the bounded session-start recall surface for hookless hosts: it assembles `current_focus`, `recent_decisions`, `critical_memories`, `recent_feedback`, `active_memoir_concepts`, `changed_files`, `graph_readiness`, and `retrieval_hints` from the resume snapshot, decision memory, saved-context hints, and global memory; large artifacts are referenced by `source_id` only; and the wake-up is recorded through the shared agent event service as a `session-start` event (LoadRestore parity with native hooks, including resume-snapshot consumption).
- `atlas install --mode` is implemented with `mcp` (MCP config only), `hook` (git + agent hooks only), `cli` (instruction fallback text only), and `all` (default) modes that compose with the explicit `--no-*` flags.
- `atlas install --dry-run` output identifies every instruction fallback file it would create, append, refresh, or replace, and the install summary is dry-run aware.
- Hookless-host continuity is documented in `wiki/session-memory-fallback.md` (recall, capture triggers, artifact labels, compaction, do-not-store rules) with a native-hooks-vs-MCP-fallback compatibility matrix (`surface`, `works_with`, `automatic`, `token_cost`, `best_for`) also mirrored in README; Claude/Codex hook docs state that native hooks and the MCP fallback share one event service.
- Event-name parity is enforced by tests: installed instruction blocks, MCP registry descriptions, and wiki docs may only reference event names supported by `record_session_event` / native hooks (`atlas_mcp::supported_event_names` / `is_supported_event_name`).

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
- Embedding configuration is implemented in `.atlas/config.toml` for `atlas-search` URL and model settings instead of relying on `ATLAS_EMBED_URL` and related environment getters.
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

- Discriminated input objects are implemented for previously ambiguous families: `get_context.target`, `read_file_excerpt.selector`, `get_docs_section.selector`, shared `repo_scope`, shared `change_source`, and `build_or_update_graph.operation`, each with required `kind` enum values and per-kind companion fields.
- Legacy ambiguous fields were removed after a compatibility window: no `get_context` `query`/`file`/`files` precedence, no `batch_query_graph` `text`/`queries` override behavior, no flat `repo_id`/`all_repos` scope fields, and no change-source boolean/mode duplication.
- `batch_query_graph` uses an `items` array with the same query object shape as `query_graph`, max-length and empty-array enforcement, and fail-closed conflict rejection.
- Query-intent grammar is explicit for `query_graph` and `get_context` query targets: plain identifiers, exact qualified names, `who calls <symbol>`, `what breaks <symbol>`, and `tests for <symbol>`, with retry guidance for prose-only queries.
- Stable object results are implemented for `query_graph`, `batch_query_graph`, `search_saved_context`, `search_decisions`, `cross_session_search`, and the file/content discovery family, each with exact `outputSchema` validating emitted `structuredContent`.
- Runtime `tool_help`/`man` output includes an `input_contract` section per tool with discriminant field, accepted enum values, required companion fields, mutually exclusive legacy fields, and one minimal valid example per variant; generated `MCP_TOOLS.md` and installed AGENTS instructions reference runtime docs as canonical.
- Registry and snapshot tests enforce the contracts: hidden precedence wording, legacy ambiguous field groups, missing object `structuredContent` schemas, and stale manual docs fail CI.

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

## Still Open

- Phase 30 remaining code intelligence, Phase 31 lowest-priority features, and the ICM-inspired memory follow-on roadmap (ICM-A through ICM-H) remain in ISSUES.md.
- Retrieval post-retrieval compaction experiment, runtime event enrichment and graph linking, Rust reachability guard, shared parser query migration, context escalation contract, dynamic agent policy and hook enforcement, graph store corruption recovery, and measured SQLite read pooling remain in ISSUES.md.
