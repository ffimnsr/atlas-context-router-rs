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

- Part III. Remaining product expansion roadmap: Phases 29 through 31
- Part IV. Remaining context continuity roadmap: ICM-inspired memory follow-on roadmap (ICM-0 through ICM-H)
- Part V. Remaining focused follow-up patches: Retrieval Follow-Up Patch remainder, Runtime Event Enrichment and Graph Linking Patch, Rust Reachability Guard Patch, Shared Parser Query Migration Patch, Context Escalation Contract Patch, Dynamic Agent Policy and Hook Enforcement Patch, Graph Store Corruption Recovery Patch, SQLite Connection Concurrency Policy Patch remainder

## Cross-Cutting Track Map

- Historical and analytics work: Phase 29, Phase 30, Phase 31
- Retrieval and search follow-ups: Retrieval Follow-Up Patch remainder
- Context continuity and runtime memory: ICM-0 universal MCP session capture fallback, ICM-inspired memory follow-on roadmap, Runtime Event Enrichment and Graph Linking Patch
- Graph safety and workflow: Context Escalation Contract Patch, Graph Store Corruption Recovery Patch, SQLite Connection Concurrency Policy Patch remainder
- Rust parser correctness: Rust Reachability Guard Patch, Shared Parser Query Migration Patch
- Agent policy and enforcement: Dynamic Agent Policy and Hook Enforcement Patch

---

## Part III — Post-MVP Product Expansion

Use this part for advanced retrieval, analysis, refactoring, observability, real-time updates, insights, optional features, and MCP-facing payload optimizations.

These phases extend v1 after core graph/build/update/query path is reliable.

### Phase 29 — Intelligence & Insights

Shipped: insights engine foundation, code health metrics engine, large/complex function finder, architecture analysis, risk assessment, pattern detection, and CLI/MCP insight surfaces. See SHIPPED.md for details.

Remaining:

- [x] add configurable layer-rules file surface for architecture validation:
  - [x] config shape:
    - [x] add `insights.layer_rules_file: Option<String>` to `packages/atlas-engine/src/config.rs`
    - [x] keep existing inline `[[insights.layer_rules]]` support for now; define precedence as `layer_rules_file` replaces inline rules when set
    - [x] resolve relative `layer_rules_file` paths from `.atlas/`, matching `sanitization.redaction_rules_file`
  - [x] external file schema:
    - [x] support TOML files containing `[[layer_rules]]` entries with same fields as inline rules: `name`, `path_prefixes`, `module_prefixes`
    - [x] deserialize through existing `InsightsLayerRule` so validation stays shared
    - [x] document example file as `.atlas/layer-rules.toml`
  - [x] loader/validation:
    - [x] add `InsightsConfig::resolve_layer_rules_file(atlas_dir)` helper
    - [x] add `InsightsConfig::effective_layer_rules(atlas_dir) -> Result<Vec<InsightsLayerRule>>` or equivalent runtime loader
    - [x] reject blank `insights.layer_rules_file`
    - [x] reject missing paths with error naming `insights.layer_rules_file` and resolved path
    - [x] reject directories/unreadable files with actionable error naming `insights.layer_rules_file`
    - [x] reject malformed TOML with parse context naming `insights.layer_rules_file`
    - [x] reuse existing layer-rule validation for duplicate names, empty matchers, and rules missing both matcher lists
  - [x] runtime integration:
    - [x] update architecture analysis config construction so `atlas-reasoning/src/engine/architecture.rs` receives effective runtime rules
    - [x] verify CLI `insights architecture` and MCP `analyze_architecture` both load external rules via normal `Config::load` paths
    - [x] preserve no-rules behavior when neither inline rules nor file path is configured
  - [x] templates/docs:
    - [x] update config template rendering to include commented `layer_rules_file = "layer-rules.toml"` near `[insights]`
    - [x] decide whether full profile should prefer inline sample rules or external sample path; do not emit a non-existent active file path
    - [x] update relevant docs or config examples if present
  - [x] tests:
    - [x] unit: `Config::load` accepts valid external layer-rules file
    - [x] unit: `Config::load` rejects missing external layer-rules file
    - [x] unit: `Config::load` rejects directory/unreadable external layer-rules file
    - [x] unit: `Config::load` rejects malformed external layer-rules TOML
    - [x] unit: external file reuses duplicate-name and empty-matcher validation
    - [x] CLI quality gate: `insights architecture` reports `layer_violation` from `.atlas/layer-rules.toml`
    - [x] regression: inline `[[insights.layer_rules]]` still works when `layer_rules_file` unset
  - [x] validation commands:
    - [x] `cargo fmt --all`
    - [x] `cargo clippy --workspace --all-targets --quiet`
    - [x] `cargo test --quiet -p atlas-engine config::tests::load_accepts_valid_external_layer_rules_file`
    - [x] `cargo test --quiet -p atlas-cli --test cli_quality_gates insights_architecture_reports_layer_violations_from_external_config_file`
    - [x] `./scripts/test-workspace-summary.sh`
- [x] completion criteria: config supports runtime-loaded external layer-rules files with validation, CLI/MCP architecture analysis both use effective external rules, and tests cover valid/missing/unreadable/malformed files

### Phase 30 — Optional Advanced Features

#### 30.1 Multi-repo

Shipped: repo registry, discovery and bootstrap, identity and storage model, per-repo build/update flows, cross-repo resolution and graph semantics, CLI/MCP surfaces, review/context/saved-artifact integration, and safety/rollout behavior. See SHIPPED.md for details.

Remaining:

- [x] store repo provenance on nodes, edges, files, saved context, and diagnostics output:
  - [x] data model:
    - [x] define shared repo provenance shape with canonical repo identity fields: `repo_id`, `repo_root`, `repo_fingerprint`/registry fingerprint, and optional `remote_url` when already available
    - [x] persist provenance on graph nodes and edges without deriving identity from non-canonical paths
    - [x] persist provenance on file/content records and source references used by retrieval/review context
    - [x] persist provenance on saved context artifacts and session continuity records
    - [x] include provenance in diagnostics records/output payloads so stale or cross-repo diagnostics can be traced
  - [x] write path:
    - [x] populate provenance from repo registry/discovery bootstrap during full graph build
    - [x] preserve/update provenance during incremental graph updates, deletes, and cross-repo edge creation
    - [x] reject or fail closed when required repo provenance is missing for persisted multi-repo data
  - [x] read/API surfaces:
    - [x] expose provenance in CLI JSON outputs for graph/context/review/diagnostics commands that emit nodes, edges, files, saved context, or diagnostics
    - [x] expose provenance in MCP TOON/JSON responses for affected tools, keeping existing fields stable where possible
    - [x] ensure diagnostics output includes enough repo metadata to distinguish same relative path across repos
  - [x] migration/backfill:
    - [x] add schema migration for new provenance columns/tables/indexes
    - [x] backfill single-repo databases from current repo registry/default repo identity
    - [x] make `doctor`/`db_check` report missing or inconsistent repo provenance separately from `noncanonical_path_rows`
  - [x] tests:
    - [x] unit: provenance shape serializes/deserializes for CLI JSON and MCP responses
    - [x] integration: full graph build stores provenance on nodes, edges, and files
    - [x] integration: incremental update preserves provenance and removes stale rows for deleted files
    - [x] integration: saved context artifact round-trips repo provenance
    - [x] integration: diagnostics output distinguishes two repos with same relative file path
    - [x] regression: canonical path identity invariant still holds for path-derived IDs/cache keys
  - [x] validation commands:
    - [x] `cargo fmt --all`
    - [x] `cargo clippy --workspace --all-targets --quiet`
    - [x] targeted provenance tests with `cargo test --quiet ...`
    - [x] `./scripts/test-workspace-summary.sh`
  - [x] completion criteria: every persisted/output entity that can cross repo boundaries carries canonical repo provenance, health checks detect missing provenance, and tests prove same-relative-path multi-repo cases remain unambiguous

#### 30.2 Remaining code intelligence

Make higher-level code intelligence explicit, deterministic, and implementable on top of canonical graph/content inputs.

- [x] similar-function detection beyond graph-shape heuristics:
  - [x] compute deterministic callable fingerprints from canonical file paths, symbol kind/name tokens, normalized signature tokens, call/import/reference neighborhood, module bucket, source-body shingles, and normalized duplicate shingles
  - [x] score candidates with weighted name/signature/body/neighborhood/module/size buckets and return feature score breakdowns, matched features, differing features, and similarity band
  - [x] bound candidates by language, callable kind, arity shape, same-file option, and deterministic result limits
  - [x] expose CLI/MCP entry points with stable JSON schemas and freshness/provenance metadata from existing command/tool wrappers
  - [x] tests: known similar but non-identical callable fixture
- [x] duplicate detection beyond exact structural patterns:
  - [x] normalize callable source tokens by preserving keywords/control-flow structure while replacing identifiers/literals
  - [x] detect `exact_normalized` and `near_duplicate` callable groups with normalized token shingles
  - [x] rank duplicate groups by confidence, duplicated token/line count, member count, and stable deterministic group ID
  - [x] expose groups through CLI/MCP with members, files, normalized pattern summary, and suggested extraction target
  - [x] tests: multi-file near-duplicate callable fixture
- [x] infer modules:
  - [x] define inferred module model with stable ID, display name, root paths, owned symbols, inbound/outbound dependencies, confidence, evidence, and explicit-owner flag
  - [x] prefer explicit package ownership when stored; otherwise infer from `packages/<name>`, `src/<segment>`, `tests`, docs/wiki/markdown paths, or parent-directory fallback
  - [x] compute module dependency edges from graph edges with deterministic ordering
  - [x] expose inferred modules through CLI/MCP with stable JSON schemas
  - [x] tests: explicit owner plus path-bucket fixture
- [x] label components:
  - [x] define Atlas taxonomy: repo scan, parse, persist graph, incremental update, search/traverse, review context, context memory, session continuity, CLI, MCP, config, diagnostics, tests, docs
  - [x] implement deterministic multi-label file/symbol assignment from path and symbol-name rules with confidence/evidence
  - [x] expose scoped CLI/MCP label queries for files and symbols with stable JSON schemas
  - [x] tests: multi-label CLI/review fixture
- [x] Follow-up hardening:
  - [x] configurable similarity and duplicate band thresholds via `insights.*_threshold` config
  - [x] graph-community module clustering used before path fallback when explicit package owner is absent
  - [x] duplicate suppressions from config and CLI/MCP request filters
  - [x] source-span records inside duplicate members
  - [x] component-label propagation into review/context workflow impacted components
  - [x] persisted fingerprint cache with incremental invalidation

### Phase 31 — Lowest Priority

#### 31.1 Docs generation (CLI command)

- [x] add `atlas docs generate` CLI command for Markdown documentation:
  - [x] support `--repo <path>` and reuse existing graph DB/config discovery
  - [x] support `--output <dir>` and create deterministic `index.md`, `files.md`, `symbols.md`, `modules.md`, and `components.md`
  - [x] include repo summary, graph stats, generated timestamp, indexed file count, symbol counts by kind, inferred modules, component labels, and top-level dependency summaries
  - [x] include per-file sections with canonical path, language, package/module/component labels, owned symbols, inbound/outbound dependency counts, and notable duplicates when available
  - [x] include per-symbol sections with qualified name, kind, file/span, documentation snippet when available, callers/callees, tests, and owning module/component labels
  - [x] make output stable for tests by sorting paths/symbols/modules/components deterministically and allowing timestamp override in test fixtures
  - [x] fail with actionable errors when graph DB missing, stale, or unreadable; never silently generate partial docs unless `--allow-partial` is passed
  - [x] tests: fixture repo generates deterministic Markdown snapshots and validates missing/stale graph error paths
- [x] add visualization/export support for generated docs:
  - [x] support `atlas docs export --format mermaid` to emit dependency diagrams for modules, components, and selected symbol neighborhoods
  - [x] support `atlas docs export --format dot` for Graphviz-compatible module/component dependency graphs
  - [x] support `--scope repo|module|component|file|symbol` plus `--name <value>` for focused exports
  - [x] include stable node IDs, human-readable labels, edge kinds, and edge counts; collapse high-volume edges with summarized counts
  - [x] integrate export links or fenced Mermaid diagrams into generated Markdown when `atlas docs generate --include-diagrams` is passed
  - [x] cap diagram size by default with `--max-nodes`/`--max-edges` and report omitted nodes/edges deterministically
  - [x] tests: deterministic Mermaid/DOT snapshot fixtures for repo, module, component, and symbol-neighborhood scopes

#### 31.2 MCP tool manual and schema introspection

Add built-in manual and schema-introspection surface for MCP tools so agents and users can request authoritative tool docs at runtime instead of relying on external docs or stale prompt text.

- [x] add shared manual-documentation service for MCP tools:
  - [x] load tool metadata from live MCP tool registry instead of duplicating per-tool docs in separate hardcoded tables
  - [x] require canonical tool identity lookup by exact tool name and preserve case-sensitive output name
  - [x] allow manual namespace `mcp` first and reject unknown manual namespaces with clear validation errors
  - [x] return deterministic document payload without executing target tool
  - [x] keep service read-only and safe to call in restricted environments
- [x] define `man` response shape for MCP tool docs:
  - [x] include requested namespace and requested tool name
  - [x] include resolved tool name and description from registry
  - [x] include tool structure section describing tool purpose, exposed operation name, and top-level request/response shape
  - [x] include input-args section with field name, type, required/optional state, default value when available, accepted enum values when applicable, and per-field description
  - [x] include output-response section with response fields, field meanings, optional/required state, and metadata/error payload shape when available
  - [x] include usage section with exact form `man mcp <mcp_tool_name>` plus direct target-tool invocation examples when available
  - [x] include error section for unknown tool, deprecated tool, hidden/internal tool, or schema-unavailable cases
  - [x] keep field ordering deterministic so CLI output, MCP output, snapshots, and future generated docs stay stable
- [x] add MCP surface:
  - [x] expose MCP tool `man`
  - [x] accept exact arguments representing `man mcp <mcp_tool_name>` request
  - [x] require namespace `mcp` and target tool name
  - [x] return compact default output suitable for agent consumption
  - [x] add optional verbose or structured output mode if existing MCP tool patterns already support it
- [x] add CLI parity surface:
  - [x] add `atlas man mcp <mcp_tool_name>`
  - [x] keep human-readable output aligned with MCP default text
  - [x] add `--json` output that matches MCP structured payload as closely as current CLI/MCP parity rules allow
- [x] implement lookup and rendering behavior:
  - [x] resolve visible registered tools only and exclude disabled or non-exported tools unless explicit internal-doc mode is added later
  - [x] derive structure, input-args, output-response, and usage sections from registry/schema data when available instead of duplicating static prose
  - [x] suggest nearest tool names on unknown target using existing deterministic ranking helper if available
  - [x] truncate oversized examples or descriptions using same bounded-output policy used by other MCP-facing surfaces without dropping required structure, input, output, or usage sections
  - [x] include freshness/provenance metadata when manual output depends on generated registry state
- [x] add docs and tests:
  - [x] document `atlas man mcp <mcp_tool_name>` and MCP `man` in README and MCP tool docs
  - [x] add snapshot tests for human-readable output and JSON output
  - [x] add unknown-tool test with deterministic suggestion order
  - [x] add hidden-tool or disabled-tool behavior test
  - [x] add CLI/MCP parity test for at least one representative tool with required and optional args

Why:
- agents need fast authoritative tool docs during tool selection and argument construction
- one runtime-backed manual surface reduces drift between registry schema, CLI help, MCP docs, and prompt instructions

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

Phases CM14 (Decision Memory) and CM15 (Agent-Aware Context) are shipped. See SHIPPED.md for details. Remaining memory quality work is tracked in the ICM roadmap below.

### ICM-Inspired Memory Follow-On Roadmap

Use this section to merge compatible parts of `atlas-icm-inspired-memory-roadmap.md` into the existing continuity architecture.

Priority order below is implementation order. Extend shipped Phase CM14 and Phase CM15 behavior. Do not introduce a parallel memory stack that conflicts with current `session.db` / `context.db` / `worldtree.db` boundaries.

This grouped roadmap covers the full source document at theme level, except shell-first simplifications requested here: no slash-command track, no skill-install track, and no web dashboard track.

Before implementing any ICM checklist item:

- read the parent ICM section `Rules:` block
- treat `Rules:` bullets as mandatory constraints, not tasks
- do not mark `Rules:` bullets done; they are never checklist items
- implement checklist items only under `Implementation structure` and `completion criteria`
- if a checklist item conflicts with `Rules:`, follow `Rules:` and update the checklist wording before implementation

#### ICM-0 — Universal MCP Session Capture Fallback

Priority: implement this before ICM-A through ICM-H unless a blocking dependency is explicitly discovered.

Rules:

- provide hook-equivalent session capture for MCP-capable agents whose host does not expose native LLM hooks
- keep native hooks as the automatic path and MCP fallback as the universal instruction-driven path
- route native hooks and MCP fallback through one shared event service so event semantics, redaction, storage routing, lifecycle actions, graph freshness, and review refresh never drift
- do not make generated instructions write SQLite directly; instructions may only tell agents to call Atlas MCP tools or Atlas CLI commands
- keep fallback capture best-effort and non-blocking; failures must be visible as structured metadata or warnings
- store compact summaries and source references, never raw secrets or large unbounded transcripts
- preserve existing `session.db` / `context.db` / `worldtree.db` boundaries and never store runtime memory bodies in `worldtree.db`

Rules apply to every checklist item in this ICM-0 section.

Implementation structure:

##### ICM-0A — Shared agent event service

- [x] create shared crate or service module for hook-compatible agent events outside `atlas-cli`, preferably `packages/atlas-agent-events`
- [x] move hook event policy, canonical event names, aliases, priorities, storage mode, lifecycle mode, prompt-routing flag, freshness flag, graph-refresh flag, review-refresh flag, resume-snapshot flag, and session-start flag out of `packages/atlas-cli/src/commands/hook/policy.rs`
- [x] move hook payload extraction helpers, payload redaction/sanitization, changed-file extraction, tool-name extraction, command/status extraction, and graph-relevance checks out of CLI-only hook modules
- [x] move hook metadata builders, prompt-routing metadata, freshness metadata, and context-hint collection into the shared event service
- [x] move lifecycle actions into the shared event service: load restore state, persist handoff artifact, verify restore state, prompt routing, graph refresh, freshness metadata, and review refresh
- [x] expose `AgentEventRequest` with fields `repo_root`, `graph_db_path`, `frontend`, `event`, `session_id`, `agent_id`, `payload`, and `source`
- [x] expose `AgentEventSource` with exact values `hook` and `mcp_fallback`
- [x] expose `AgentEventResult` with fields `event`, `canonical_event`, `frontend`, `session_id`, `pending_resume`, `stored`, `event_id`, `source_id`, `storage_kind`, `snapshot`, `actions`, and `warnings`
- [x] expose one `record_agent_event` function that validates event aliases, redacts payloads, persists session events, routes oversized payloads through content-store, and executes policy actions
- [x] keep service APIs canonical-path aware for repo roots and path-derived identity; use existing `atlas_repo::CanonicalRepoPath` or helper APIs built on it
- [x] add unit tests for policy alias resolution covering `SessionStart`, `UserPromptSubmit`, `PostToolUse`, `FileChanged`, `Stop`, and kebab-case equivalents
- [x] add unit tests for redaction and oversized payload routing through raw, preview, and pointer content-store paths

##### ICM-0B — Rewire native `atlas hook` to shared service

- [x] update `packages/atlas-cli/src/commands/hook/mod.rs` so `run_hook` builds `AgentEventRequest` and calls `record_agent_event`
- [x] keep CLI-only behavior limited to resolving repo, deriving graph DB path, reading stdin payload, reading `ATLAS_HOOK_FRONTEND`, and printing JSON output
- [x] preserve existing `atlas hook <event>` JSON fields so current hook users do not need config changes
- [x] keep `atlas hook` failures non-blocking only in generated shell runners; direct CLI invocation should still return actionable errors
- [x] update hook tests to assert parity with pre-refactor output for `session-start`, `user-prompt`, `post-tool-use`, `pre-compact`, `post-compact`, `stop`, and `file-changed`
- [x] add regression test proving generated `.atlas/hooks/atlas-hook` still calls `atlas hook` and never writes SQLite directly

##### ICM-0C — MCP `record_session_event` fallback tool

- [x] add MCP tool descriptor `record_session_event` in `packages/atlas-mcp/src/tools/registry.rs`
- [x] add MCP dispatch arm for `record_session_event` in `packages/atlas-mcp/src/tools/dispatch.rs`
- [x] implement MCP handler in a new module such as `packages/atlas-mcp/src/session_events.rs` instead of growing `session_tools.rs` further
- [x] accept input fields `event`, `payload`, `frontend`, `session_id`, `agent_id`, `repo_scope`, and `output_format`
- [x] default `frontend` to `mcp` and event `source` to `mcp_fallback`
- [x] accept the same event names and aliases as native hooks, including `session-start`, `user-prompt`, `pre-tool-use`, `post-tool-use`, `pre-compact`, `post-compact`, `stop`, `session-end`, `permission-request`, `permission-denied`, `tool-failure`, `stop-failure`, `error`, `elicitation`, `elicitation-result`, `instructions-loaded`, `notification`, `subagent-start`, `subagent-stop`, `task-created`, `task-completed`, `config-change`, `cwd-changed`, `file-changed`, `worktree-create`, and `worktree-remove`
- [x] return stable object `structuredContent` with `event`, `canonical_event`, `frontend`, `session_id`, `stored`, `event_id`, `source_id`, `storage_kind`, `pending_resume`, `snapshot`, `actions`, and `warnings`
- [x] add output schema for `record_session_event` and ensure schema compiles under JSON Schema 2020-12
- [x] add MCP tests proving `record_session_event` persists session events for `session-start`, `user-prompt`, `post-tool-use`, `file-changed`, and `stop`
- [x] add MCP test proving unknown event names return structured validation errors with supported examples
- [x] add MCP test proving `post-tool-use` with changed files returns graph-refresh/review-refresh action metadata or explicit skip/error metadata

##### ICM-0D — Mandatory installed instruction fallback protocol

- [x] update `packages/atlas-cli/src/install/instructions.rs` injected block with a dedicated `Atlas session memory fallback` section
- [x] require agents to call `wake_up` when available, otherwise `resume_session`, at session start before substantive work
- [x] require agents to call `record_session_event` with event `user-prompt` when user gives a substantial task and hooks are unavailable or unknown
- [x] require agents to call `record_session_event` with event `post-tool-use` after MCP/client tool calls that create, edit, delete, rename, or generate files
- [x] require agents to call `record_session_event` with event `file-changed` when they know exact changed files but not the originating tool payload
- [x] require agents to call `save_context_artifact` for resolved errors, architecture/design decisions, user preferences, major investigation summaries, review summaries, and handoff summaries
- [x] require agents to call `compact_session` and `record_session_event` with `pre-compact` before context compression when they can detect compaction risk
- [x] require agents to call `record_session_event` with `stop` or `session-end` plus a handoff artifact before final response when major work was completed
- [x] tell agents not to store trivial logs, raw secrets, duplicate facts, raw unbounded transcripts, or facts already in repository instruction files
- [x] keep installed instruction block idempotent under existing `<!-- atlas MCP tools -->` markers
- [x] add install tests proving generated `AGENTS.md` and `CLAUDE.md` contain the fallback protocol and preserve user-authored content before and after the managed block

##### ICM-0E — Wake-up MCP parity for session start

- [x] add MCP tool `wake_up` with compact default output for hookless session-start context injection
- [x] make `wake_up` assemble bounded context from resume snapshot, decision memory, saved context hints, global memory, changed files, graph readiness, and retrieval hints
- [x] accept `topic`, `session_id`, `frontend`, `agent_id`, `max_items`, `repo_scope`, and `output_format`
- [x] include output fields `repo_root`, `session_id`, `frontend`, `current_focus`, `recent_decisions`, `critical_memories`, `recent_feedback`, `active_memoir_concepts`, `changed_files`, `graph_readiness`, `retrieval_hints`, `generated_at`, and `warnings`
- [x] reference large artifacts by `source_id` only and never inline large artifact bodies in wake-up output
- [x] record wake-up generation success or failure metadata through `record_agent_event` or the shared session event service
- [x] add snapshot tests covering empty repo memory, normal memory, stale graph, and oversized saved artifacts

##### ICM-0F — Installer modes, docs, and compatibility matrix

- [x] update install mode docs so `mcp` installs MCP config, `hook` installs native hooks, `cli` installs instruction fallback text, and `all` installs all three surfaces
- [x] update `atlas install --dry-run` output to show which instruction fallback files would be created or refreshed
- [x] document native hook support versus MCP fallback support in README and wiki with columns `surface`, `works_with`, `automatic`, `token_cost`, and `best_for`
- [x] add `wiki/session-memory-fallback.md` explaining how hookless agents should use `wake_up`, `record_session_event`, `save_context_artifact`, `search_decisions`, `search_saved_context`, `get_global_memory`, and `compact_session`
- [x] update `wiki/hooks-claude.md` and `wiki/hooks-codex.md` to say native hooks and MCP fallback share one event service
- [x] add tests or snapshots proving MCP registry descriptions, installed instructions, and docs do not describe conflicting event names

##### ICM-0 completion criteria

- [x] native hooks and MCP `record_session_event` share one event policy and one persistence/action pipeline
- [x] hookless MCP agents can capture session-start, user-prompt, post-tool-use, file-changed, stop, session-end, and handoff events through installed instructions alone
- [x] `wake_up` exists as a bounded MCP session-start recall surface and never inlines oversized artifacts
- [x] generated `AGENTS.md` and `CLAUDE.md` include mandatory fallback triggers modeled after ICM-style memory instructions
- [x] generated hooks, installed instructions, MCP tools, README, and wiki agree on event names and fallback behavior
- [x] fallback capture remains best-effort, structured, and non-blocking, with tests covering success, skipped, and failure action metadata
- [x] no hook runner, generated instruction, or MCP fallback path writes SQLite directly outside shared Atlas storage/service APIs

#### ICM-A — Shared Memory Surface Over Existing Storage

Rules:

- add one shared memory service layer over existing continuity crates so CLI and MCP reuse identical validation, visibility, and storage behavior
- restore detailed subphase structure here so `ISSUES.md` can replace source roadmap file without losing implementation guidance
- do not create a separate memory architecture that bypasses shipped decision-memory and agent-partition services
- do not store memory bodies or runtime artifacts in `worldtree.db`
- do not require an active session for `project` or `global` writes
- do not let CLI and MCP drift on record shape, defaults, or visibility rules

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-A1 — Memory model and storage schema

- [x] define `MemoryImportance` enum with exact values `critical`, `high`, `normal`, and `low`
- [x] add `importance` field to stored memory records and default manual writes to `normal`
- [x] define `MemoryScope` enum with exact values `project`, `session`, `frontend`, and `global`
- [x] add `scope` field to memory records and make `project` default
- [x] require `frontend` identifier when scope is `frontend`
- [x] add memory tables to continuity-owned storage, preferably existing session-side persistence unless a dedicated memory DB is justified later
- [x] create `memories` table with `id`, `repo_root`, `session_id`, `frontend`, `scope`, `topic`, `title`, `body`, `importance`, `created_at`, `updated_at`, `last_accessed_at`, `decay_score`, `source_id`, and `metadata_json`
- [x] add indexes for `topic`, `importance`, `scope`, `session_id`, and `last_accessed_at`
- [x] reject unknown importance and scope values at CLI, MCP, and storage boundaries
- [x] validate memory schema through `atlas db check` and golden schema tests

##### ICM-A2 — CLI memory CRUD

- [x] add `atlas memory store <text>` with flags `--topic`, `--title`, `--importance`, `--scope`, `--frontend`, `--source-id`, and `--json`
- [x] store memory text exactly as provided unless central redaction policy strips sensitive content
- [x] add `atlas memory recall <query>` with flags `--topic`, `--importance`, `--scope`, `--shared`, `--limit`, and `--json`
- [x] use lexical search first for recall and rank exact topic matches above broad text matches
- [x] add `atlas memory list` with filters `--topic`, `--importance`, `--scope`, `--older-than`, `--newer-than`, and `--json`
- [x] sort memory list by `updated_at DESC` by default
- [x] add `atlas memory delete <memory_id>` with `--dry-run` and `--json`
- [x] require exact memory id for delete and keep linked saved-context artifacts unless explicit delete-source behavior is added later

##### ICM-A3 — Frontend-aware visibility rules

- [x] normalize frontend identities to `claude`, `codex`, `copilot`, `cli`, and `mcp`
- [x] reject unknown frontend names unless config explicitly allows custom frontends
- [x] enforce visibility rules: `global` visible everywhere, `project` visible to all frontends in repo, `session` visible only to same session, `frontend` visible only to same repo plus same frontend
- [x] make `atlas memory recall --shared` return only `global` and `project` memories
- [x] ensure project-scoped writes work without an active session

##### ICM-A4 — MCP parity

- [x] add MCP `memory_store` with same fields and validation as CLI
- [x] add MCP `memory_recall` with same visibility rules and bounded default output
- [x] keep source ids and retrieval hints available in compact MCP output
- [x] add CLI/MCP parity tests so stored record shape, errors, and defaults match

##### ICM-A completion criteria

- [x] `atlas memory store --importance critical` persists `importance = critical`
- [x] `atlas memory store --scope frontend --frontend codex` stores frontend-private memory with correct visibility
- [x] `atlas memory recall --shared` excludes frontend-private memories
- [x] `atlas memory list --importance critical` filters correctly and emits stable JSON
- [x] invalid importance/scope/frontend values fail with clear validation errors
- [x] CLI and MCP memory store/recall paths produce equivalent record shapes

#### ICM-B — Memory Curation, Decay, Health, and Consolidation

Rules:

- add memory decay config with safe defaults and explicit critical-memory protection
- preserve deterministic maintenance structure from source roadmap so cleanup work stays implementation-ready after source file deletion
- do not auto-prune `critical` memories by default
- do not hard-delete linked saved-context artifacts unless explicitly requested
- do not make health scoring or consolidation depend on opaque LLM behavior
- do not mutate state during `--dry-run`

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-B1 — Decay policy config and scoring

- [ ] add memory decay config to `.atlas/config.toml`
- [ ] add default retention policy with `critical` never auto-pruned, `high` long retention, `normal` normal retention, and `low` short retention
- [ ] add config fields `memory.decay.enabled`, `memory.decay.low_days`, `memory.decay.normal_days`, `memory.decay.high_days`, and `memory.decay.critical_never_prune`
- [ ] validate retention days as positive integers and fail `atlas doctor` clearly on invalid config
- [ ] add `atlas memory decay` with `--dry-run`, `--topic`, and `--json`
- [ ] calculate updated `decay_score` without deleting rows

##### ICM-B2 — Stale, prune, and health commands

- [ ] add `atlas memory stale` with `--topic`, `--scope`, and `--json`
- [ ] list only stale memories and never report critical memories as auto-prune candidates
- [ ] add `atlas memory prune` with `--dry-run`, `--topic`, `--importance`, `--older-than`, and `--json`
- [ ] delete only memories marked pruneable by policy and require explicit override before any critical-memory prune path exists
- [ ] add memory health categories `healthy`, `stale`, `noisy`, `duplicated`, `orphaned`, and `oversized`
- [ ] detect low-importance old memories, repeated memories, missing `source_id` references, noisy topics, and topics with no critical decisions
- [ ] add `atlas memory health` with `--topic`, `--scope`, and `--json`
- [ ] emit actionable suggestions and exact follow-up commands in human output

##### ICM-B3 — Deterministic consolidation

- [ ] add deterministic consolidation planner grouping by topic, similar title, similar body, same source id, and same feedback or decision category
- [ ] preserve all `source_id` references in consolidation plan output
- [ ] add `atlas memory consolidate` with `--topic`, `--scope`, `--dry-run`, and `--json`
- [ ] in dry-run mode, report kept ids, merged ids, and source preservation without mutating storage
- [ ] add apply mode that creates consolidated memory, marks merged rows as superseded, and stores supersession links `old_memory_id`, `new_memory_id`, and `reason`
- [ ] make recall prefer consolidated rows while allowing explicit inspection of superseded rows later

##### ICM-B completion criteria

- [ ] default decay config loads without a memory section present
- [ ] `atlas memory decay --dry-run` reports protected critical memories and updated scores
- [ ] `atlas memory prune --importance low --dry-run` reports only pruneable low-priority rows
- [ ] `atlas memory health --topic hooks` returns deterministic findings and suggestions
- [ ] consolidation preserves source references and leaves dry-run fully read-only

#### ICM-C — Feedback Memory and Analysis Confidence Adjustment

Rules:

- add feedback storage and search for predicted vs actual outcomes, correction text, related symbol/file, and `source_id`
- keep feedback as first-class deterministic correction memory rather than loose comments or opaque notes
- do not let feedback override deterministic graph evidence silently
- do not lower confidence without explicit matching evidence
- do not couple feedback storage to graph tables or graph-node lifecycle

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-C1 — Feedback storage and search model

- [ ] create `feedback_records` table with `id`, `repo_root`, `session_id`, `tool_name`, `analysis_kind`, `predicted`, `actual`, `correction`, `related_symbol`, `related_file`, `source_id`, `created_at`, and `metadata_json`
- [ ] add FTS index for `predicted`, `actual`, `correction`, `related_symbol`, and `related_file`
- [ ] keep feedback searchable by symbol, file, correction text, and analysis kind

##### ICM-C2 — CLI and MCP feedback commands

- [ ] add `atlas feedback record` with required `--predicted` and `--actual`
- [ ] add optional `--correction`, `--tool`, `--analysis-kind`, `--symbol`, `--file`, `--source-id`, and `--json`
- [ ] add `atlas feedback search <query>` with filters `--tool`, `--analysis-kind`, `--symbol`, `--file`, `--limit`, and `--json`
- [ ] add `atlas feedback stats` with deterministic summary and `--json`
- [ ] add MCP `feedback_record` using same service layer and validation contract

##### ICM-C3 — Confidence adjustment integration

- [ ] query feedback before returning results from `atlas analyze dead-code`, `atlas analyze remove`, `atlas analyze safety`, and `atlas refactor remove-dead --dry-run`
- [ ] lower confidence only when prior feedback indicates false positives for same symbol, file, pattern, or analysis kind
- [ ] expose `feedback_evidence` in analysis JSON whenever scoring changes
- [ ] add config flag `analysis.feedback_adjustment.enabled`

##### ICM-C completion criteria

- [ ] missing `--predicted` or `--actual` fails validation
- [ ] feedback search returns predicted, actual, correction, related symbol/file, score, and created time
- [ ] empty feedback DB returns stable zero-count stats
- [ ] stored false-positive feedback can lower confidence only when evidence actually matches

#### ICM-D — Wake-Up Packs and Session Start Recall

Rules:

- define a bounded wake-up pack that summarizes current focus, critical memories, recent decisions, recent feedback, graph readiness, changed files, and retrieval hints
- keep wake-up path compact, retrieval-backed, and consistent with resume architecture already shipped in continuity work
- do not inline raw large artifacts into wake-up or resume payloads
- do not block session start on wake-up generation failure
- do not replay raw command history as wake-up context

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-D1 — Wake-up pack model

- [ ] define `WakePack` model with `repo_root`, `session_id`, `frontend`, `current_focus`, `recent_decisions`, `critical_memories`, `recent_feedback`, `active_memoir_concepts`, `changed_files`, `graph_readiness`, `retrieval_hints`, and `generated_at`
- [ ] bound wake-up pack size through config and central budget policy
- [ ] serialize wake-up packs to stable JSON

##### ICM-D2 — CLI and MCP wake-up

- [ ] add `atlas wake-up` with flags `--topic`, `--session`, `--frontend`, `--max-items`, and `--json`
- [ ] pull wake-up content from memory, feedback, session resume, and graph readiness services
- [x] add MCP `wake_up` with compact default output, retrieval hints, and source ids instead of raw artifact bodies (shipped early by ICM-0E)

##### ICM-D3 — Hook integration

- [ ] call wake-up generation from `SessionStart` hook paths where host supports it
- [ ] attach wake-up packs to session resume only through bounded injection paths
- [ ] store wake-up generation success or failure metadata in session events
- [ ] keep hook failures non-blocking and best-effort

##### ICM-D completion criteria

- [ ] `atlas wake-up --topic hooks` prioritizes topic-relevant memories and feedback
- [ ] wake-up output references large artifacts by `source_id` only
- [ ] hook failures do not stop host command flow
- [ ] snapshot tests cover empty, normal, and large sessions

#### ICM-E — Cross-Session Recall Quality and Optional Semantic Recall

Rules:

- improve recall ranking with topic match, importance, recency, scope visibility, and source-backed evidence
- preserve lexical-first default and make cross-session recall quality measurable before adding vector complexity
- do not make embeddings required for baseline memory recall
- do not let vector scores outrank exact lexical or stronger structural evidence by default
- do not widen frontend-private or session-private recall unless caller explicitly asks for it

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-E1 — Cross-session recall quality

- [ ] extend memory recall across prior repo sessions while preserving agent/frontend visibility boundaries
- [ ] rank recall by topic match, importance, recency, scope visibility, and source-backed evidence
- [ ] make system capable of recalling past sessions without mixing raw session history into future context
- [ ] optimize context selection so recall surfaces the highest-signal memories first

##### ICM-E2 — Optional semantic and vector recall

- [ ] add config `memory.embedding.enabled`, `memory.embedding.provider`, `memory.embedding.model`, `memory.embedding.dimension`, `memory.search.hybrid_weight_fts`, and `memory.search.hybrid_weight_vector`
- [ ] keep embeddings disabled by default and require explicit opt-in
- [ ] add `memory_embeddings` table with `memory_id`, `embedding_model`, `dimension`, `vector_blob`, and `created_at`
- [ ] reject vector inserts when configured dimension does not match stored dimension
- [ ] add `atlas memory recall <query> --hybrid` using reciprocal-rank fusion only after lexical evaluation and budget metrics exist
- [ ] keep graph-backed and exact lexical evidence stronger than vector-only matches by default

##### ICM-E completion criteria

- [ ] baseline memory recall works lexically with no embedding provider configured
- [ ] enabling embeddings without provider or valid dimension fails clearly
- [ ] hybrid recall returns ranking explanation fields without burying exact keyword hits
- [ ] cross-session recall respects `global`, `project`, `session`, and `frontend` visibility boundaries

#### ICM-F — Memoir Concept Graph as Separate Knowledge Layer

Rules:

- add separate memoir tables, concepts, relations, and graph ids outside the code graph schema
- keep memoir path explicit and bounded so semantic memory does not leak into code graph semantics
- do not merge memoir concepts into code graph `nodes` and `edges`
- do not allow unbounded custom relation types by default
- do not auto-create missing concepts unless caller explicitly opts in

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-F1 — Memoir schema and vocabulary

- [ ] create `memoir_graphs`, `memoir_concepts`, and `memoir_relations` tables separate from code graph storage
- [ ] store relation fields `graph_id`, `source_concept_id`, `target_concept_id`, `relation_type`, `confidence`, `source_id`, `created_at`, and `metadata_json`
- [ ] add controlled relation vocabulary `depends_on`, `part_of`, `contradicts`, `refines`, `replaces`, `caused_by`, `fixed_by`, `blocked_by`, `decided_by`, and `related_to`
- [ ] normalize aliases such as `replaced_by` and `separate_from` with explicit direction or tagging rules
- [ ] reject unknown relation types unless config later enables custom relations

##### ICM-F2 — CLI and MCP memoir commands

- [ ] add `atlas memoir create <name>` with `--description`, `--scope`, and `--json`
- [ ] add `atlas memoir add-concept <graph> <name> <description>` with `--kind`, `--source-id`, and `--json`
- [ ] add `atlas memoir link <graph> <source> <target> --relation <type>` with `--confidence`, `--source-id`, and `--json`
- [ ] add `atlas memoir inspect <concept>` with `--graph`, `--depth`, `--relation`, and `--json`
- [ ] add MCP `memoir_create`, `memoir_add_concept`, `memoir_link`, and `memoir_inspect` as thin wrappers over same service layer

##### ICM-F completion criteria

- [ ] duplicate memoir graph names fail deterministically in same repo and scope
- [ ] `atlas memoir link A B --relation depends_on` succeeds and invalid relation names fail clearly
- [ ] bounded inspect output includes relation direction and source evidence ids
- [ ] code graph queries remain unaware of memoir tables unless explicit memoir surface is invoked

#### ICM-G — Code Overview Memory for External Analysis

Rules:

- add a graph-linked overview memory layer for project, package, module, file, symbol, function, and method descriptions
- let Atlas export bounded analysis packets and ingest externally produced LLM or human analysis
- keep Atlas non-LLM by default; no built-in model provider, prompt runner, or hidden network call in this track
- use overview memory as readable package/module/spec documentation and as guidance inside context/review payloads
- keep overview records tied to commit SHA, graph freshness, canonical repo paths, and qualified names so stale guidance is detectable
- do not store overview bodies in `worldtree.db`
- do not make overview text a graph fact or merge it into graph `nodes` and `edges`
- do not let stale overview records appear as fresh context without explicit stale metadata
- do not require embeddings or an LLM provider for baseline overview search
- do not add compatibility shims for old overview schemas until a first stable schema ships
- do not auto-run external analyzers during normal `build`, `update`, `query`, `context`, or MCP flows unless user explicitly configures hooks

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-G1 — Overview domain model and subject identity

- [ ] define `OverviewSubjectKind` enum with exact values `project`, `package`, `module`, `file`, `symbol`, `function`, and `method`
- [ ] define `OverviewFreshness` enum with exact values `fresh`, `possibly_stale`, `stale`, `orphaned`, and `unverified`
- [ ] define `OverviewSourceKind` enum with exact values `external_llm`, `manual`, and `imported`
- [ ] define `OverviewSubject` with `kind`, `repo_root`, `commit_sha`, optional `package_name`, optional `module_path`, optional `canonical_file_path`, optional `qualified_name`, optional `node_kind`, optional `line_start`, optional `line_end`, and optional `content_hash`
- [ ] validate `project` subjects require only `repo_root` and `commit_sha`
- [ ] validate `package` subjects require `package_name`
- [ ] validate `module` subjects require `module_path`
- [ ] validate `file` subjects require `canonical_file_path`
- [ ] validate `symbol`, `function`, and `method` subjects require `qualified_name`, `canonical_file_path`, `line_start`, and `line_end`
- [ ] canonicalize every subject path through `atlas_repo::CanonicalRepoPath` before hashing, persistence, dedupe, lookup, or stale matching
- [ ] define deterministic `subject_id` as versioned hash over `repo_root`, `kind`, canonical subject fields, and schema version
- [ ] reject local path-normalization helpers and add tests proving `./src/lib.rs` and `src/lib.rs` resolve to same subject identity
- [ ] define `OverviewRecord` with subject, title, summary, description, responsibilities, flow, inputs, outputs, invariants, gotchas, examples, tags, source metadata, freshness, confidence, source ids, and timestamps
- [ ] represent list fields as typed vectors at service boundary and JSON arrays at storage boundary
- [ ] bound title, summary, description, and each list field through central budget policy before storage
- [ ] route oversized overview bodies through existing content-store artifact routing and store only preview plus `source_id` in overview table
- [ ] add unit tests for subject validation, enum parsing, subject-id stability, path canonicalization, and budget truncation metadata

##### ICM-G2 — Overview storage schema and migrations

- [ ] add overview tables to continuity-owned storage, preferably the memory/session-side persistence used by ICM-A unless a dedicated continuity DB is justified in code comments
- [ ] create `overview_records` table with `id`, `repo_root`, `subject_id`, `subject_kind`, `package_name`, `module_path`, `canonical_file_path`, `qualified_name`, `node_kind`, `line_start`, `line_end`, `content_hash`, `title`, `summary`, `description_preview`, `responsibilities_json`, `flow_json`, `inputs_json`, `outputs_json`, `invariants_json`, `gotchas_json`, `examples_json`, `tags_json`, `source_kind`, `analyzer_name`, `analyzer_version`, `model_name`, `commit_sha`, `graph_last_indexed_at`, `freshness`, `confidence`, `source_id`, `supersedes_id`, `created_at`, `updated_at`, and `metadata_json`
- [ ] create `overview_record_fts` over `title`, `summary`, `description_preview`, `responsibilities_json`, `flow_json`, `invariants_json`, `gotchas_json`, and `tags_json`
- [ ] add indexes for `repo_root`, `subject_id`, `subject_kind`, `canonical_file_path`, `qualified_name`, `commit_sha`, `freshness`, `source_kind`, and `updated_at`
- [ ] add uniqueness rule for active records by `repo_root`, `subject_id`, `commit_sha`, and `source_kind` unless `supersedes_id` is set
- [ ] add supersession support so re-ingest creates a new active record and points to previous active record through `supersedes_id`
- [ ] preserve old overview records for audit unless user later adds explicit prune command
- [ ] add storage API methods `insert_overview_record`, `get_overview_record`, `search_overview_records`, `list_stale_overview_records`, `supersede_overview_record`, and `mark_overview_freshness`
- [ ] make storage API reject invalid enum strings and malformed JSON arrays before writing
- [ ] update `atlas db check` to validate overview schema, FTS integrity, orphan `source_id` references, invalid enum values, and noncanonical path rows
- [ ] add migration golden tests, in-memory storage tests, FTS search tests, and db-check failure fixture tests

##### ICM-G3 — Analysis packet export contract

- [ ] add `OverviewExportRequest` with `scope`, `subjects`, `changed_only`, `since`, `limit`, `include_code_spans`, `include_callers`, `include_callees`, `include_tests`, `max_tokens`, and `json` fields
- [ ] add `OverviewAnalysisPacket` with `schema_version`, `repo_root`, `commit_sha`, graph provenance, freshness metadata, subject, concise graph evidence, bounded code excerpt, callers, callees, related files, test adjacency, and retrieval hints
- [ ] support export scopes `project`, `packages`, `modules`, `files`, `symbols`, `functions`, `methods`, and `changed`
- [ ] implement `atlas overview export --scope <scope> --json`
- [ ] implement `atlas overview export --subject <qualified_name_or_path> --json`
- [ ] implement `atlas overview export --changed --since <rev> --json`
- [ ] use graph-backed context resolution first for symbols and functions, then companion content lookup only for docs/config/assets surfaced by graph/context evidence
- [ ] include canonical subject identity in every packet so ingest can validate exact target later
- [ ] include stable packet id as hash over subject id, commit SHA, selected evidence ids, and export schema version
- [ ] include `safe_to_answer`, graph freshness, omitted counts, and budget-hit metadata in every packet
- [ ] fail export clearly when graph readiness is `corrupt`; allow stale export only with explicit stale metadata
- [ ] add tests for project export, package export, changed-only export, function export, ambiguous subject failure, stale graph metadata, and budget truncation

##### ICM-G4 — External analysis ingest contract

- [ ] define `OverviewAnalysisInput` JSON schema with `schema_version`, `packet_id`, subject identity, title, summary, description, responsibilities, flow, inputs, outputs, invariants, gotchas, examples, tags, confidence, analyzer metadata, and optional source artifact ids
- [ ] implement `atlas overview ingest <path>` for JSON file input
- [ ] implement `atlas overview ingest -` for stdin input
- [ ] validate input `schema_version` exactly before field validation
- [ ] validate `packet_id` when present and return clear mismatch error when packet id does not match exported packet metadata
- [ ] validate subject exists in current or indexed graph by canonical file path and qualified name where applicable
- [ ] validate line ranges still overlap same graph node before marking ingested record `fresh`
- [ ] mark record `possibly_stale` when commit SHA differs but subject identity still resolves
- [ ] mark record `orphaned` when canonical path or qualified name no longer resolves
- [ ] mark record `unverified` for manual/imported records without packet id
- [ ] enforce confidence range `0.0..=1.0`
- [ ] require `summary` for all records and require either `description` or at least one non-empty detail array
- [ ] reject unknown top-level fields unless `metadata_json.extra` explicitly captures them through a controlled importer path
- [ ] run overview text through central redaction policy before persistence
- [ ] route large descriptions and examples through content-store when over inline budget
- [ ] add JSON schema fixture tests for valid external LLM result, valid manual import, missing summary, bad confidence, subject mismatch, stale commit, orphan subject, and secret redaction

##### ICM-G5 — Freshness, commit update, and refresh planning

- [ ] add `OverviewFreshnessService` that compares overview records against current graph readiness, current commit SHA, changed files, content hashes, and line-span overlap
- [ ] implement direct stale marking when record canonical file path changed since stored commit
- [ ] implement direct stale marking when record content hash differs from current file hash
- [ ] implement direct stale marking when record qualified name no longer resolves
- [ ] implement `possibly_stale` marking for callers, containing modules, containing packages, and project records when dependent files changed
- [ ] implement orphan marking for deleted files, removed symbols, and renamed subjects without confident canonical target
- [ ] use `detect_changes`/history data when available; fall back to git diff only through existing repo/change services, not ad hoc shell parsing in service code
- [ ] implement `atlas overview stale` with filters `--subject-kind`, `--file`, `--package`, `--module`, `--since`, `--limit`, and `--json`
- [ ] implement `atlas overview refresh-plan --changed --since <rev> --json`
- [ ] make refresh plan output include subject id, stale reason, suggested export command, affected dependents, previous record id, and priority
- [ ] rank refresh plan priority by direct change before dependent change, subject kind specificity, current context relevance, confidence, and updated_at age
- [ ] keep refresh planning read-only unless caller passes explicit apply flag for freshness marking
- [ ] add tests for changed function, changed file, deleted file, renamed file, caller possibly stale, package possibly stale, and no-op unchanged commit

##### ICM-G6 — Overview CLI read and maintenance surfaces

- [ ] implement `atlas overview show <subject>` resolving exact subject id, qualified name, canonical file path, package name, or module path
- [ ] make ambiguous `show` results fail with candidate list and required disambiguation fields
- [ ] implement `atlas overview search <query>` with filters `--subject-kind`, `--freshness`, `--package`, `--module`, `--file`, `--source-kind`, `--limit`, and `--json`
- [ ] rank search by exact subject match, exact title/tag match, FTS score, freshness, confidence, recency, and subject specificity
- [ ] implement `atlas overview list` with filters `--subject-kind`, `--freshness`, `--source-kind`, `--older-than`, `--newer-than`, and `--json`
- [ ] implement `atlas overview delete <overview_id> --dry-run --json`
- [ ] require exact overview id for delete and keep routed content-store artifacts unless explicit artifact-delete behavior is added later
- [ ] implement `atlas overview export-docs --format markdown --output <path>` for readable project/package/module specs
- [ ] make exported docs group by project, package, module, file, then symbol, with stale records labeled visibly
- [ ] make human output use `println!`/`eprintln!`; reserve tracing macros for diagnostics
- [ ] add CLI smoke tests and JSON snapshot tests for show, search, list, stale, refresh-plan, delete dry-run, and markdown export

##### ICM-G7 — Context engine and retrieval integration

- [ ] extend context request controls with `include_overviews`, `overview_limit`, `overview_freshness`, `overview_subject_kinds`, and `overview_max_bytes`
- [ ] include fresh overview records in `atlas context`, `atlas review-context`, MCP `get_context`, and MCP `get_review_context` when they match selected symbols, files, modules, packages, or changed files
- [ ] include stale records only when request allows stale overviews and always emit stale reason
- [ ] rank overview context by exact symbol match, exact file match, containing module/package, changed-file relevance, freshness, confidence, and recency
- [ ] emit overview selection reasons such as `same_symbol`, `same_file`, `containing_module`, `package_summary`, `project_summary`, and `changed_dependency`
- [ ] keep overview payload preview-only by default and expose `source_id` for full body retrieval
- [ ] merge overview records under existing graph/content/session budget policy instead of adding separate truncation rules
- [ ] ensure overview text cannot override graph facts in risk, impact, dead-code, or refactor analysis
- [ ] add tests for graph-only context, overview-only companion context, mixed graph/overview context, stale overview exclusion, stale overview explicit inclusion, and budget trimming

##### ICM-G8 — MCP parity and external analyzer handoff

- [ ] add MCP `overview_export` with same request fields, defaults, and JSON shape as CLI export
- [ ] add MCP `overview_ingest` with same validation and error shape as CLI ingest
- [ ] add MCP `overview_search` with same filters and ranking evidence as CLI search
- [ ] add MCP `overview_show` with same ambiguity behavior as CLI show
- [ ] add MCP `overview_refresh_plan` with same read-only default behavior as CLI refresh-plan
- [ ] keep MCP default output compact and include `source_id`, freshness, selection reason, and next export command where relevant
- [ ] add parity tests proving CLI and MCP record shapes, validation failures, freshness states, and default limits match
- [ ] expose optional external analyzer handoff only as packet generation plus documented command contract, not as built-in model execution

##### ICM-G9 — Hook and manual trigger integration

- [ ] add config section `overview` with `enabled`, `auto_export_on_commit`, `auto_mark_stale_on_commit`, `external_analyzer_command`, `ingest_after_external_command`, `max_subjects_per_run`, and `max_packet_bytes`
- [ ] default `overview.enabled = true`, `auto_export_on_commit = false`, `auto_mark_stale_on_commit = true`, and `ingest_after_external_command = false`
- [ ] validate external analyzer command is absent or explicit string path/command; never infer model provider from environment variables
- [ ] add hook integration that can run `overview refresh-plan` after commit or manual hook trigger
- [ ] keep hook failures best-effort and non-blocking for git/host flow
- [ ] store hook-generated packet exports in content-store when oversized and reference them by `source_id`
- [ ] record hook outcome as session event with command, status, packet count, ingested count, stale count, and source ids
- [ ] add manual command `atlas overview run-external --dry-run --json` that prints exact external command invocations without executing them
- [ ] add apply mode for `run-external` that executes configured command, requires JSON output, validates ingest, and reports per-subject success/failure
- [ ] add tests for default config, invalid config, dry-run command generation, nonblocking hook failure, successful external ingest, malformed external output, and max-subject cap

##### ICM-G10 — Overview docs, fixtures, and release gate

- [ ] add `wiki/overview-memory.md` documenting storage ownership, external analyzer contract, JSON schemas, freshness states, context integration, hook behavior, and CLI/MCP parity
- [ ] add reusable fixtures for project overview, package overview, module overview, file overview, function overview, stale function overview, orphaned symbol overview, manual overview, and oversized overview body
- [ ] add JSON snapshots for `overview export`, `overview ingest --json`, `overview show --json`, `overview search --json`, `overview stale --json`, `overview refresh-plan --json`, and MCP overview tools
- [ ] add markdown snapshot for `overview export-docs --format markdown`
- [ ] add schema evolution note that first stable schema is `schema_version = 1` and later breaking changes must ship migration or explicit import rejection
- [ ] update `wiki/memory-architecture.md` to explain overview memory as evidence-linked code documentation separate from generic memories, feedback records, and memoir concepts
- [ ] define release gate `ICM Overview Memory Complete`
- [ ] require for release gate: storage schema, export/ingest contracts, freshness planner, CLI read surfaces, context integration, MCP parity, hook/manual trigger path, docs, fixtures, and JSON snapshots
- [ ] require for release gate: no overview body writes to `worldtree.db`, no built-in LLM provider, no stale overview emitted as fresh, and no path-derived identity without `CanonicalRepoPath`

##### ICM-G completion criteria

- [ ] `atlas overview export --scope functions --json` emits bounded packets with subject ids, commit SHA, graph freshness, and budget metadata
- [ ] `atlas overview ingest analysis.json` stores a fresh record when packet id, commit SHA, canonical path, qualified name, and line span match current graph
- [ ] `atlas overview ingest analysis.json` marks record `possibly_stale` when commit SHA differs but subject still resolves
- [ ] `atlas overview ingest analysis.json` marks record `orphaned` when subject no longer resolves
- [ ] `atlas overview refresh-plan --changed --since HEAD~1 --json` reports directly stale and possibly stale subjects with suggested export commands
- [ ] `atlas context --include-overviews` includes fresh overview records with selection reasons and source ids
- [ ] MCP overview tools match CLI JSON defaults and validation behavior
- [ ] `atlas db check` reports invalid overview schema, invalid enum values, orphan source ids, and noncanonical overview paths
- [ ] `./scripts/test-workspace-summary.sh` passes after overview memory implementation

#### ICM-H — Shell-First Install Modes, TUI, Docs, and Release Gates

Rules:

- add install/init mode split for `mcp`, `hook`, `cli`, and `all`, with idempotent generation and dry-run preview
- keep shell-first and TUI-first operational structure from source roadmap while dropping slash-command, skill, and dashboard work
- do not add slash-command generators or skill-install surfaces for this track
- do not add web dashboard routes for memory inspection in this track
- do not build TUI surfaces before core service contracts and tests stabilize
- do not introduce host-specific command generators that bypass shared service logic

Rules apply to every checklist item in this ICM section.

Implementation structure:

##### ICM-H1 — Shell-first install and init modes

- [ ] add supported `atlas init --mode` values `mcp`, `hook`, `cli`, and `all`
- [ ] make each mode idempotent and emit files to be created during `--dry-run`
- [ ] ensure `--mode all` installs only MCP config, hooks, and CLI config relevant to shell-first memory workflows

##### ICM-H2 — TUI only, read-only first

- [ ] add `atlas memory tui` with read-only browsing for memories, topics, feedback, memoir concepts, overview records, health findings, and saved artifacts
- [ ] add filters for topic, scope, importance, and frontend
- [ ] add overview filters for subject kind, freshness, package, module, file, source kind, and updated time
- [ ] keep first version non-mutating and smoke-testable without panic

##### ICM-H3 — Tests, docs, and release gates

- [ ] create reusable fixtures for critical decision memory, low-priority stale memory, dead-code false-positive feedback, memoir dependency graph, overview memory records, wake-up pack with saved artifact references, and frontend-private memory
- [ ] snapshot JSON output for `atlas memory store --json`, `atlas memory recall --json`, `atlas memory health --json`, `atlas feedback record --json`, `atlas feedback search --json`, `atlas memoir inspect --json`, `atlas overview show --json`, `atlas overview search --json`, and `atlas wake-up --json`
- [ ] add `wiki/memory-architecture.md` documenting memory DB ownership, importance and decay policy, scope and visibility rules, feedback integration, memoir graph separation, overview memory separation, wake-up behavior, and CLI/MCP mapping
- [ ] define release gate `ICM Memory Layer Complete`
- [ ] require for release gate: CLI and MCP memory store/recall parity, importance and decay policies, feedback-adjusted analysis, memoir typed relations, overview memory export/ingest/context parity, wake-up packs without raw large content, health audit coverage, shared/private visibility rules, complete docs, and JSON snapshot coverage

##### ICM-H completion criteria

- [ ] every new shell-first memory command has CLI smoke coverage
- [ ] every MCP memory tool has handler tests and parity assertions where applicable
- [ ] `cargo test --workspace` passes with fixtures and JSON snapshots committed
- [ ] no memory feature writes directly to graph DB
- [ ] no large artifact is inlined into wake-up or resume output by default
- [ ] no overview memory feature treats external analysis text as authoritative graph fact

---

## Part V — Follow-Up Patches

Use these patch sections for focused improvements that cut across existing roadmap phases without rewriting phase scope.

### Retrieval Follow-Up Patch

These are the high-value retrieval/indexing improvements still missing or only partially specified after the current v3 plan.

They are meant to strengthen Atlas’s retrieval/content sidecar without changing the graph-first core.

#### Patch R7 — Later experimental post-retrieval compaction

This is not core and should stay late, but it is a useful optional experiment once retrieval and context engine behavior are stable.

- [ ] add backlog item for post-retrieval compaction experiment
- [ ] only evaluate after:
  - [ ] hybrid retrieval is stable
  - [ ] context engine output quality is stable
  - [ ] token-efficiency metrics exist
- [ ] keep initial experiment strictly optional
- [ ] require evidence that compaction reduces tokens without harming answer quality
- [ ] do not let this replace retrieval filtering or graph-based selection

Why:
- useful possible optimization later
- should not destabilize current graph-first + retrieval-assisted architecture

#### Patch completion criteria

This patch is complete when:

- [x] retrieval/content index has explicit searchable state
- [x] retrieval indexing has batch and chunk guardrails
- [x] embedding dimension rules are explicit and enforced
- [x] retrieval backend capabilities are validated, not assumed
- [x] stable `chunk_id` exists and is used for dedupe/reuse
- [x] retrieval/token-efficiency benchmarks are in place
- [ ] optional post-retrieval compaction is tracked as a late experiment only

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
- [x] enriched events are deterministic, bounded, redacted, and deduplicated
- [ ] redaction policy supports runtime-loaded external rule files with validation
- [ ] event-to-graph links use stable identifiers and treat row IDs as optional cache hints
- [ ] graph linking obeys readiness state and budget policy
- [ ] context engine can merge runtime events/artifacts with graph and saved context under one bounded ranking policy
- [ ] CLI, MCP, and hook flows feed enrichment best-effort
- [ ] resume snapshots include compact enriched runtime signals
- [ ] tests cover classification, artifact routing, graph linking, context integration, and resume enrichment

---


### Rust Reachability Guard Patch

Atlas Rust call resolution can over-report cross-file references for orphan files because `same_package` heuristics use package ownership plus simple-name matching, but do not verify crate-root module reachability. A file can be outside the compiled module tree and still accumulate inbound graph edges. `cross_file_links` then treats those heuristic edges as evidence that the file is connected.

The current `resolve_same_package_target` in `atlas-engine/src/call_resolution.rs` filters candidates by `owner_id` match (Cargo package) and then optionally by same directory. Neither check requires the candidate file to be reachable from any crate root via `mod` declarations. This lets stale, deleted, or orphan files remain as resolution targets as long as they share a Cargo package.

Design overview:

Two complementary data structures carry the fix:

1. **`CrateReachabilityIndex`** — built once per Cargo package during the parse/build phase. Stores the set of canonical file paths reachable from each crate root (lib, main, example, test, bench) within the package. Built by walking `mod` declarations in parsed ASTs rather than filesystem scanning. Lives in `atlas-engine` or `atlas-parser`; never written to `worldtree.db`.

2. **`ReachabilityGuard`** — thin wrapper passed into `resolve_same_package_target` alongside the existing `Store` and owner cache. Given a `(caller_file, candidate_file)` pair, it answers `is_reachable(candidate_file, from_crate_root_of: caller_file)`. Returns `false` when the index is absent (safe default: block heuristic edge rather than assume live).

Edge provenance gets one new field: `reachability_checked: bool`. When `true` and `same_package` tier is set, the candidate passed crate-root reachability. When `false`, the edge is a legacy heuristic edge emitted before the guard existed.

#### Patch R1 — `CrateReachabilityIndex` model and builder

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

#### Patch R2 — `ReachabilityGuard` and integration into `resolve_same_package_target`

- [ ] define `ReachabilityGuard` in `atlas-engine`:
  - [ ] wraps `HashMap<String, CrateReachabilityIndex>` keyed by `owner_id`
  - [ ] `is_reachable_from_same_root(caller: &str, candidate: &str) -> ReachabilityResult`
  - [ ] `ReachabilityResult` variants: `Reachable`, `Unreachable`, `IndexAbsent`
  - [ ] treat `IndexAbsent` as non-reachable (safe default: do not emit heuristic edge without evidence)
- [ ] build `ReachabilityGuard` once per engine build/update run, before resolution pass
- [ ] thread `ReachabilityGuard` into `resolve_same_package_target` alongside existing `owner_cache`
- [ ] update `resolve_same_package_target` resolution order:
  1. filter candidates by `owner_id` (existing step — coarse package filter)
  2. apply receiver-hint filtering (existing step — keep)
  3. **new**: filter `same_owner_matches` to retain only candidates where `ReachabilityGuard::is_reachable_from_same_root(caller, candidate)` returns `Reachable`
  4. apply existing same-dir tie-break on the reachability-filtered set
  5. if reachability index is absent (`IndexAbsent`), fall back to existing behavior but mark edge with `reachability_checked: false`
- [ ] add `reachability_checked: bool` to edge metadata or edge extra fields (stored in existing `metadata` JSON or new column)
- [ ] add regression tests:
  - [ ] orphan file in same Cargo package is rejected as same-package target after reachability filtering
  - [ ] live file reachable via `mod` chain is accepted as same-package target
  - [ ] receiver-hint still narrows candidates correctly after reachability filtering
  - [ ] absent index falls back gracefully and does not panic

Why:
- package membership alone is too broad; reachability narrows to files the compiler actually sees
- `IndexAbsent` fallback prevents breaking existing resolution for languages or layouts where index is not built

#### Patch R3 — Edge provenance and `cross_file_links` filtering

- [ ] audit `cross_file_links` query for Rust heuristic-edge false positives:
  - [ ] identify whether `cross_file_links` joins only on edge existence or also on confidence tier
  - [ ] determine whether filtering at read time or write time is safer given incremental update semantics
- [ ] decide and document filter strategy:
  - [ ] **preferred**: filter at write time — do not persist `same_package` edges for unreachable candidates; `cross_file_links` naturally sees correct graph
  - [ ] **acceptable fallback**: filter at read time — add `reachability_checked = true` predicate to `cross_file_links` query for Rust `same_package` edges
  - [ ] document chosen strategy in a code comment near the `cross_file_links` query
- [ ] ensure incremental update removes stale node rows and their inbound `same_package` edges when a file is deleted
  - [ ] verify existing node deletion cascade covers edge rows; add explicit edge cleanup if missing
- [ ] expose edge provenance in `cross_file_links` output:
  - [ ] add `confidence_tier` to `CrossFileLink` result struct if not already present
  - [ ] add `reachability_checked` flag to `CrossFileLink` when available
- [ ] add tests:
  - [ ] orphan Rust file shows zero `cross_file_links` inbound edges after reachability-gated build
  - [ ] deleted Rust file shows zero `cross_file_links` results after incremental refresh removes its nodes
  - [ ] import-backed edge (`use` / `extern crate`) still appears in `cross_file_links` regardless of reachability guard

Why:
- `cross_file_links` is the user-visible surface; false-positive heuristic edges here mislead dead-code and impact analysis
- write-time filtering is cleaner than read-time masking

#### Patch R4 — Diagnostics and observability

- [ ] expose reachability index stats in `atlas doctor` / `atlas db_check` output:
  - [ ] number of Cargo packages with reachability index built
  - [ ] number of packages where index build failed or was skipped
  - [ ] number of unresolved `mod` targets across all packages
  - [ ] number of `same_package` edges emitted with `reachability_checked: true` vs `false`
- [ ] expose reachability status per file in `atlas status --json` or a dedicated debug command:
  - [ ] file is reachable from which crate root(s)
  - [ ] file has no reachable crate root (orphan)
- [ ] log reachability index build failures at `warn` level with package path; do not fail the build
- [ ] add MCP `doctor` response fields for reachability index health when data is available

Why:
- operators need to see whether the guard is active and which packages lack an index
- silent guard absence produces the same false positives as before, so visibility is required

#### Patch R completion criteria

- [ ] `CrateReachabilityIndex` model exists and is built from parsed AST `mod` declarations
- [ ] `ReachabilityGuard` wraps the index and answers caller/candidate reachability queries
- [ ] `resolve_same_package_target` in `atlas-engine/src/call_resolution.rs` filters candidates through `ReachabilityGuard` before emitting `same_package` edges
- [ ] `same_package` edges carry `reachability_checked` provenance
- [ ] `cross_file_links` does not claim orphan Rust files are connected after a reachability-gated build
- [ ] incremental refresh removes deleted-file nodes and clears their inbound edges
- [ ] `atlas doctor` reports reachability index coverage and unresolved mod counts
- [ ] tests cover: orphan file rejection, live file acceptance, receiver-hint interaction, absent index fallback, deleted-file cleanup, and `cross_file_links` false-positive regression

---

Rust Parser Query-Backed Extraction Patch is shipped. See SHIPPED.md for details.

### Shared Parser Query Migration Patch

Implement this only after Patch Q is complete. Rust is the pilot for shared query infrastructure and capture conventions. This patch migrates the remaining tree-sitter-backed language parsers to the same query-backed extraction model without changing parser public APIs, database schemas, or graph output contracts.

The migration rule is: `.scm` queries identify language syntax facts; Rust code in each language parser still owns Atlas graph semantics, including qualified names, parent scopes, edge kinds, confidence tiers, source metadata, and language-specific heuristics. Do not replace semantic resolution with query captures alone.

Check https://github.com/helix-editor/helix/tree/master/runtime/queries for scm grammar references for the languages.

Use Helix queries only as grammar reference for tree-sitter node names and scope patterns, especially `runtime/queries/*/tags.scm` and `runtime/queries/*/locals.scm`. Do not copy Helix query files verbatim unless license handling is added, because Helix is MPL-2.0. Atlas query files must be authored for Atlas captures.

#### Patch SQ1 — Shared query contract and migration harness

- [ ] document the shared query-backed parser contract in `packages/atlas-parser/README.md`:
  - [ ] query files live under `packages/atlas-parser/queries/<language>.scm`
  - [ ] capture names use the `@atlas.*` namespace
  - [ ] queries capture syntax facts only
  - [ ] language parser code maps captures into `Node`, `Edge`, and `ParsedFile`
  - [ ] language parser public APIs remain unchanged
- [ ] harden shared query helpers created by Patch Q:
  - [ ] support loading one static query per language via `include_str!`
  - [ ] expose helper for capture lookup by exact capture name
  - [ ] expose helper for optional and required captures with clear test failures
  - [ ] expose helper to sort captures by byte range for deterministic output
  - [ ] expose helper to preserve source-order traversal when multiple query matches overlap
- [ ] define common capture naming conventions:
  - [ ] `@atlas.definition.function`
  - [ ] `@atlas.definition.method`
  - [ ] `@atlas.definition.class`
  - [ ] `@atlas.definition.module`
  - [ ] `@atlas.definition.struct`
  - [ ] `@atlas.definition.enum`
  - [ ] `@atlas.definition.interface`
  - [ ] `@atlas.definition.trait`
  - [ ] `@atlas.definition.constant`
  - [ ] `@atlas.definition.variable`
  - [ ] `@atlas.import`
  - [ ] `@atlas.call`
  - [ ] `@atlas.reference`
  - [ ] `@atlas.name`
  - [ ] `@atlas.parameters`
  - [ ] `@atlas.return_type`
  - [ ] `@atlas.receiver`
- [ ] add query helper tests:
  - [ ] invalid query text returns a clear error
  - [ ] missing required capture returns a clear error
  - [ ] optional capture absence does not fail
  - [ ] capture order is deterministic across repeated runs
  - [ ] overlapping captures preserve match order before graph builder filtering
- [ ] add migration checklist comments in each remaining parser file naming the existing manual extraction responsibilities before refactor starts

Why:
- prevents each language migration from inventing incompatible capture names
- makes query-backed parser behavior testable before broad parser churn
- keeps graph semantics explicit and separate from tree-sitter syntax matching

#### Patch SQ2 — Migrate C-family compiled language parsers

- [ ] migrate `packages/atlas-parser/src/lang/c.rs`:
  - [ ] add `packages/atlas-parser/queries/c.scm`
  - [ ] query functions, structs, enums, typedefs, includes, and calls
  - [ ] preserve existing C qualified names and `NodeKind` choices
  - [ ] preserve existing include/import edge behavior
  - [ ] preserve existing same-file call behavior
  - [ ] keep `tests/fixtures/c/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] migrate `packages/atlas-parser/src/lang/cpp.rs`:
  - [ ] add `packages/atlas-parser/queries/cpp.scm`
  - [ ] query functions, methods, classes, structs, namespaces, includes, and calls
  - [ ] preserve existing C++ qualified names and `NodeKind` choices
  - [ ] preserve existing namespace and class parent scope behavior
  - [ ] keep `tests/fixtures/cpp/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] migrate `packages/atlas-parser/src/lang/csharp.rs`:
  - [ ] add `packages/atlas-parser/queries/csharp.scm`
  - [ ] query namespaces, classes, interfaces, methods, fields, using directives, and calls
  - [ ] preserve existing C# qualified names and `NodeKind` choices
  - [ ] preserve existing test detection behavior
  - [ ] keep `tests/fixtures/csharp/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] run tests after this batch:
  - [ ] `cargo test -p atlas-parser lang::c`
  - [ ] `cargo test -p atlas-parser lang::cpp`
  - [ ] `cargo test -p atlas-parser lang::csharp`
  - [ ] `cargo test -p atlas-parser --test parser_golden`

Why:
- C-family parsers share enough syntax shape to validate query conventions for compiled languages
- batch keeps blast radius bounded before dynamic-language migrations

#### Patch SQ3 — Migrate JVM and static OO language parsers

- [ ] migrate `packages/atlas-parser/src/lang/java.rs`:
  - [ ] add `packages/atlas-parser/queries/java.scm`
  - [ ] query packages, imports, classes, interfaces, enums, methods, fields, and calls
  - [ ] preserve existing Java qualified names and `NodeKind` choices
  - [ ] preserve existing parent scope behavior
  - [ ] keep `tests/fixtures/java/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] migrate `packages/atlas-parser/src/lang/scala.rs`:
  - [ ] add `packages/atlas-parser/queries/scala.scm`
  - [ ] query packages, imports, classes, objects, traits, functions, vals, vars, and calls
  - [ ] preserve existing Scala qualified names and `NodeKind` choices
  - [ ] preserve existing object/class/trait scope behavior
  - [ ] keep `tests/fixtures/scala/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] run tests after this batch:
  - [ ] `cargo test -p atlas-parser lang::java`
  - [ ] `cargo test -p atlas-parser lang::scala`
  - [ ] `cargo test -p atlas-parser --test parser_golden`

Why:
- Java and Scala exercise package/import scope semantics after C-family migration
- keeps static OO migration separate from JavaScript/TypeScript complexity

#### Patch SQ4 — Migrate JavaScript and TypeScript parsers

- [ ] migrate shared JavaScript/TypeScript parser code in `packages/atlas-parser/src/lang/javascript.rs`:
  - [ ] add `packages/atlas-parser/queries/javascript.scm`
  - [ ] add `packages/atlas-parser/queries/typescript.scm`
  - [ ] query imports, exports, functions, arrow functions assigned to names, classes, methods, variables, and calls
  - [ ] preserve existing JavaScript qualified names and `NodeKind` choices
  - [ ] preserve existing TypeScript qualified names and `NodeKind` choices
  - [ ] preserve existing JSX/TSX support behavior
  - [ ] preserve existing call/reference confidence tiers
  - [ ] keep `tests/fixtures/javascript/*.golden.json` unchanged unless a semantic fix is explicitly itemized
  - [ ] keep `tests/fixtures/typescript/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] run tests after this batch:
  - [ ] `cargo test -p atlas-parser lang::javascript`
  - [ ] `cargo test -p atlas-parser --test parser_golden`

Why:
- JavaScript and TypeScript share parser code and must migrate together to avoid divergent behavior
- this batch validates query helpers against two grammars behind one language module

#### Patch SQ5 — Migrate dynamic language parsers

- [ ] migrate `packages/atlas-parser/src/lang/python.rs`:
  - [ ] add `packages/atlas-parser/queries/python.scm`
  - [ ] query imports, classes, functions, methods, assignments, and calls
  - [ ] preserve existing Python qualified names and `NodeKind` choices
  - [ ] preserve existing indentation/scope behavior from AST parentage
  - [ ] keep `tests/fixtures/python/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] migrate `packages/atlas-parser/src/lang/ruby.rs`:
  - [ ] add `packages/atlas-parser/queries/ruby.scm`
  - [ ] query requires, modules, classes, instance methods, singleton methods, constants, and calls
  - [ ] preserve existing Ruby qualified names and `NodeKind` choices
  - [ ] preserve existing current-owner behavior
  - [ ] keep `tests/fixtures/ruby/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] migrate `packages/atlas-parser/src/lang/php.rs`:
  - [ ] add `packages/atlas-parser/queries/php.scm`
  - [ ] query namespaces, uses, classes, interfaces, traits, functions, methods, constants, and calls
  - [ ] preserve existing PHP qualified names and `NodeKind` choices
  - [ ] preserve existing PHP language mode setup
  - [ ] keep `tests/fixtures/php/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] migrate `packages/atlas-parser/src/lang/bash.rs`:
  - [ ] add `packages/atlas-parser/queries/bash.scm`
  - [ ] query function definitions, command invocations, variables, and source/import-like commands
  - [ ] preserve existing Bash qualified names and `NodeKind` choices
  - [ ] keep `tests/fixtures/bash/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] run tests after this batch:
  - [ ] `cargo test -p atlas-parser lang::python`
  - [ ] `cargo test -p atlas-parser lang::ruby`
  - [ ] `cargo test -p atlas-parser lang::php`
  - [ ] `cargo test -p atlas-parser lang::bash`
  - [ ] `cargo test -p atlas-parser --test parser_golden`

Why:
- dynamic languages rely heavily on scope heuristics, so they should migrate after query helpers are proven
- batch validates method/function owner handling across multiple dynamic grammar styles

#### Patch SQ6 — Migrate data, markup, and style parsers where queries add value

- [ ] evaluate query migration for `packages/atlas-parser/src/lang/json.rs`:
  - [ ] migrate to `packages/atlas-parser/queries/json.scm` only if it reduces manual traversal without losing object/key path semantics
  - [ ] otherwise document why JSON remains manual
  - [ ] keep `tests/fixtures/json/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] evaluate query migration for `packages/atlas-parser/src/lang/toml.rs`:
  - [ ] migrate to `packages/atlas-parser/queries/toml.scm` only if it reduces manual traversal without losing table/key path semantics
  - [ ] otherwise document why TOML remains manual
  - [ ] keep `tests/fixtures/toml/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] evaluate query migration for `packages/atlas-parser/src/lang/html.rs`:
  - [ ] migrate to `packages/atlas-parser/queries/html.scm` only if query captures improve element/script/style extraction
  - [ ] otherwise document why HTML remains manual
  - [ ] keep `tests/fixtures/html/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] evaluate query migration for `packages/atlas-parser/src/lang/css.rs`:
  - [ ] migrate to `packages/atlas-parser/queries/css.scm` only if query captures improve selector/rule extraction
  - [ ] otherwise document why CSS remains manual
  - [ ] keep `tests/fixtures/css/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] evaluate query migration for `packages/atlas-parser/src/lang/markdown.rs`:
  - [ ] migrate to `packages/atlas-parser/queries/markdown.scm` only if tree-sitter-md query behavior stays stable for malformed shorter inputs
  - [ ] otherwise document why Markdown remains manual
  - [ ] preserve current decision to avoid unstable incremental reuse for Markdown unless separately fixed
  - [ ] keep `tests/fixtures/markdown/*.golden.json` unchanged unless a semantic fix is explicitly itemized
- [ ] run tests after this batch:
  - [ ] `cargo test -p atlas-parser lang::json`
  - [ ] `cargo test -p atlas-parser lang::toml`
  - [ ] `cargo test -p atlas-parser lang::html`
  - [ ] `cargo test -p atlas-parser lang::css`
  - [ ] `cargo test -p atlas-parser lang::markdown`
  - [ ] `cargo test -p atlas-parser --test parser_golden`

Why:
- data/markup/style parsers may not benefit equally from queries
- this batch requires explicit migrate-or-document decisions instead of forced churn

#### Patch SQ completion criteria

- [ ] every non-Rust parser has either an Atlas-owned query file or a documented reason to remain manual
- [ ] all migrated parsers use shared query helpers instead of ad hoc `tree_sitter::QueryCursor` code
- [ ] all migrated parsers keep public parser APIs unchanged
- [ ] golden outputs remain unchanged unless semantic fixes are explicitly itemized in the corresponding patch
- [ ] parser docs describe the query-backed extraction contract and capture naming convention
- [ ] `cargo test -p atlas-parser --test parser_golden` passes after each migration batch
- [ ] `cargo test -p atlas-parser` passes after the final migration batch
- [ ] `./scripts/test-workspace-summary.sh` passes after the final migration batch
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes after the final migration batch
- [ ] `cargo fmt --all` has been run after the final migration batch

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
- [ ] update installed AGENTS instructions to state escalation order clearly
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
- install flow already owns AGENTS/CLAUDE injected guidance and should remain source for static bootstrap text

#### Patch A4 — Platform hook preload integration

- [ ] extend `packages/atlas-cli/src/install/platform_hooks.rs` generated Copilot hook config to preload policy on:
  - [ ] `SessionStart`
  - [ ] `UserPromptSubmit`
- [ ] extend generated Claude hook config to preload policy on:
  - [ ] `SessionStart`
  - [ ] `UserPromptSubmit`
  - [ ] `InstructionsLoaded`
- [ ] extend generated Codex hook config to preload policy on:
  - [ ] `SessionStart`
  - [ ] `UserPromptSubmit`
- [ ] extend shared `.atlas/hooks/atlas-hook` runner so preload path calls shared Rust policy service instead of duplicating JSON assembly in shell
- [ ] cache last successful compact policy payload under `.atlas/hooks/lib/` with version/hash metadata
- [ ] define bounded cache TTL or invalidation rule so long sessions can refresh policy safely
- [ ] add tests for generated hook configs and runner output after install

Why:
- existing install-generated hook path already exists and should carry runtime policy preload
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
- [ ] install-generated Copilot/Claude/Codex hooks preload runtime policy on session/prompt start
- [ ] protected Atlas tools are blocked when required policy preload has not happened
- [ ] fallback mode is explicit, deterministic, and covered by tests
- [ ] prompts, installed instructions, README, and wiki workflow docs agree on runtime-policy-first behavior
- [ ] adapter/session event flow records policy preload and enforcement decisions without direct hook SQLite writes

---

### Graph Store Corruption Recovery Patch

Atlas can detect SQLite integrity failures, orphan nodes, dangling edges, stale graph state, and interrupted builds, but the operational policy for a damaged `.atlas/worldtree.db` is not explicit enough. Detection should lead to one clear outcome: quarantine unusable graph data, rebuild from repository source, and block graph-backed answers while stored graph facts are unsafe.

#### Patch C1 — Graph DB corruption classification

- [ ] define graph-store health classes:
  - [ ] `healthy`
  - [ ] `stale`
  - [ ] `interrupted_build`
  - [ ] `failed_build`
  - [ ] `sqlite_corrupt`
  - [ ] `schema_mismatch`
  - [ ] `logical_inconsistency`
- [ ] classify evidence sources consistently:
  - [ ] `Store::open` errors
  - [ ] `PRAGMA integrity_check`
  - [ ] `PRAGMA foreign_key_check`
  - [ ] orphan-node scan
  - [ ] dangling-edge scan
  - [ ] graph build lifecycle state
  - [ ] freshness check against changed graph-relevant files
- [ ] ensure CLI and MCP use the same classification and `error_code` values
- [ ] add tests for each health class and error-code mapping

Why:
- makes corruption versus stale data explicit
- avoids treating dangling/orphan graph rows as a generic diagnostics warning

#### Patch C2 — Quarantine and rebuild policy for `worldtree.db`

- [ ] define no partial salvage for graph DB corruption unless a future task explicitly adds verified salvage
- [ ] define recovery modes:
  - [ ] `manual_rebuild_required` — diagnostics report command; operator runs rebuild
  - [ ] `auto_quarantine_and_rebuild` — Atlas quarantines DB and rebuilds when command policy allows
  - [ ] `block_only` — graph-backed tools refuse answers but do not mutate DB
- [ ] define default recovery mode per entry point:
  - [ ] `status` / `doctor` / `db_check`: `block_only` diagnostics, no mutation
  - [ ] explicit `build` / `update`: `auto_quarantine_and_rebuild` when corruption is detected
  - [ ] graph-backed query/context/analyze tools: `block_only` with rebuild command
- [ ] require explicit flag for automatic quarantine outside build/update commands
- [ ] quarantine physically corrupt or logically inconsistent `.atlas/worldtree.db` before rebuilding
- [ ] use deterministic quarantine path with timestamp or collision-safe suffix
- [ ] keep quarantined DB for inspection instead of deleting it
- [ ] create fresh `worldtree.db` from migrations after quarantine
- [ ] run full graph rebuild from repository source after quarantine
- [ ] record rebuild result in graph build lifecycle state
- [ ] surface quarantine path, rebuild result, and failure reason in CLI JSON output
- [ ] surface same fields in MCP `build_or_update_graph`, `status`, `doctor`, and `db_check` where relevant
- [ ] add tests:
  - [ ] corrupt SQLite file is quarantined
  - [ ] logical dangling-edge inconsistency triggers rebuild policy
  - [ ] rebuild after quarantine creates usable fresh graph DB
  - [ ] failed rebuild leaves graph unavailable with actionable error

Why:
- graph data is derived from repo source, so clean rebuild is safer than partial salvage
- quarantine preserves evidence without serving unsafe facts

#### Patch C3 — Block unsafe graph-backed answers

- [ ] block graph-backed query/context tools when health class is `sqlite_corrupt`, `schema_mismatch`, or `logical_inconsistency`
- [ ] return machine-readable failure with:
  - [ ] `error_code`
  - [ ] `health_class`
  - [ ] `db_path`
  - [ ] `quarantine_path` when available
  - [ ] recommended rebuild command
- [ ] allow non-graph diagnostics tools to keep working:
  - [ ] `status`
  - [ ] `doctor`
  - [ ] `db_check`
  - [ ] `debug_graph` only when DB can open safely
- [ ] distinguish stale-but-queryable graph state from corrupt-and-blocked graph state
- [ ] document agent behavior: do not answer from graph facts when corrupt/inconsistent
- [ ] add MCP tests that graph-backed tools fail closed on corrupt/inconsistent DB

Why:
- prevents confident answers from known-bad graph rows
- keeps diagnostics available while blocking unsafe context

#### Patch C completion criteria

- [ ] graph DB health classes are explicit and shared by CLI/MCP
- [ ] corrupt graph execution state maps to block + quarantine + rebuild behavior
- [ ] auto rebuild, manual rebuild, and block-only recovery modes are explicit per command/tool
- [ ] corrupt or logically inconsistent `worldtree.db` is quarantined before rebuild
- [ ] rebuild from source is default policy; partial salvage is explicitly out of scope
- [ ] graph-backed tools fail closed when graph facts are corrupt or inconsistent
- [ ] diagnostics expose exact reason, quarantine path, and next command
- [ ] tests cover physical corruption, logical inconsistency, rebuild success, rebuild failure, and fail-closed query behavior

### SQLite Connection Concurrency Policy Patch

Atlas currently uses one `rusqlite::Connection` per store struct. That is safe for the current architecture because `atlas-engine` uses Rayon only for file hashing, reading, and parsing; SQLite persistence happens after parallel work completes. What is still underspecified is the operational contract around thread confinement, separate-connection concurrency, and future pooling. This patch makes the current model explicit, adds regression coverage, and leaves a clean boundary for future separate-connection read pooling without adding one now.


#### Patch T4 — Measured separate-connection read pool

- [ ] add baseline contention harness for graph reads before pool code lands:
  - [ ] run concurrent `atlas status`, `atlas query`, and MCP graph-read workload against one `worldtree.db` with `read_pool_active = false`
  - [ ] record baseline metrics `sqlite_busy_count`, `read_ops_total`, `read_latency_p50_ms`, `read_latency_p95_ms`, and writer success rate
  - [ ] check benchmark fixture and command into repo so pooled and non-pooled runs use same workload
- [ ] define explicit merge gate for pooled reads:
  - [ ] require one stable success metric such as lower `sqlite_busy_count` or lower `read_latency_p95_ms` under same concurrent workload
  - [ ] require no regression in write success rate, WAL health, or `atlas status` readiness output
  - [ ] keep pool default-off until benchmark evidence is committed
- [ ] add config surface for read pool without changing current default:
  - [ ] add `.atlas/config.toml` fields `graph.read_pool.enabled`, `graph.read_pool.size`, `graph.read_pool.read_only`, and `graph.read_pool.checkout_timeout_ms`
  - [ ] validate `graph.read_pool.size >= 1` when enabled
  - [ ] reject pool enablement when `graph.read_pool.read_only = false`
- [ ] keep writer ownership explicit while adding pooled readers:
  - [ ] preserve one write-owning `rusqlite::Connection` per mutable store instance unless broader store split is designed first
  - [ ] add explicit graph-read checkout path separate from write-owner methods
  - [ ] do not route writes, migrations, or transactions through read-pool checkout APIs
  - [ ] document exact read/write boundary in store docs before mixed concurrency lands
- [ ] if pool is implemented, open separate checked-out SQLite connections only:
  - [ ] add shared helper in `atlas-db-utils` for pooled read connection open flags plus `apply_atlas_pragmas`
  - [ ] allow `r2d2_sqlite` or equivalent only for read-only or read-mostly checked-out connections
  - [ ] keep pooled connection wrappers out of `Store`, `ContentStore`, and `SessionStore` types that own write transactions
  - [ ] reject designs that share one `Connection` across threads behind `Arc<Mutex<_>>`, `RwLock<_>`, or similar
- [ ] add pool-specific diagnostics and safety checks:
  - [ ] surface `read_pool_active`, `read_pool_size`, `read_pool_read_only`, and `read_pool_fallback` in `atlas status --json`
  - [ ] surface same fields plus checkout timeout and pool-creation failures in `atlas doctor --json`
  - [ ] verify every checked-out read connection reports canonical WAL mode and busy-timeout settings
- [ ] add tests before enabling by default:
  - [ ] concurrent read test proves two threads hold distinct checked-out read connections at same time
  - [ ] mixed read/write test proves readers never borrow or lock-wrap write-owner connection
  - [ ] disabled-mode test proves current single-connection-per-store behavior stays unchanged when pool config is absent
  - [ ] shutdown/drop test proves pool teardown does not strand transactions or WAL checkpoints

Why:
- gives clear follow-on slot for `r2d2_sqlite`-style pooling without weakening current contract
- keeps future pool design anchored on separate connections, explicit writer ownership, and measured benefit

#### Patch T completion criteria

- [x] one canonical SQLite connection/thread policy exists and all Atlas stores reference it
- [x] engine Rayon parse code is explicitly separated from SQLite access
- [x] tests fail if store types become cross-thread sharable
- [x] docs say current mode is single-connection per store instance with separate-connection concurrency only
- [x] future pool direction is documented as separate-connection only, not shared-connection wrappers
- [ ] any future read pool remains evidence-driven and preserves explicit writer ownership

---

## Additional Backlog

- [x] add canonical `docs/error_codes.md` file and make README, MCP responses, and tests reference that single error-code catalog
- [x] add generated `MCP_TOOLS.md` from tool registry and test/docs check that catches drift from hand-maintained tool tables
- [ ] add build/query/MCP metrics counters and histograms for build duration, parsed file count, parser cache reuse ratio, query latency by mode, and MCP tool call counts
- [x] add informational `cargo-llvm-cov` coverage task and a new github workflow job that reports coverage without gating merge
- [x] add `criterion` bench suites per crate for build, incremental update, query modes, context engine, and history reconstruction workloads
- [x] add CI regression harness for `cargo bench --message-format=json` and store benchmark output as comparable artifact
- [ ] add CI-visible parser cache hit-ratio metric and fail when cache reuse drops below configured threshold
- [ ] add thin LSP shim that maps Atlas query/context/impact/reference flows onto standard LSP requests
- [ ] add documented `budget_policy` block to `.atlas/config.toml` with defaults and `--budget-profile` selection:
  - [ ] document which budget limits are byte-based heuristics versus tokenizer-backed counts
  - [ ] add tokenizer config fields for budget accounting provider, model, and fallback mode
  - [ ] add tokenizer-backed budget accounting for context/review/export paths that already expose `max_tokens`
  - [ ] keep deterministic byte/char fallback when tokenizer is unavailable and surface fallback metadata in JSON output
  - [ ] add tests for tokenizer-backed counts, heuristic fallback, and stable truncation behavior across both modes
- [x] add `proptest` coverage for ranking/trimming, canonical-path normalization, and FTS query escaping

---

## Part VI — MCP 2025-11-25 Spec Upgrade Roadmap

All phases and the MCP Tools Schema Compliance Patch in this part are shipped. See SHIPPED.md for feature details.

---

## MCP Tool Error Payload Normalization Patch

All sub-patches are shipped. See SHIPPED.md for feature details.

---

## MCP Transcript Failure Hardening Patch

All sub-patches are shipped. See SHIPPED.md for feature details.

---

## Part VII — Dynamic MCP Repo Resolution for Multi-Workspace Editors

All phases in this part are shipped. See SHIPPED.md for feature details.

---

## Part VIII — MCP 2026-07-28 Spec Migration Roadmap

All phases in this part are shipped. See SHIPPED.md for feature details.

---

## MCP Mixed Result Contract Normalization Patch

All sub-patches are shipped. See SHIPPED.md for feature details.

---

## Part IX — MCP Tool Agent-Ergonomics Simplification Roadmap

All phases in this part are shipped. See SHIPPED.md for feature details.

---

## Part X — MCP `rmcp` Official SDK Conversion Roadmap

Goal: replace Atlas handrolled MCP protocol, descriptor, transport, task, and result plumbing with the official `rmcp` Rust SDK while preserving Atlas tool behavior, repo identity invariants, structured JSON contracts, provenance, freshness, budgets, and tests.

Overview: first remove Atlas-specific TOON output because `rmcp` should expose official typed JSON results with `structuredContent` as source of truth. Then add an `rmcp` server adapter around existing Atlas tool business logic, migrate stdio/HTTP/socket transports to official SDK transports, convert tasks/MRTR/elicitation to official SDK types, remove handrolled JSON-RPC code, and finish with typed tool schemas.

Rules:
- Implement smallest safe phase slices; keep Atlas tool business logic stable until transport parity tests pass.
- Do not keep TOON compatibility shims; MCP output becomes JSON-only.
- Use `rmcp::model::*` types at protocol boundaries; keep `serde_json::Value` only inside Atlas tool payload assembly until typed schema migration.
- Keep `structuredContent` authoritative; `content` may contain concise text only.
- Preserve `atlas_provenance`, `atlas_freshness`, budget metadata, truncation metadata, and user-visible tool errors.
- Preserve current public crate API names until downstream CLI tests are migrated.
- Validate each phase with targeted tests before deleting old code.

### Phase X1 — Remove TOON MCP Output Mode

Goal: remove Atlas-specific TOON rendering and make MCP responses JSON-only before introducing `rmcp`.

Overview: delete `OutputFormat::Toon`, remove `output_format` arguments from MCP schemas and tests, remove `toon-format`, and make budget enforcement measure JSON payload bytes only.

Rules:
- `structuredContent` is canonical output for all MCP tools.
- `content` text must be JSON-compatible summary or compact human-readable text, never TOON.
- Tool callers must not pass `output_format`; JSON is implicit.
- No `ATLAS_MCP_OUTPUT_FORMAT` environment fallback remains.

- [x] remove TOON dependency and renderer code:
  - [x] remove `toon-format` from `packages/atlas-mcp/Cargo.toml`
  - [x] delete TOON-specific imports from `packages/atlas-mcp/src/output.rs`
  - [x] remove `OutputFormat::Toon` and keep only JSON behavior or replace `OutputFormat` with JSON-only helper functions
  - [x] remove `ATLAS_MCP_OUTPUT_FORMAT` parsing from `packages/atlas-mcp/src/output.rs`
  - [x] remove TOON fallback metadata generation for `atlas:fallbackReason`
  - [x] remove `text/x-toon` MIME type handling from MCP result construction
- [x] update MCP result construction for JSON-only output:
  - [x] change `packages/atlas-mcp/src/tool_result.rs` so `ToolResultBuilder` renders JSON-only text content
  - [x] remove `atlas:outputFormat` from result `_meta`
  - [x] remove `atlas:requestedOutputFormat` from result `_meta`
  - [x] remove `atlas:fallbackReason` from result `_meta`
  - [x] keep `structuredContent` populated for object payloads
  - [x] keep resource-link inference unchanged when source JSON includes known resource IDs or URIs
- [x] remove `output_format` from MCP tool inputs:
  - [x] remove `output_format` properties from all tool input schemas in `packages/atlas-mcp/src/tools/**`
  - [x] remove `output_format` parsing from `packages/atlas-mcp/src/tools/dispatch.rs`
  - [x] remove `output_format` parsing from `packages/atlas-mcp/src/discovery/**`
  - [x] remove `output_format` parsing from `packages/atlas-mcp/src/session_tools.rs`
  - [x] remove `output_format` parsing from `packages/atlas-mcp/src/session_events.rs`
  - [x] remove `output_format` parsing from `packages/atlas-mcp/src/tasks.rs`
  - [x] update helper signatures that currently accept `OutputFormat` to use JSON-only result builders
- [x] update completions and prompts for JSON-only output:
  - [x] remove `output_format` completion branch from `packages/atlas-mcp/src/completion.rs`
  - [x] update completion tests that expect `json` or `toon` suggestions
  - [x] remove prompt instructions that recommend `output_format = "toon"`
  - [x] update MCP prompt tests to expect JSON-only workflow text
- [x] update budget enforcement to measure JSON only:
  - [x] change `packages/atlas-mcp/src/context.rs` byte measurement to use `serde_json::to_vec` or compact JSON string length
  - [x] remove output-format-dependent budget calculations
  - [x] keep existing truncation fields and budget reports stable except output-format fields
  - [x] add regression test proving budget trimming is deterministic with JSON-only measurement
- [x] remove TOON docs, fixtures, and installed instructions:
  - [x] remove `docs/contracts/atlas_toon.v1.md` references from source and docs indexes
  - [x] delete `packages/atlas-mcp/testdata/atlas_toon.v1/**`
  - [x] update `packages/atlas-cli/src/install/instructions.rs` to say `Use default JSON output. Trust structuredContent as source of truth.`
  - [x] update `packages/atlas-cli/src/install/tests.rs` assertion for installed MCP instructions
- [x] update JSON-RPC and CLI quality-gate fixtures:
  - [x] remove `"output_format":"json"` from MCP `tools/call` fixtures under `packages/atlas-cli/tests/cli_quality_gates/**`
  - [x] remove JSON output-format arguments from `packages/atlas-mcp/src/**/tests/**`
  - [x] assert representative tool calls still include object `structuredContent`
  - [x] assert representative tool calls no longer include `atlas:outputFormat`
  - [x] assert representative tool calls no longer include `atlas:requestedOutputFormat`
- [x] validate Phase X1:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet`
  - [x] run `cargo test --quiet -p atlas-mcp output::tests`
  - [x] run targeted MCP contract tests with `cargo test --quiet -p atlas-cli stdio_transport_representative_stable_tools_keep_object_structured_content`
  - [x] run `./scripts/test-workspace-summary.sh`

Phase X1 completion criteria:
- [x] `toon-format` no longer appears in any `Cargo.toml`
- [x] `OutputFormat::Toon`, `ATLAS_MCP_OUTPUT_FORMAT`, `text/x-toon`, and `output_format` MCP inputs are removed
- [x] all MCP tool responses use JSON `structuredContent` as authoritative data
- [x] installed instructions no longer mention TOON
- [x] targeted MCP contract tests and workspace summary pass

### Phase X2 — Add `rmcp` Dependency and Server Adapter Shell

Goal: introduce official SDK dependency and compile an `rmcp` `ServerHandler` wrapper without changing default runtime behavior yet.

Overview: add `rmcp` with required features, create `AtlasRmcpServer`, and map server info/discovery/list methods to existing Atlas registries through official model types.

Rules:
- Keep handrolled transport as default until adapter parity tests pass.
- Adapter must not duplicate Atlas tool business logic.
- Adapter must own repo root and DB path explicitly.
- Adapter must expose only JSON-only contracts from Phase X1.

- [x] add SDK dependencies:
  - [x] add `rmcp = "3.1.0"` to workspace or `packages/atlas-mcp/Cargo.toml` with server, macros, schemars, transport-io, transport-async-rw, transport-streamable-http-server, and elicitation features
  - [x] add `schemars = "1"` where typed schema generation will be implemented
  - [x] align optional HTTP auth dependencies so `cargo tree -p atlas-mcp` has one intended auth stack
- [x] create adapter module structure:
  - [x] add `packages/atlas-mcp/src/rmcp_server.rs`
  - [x] add `packages/atlas-mcp/src/rmcp_types.rs`
  - [x] add `packages/atlas-mcp/src/rmcp_error.rs`
  - [x] export adapter modules behind internal crate visibility from `packages/atlas-mcp/src/lib.rs`
- [x] implement `AtlasRmcpServer` state:
  - [x] store canonical `repo_root: String`
  - [x] store `db_path: String`
  - [x] store `ServerOptions`
  - [x] add constructor `AtlasRmcpServer::new(repo_root, db_path, options)`
  - [x] add tests proving constructor preserves repo and DB paths exactly as passed by existing launcher
- [x] implement `rmcp::handler::server::ServerHandler` shell:
  - [x] implement `get_info` from current package name, version, and description
  - [x] implement `supported_protocol_versions` with the currently supported MCP version only
  - [x] implement `discover` using official `rmcp::model::DiscoverResult`
  - [x] implement `list_tools` by converting current `tools::tool_descriptors()` into `rmcp::model::Tool`
  - [x] implement `list_prompts` by converting current prompt descriptors into `rmcp::model::Prompt`
  - [x] implement `list_resources` by converting current resource descriptors into `rmcp::model::Resource`
  - [x] implement `list_resource_templates` by converting current resource template descriptors into `rmcp::model::ResourceTemplate`
- [x] add adapter parity tests:
  - [x] assert `AtlasRmcpServer::get_info` matches current `spec::server_info`
  - [x] assert `list_tools` names equal current `tools::tool_list()["tools"]` names
  - [x] assert `list_prompts` names equal current `prompts::prompt_list()["prompts"]` names
  - [x] assert `list_resources` URIs equal current `resources::resources_list` URIs
  - [x] assert `list_resource_templates` URI templates equal current `resources::resources_templates_list` URI templates
- [x] validate Phase X2:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet`
  - [x] run `cargo test --quiet -p atlas-mcp rmcp_server`

Phase X2 completion criteria:
- [x] `atlas-mcp` compiles with `rmcp`
- [x] `AtlasRmcpServer` exposes info, discovery, tools, prompts, resources, and templates through official model types
- [x] default handrolled transport still works unchanged
- [x] adapter parity tests pass

### Phase X3 — Convert Tool Calls to `rmcp` Results

Goal: route `tools/call` through `rmcp::model::CallToolRequestParams` and return official tool result types while reusing existing Atlas tool handlers.

Overview: implement `call_tool`, error mapping, metadata preservation, and task deferral behavior through SDK result types.

Rules:
- User-fixable tool failures become successful MCP tool responses with error content, not JSON-RPC protocol errors.
- Unknown tool remains protocol method/tool error.
- `structuredContent` stays object-shaped for normalized Atlas contracts.
- Existing `McpAdapter` session hooks must still run around tool execution.

- [x] implement request conversion:
  - [x] parse `CallToolRequestParams.name` into existing Atlas tool name
  - [x] pass `CallToolRequestParams.arguments` into existing `tasks::execute_tool_call`
  - [x] strip repo selector fields using existing repo-selection helper before business logic runs
  - [x] preserve request context fields needed for progress, cancellation, and session tracking
- [x] implement result conversion:
  - [x] convert existing JSON tool result envelope into `rmcp::model::CallToolResult` or equivalent `CallToolResponse`
  - [x] map existing `content` array to official content blocks
  - [x] map existing `structuredContent` to official structured content field
  - [x] map existing `_meta` to official result metadata extension map
  - [x] preserve resource links in official content block form when supported by rmcp
- [x] implement error conversion:
  - [x] convert unknown tool to rmcp method-not-found or invalid-params error as appropriate
  - [x] convert invalid `arguments` shape to rmcp invalid-params error
  - [x] convert Atlas `ToolErrorPayload` to user-visible tool error result
  - [x] convert unexpected internal failures with `atlas_error_code` metadata retained where official SDK permits metadata
  - [x] add tests that user-visible validation failures appear in tool content
- [x] preserve tool session side effects:
  - [x] ensure `McpAdapter::before_command` runs before delegated tool execution
  - [x] ensure `McpAdapter::after_command` records success/failure
  - [x] ensure session event best-effort emission still runs after successful calls
  - [x] add regression test around `get_session_status` event count after one rmcp adapter tool call
- [x] add representative tool parity tests:
  - [x] compare `query_graph` structured content between handrolled dispatch and rmcp adapter
  - [x] compare `status` structured content between handrolled dispatch and rmcp adapter
  - [x] compare `get_context` structured content between handrolled dispatch and rmcp adapter
  - [x] compare `search_files` structured content between handrolled dispatch and rmcp adapter
  - [x] compare user-visible error shape for invalid input between handrolled dispatch and rmcp adapter
- [x] validate Phase X3:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet`
  - [x] run `cargo test --quiet -p atlas-mcp rmcp_server::tests::call_tool`

Phase X3 completion criteria:
- [x] `AtlasRmcpServer::call_tool` uses official `rmcp` request and result types
- [x] representative tools produce matching structured content through handrolled and rmcp paths
- [x] user-visible tool errors render as tool results
- [x] session hooks remain active for rmcp tool calls

Audit note:
- second-pass Phase X3 audit found duplicate outer rmcp hook/session wrapper around delegated tool execution
- fixed by installing rmcp runtime/tool-call context, then delegating directly to `crate::tasks::execute_tool_call(...)`
- regression coverage now compares rmcp vs handrolled session-event delta after one tool call

### Phase X4 — Convert Prompts, Resources, Completions, and Subscriptions

Goal: move non-tool MCP surfaces to official SDK handler methods.

Overview: implement prompt get/list, resource list/read/templates, completions, and subscription/listen hooks with official model types while preserving Atlas content generation.

Rules:
- Prompt and resource payload text must remain byte-for-byte stable where current tests assert text.
- Resource URIs remain stable.
- Cache metadata remains in `_meta` or official cache fields when available.
- Completion behavior remains deterministic and sorted.
- Official `rmcp::model::PaginatedRequestParams` is cursor-only; preserve Atlas pagination through `cursor` semantics and keep default page sizing on rmcp path.

- [x] implement prompt handlers:
  - [x] convert `prompts::prompt_list` output into `rmcp::model::ListPromptsResult`
  - [x] convert `prompts::prompt_get` output into `rmcp::model::GetPromptResult`
  - [x] preserve prompt descriptions, arguments, titles, icons, and metadata
  - [x] add tests for all prompt names and required arguments
- [x] implement resource handlers:
  - [x] convert `resources::resources_list` into `rmcp::model::ListResourcesResult`
  - [x] convert `resources::resources_templates_list` into `rmcp::model::ListResourceTemplatesResult`
  - [x] convert `resources::resources_read` into `rmcp::model::ReadResourceResult`
  - [x] preserve MIME types and text content
  - [x] preserve resource pagination behavior
  - [x] add tests for docs index, health status, graph provenance, saved context, tool docs, prompt docs, and docs-section resources
- [x] implement completion handler:
  - [x] convert current `completion::complete` result into `rmcp::model::CompleteResult`
  - [x] remove any leftover output-format completion path
  - [x] preserve resource URI, prompt argument, tool name, intent, source ID, docs heading, and git ref completions
  - [x] add tests for each completion source
- [x] implement subscription/listen support:
  - [x] map accepted subscription filters to current tools/prompts/resources list-changed categories
  - [x] map resource updated notifications to official rmcp notification type
  - [x] add tests that unsupported subscription filters are rejected or reduced deterministically
- [x] validate Phase X4:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet`
  - [x] run `cargo test --quiet -p atlas-mcp prompts resources completion`

Phase X4 completion criteria:
- [x] prompts, resources, resource templates, reads, completions, and subscriptions are implemented on `AtlasRmcpServer`
- [x] resource URIs and prompt names remain stable
- [x] leftover output-format completion behavior is gone
- [x] targeted non-tool MCP tests pass

### Phase X4.5 — Convert Lifecycle, Logging, Roots, and Capability Context

Goal: preserve MCP lifecycle behavior and client interaction context before replacing transports.

Overview: implement official rmcp handlers for ping, initialized notifications, logging level changes, trace/log messages, client capability capture, and roots-list-changed dynamic repo refresh.

Rules:
- Lifecycle methods must be covered before stdio/HTTP transport replacement.
- Client capability data must flow into Atlas runtime context before tool execution.
- Dynamic repo/root behavior must stay canonical-path-safe.
- Logging and tracing are diagnostics only; user-facing command output remains tool result content.

- [x] implement protocol lifecycle handlers:
  - [x] implement `ServerHandler::ping` with an empty success result
  - [x] implement `ServerHandler::on_initialized` and preserve current initialized-session side effects
  - [x] add tests that initialized notification produces no response and marks session ready where existing tests observe readiness
  - [x] add tests that ping returns a successful empty response through the rmcp adapter
- [x] implement logging and trace controls:
  - [x] implement `ServerHandler::set_level` using current `logging::LogLevel` threshold behavior
  - [x] map rmcp logging levels to Atlas `LogLevel` values deterministically
  - [x] preserve stderr diagnostic fallback for clients without logging capability
  - [x] replace custom `$/logMessage` emission with official rmcp logging notification APIs
  - [x] add tests for threshold filtering and emitted log notification shape
  - [x] note typed `rmcp::model::LoggingLevel` makes invalid logging-level parsing unreachable on rmcp handler path
- [x] preserve trace notification behavior:
  - [x] map current `$/setTrace` support to rmcp custom notification handling if rmcp has no typed trace method
  - [x] keep `off`, `messages`, and `verbose` values accepted exactly as current parser accepts them
  - [x] add tests for invalid trace level rejection
  - [x] note outbound trace lifecycle emission tests are transport-owned and deferred to Phase X5
- [x] capture client capabilities and request metadata:
  - [x] map rmcp initialize client capabilities into `runtime_context::ClientInteractionCapabilities`
  - [x] preserve detection of elicitation form support
  - [x] preserve detection of elicitation URL support
  - [x] map authenticated principal from HTTP auth wrapper into rmcp request context metadata
  - [x] add tests that tool execution sees expected client capability flags
- [x] preserve roots and dynamic repo refresh behavior:
  - [x] implement `ServerHandler::on_roots_list_changed` to invalidate cached dynamic roots where current transport state does so
  - [x] request or read client roots through rmcp peer APIs where current dynamic repo resolution depends on client roots
  - [x] canonicalize all root paths through existing `atlas_repo` helper APIs
  - [x] add tests for roots-list-changed invalidating cached candidate roots
  - [x] add tests for noncanonical root inputs resolving to canonical repo roots
- transport-owned protocol parity deferred to Phase X5:
  - missing required params through full rmcp stdio request handling
  - unknown methods through full rmcp stdio request handling
  - unsupported protocol versions during rmcp initialize/discover negotiation
  - malformed JSON-RPC through rmcp transport parser
- [x] validate Phase X4.5:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet`
  - [x] run `cargo test --quiet -p atlas-mcp rmcp_server lifecycle logging repo_selection`

Phase X4.5 completion criteria:
- [x] ping, initialized, logging level, trace configuration, and roots-list-changed behavior are implemented on rmcp-backed handlers
- [x] client capability and authenticated-principal metadata reach Atlas runtime context
- [x] dynamic roots remain canonical-path-safe and covered by tests
- [x] transport-owned negative protocol parity items are explicitly deferred to Phase X5

Audit note:
- handler-layer X4.5 work is complete
- remaining unticked items were transport-owned, not handler-owned
- moved those checks to Phase X5 so X4.5 status reflects actual scope

### Phase X5 — Replace Stdio Transport with `rmcp` Stdio

Goal: make default stdio server run through official `rmcp` transport.

Overview: preserve `run_server` and `run_server_with_options` public functions while replacing newline JSON-RPC parsing and dispatch with rmcp stdio service.

Rules:
- Public launcher function names remain stable.
- Existing CLI commands that launch MCP stdio must not change flags.
- Tool timeout and cancellation semantics must remain covered by tests.
- Legacy compatibility code is removed after parity tests pass.

- [x] add rmcp stdio runner:
  - [x] update `packages/atlas-mcp/src/transport/stdio.rs` to construct `AtlasRmcpServer`
  - [x] start rmcp stdio transport from existing `run_server_with_options`
  - [x] preserve `mark_server_started()` behavior
  - [x] preserve stderr startup diagnostics expected by tests
- [x] bridge blocking Atlas tools into async handler execution:
  - [x] run synchronous tool calls with `tokio::task::spawn_blocking` or equivalent rmcp-safe worker execution
  - [x] apply `ServerOptions.tool_timeout_ms` to rmcp tool calls
  - [x] apply `ServerOptions.tool_timeout_ms_by_tool` overrides
  - [x] return timeout as user-visible tool error with existing timeout error code
- [x] bridge cancellation and progress:
  - [x] map rmcp cancellation notification into existing cancel flags
  - [x] map existing `progress::report` calls into official rmcp progress notifications
  - [x] add test for cancellation of a long-running test tool
  - [x] add test for progress notification emission from a long-running test tool
- [x] migrate stdio tests:
  - [x] update interactive stdio harness to drive rmcp stdio transport
  - [x] preserve initialized-session helper behavior
  - [x] assert representative requests return JSON-RPC 2.0 compliant responses through rmcp
  - [x] assert notifications do not produce responses
  - [x] assert malformed `tools/call` request shapes fail with official rmcp request-validation errors at transport boundary
  - [x] assert unknown methods produce method-not-found errors
  - [x] assert unsupported protocol versions fail during rmcp initialize/discover negotiation
  - [x] assert malformed JSON-RPC is handled by rmcp transport
  - [x] assert verbose trace emits request lifecycle diagnostics and off emits none
- [x] remove obsolete stdio internals:
  - [x] stop compiling manual stdin line parser as stdio transport code by moving it into socket-only transport modules
  - [x] stop compiling manual JSON-RPC response builders as stdio transport code by moving them into socket-only transport modules
  - [x] stop compiling manual method dispatch as stdio transport code by moving it into socket-only transport modules
- [x] validate Phase X5:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet`
  - [x] run stdio-focused MCP tests with `cargo test --quiet -p atlas-mcp transport::tests`
  - [x] run CLI MCP quality gates with `cargo test --quiet -p atlas-cli cli_quality_gates`

Phase X5 completion criteria:
- [x] default stdio MCP server uses rmcp transport
- [x] public launch functions and CLI flags remain stable
- [x] progress, cancellation, timeout, and notification tests pass
- [x] manual stdio JSON-RPC parser is no longer active

Audit note:
- rmcp stdio progress and cancellation bridge is covered and passing
- rmcp stdio unknown-method parity is covered and passing
- request-shape failures for malformed `tools/call` params are now asserted against official rmcp transport-boundary behavior instead of handrolled `invalid_params` assumptions
- unsupported initialize version and malformed-first-frame coverage now use raw rmcp stdio capture because server setup can fail before normal response collection completes
- verbose trace now emits official `notifications/message` lifecycle diagnostics on rmcp stdio, while `off` stays silent
- dead direct stdio wrapper entrypoints were removed and remaining manual parser/response/dispatch code was renamed into socket-only transport modules

### Phase X6 — Replace Streamable HTTP Transport with `rmcp` HTTP

Goal: replace custom HTTP MCP parser/SSE response code with official rmcp streamable HTTP server transport.

Overview: keep Atlas route surface stable while delegating MCP protocol handling to rmcp and preserving protected-resource metadata/auth behavior.

Rules:
- `POST /mcp` remains MCP ingress.
- `GET /health` remains unauthenticated liveness endpoint.
- `GET /.well-known/oauth-protected-resource` remains available when HTTP auth is configured.
- Gateway-owned security controls remain out of this crate.

- [x] implement rmcp HTTP service wiring:
  - [x] construct `AtlasRmcpServer` inside `run_http_server_with_options`
  - [x] mount rmcp streamable HTTP service at `/mcp`
  - [x] preserve `ATLAS_HTTP_BIND` behavior
  - [x] preserve `mark_server_started()` behavior
  - [x] preserve health route response contract
- [x] preserve auth behavior:
  - [x] wrap rmcp HTTP service with existing `ProtectedResourceAuthPolicy` middleware or equivalent rmcp auth layer
  - [x] keep bearer token validation tests
  - [x] keep allowed-origin enforcement tests
  - [x] keep protected-resource metadata tests
  - [x] ensure unauthenticated `/health` still passes
- [x] migrate HTTP test harness:
  - [x] update `HttpTestHarness::post_jsonrpc` to target rmcp HTTP service
  - [x] preserve helpers for bearer tokens and metadata reads
  - [x] update tests to official streamable HTTP response shapes where rmcp differs
  - [x] assert JSON and SSE modes when rmcp exposes both modes
- [x] remove custom HTTP parser code:
  - [x] remove manual `MCP-Protocol-Version` header validation when rmcp validates protocol version
  - [x] remove manual one-shot SSE encoder when rmcp owns it
  - [x] remove manual JSON-RPC body parsing from `transport_http.rs`
  - [x] keep only health/auth/router glue code
- [x] validate Phase X6:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet --features http-transport`
  - [x] run `cargo test --quiet -p atlas-mcp --features http-transport transport_http`

Phase X6 completion criteria:
- [x] HTTP MCP protocol handling is delegated to rmcp streamable HTTP transport
- [x] `/mcp`, `/health`, and protected-resource metadata endpoints remain covered by tests
- [x] HTTP auth tests pass
- [x] custom HTTP JSON-RPC/SSE parsing code is removed or unreachable

Audit note:
- official rmcp HTTP responses do not guarantee the old `MCP-Protocol-Version` response header on successful JSON replies, so HTTP fixtures/assertions were updated to check stable body fields instead
- HTTP test harness now normalizes test-only initialize payloads and request headers to official rmcp transport expectations while keeping runtime server behavior strict
- SSE fallback coverage now uses progress-emitting tool execution because ordinary request/response methods stay JSON when rmcp can complete them without intermediate messages

### Phase X7 — Convert Socket, Pipe, Broker, and Dynamic Repo Context

Goal: preserve Atlas daemon/socket workflows while moving post-handshake MCP bytes through rmcp async read/write transport.

Overview: keep Atlas-specific repo resolution and broker handshake, then pass the established stream into rmcp transport instead of custom JSON-RPC loop.

Rules:
- Canonical repo path identity invariant applies to every repo root selected by dynamic repo resolution.
- Broker handshake remains Atlas-specific only before MCP transport starts.
- Unix socket and Windows named-pipe behavior remain separately tested.
- Dynamic repo selection must not create local path-normalization helpers.

- [x] adapt Unix socket transport:
  - [x] keep existing daemon handshake request/response schema until a separate issue removes it
  - [x] after successful handshake, wrap Unix stream with rmcp async read/write transport
  - [x] construct `AtlasRmcpServer` from handshake repo root and DB path
  - [x] add test proving socket `tools/call` reaches rmcp handler after handshake
- [x] adapt Windows named pipe transport:
  - [x] keep existing named-pipe creation and permission behavior
  - [x] wrap connected pipe stream with rmcp async read/write transport
  - [x] construct `AtlasRmcpServer` from handshake repo root and DB path
  - [x] add cfg-gated compile test for Windows pipe rmcp wiring
- [x] preserve broker status behavior:
  - [x] keep `broker_status` MCP tool response schema stable
  - [x] update broker test harness to use rmcp-backed socket server
  - [x] assert broker liveness and version fields still populate
- [x] preserve dynamic repo selection:
  - [x] port dynamic roots and repo selector stripping into rmcp request context setup
  - [x] ensure selected repo root uses `atlas_repo::CanonicalRepoPath` or existing helper APIs
  - [x] add test for tool call with repo selector switching active repo
  - [x] add test for multi-workspace ambiguous repo selection error
- [x] validate Phase X7:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet`
  - [x] run socket/broker focused tests with `cargo test --quiet -p atlas-mcp broker && cargo test --quiet -p atlas-mcp socket && cargo test --quiet -p atlas-mcp repo_selection`

Phase X7 completion criteria:
- [x] Unix socket and broker MCP flows use rmcp after Atlas handshake
- [x] Windows pipe rmcp wiring compiles behind cfg gates
- [x] dynamic repo selection remains covered and canonical-path-safe
- [x] broker status tool remains stable

Audit note:
- roadmap validation command used invalid `cargo test` multi-filter syntax; validation was run with equivalent chained quiet test commands instead
- dynamic roots now auto-activate single advertised client root and fail closed with `atlas_repo_selection.candidate_roots` when multiple workspace roots remain ambiguous

### Phase X8 — Convert Tasks, MRTR, Elicitation, Progress, and Request State

Goal: replace Atlas handrolled MCP task/input-required shapes with official rmcp task, elicitation, progress, and request-state types.

Overview: keep durable task persistence in `atlas-session`, but use rmcp model types for all MCP-visible task and input-required payloads.

Rules:
- Durable task records stay in Atlas session storage.
- MCP-visible task status and input-required schemas use official rmcp types.
- Destructive operations that require confirmation continue to fail closed without accepted input.
- Request-state validation remains signed and bound to method, tool, args, and principal.

- [x] convert task API models:
  - [x] replace local create-task result JSON shape with `rmcp::model::CreateTaskResult`
  - [x] replace local detailed-task output with `rmcp::model::DetailedTask`
  - [x] map `atlas_session::DurableTaskStatus` to `rmcp::model::TaskStatusCanonical`
  - [x] map stored task result/error/progress into official task payload fields
  - [x] add round-trip tests from durable task records to rmcp task results
- [x] implement rmcp task handler methods:
  - [x] implement `ServerHandler::get_task` using existing `tasks_get` storage logic
  - [x] implement `ServerHandler::update_task` using existing `tasks_update` response ingestion logic
  - [x] implement `ServerHandler::cancel_task` using existing cancellation logic
  - [x] remove custom `tasks/get` and `tasks/update` branches from manual dispatch after rmcp transport owns them
- [x] convert MRTR/input-required:
  - [x] replace local `mrtr::InputRequiredResult` with `rmcp::model::InputRequiredResult`
  - [x] replace local `mrtr::InputRequest` with official rmcp input request type
  - [x] replace local `InputResponses` parsing with official rmcp input response structures where available
  - [x] preserve `resultType: "input_required"` behavior through official SDK type
  - [x] add test for purge confirmation first call returning input-required
  - [x] add test for accepted confirmation completing purge
  - [x] add test for declined confirmation canceling purge
- [x] convert request-state signing:
  - [x] replace custom request-state issue/validate helpers with `rmcp::model::RequestStateCodec` if it supports equivalent HMAC binding
  - [x] if a thin Atlas binding wrapper remains, back it with rmcp `RequestStateCodec` instead of local JSON signature code
  - [x] bind request state to method, tool name, arguments digest, authenticated principal, issue time, expiry, and nonce
  - [x] add tests for tampered state, expired state, mismatched arguments, mismatched principal, and mismatched tool
- [x] convert elicitation types:
  - [x] replace local `ElicitationAction` with rmcp elicitation action enum
  - [x] replace local form schema output with rmcp elicitation schema builder where practical
  - [x] preserve confirmation schema field name `confirmation`
  - [x] add tests for invalid response content, unknown fields, and default cancel behavior
- [x] validate Phase X8:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet`
  - [x] run `cargo test --quiet -p atlas-mcp tasks mrtr elicitation progress`

Phase X8 completion criteria:
- [x] MCP-visible task, input-required, elicitation, progress, and request-state payloads use rmcp model types
- [x] durable Atlas task storage remains unchanged except typed conversion boundaries
- [x] destructive confirmation tests pass
- [x] request-state security regression tests pass

Audit note:
- first X8 slice landed official rmcp `CreateTaskResult`, `DetailedTask`, `tasks/get`, `tasks/update`, and `tasks/cancel` wiring on current durable-task storage
- rmcp `tasks/update` now accepts official `inputResponses` and bridges them into existing Atlas task update ingestion
- cooperative durable-task cancellation now sets persisted `cancel_requested`, marks task `cancelled`, and flips live worker cancel flags when present
- official rmcp task-status notifications now serialize for durable states that can be represented from current storage
- durable task storage now persists `input_requests_json` and `request_state` via session migration `008_durable_task_input_requests`
- deferred rmcp tool calls that return `input_required` now persist official task `inputRequests` payloads and can round-trip through rmcp `tasks/get`
- explicit task TTL parsing now accepts rmcp `tools/call.arguments.task.ttl`, not only legacy top-level `task.ttl`
- MRTR/request-state/elicitation code now uses official rmcp `InputRequiredResult`, `InputRequest`, `InputResponses`, `ElicitationAction`, and `RequestStateCodec`-backed sealing instead of local MCP-visible structs
- request-state integrity now binds method, tool, arguments digest, and principal through codec associated data, while Atlas keeps explicit issue/expiry/nonce payload checks for deterministic tests
- purge confirmation retry coverage now includes first-round `input_required`, accepted retry success, and declined retry user-visible cancellation
- remaining blocker: official task payloads still have no direct field for Atlas progress snapshots, and confirmation schema construction still uses validated JSON value input instead of rmcp schema builder helpers

### Phase X9 — Replace Descriptor and Schema Plumbing with Official Types

Goal: eliminate Atlas MCP descriptor structs and move tool, prompt, resource, and schema exports to official rmcp model types.

Overview: convert descriptor registries first using existing JSON schemas, then incrementally type tool arguments with `schemars`.

Rules:
- Tool names, prompt names, resource URIs, and resource template URI templates remain stable.
- JSON Schema remains draft-compatible with current tests until schemars migration updates snapshots.
- Avoid macro big-bang; use typed structs in batches.
- Public APIs stay minimal and `pub(crate)` unless external crate use requires otherwise.

- [x] replace descriptor structs:
  - [x] replace `ToolDescriptor` with `rmcp::model::Tool`
  - [x] replace `ToolAnnotations` with `rmcp::model::ToolAnnotations`
  - [x] replace `PromptDescriptor` with `rmcp::model::Prompt`
  - [x] replace `PromptArgumentDescriptor` with `rmcp::model::PromptArgument`
  - [x] replace `ResourceDescriptor` with `rmcp::model::Resource`
  - [x] replace `ResourceTemplateDescriptor` with `rmcp::model::ResourceTemplate`
  - [x] replace `IconDescriptor` with `rmcp::model::Icon`
- [x] preserve descriptor validation:
  - [x] keep descriptor name regex validation if rmcp does not enforce Atlas naming rules
  - [x] keep local `$ref` validation for any raw JSON schema that remains
  - [x] keep tests for descriptor sorting and uniqueness
  - [x] keep tests that crate docs list all current MCP tools and prompts until generated docs replace them
- [x] introduce typed argument schemas in batches:
  - [x] add typed args and `schemars::JsonSchema` for health tools: `status`, `doctor`, `db_check`, `debug_graph`, `broker_status`
  - [x] add typed args and schemas for discovery tools: `search_files`, `search_content`, `read_file_excerpt`, `get_docs_section`, `read_file_around_match`, `search_templates`, `search_text_assets`
  - [x] add typed args and schemas for graph tools: `query_graph`, `batch_query_graph`, `resolve_symbol`, `symbol_neighbors`, `traverse_graph`, `cross_file_links`, `concept_clusters`, `explain_query`
  - [x] add typed args and schemas for context/review tools: `detect_changes`, `get_context`, `get_review_context`, `get_minimal_context`, `get_impact_radius`, `explain_change`, `build_or_update_graph`, `postprocess_graph`
  - [x] add typed args and schemas for analysis/refactor tools: `analyze_*`, `find_*`, `infer_modules`, `label_components`
  - [x] add typed args and schemas for session/memory tools: `get_session_status`, `compact_session`, `resume_session`, `search_saved_context`, `search_decisions`, `read_saved_context`, `save_context_artifact`, `purge_saved_context`, `cross_session_search`, `get_global_memory`, `memory_store`, `memory_recall`, `record_session_event`, `wake_up`
- [x] add schema parity tests:
  - [x] assert required fields remain required for migrated typed schemas
  - [x] assert enum values remain stable for migrated typed schemas
  - [x] assert descriptions remain non-empty for all tools
  - [x] assert output schemas remain present where current contract requires them
- [x] validate Phase X9:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet`
  - [x] run `cargo test --quiet -p atlas-mcp descriptors`
  - [x] run `cargo test --quiet -p atlas-mcp tools::tests`

Phase X9 completion criteria:
- [x] custom descriptor structs are removed or reduced to Atlas-only validation helpers
- [x] official rmcp model types define MCP descriptors
- [x] migrated typed schemas pass parity tests
- [x] all tools still appear in generated registry and docs checks

### Phase X10 — Remove Handrolled Protocol, Transport, and Legacy Code

Goal: delete the old MCP JSON-RPC implementation after rmcp stdio, HTTP, socket, and task paths pass parity tests.

Overview: remove obsolete parser, dispatcher, response builder, legacy protocol, and duplicated metadata code while keeping only Atlas business logic and thin SDK adapters.

Rules:
- Delete only code made unreachable by earlier phases.
- Keep tests that protect externally visible behavior.
- Do not preserve legacy protocol branches unless covered by current supported version tests.
- Any deleted compatibility behavior must have an rmcp-backed replacement test or be explicitly unsupported by failing tests.

- [x] remove handrolled JSON-RPC modules:
  - [x] delete `packages/atlas-mcp/src/transport/jsonrpc.rs`
  - [x] delete manual method routing from `packages/atlas-mcp/src/transport/dispatch.rs`
  - [x] delete manual input line parsing from `packages/atlas-mcp/src/transport/input.rs`
  - [x] delete manual output writer helpers that only format JSON-RPC strings
  - [x] remove module exports from `packages/atlas-mcp/src/transport/mod.rs`
- [x] remove legacy protocol code:
  - [x] delete `packages/atlas-mcp/src/transport/legacy_2025.rs` if no current tests require it
  - [x] remove legacy version negotiation tests that conflict with current official rmcp version support
  - [x] add test that unsupported protocol versions fail through rmcp negotiation
- [x] remove duplicate spec parsing:
  - [x] remove `parse_initialize_request`
  - [x] remove `parse_request_meta` if rmcp context exposes equivalent metadata
  - [x] remove `negotiate_initialize`
  - [x] keep only server constants and Atlas-specific metadata helpers that remain used
- [x] shrink transport modules:
  - [x] keep stdio public API wrapper only
  - [x] keep socket handshake wrapper only
  - [x] keep HTTP router/auth/health wrapper only
  - [x] keep worker code only if still needed for blocking tool execution and timeout enforcement
- [x] add dead-code guard tests:
  - [x] run `cargo clippy --workspace --all-targets --quiet` with no dead-code allowances added for removed protocol modules
  - [x] add grep-like test or crate test ensuring `jsonrpc_ok` and `jsonrpc_error` helpers no longer exist
  - [x] add crate test ensuring `transport::dispatch::dispatch` no longer exists or is not exported
- [x] validate Phase X10:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet --features http-transport`
  - [x] run `cargo test --quiet -p atlas-mcp --features http-transport`
  - [x] run `cargo test --quiet -p atlas-cli cli_quality_gates`

Phase X10 completion criteria:
- [x] handrolled MCP JSON-RPC parser, dispatcher, response builders, and legacy protocol modules are removed
- [x] rmcp-backed stdio, HTTP, socket, tasks, resources, prompts, completions, and tools pass tests
- [x] clippy passes without new dead-code suppressions for removed protocol paths

### Phase X11 — Final Contract, Docs, and Workspace Validation

Goal: prove the rmcp conversion is complete, documented, and stable across workspace quality gates.

Overview: update generated docs/instructions, compatibility snapshots, and release-facing tests to reflect JSON-only official SDK behavior.

Rules:
- Do not add manual-only acceptance steps; every completion item must be verifiable by code, tests, or generated artifacts.
- Generated docs must be reproducible from source registries.
- MCP and CLI parity tests remain the strongest compatibility gate.
- Final validation uses workspace summary script.

- [x] update generated MCP documentation:
  - [x] regenerate `MCP_TOOLS.md` from the rmcp-backed tool registry
  - [x] update docs tests to assert generated docs include official SDK-backed schema fields
  - [x] remove TOON references from generated docs
  - [x] assert every exported tool has non-empty title, description, input schema, and output schema when required
- [x] update installed instructions and prompts:
  - [x] ensure installed instructions mention JSON `structuredContent` as source of truth
  - [x] ensure installed instructions still require Atlas graph tools before file search
  - [x] ensure prompts no longer include `output_format` examples
  - [x] add tests for instruction and prompt text drift
- [x] update compatibility snapshots:
  - [x] refresh stdio transcript snapshots for rmcp response formatting
  - [x] refresh HTTP transcript snapshots for rmcp streamable HTTP formatting
  - [x] refresh task/input-required snapshots for official rmcp type field ordering and naming
  - [x] assert `atlas_provenance` and `atlas_freshness` still appear in representative structured content
- [x] update dependency and feature checks:
  - [x] add dependency-deny allowlist entry for `rmcp` and transitive crates if deny config requires it
  - [x] assert `cargo tree -p atlas-mcp` contains no `toon-format`
  - [x] assert `cargo tree -p atlas-mcp --features http-transport` resolves HTTP dependencies without duplicate incompatible major versions where avoidable
- [x] run final validation:
  - [x] run `cargo fmt --all`
  - [x] run `cargo clippy --workspace --all-targets --quiet --features http-transport`
  - [x] run `cargo test --quiet -p atlas-mcp --features http-transport`
  - [x] run `cargo test --quiet -p atlas-cli cli_quality_gates`
  - [x] run `./scripts/test-workspace-summary.sh`

Phase X11 completion criteria:
- [x] docs, prompts, and installed instructions describe JSON-only rmcp-backed MCP behavior
- [x] no TOON dependency, docs, fixtures, schema fields, or instructions remain
- [x] dependency checks pass for rmcp-backed atlas-mcp
- [x] full workspace summary passes

Part X completion criteria:
- [x] Atlas MCP server uses `rmcp` official SDK types and transports for all MCP protocol surfaces
- [x] handrolled MCP JSON-RPC protocol implementation is removed
- [x] MCP output is JSON-only with `structuredContent` as source of truth
- [x] stdio, HTTP, socket/broker, lifecycle, logging, roots, prompts, resources, completions, tools, tasks, MRTR, elicitation, progress, and cancellation are covered by rmcp-backed tests
- [x] CLI/MCP parity remains covered by quality gates
- [x] `cargo fmt --all`, `cargo clippy --workspace --all-targets --quiet --features http-transport`, and `./scripts/test-workspace-summary.sh` pass
