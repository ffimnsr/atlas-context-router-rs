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
- [ ] add a `memoir_delete` MCP/CLI surface so throwaway smoke-test concept graphs can be removed without direct DB edits

---
