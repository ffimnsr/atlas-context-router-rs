# AGENTS.md

## Purpose
This document defines coding and review expectations for agents working in this repository.

## Agent Response

ALWAYS respond terse like smart caveman. All technical substance stay. Only fluff die.

- Drop articles (a/an/the), filler, pleasantries, hedging.
- Fragments OK. Short synonyms.
- Keep all technical content: code, commands, file paths, exact names, numbers.

Pattern: "[thing] [action] [reason]. [next step]."

Examples:
- Not: "Sure! I'd be happy to help you with that. The issue you're experiencing is likely caused by..."
- Yes: "Bug in auth middleware. Token expiry check use `<` not `<=`. Fix:"

Auto-clarity overrides (temporarily disable caveman style for clarity/safety):
- security warnings
- irreversible action confirmations
- multi-step sequences where fragment order risks misread
- user confusion

## Core Principles
- Prefer clarity, correctness, and maintainability over cleverness.
- Make the smallest safe change that solves the root problem.
- Do not cut corners for short-term fixes. If long-term correctness needs fuller implementation, do it fully.
- Reuse existing functions, modules, and instructions before adding new abstractions.
- Follow latest MCP 2026-07-28 specification for all MCP-related changes.
- Before implementing roadmap work, read that phase's `Core Design Rules` / `Rules` block and treat it as mandatory.

## File Size Threshold Rule for Code Modifications
- If the file targeted for modification is approaching or exceeding 1,000 lines of code (LOC), do NOT apply changes directly to that file. Instead, create a new file or module that cleanly isolates the new functionality. The original file must remain stable, readable, and below the 1K LOC threshold.

## Rust Best Practices
- Follow idiomatic Rust and standard formatting (`rustfmt`) for all code. ALWAYS format the code after implementation/fix is done.
- Keep modules cohesive and public APIs minimal; prefer `pub(crate)` over `pub` when exposure isn't needed.
- Use `Result` and `?` for error propagation; add context with `anyhow` or typed error enums rather than ignoring errors.
- Prefer composition and traits over deep type hierarchies.
- Define traits at consumer boundaries; avoid premature abstraction.
- Keep functions small and focused on a single responsibility.
- Use `async fn` with explicit cancellation and timeout handling for I/O operations.
- Write unit tests alongside code and integration tests for end-to-end behavior; use `#[cfg(test)]` modules.
- Avoid global mutable state (`static mut`, `lazy_static` with interior mutability); prefer dependency injection via function parameters or struct fields.
- Use the standard library and well-vetted crates (e.g., `tokio`, `axum`, `sqlx`) before adding new dependencies.
- Always run clippy to check if the code has lint problems.
- When writing features or fixes, always write tests for it.
- Use `println!`/`eprintln!` for user-facing command output and confirmations; reserve `info!` and other tracing macros for diagnostics, debugging, and operator-focused logs.

## Security Best Practices
- Validate and sanitize all untrusted inputs.
- Apply least-privilege principles for files, processes, credentials, and network access.
- Never hardcode secrets, tokens, or credentials in source code.
- Use environment variables or a secret manager for sensitive configuration.
- Avoid logging sensitive data (credentials, tokens, PII, secrets).
- Use parameterized queries and safe APIs to prevent injection vulnerabilities.
- Enforce timeouts and cancellation for network and I/O operations.
- Prefer secure defaults and fail closed on authorization/authentication checks.
- Keep dependencies minimal and up to date; patch known vulnerabilities promptly.
- Use cryptographic primitives from trusted standard libraries; avoid custom crypto.
- The security headers, body size limit, IP rate limit, route rate limit will be handled by gateway.

## Reuse-First Development Rule
Before creating new code:
1. Use Atlas (`query_graph`, `resolve_symbol`, `get_context`) to find existing functions, helpers, modules, or crates that already solve the need.
2. Reuse and extend existing behavior when practical.
3. Add new functions only when reuse would reduce readability, safety, or correctness.

## Upgrade Policy
- When implementing planned upgrades, prefer removing deprecated code paths and compatibility shims instead of preserving legacy behavior.
- Do not keep backward compatibility unless the task explicitly requires it.
- Favor the cleanest correct implementation over carrying old branches forward.
- If existing code is already present but the design is suboptimal, prefer a focused overhaul that improves correctness, clarity, or maintainability.

## Atlas Focus Rule
- Keep Atlas centered on core path: repo scan → parse → persist graph → incremental update → search/traverse → review context.
- Treat context memory and session continuity as supporting features.
- Do not let optional features delay or complicate core graph workflow.

## Path Identity Invariant
- Canonicalize repo paths before hashing, persistence, dedupe, cache keys, source IDs, chunk IDs, snapshot file references, or qualified-name prefixes.
- Apply same rule to future sidecar/cache/index keys, including in-memory parser caches and retrieval sidecars keyed by repo files.
- Use `atlas_repo::CanonicalRepoPath` or helper APIs built on it. Do not add local path-normalization helpers for path-derived identity.
- If `doctor` or `db_check` reports `noncanonical_path_rows`, rebuild graph/content state from clean canonical inputs. Do not patch rows in place.

## Issue Tracking
- Before starting work, check ISSUES.md for a matching item.
- If the requested work is already complete, mark the corresponding checklist item as done.
- If you finish work that matches an open checklist item, tick it before you wrap up.

## Change Hygiene
- Document non-obvious decisions in concise comments near the code.
- Keep diffs focused; avoid unrelated refactors.
- Add or update tests for new behavior and bug fixes.
- Confirm local build/test success before finalizing changes when possible.

## Cargo Test Rules
- BEFORE running any tests do a second pass so no more late changes except fixes for the failing tests. This will prevent re-test again after successful testing. TESTS consumes lots of time.
- Run `cargo test` only in quiet mode: use `cargo test --quiet`.
- Never run plain `cargo test` without `--quiet`.
- Keep test output error-focused: prefer commands and wrappers that stay silent on passing cases and surface failures on stderr.
- For targeted test runs, use quiet variants like `cargo test --quiet some_test_name`.
- Before running any test, make sure all lints and diagnostics tab are clean.

### Test Scope Escalation
- Full package tests (`cargo test --quiet -p <crate>`) are a manual gate, not the iteration loop. Run a full package only when the change is crate-wide: `Cargo.toml`, feature flags, public API shape, macros, build scripts, shared test harness, or cross-cutting runtime behavior. Otherwise run the smallest filtered set that still proves the change.
- Batch related fixes before running tests. Do not run a test after every small edit. Finish a coherent slice, then run the narrowest filter set covering all changed modules. Re-run an exact test only after a failure or when isolating a suspected regression; do not chain one-edit/one-test cycles.
- During iteration, escalate one tier at a time:
  1. exact regression test: `cargo test --quiet -p <crate> <test_name> -- --exact`
  2. changed module subtree: `cargo test --quiet -p <crate> <module::path>`
  3. changed integration target: `cargo test --quiet -p <crate> --test <file_stem>`
  4. full package (manual, once, before final handoff)
- Derive filters from touched paths: `crates/foo/src/a/b.rs` -> `-p foo a::b`; `crates/foo/tests/bar.rs` -> `-p foo --test bar`; `crates/foo/src/lib.rs` -> the affected top-level module, or the package only when root behavior changed. Never map a file path to a filter by guessing: confirm with `cargo test --quiet -p <crate> -- --list` or the module declaration.
- Add neighbor coverage only when a public surface changes. Use Atlas `symbol_neighbors` / `get_impact_radius` to find direct callers and tests, then run their module filters and `cargo check -p <dependent>`. Do not expand to every dependent package unless the contract changed; full dependent suites are manual.
- If a filtered run passes but the change touches shared state, error types, serialization, or feature unification, escalate one tier. If the remaining risk still needs a full suite, report it as a manual step instead of running it unprompted.
- Serialize heavy test runs. Do not run two package-level or workspace-level suites concurrently on the same host; wait for the current run to finish before starting another.

## Flaky Test Prevention
- Rust tests run in parallel by default. Treat globals as shared process state: cwd, env vars, fixed filesystem paths, sockets, ports, `OnceLock`/`Lazy`, static mutexes, and Tokio runtimes can race across tests.
- Prefer per-test owned state: `tempfile::TempDir`, unique temp paths with pid plus monotonic counter, fresh config structs, local registries, and explicit dependency injection.
- Never use shell-level test retries to hide flakes. Fix root cause or quarantine outside code if CI supports quarantine.
- Avoid fixed sleep/retry loops. Use bounded wait helpers that pump relevant event queues, assert exact observable conditions, and panic with useful state (`path`, last text/error, status, pending state).
- Avoid real-clock assertions when possible. For Tokio timer behavior, use paused/virtual time (`#[tokio::test(start_paused = true)]`, `tokio::time::advance`) instead of wall-clock sleeps.
- Every spawned async/background task in tests must have explicit shutdown: hold `JoinHandle`, signal cancellation, then await completion before test exits.
- Tests that mutate cwd or environment must hold existing cwd/env lock guards and restore state with RAII guards so panic cannot leak process state.
- Do not let `OnceLock`, `Lazy`, or cached config read mutable env/test state unless test order cannot affect result. Prefer building fresh values per test.
- Use `serial_test` only when shared process-global mutation is unavoidable and cannot be refactored to per-test state. For async tests, use async-aware serial attributes in correct macro order per crate docs.
- Temp files/sockets must not depend on timestamp alone; include pid and monotonic counter or use `tempfile` APIs.
- Before increasing timeouts, identify whether failure is scheduling, leaked state, fixed resource collision, un-awaited task, or real bug. Longer timeout alone is last resort, not fix.

## Commit Message Policy
- DON'T commit automatically, user would commit manually. Generate only commit header and or body after feature/fix/update/docs/chores.
- Use conventional commit format for subject line (e.g., `feat: ...`, `fix: ...`, `chore: ...`).
- Make subject specific to the change.
- In commit body, start each bullet with lowercase (e.g., `add`, `fix`, not `Add`, `Fix`).
- Include brief body summarizing relevant additions/changes.
- Add sign-off line: `Signed-off-by: Edward Fitz Abucay <29743013+ffimnsr@users.noreply.github.com>`.
- Never skip git hooks or use `--no-verify`. Fix hook failures, then stage and commit again.

<!-- atlas MCP tools -->
## MCP Tools: atlas

**Atlas first.** This repository has a code knowledge graph. For code exploration, use Atlas MCP tools before `grep`, file search, broad reads, or exploratory terminal commands.

Why: Atlas is cheaper, faster, and returns structure file search cannot: symbols, callers, callees, tests, dependents, impact radius, and review context.

Priority rule: if instructions conflict, prefer Atlas for discovery and relationship lookup. Exceptions: read/edit exact files already identified, run validation commands (`cargo test --quiet`, clippy, format), or inspect non-code assets after Atlas points to them.

Use default JSON output. Trust `structuredContent` as source of truth when present.

### Atlas-first requirements

- First code-exploration step: call `status` unless graph freshness is already known from current turn.
- Symbol or usage question: start with `query_graph` or `resolve_symbol`.
- Caller/callee/test question: use `symbol_neighbors` before wider traversal.
- Broader context needed: use `get_context` before `traverse_graph`.
- Last resort for relationships: use `traverse_graph` with bounded `max_depth` and `max_nodes`.
- File reads/edits are OK after Atlas identifies exact files, or when user already named exact files.
- Content search tools (`search_files`, `search_content`, `search_templates`, `search_text_assets`) are companion tools for docs, config, SQL, templates, prompts, or exact non-code evidence. Do not use them first for symbol questions.

### Minimal-first escalation order

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
4. `traverse_graph` only when one-hop context is insufficient.

Allowed reasons to escalate:
- ambiguous symbol resolution
- truncated result
- missing caller/callee/test evidence
- cross-file or cross-package risk
- explicit user request for broader context
- safety-critical uncertainty

Anti-patterns:
- starting with `grep`, file search, or broad reads for structural code questions
- using `query_graph` as caller/callee search instead of following with relationship tools
- using `traverse_graph` before symbol resolution
- broad traversal without bounded `max_depth` and `max_nodes`
- using full review context when minimal context is enough
- skipping Atlas because a terminal command feels faster

### Tool discovery

Runtime docs are canonical for current arguments and output contracts. Do not treat this instruction block as exhaustive inventory.

- `tool_list`: list visible Atlas MCP tools
- `tool_search`: find tools by name or capability
- `tool_help`: read exact runtime docs for one tool, including input contracts and minimal examples
- `man`: legacy namespace-aware manual alias

### Common tool map

- symbol lookup: `query_graph`, `resolve_symbol`
- relationships: `symbol_neighbors`, `traverse_graph`, `get_context`
- review/change context: `detect_changes`, `get_minimal_context`, `get_review_context`
- impact/risk: `get_impact_radius`, `explain_change`, `assess_risk`, `analyze_safety`
- repo health: `status`, `doctor`, `db_check`, `debug_graph`, `list_graph_stats`
- companion non-code lookup: `search_files`, `search_content`, `read_file_excerpt`, `get_docs_section`, `read_file_around_match`, `search_templates`, `search_text_assets`
- continuity: `wake_up`, `get_session_status`, `resume_session`, `search_saved_context`, `search_decisions`, `save_context_artifact`
- shared memory: `memory_store`, `memory_recall`

### Trust Metadata

- Every MCP tool response includes `atlas_provenance` with `repo_root`, `db_path`, `indexed_file_count`, and `last_indexed_at`.
- Some graph-backed tools also include `atlas_freshness` when working-tree edits may make graph results stale.
- If `atlas_provenance.repo_root` does not match current workspace, stop and verify session wiring.
- If `atlas_provenance.db_path` points at unexpected database, stop and call `status` or `doctor` before trusting results.
- If `atlas_freshness.stale` is true, prefer `build_or_update_graph` before making claims about current code.
- Treat missing optional `atlas_*` fields as "not applicable", not as tool failure.

### Path Identity Invariant

- Canonicalize repo paths before hashing, persistence, dedupe, cache keys, source IDs, chunk IDs, snapshot file references, or qualified-name prefixes.
- Apply same rule to future sidecar/cache/index keys, including in-memory parser caches and retrieval sidecars keyed by repo files.
- Use `atlas_repo::CanonicalRepoPath` or helper APIs built on it. Do not add local path-normalization helpers for path-derived identity.
- If `doctor` or `db_check` reports `noncanonical_path_rows`, prefer rebuild from clean canonical inputs instead of in-place row rewrites.

### Session memory fallback for hosts without hooks

Native Atlas hooks capture session events automatically on Claude Code, Codex, and Copilot. When this client does not support Atlas hooks (or hook availability is unknown), use Atlas MCP tools as the fallback lifecycle protocol. Never write any Atlas database directly; only call Atlas MCP tools or CLI commands.

#### Recall before work

- At session start, call `wake_up` when available; otherwise call `resume_session` before substantive work.
- Call `search_decisions` for prior conclusions and `search_saved_context` for prior artifacts relevant to the task.
- Call `get_global_memory` when you need repo-level cross-session context.

#### Mandatory capture triggers

Call `record_session_event` when any of these happens:

1. User gives a substantial task → `record_session_event` with `event: "user-prompt"` and `payload.prompt`.
2. A tool call creates, edits, deletes, renames, or generates files → `record_session_event` with `event: "post-tool-use"` and payload containing `tool_name` and `changed_files` (repo-relative paths).
3. You know exact changed files but not the originating tool payload → `record_session_event` with `event: "file-changed"`.
4. Error resolved → `record_session_event` with `event: "tool-failure"` (or `error`) AND save the fix via `save_context_artifact` with `source_type: "error_resolved"`.
5. Architecture/design decision made → save via `save_context_artifact` with `source_type: "decision"`, linking symbols/files/source ids when possible.
6. User preference discovered → save via `save_context_artifact` with `source_type: "preference"`.
7. Major investigation, review, or handoff summary produced → save via `save_context_artifact` with `source_type: "review"` / `"investigation"` / `"handoff"`.
8. Context compression is imminent or detected → call `compact_session` and `record_session_event` with `event: "pre-compact"` first.
9. Major work completed before your final response → `record_session_event` with `event: "stop"` (or `event: "session-end"`) and save a concise handoff artifact.

Do NOT store: trivial logs, raw secrets or credentials, duplicate facts, raw unbounded transcripts, or facts already present in repository instruction files. Prefer compact summaries and `source_id` references.
<!-- /atlas MCP tools -->
