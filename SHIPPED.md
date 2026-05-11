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

## Cross-Cutting Infrastructure

- Shared ranking and trimming primitives are implemented across query, context, review, impact, and analysis surfaces.
- Graph build lifecycle state is implemented and surfaced through status, doctor, and MCP responses.
- Canonical graph readiness is implemented as single source of truth for built/queryable/current/integrity state, including explicit `fresh`, `stale`, `partial`, and `corrupt` execution states.
- CLI, MCP, adapters, and graph-backed analysis flows consume the same readiness contract and surface consistent safe-to-answer, freshness, and degraded-mode metadata.
- Canonical repo path identity is implemented across graph, content, session, adapter, and saved-context keys.
- Central budget policy and shared budget metadata are implemented across public surfaces.
- Repo-scoped MCP backend brokering is implemented without breaking stdio compatibility.
- Hook policy ownership, bounded payload routing, freshness handling, and review-refresh artifact flows are implemented.

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

## Parser Fuzz and Validation Hardening

- Stateful `TreeCache` fuzz coverage is implemented for parse, reparse-with-old-tree, insert, remove, evict, rename-key, delete/rename transitions, and old-tree reuse with changed bytes.
- Engine update-flow fuzz coverage is implemented against temp git repos and temp SQLite databases for add/modify/delete/rename sequences, supported and unsupported paths, working-tree diff mode, explicit file-list mode, old-tree reuse, and deleted-file cleanup.
- Parser output invariant fuzzing is implemented across all built-in language handlers, asserting file-node presence, path consistency, non-empty node/edge identities, valid line spans, and size consistency.
- AST helper fuzzing is implemented across built-in grammars, walking arbitrary parse trees and exercising `node_text`, line helpers, ancestor checks, common field lookups, and `find_all` without panics on malformed or invalid UTF-8 input.
- Refactor validation parser-reuse fuzzing is implemented for supported/unsupported paths, empty files, malformed supported-language content, and UTF-8-safe validation diagnostics.
- Parser fuzz corpora and dictionaries are seeded from parser fixtures and regex samples, with README/toolchain documentation and corpus refresh commands.

## SQLite Store Concurrency Contract

- Canonical SQLite ownership policy is documented and implemented across Atlas stores: each store owns one `rusqlite::Connection`, store structs are thread-confined, concurrent DB access uses separate connections, and current write behavior remains single-owner per store instance.
- Store, content-store, session-store, and DB utility docs consistently describe single-connection-per-store ownership, WAL behavior across separate connections, and why graph DB opens with `SQLITE_OPEN_NO_MUTEX` under thread confinement.
- Engine build/update boundaries keep Rayon parse work separate from sequential SQLite write/update phases, with regression coverage and trait-bound checks preventing store types from becoming `Send` or `Sync`.
- Shared-connection wrappers such as `Arc<Mutex<Connection>>` or `RwLock<Connection>` are explicitly rejected for Atlas store concurrency.
- Future read concurrency is documented as separate checked-out connections only; read pooling remains a measured, default-off follow-up rather than current behavior.

## Still Open

- Insights (Phase 29), multi-repo federation (Phase 30.1), advanced features (Phase 30.2, Phase 31), retrieval follow-ups, large-function parity, corruption recovery, runtime enrichment, context escalation, dynamic agent policy, Rust parser query migration, shared parser query migration, Rust reachability, and measured SQLite read pooling remain in ISSUES.md.
