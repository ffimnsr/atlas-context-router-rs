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

## Query, Search, and Impact Surfaces

- Symbol lookup is implemented through graph query surfaces with ranked exact-match, qualified-name, filtered, fuzzy, regex, and hybrid retrieval modes.
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

- CLI command surfaces are implemented for init, build, update, detect-changes, status, query, impact, review-context, context, doctor, db-check, debug-style diagnostics, reasoning, refactor, install, and serve workflows.
- MCP tool registry is implemented for graph queries, traversal, review/impact/context flows, health/debug tools, saved-context tools, content/file search tools, and reasoning analysis tools.
- MCP transport support is implemented with stdio-compatible serving and repo-scoped backend brokering.
- CLI and MCP parity is implemented across major shared service surfaces rather than maintained as unrelated code paths.

## Session, Saved Context, and Continuity

- Separate session and content stores are implemented instead of mixing runtime/session data into the graph database.
- Session event persistence is implemented with bounded event payload handling, artifact references, and session metadata.
- Saved-context artifact storage, previewing, retrieval, search, and purge flows are implemented.
- Resume snapshots and retrieval-backed restoration are implemented for session continuity.
- Context storage compaction and budget-aware retention behavior are implemented.

## Hook and Agent Host Integration

- Install flows are implemented for Copilot, Claude, and Codex integration.
- Generated hook and MCP configuration support repo-local integration flows instead of requiring manual setup.
- Hook lifecycle coverage is implemented for session start, prompt submission, tool execution, compaction, stop, session end, and related host events.
- Thin hook runner architecture is implemented so shell launchers remain stable while Rust code owns normalization, routing, storage, and policy behavior.

## Cross-Cutting Infrastructure

- Shared ranking and trimming primitives are implemented across query, context, review, impact, and analysis surfaces.
- Graph build lifecycle state is implemented and surfaced through status, doctor, and MCP responses.
- Canonical repo path identity is implemented across graph, content, session, adapter, and saved-context keys.
- Central budget policy and shared budget metadata are implemented across public surfaces.
- Repo-scoped MCP backend brokering is implemented without breaking stdio compatibility.
- Hook policy ownership, bounded payload routing, freshness handling, and review-refresh artifact flows are implemented.

## Still Open

- Historical graphs, insights, multi-repo federation, predictive and decision memory, readiness unification, corruption recovery, runtime enrichment, and several retrieval/context follow-up patches remain in ISSUES.md.
