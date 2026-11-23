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

- Focused follow-up patches: External Grammar Offload Patch and Minimal-Context Escalation Metadata Patch. The retrieval follow-up, runtime classification, reachability, SQLite index audit, tokenizer, markdown continuity storage, and shared parser query migration patches are shipped; see SHIPPED.md. Runtime policy surface is deferred.

---

## Follow-Up Patches

Use these patch sections for focused improvements that cut across existing roadmap phases without rewriting phase scope.


### External Grammar Offload Patch

Plan: [`docs/PLAN-EXTERNAL-GRAMMARS.md`](docs/PLAN-EXTERNAL-GRAMMARS.md)

Scope: offload `tree-sitter-php`, `tree-sitter-cpp`, `tree-sitter-c-sharp`,
`tree-sitter-java`, `tree-sitter-scala`, `tree-sitter-ruby`, and
`tree-sitter-bash` to runtime grammars while keeping existing extractors; close
generic external parser gaps with rules, SCM captures, and an out-of-process
plugin. Rust `dylib` loading stays a non-goal.

Rules:

- plan §2 is the design source; this checklist is execution status
- do not remove bundled handlers until profile goldens pass
- keep every touched module below 1,000 LOC; split `external/generic.rs` or
  `watch.rs` if needed
- no Rust `dylib` loader or stable-ABI shim

#### Phase 0 — Baseline

- [ ] record baseline `cargo test -p atlas-parser`, release binary size, and `cargo tree -p atlas-cli -e normal` output
- [ ] add fixture inventory assertion for the seven offloaded languages in `packages/atlas-parser/tests/parser_golden.rs`

#### Phase 1 — Grammar injection

- [ ] add `language: tree_sitter::Language` and `new(language)` to `packages/atlas-parser/src/lang/{php,cpp,csharp,java,scala,ruby,bash}.rs`
- [ ] thread the language through each `*SyntaxFacts::extract` and add module `validate_query` helpers
- [ ] update module tests and confirm existing golden JSON stays byte-identical

#### Phase 2 — Profile support

- [ ] split `packages/atlas-parser/src/external.rs` into `external/{mod,loader,generic,profile}.rs`
- [ ] add `ExternalExtractor`, profile defaults, `apply_profile_defaults`, and shared `validate`
- [ ] add `load_external_language`, `ExternalLangParser::from_language`, extension override, and profile dispatch
- [ ] normalize and validate profile config in `packages/atlas-engine/src/config/{load,parsers}.rs`
- [ ] add tests for defaults, conflicts, canonical names, C#/Java lib functions, and query validation failure

#### Phase 3 — Remove bundled registration

- [ ] remove the seven parsers from `ParserRegistry::with_defaults`
- [ ] move the seven grammar crates from `packages/atlas-parser` dependencies to dev-dependencies; add them to `fuzz/Cargo.toml`
- [ ] register profile parsers with `ExternalLangParser::from_language` in `packages/atlas-parser/tests/parser_golden.rs`; keep goldens unchanged
- [ ] update `registry.rs` tests: default registry rejects the seven, profile registration restores support
- [ ] rewrite offloaded-language fixtures in `packages/atlas-cli/tests/cli_quality_gates/resolution.rs` or move extraction assertions to parser goldens
- [ ] assert normal dependency graph excludes the seven grammar crates

#### Phase 4 — Docs and diagnostics

- [ ] add profile/query/plugin examples to `packages/atlas-engine/src/config/template.rs`
- [ ] update README external-language docs, path-resolution rules, migration steps, and supported-language list
- [ ] update `packages/atlas-parser/README.md` and `CHANGELOG.md`
- [ ] update threat-model docs for grammar/plugin trusted-code boundaries
- [ ] add `atlas doctor` warning for tracked offloaded extensions with no registered parser

#### Phase 5 — Generic external fidelity

- [ ] add `ExternalImportRule` and emit import nodes/`Imports` edges from file scope
- [ ] add `ExternalAttributeRule` with `attach_to = scope | next_symbol` and emit `Contains` edges
- [ ] add symbol-rule `params_field`, `return_type_field`, and `modifier_node_kinds`; populate node metadata
- [ ] add `qn_style` + `owner_node_kinds`; implement `nested` and `flat_owner`
- [ ] add compiled `test_name_patterns` / `test_path_patterns`; set `Node.is_test`
- [ ] add `call_receiver_field` and replace generic call construction with `common::call_edge` tiers
- [ ] add vendored Rust grammar tests for imports, attributes, metadata, regexes, QN style, receiver, and call confidence
- [ ] reject generic-only fields in profile configs

#### Phase 6 — Watch and fingerprint

- [ ] carry `[parsers.external]` through `WatchRunner` into `UpdateOptions` and test the forwarding helper
- [ ] add `packages/atlas-engine/src/parser_fingerprint.rs` with revision, normalized config hash, and query/plugin fields
- [ ] add fingerprint metadata getter/setter in `packages/atlas-store-sqlite/src/store/build_state.rs`
- [ ] write fingerprint after non-dry-run build
- [ ] fail closed on missing/mismatched fingerprint in update
- [ ] surface stale/missing fingerprint in `status` and `doctor`
- [ ] add stable/change-sensitive fingerprint tests
- [ ] add update-rejection tests for fingerprint mismatch

#### Phase 7 — SCM query/capture mode

- [ ] add `query_path` / inline `query`, extraction-mode validation, and `.atlas/` path resolution
- [ ] implement `packages/atlas-parser/src/external/query.rs` with `@atlas.*` capture mapping
- [ ] support definitions, names, imports, attributes, calls, receivers, params/return, and references
- [ ] reject rule fields with query mode; add valid, invalid, and conflict tests

#### Phase 8 — Out-of-process plugin

- [ ] add plugin config (`command`, `args`, `working_dir`, `protocol`, timeout, response cap) and mode validation
- [ ] implement `external/protocol.rs` and `external/plugin.rs` with argv spawn, timeout, kill, and output caps
- [ ] validate protocol version, JSON shape, path, hash, and node/edge file paths
- [ ] add protocol tests plus Unix fixture tests for success, timeout, malformed, oversize, and hash mismatch
- [ ] document trusted-code execution; confirm no `dylib` path exists

#### Phase 9 — Release validation

- [ ] run `cargo fmt --all -- --check`
- [ ] run `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] run parser, engine, store, CLI, and workspace tests
- [ ] run `cargo build --release` and record binary-size delta
- [ ] verify released dependency graph excludes the seven grammar crates
- [ ] manual fixture repo: no config, profile config, query config, plugin config, update, watch, fingerprint mismatch

#### Patch completion criteria

- [ ] all seven profile fixture sets match existing goldens byte-for-byte
- [ ] default registry excludes the seven languages; profile config restores them
- [ ] generic imports, attributes, metadata, test regexes, QN style, and call confidence are covered
- [ ] query and plugin modes are covered per plan §2.9 and §2.10
- [ ] watch mode and parser fingerprint are enforced
- [ ] all plan §4 limitations are closed except explicit Rust `dylib` non-goal
- [ ] docs and diagnostics updated; validation commands pass


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