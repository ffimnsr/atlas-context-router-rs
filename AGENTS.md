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

## Agent Skill Content
Agent-facing Atlas guidance has one home: `packages/atlas-cli/skill-data/`.

- Guide content is embedded at build time and served by `atlas skills get core`; `atlas skills list` shows the inventory.
- The managed Atlas MCP block in `AGENTS.md` is a stable pointer only. Never move workflow rules, tool maps, escalation order, or trust metadata back into it, and do not grow it per release.
- `.agents/skills/atlas/SKILL.md` is a thin discovery stub written by `atlas install` from `packages/atlas-cli/skill-data/discovery/SKILL.md`. Keep it pointing at `atlas skills get core`; never copy guide content into it.
- Update `packages/atlas-cli/skill-data/core/SKILL.md` when Atlas tool usage changes, and keep the assertions in `packages/atlas-cli/src/install/tests.rs` aligned with it.

<!-- atlas MCP tools -->
## MCP Tools: atlas

**Atlas first.** This repository has a code knowledge graph. For code exploration, use Atlas MCP tools before `grep`, file search, broad reads, or exploratory terminal commands. Atlas returns structure text search cannot: symbols, callers, callees, tests, dependents, impact radius, and review context.

Before the first Atlas tool call, run `atlas skills get core` and follow it. That guide is served by the installed Atlas binary, so it always matches your version: Atlas-first workflow, tool map, escalation order, trust-metadata checks, and path invariants. Run `atlas skills list` to see every guide this version serves.

Use default JSON output. Trust `structuredContent` as source of truth when present.

### Add Atlas as an MCP server

Atlas is a stdio MCP server. Register it with whatever MCP-capable coding tool
you use — Atlas does not write per-tool configuration files.

  command: atlas
  args:    ["serve"]   # add ["--repo", "<absolute-repo-path>", "serve"] to pin a repository

Most MCP clients accept a JSON entry shaped like:

  {
    "mcpServers": {
      "atlas": { "type": "stdio", "command": "atlas", "args": ["serve"] }
    }
  }

Place that entry in your tool's MCP configuration and restart the tool. Consult
the tool's documentation for the exact config file location and key name.
<!-- /atlas MCP tools -->
