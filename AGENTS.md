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

## Issue Tracking
- Before starting work, check ISSUES.md for a matching item.
- If the requested work is already complete, mark the corresponding checklist item as done.
- If you finish work that matches an open checklist item, tick it before you wrap up.

## Change Hygiene
- Document non-obvious decisions in concise comments near the code.
- Keep diffs focused; avoid unrelated refactors.
- Add or update tests for new behavior and bug fixes.
- Confirm local build/test success before finalizing changes when possible.

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
- DON'T commit automatically, user would commit manually. Generate only commit header and or body after feature/fix/update.
- Use conventional commit format for subject line (e.g., `feat: ...`, `fix: ...`, `chore: ...`).
- Create a relevant to the change subject line
- In commit body, start each bullet with lowercase (e.g. `add`, `fix`, not `Add`, `Fix`).
- Include brief body summarizing all relevant additions/changes in commit.
- Add sign-off line in commit message (`Signed-off-by: Edward Fitz Abucay <29743013+ffimnsr@users.noreply.github.com>`).
- DON'T ever skip git hooks or use `--no-verify`, if there's problem fix it then stage and re-commit.

## Atlas Focus Rule
- Keep Atlas centered on core path: repo scan → parse → persist graph → incremental update → search/traverse → review context.
- Treat context memory and session continuity as supporting features.
- Do not let optional features delay or complicate core graph workflow.

## Agent Skill Content
Agent-facing Atlas guidance has one home: `packages/atlas-cli/skill-data/`.

- Guide content is embedded at build time and served by `atlas skills get core`; `atlas skills list` shows the inventory.
- The managed Atlas MCP block in `AGENTS.md` is a stable pointer only. Never move workflow rules, tool maps, escalation order, or trust metadata back into it, and do not grow it per release.
- `.agents/skills/atlas/SKILL.md` is a thin discovery stub written by `atlas install` from `packages/atlas-cli/skill-data/discovery/SKILL.md`. Keep it pointing at `atlas skills get core`; never copy guide content into it.
- Update `packages/atlas-cli/skill-data/core/SKILL.md` or `skill-data/core/references/*.md` when Atlas tool usage changes, and keep the assertions in `packages/atlas-cli/src/install/tests.rs` and `packages/atlas-cli/src/skills.rs` aligned with it.
- The `cli` and `mcp-tools` references are generated at request time from the binary and MCP registry. Never add static content for them; add a test instead when the generated output must stay accurate.

<!-- atlas MCP tools -->
## MCP Tools: atlas (MANDATORY)

**STRICT RULE — ATLAS FIRST:** This repository is indexed by the Atlas code knowledge graph. You **MUST** use Atlas MCP tools (or the `atlas` CLI) **BEFORE** using `grep`, `rg`, `find`, file search, globbing, broad file reads, or exploratory shell commands. Bypassing Atlas for code discovery or structural questions is strictly prohibited.

1. **Load the workflow first (REQUIRED):** Before your first Atlas call or any code exploration, run `atlas skills get core` and strictly follow its rules (run `atlas skills list` to see all version-matched guides).
2. **Query the graph first (REQUIRED):** Use Atlas to resolve symbols, callers, callees, tests, dependents, impact radius, and review context. Only read or search exact files **after** Atlas identifies them, or when the user explicitly names the target file path.
3. **Output contract:** Use default JSON output and trust `structuredContent` as the source of truth when present.
<!-- /atlas MCP tools -->
