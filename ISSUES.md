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

## Product Name and CLI

- [x] Use binary name: `atlas`
- [x] Use hidden work dir: `.atlas/`
- [x] Use DB path: `.atlas/worldtree.db`
- [x] Use config path: `.atlas/config.toml`
- [x] Use CLI commands:
  - [x] `atlas init`
  - [x] `atlas build`
  - [x] `atlas update`
  - [x] `atlas detect-changes`
  - [x] `atlas status`
  - [x] `atlas query`
  - [x] `atlas impact`
  - [x] `atlas review-context`
  - [x] `atlas serve` (later, MCP/stdin or JSON-RPC style)

---

## Roadmap Layout

- Part III. Remaining product expansion roadmap: Phases 29 through 31
- Part IV. Remaining context continuity roadmap: Phases CM12, CM14, and CM15, plus ICM-inspired memory follow-on roadmap
- Part V. Remaining focused follow-up patches: Retrieval Follow-Up Patch, Graph/Content Companion Patch, Parity Surface Patch, Runtime Event Enrichment and Graph Linking Patch, Context Escalation Contract Patch, Graph Store Corruption Recovery Patch, SQLite Connection Concurrency Policy Patch

## Cross-Cutting Track Map

- Historical and analytics work: Phase 17, Phase 29, Phase 30, Phase 31
- Retrieval and search follow-ups: Retrieval Follow-Up Patch, Graph/Content Companion Patch, Parity Surface Patch
- Context continuity and runtime memory: Phase CM12, Phase CM14, Phase CM15, ICM-inspired memory follow-on roadmap, Runtime Event Enrichment and Graph Linking Patch
- Graph safety and workflow: Context Escalation Contract Patch, Graph Store Corruption Recovery Patch, SQLite Connection Concurrency Policy Patch

---

## Part I — Core Delivery Roadmap

Phase 17 (Historical Graphs) is now shipped. See SHIPPED.md for details.

## Part III — Post-MVP Product Expansion

Use this part for advanced retrieval, analysis, refactoring, observability, real-time updates, insights, optional features, and MCP-facing payload optimizations.

These phases extend v1 after core graph/build/update/query path is reliable.

### Phase 29 — Intelligence & Insights

Deterministic analytics layer on top of graph + stored metadata. Produce explainable architecture insights, metrics, risk assessments, pattern detection. No LLM dependency.

#### 29.1 Architecture analysis

- [ ] build module-level graph
- [ ] detect strongly connected components (SCC)
- [ ] identify cyclic dependencies
- [ ] classify cycles (`local` vs `cross-module`)
- [ ] output cycle paths
- [ ] define configurable layer rules
- [ ] map files/modules to layers
- [ ] detect invalid edges
- [ ] output layer violations
- [ ] compute coupling score per module
- [ ] detect high-coupling modules
- [ ] detect tightly coupled clusters
- [ ] compute nodes per file
- [ ] compute edges per file
- [ ] flag large/highly connected files

#### 29.2 Code health metrics

- [ ] node-level metrics:
  - [ ] fan-in
  - [ ] fan-out
  - [ ] dependency depth
  - [ ] reference count
  - [ ] test adjacency
- [ ] file-level metrics:
  - [ ] node count
  - [ ] edge count
  - [ ] average fan-in/out
  - [ ] import count
  - [ ] test coverage ratio
- [ ] module-level metrics:
  - [ ] internal vs external dependencies
  - [ ] coupling score
  - [ ] cohesion approximation
- [ ] compute percentiles
- [ ] detect outliers

#### 29.3 Risk assessment engine

- [ ] score from inputs:
  - [ ] public API
  - [ ] fan-in/out
  - [ ] cross-module dependencies
  - [ ] test adjacency
  - [ ] depth
  - [ ] unresolved edges
- [ ] implement weighted formula
- [ ] normalize to `0–100`
- [ ] classify `low` / `medium` / `high`
- [ ] output:
  - [ ] factors list
  - [ ] evidence nodes/edges

#### 29.4 Pattern detection

- [ ] duplicate patterns:
  - [ ] repeated call chains
  - [ ] similar subgraphs
- [ ] unused structures:
  - [ ] unused modules
  - [ ] isolated graphs
  - [ ] orphan nodes
- [ ] high centrality:
  - [ ] compute centrality
  - [ ] find hubs
  - [ ] find bottlenecks
- [ ] deep chains:
  - [ ] detect long call chains
  - [ ] flag complexity

#### 29.5 APIs, outputs, CLI, config, tests

- [ ] create `InsightsEngine`
- [ ] implement:
  - [ ] `analyze_architecture()`
  - [ ] `compute_metrics()`
  - [ ] `assess_risk()`
  - [ ] `detect_patterns()`
  - [ ] `find_cycles()`
- [ ] define:
  - [ ] `ArchitectureReport`
  - [ ] `MetricsReport`
  - [ ] `RiskReport`
  - [ ] `PatternReport`
- [ ] ensure each report includes:
  - [ ] summary
  - [ ] detailed findings
  - [ ] evidence
- [ ] CLI:
  - [ ] `atlas insights architecture`
  - [ ] `atlas insights metrics`
  - [ ] `atlas insights risk <symbol>`
  - [ ] `atlas insights patterns`
  - [ ] JSON output support
- [ ] config:
  - [ ] thresholds
  - [ ] layer config
  - [ ] ignore lists
- [ ] tests:
  - [ ] cycle detection
  - [ ] coupling detection
  - [ ] unused-node detection
  - [ ] risk scoring validation
  - [ ] outlier detection
- [ ] completion criteria:
  - [ ] accurate architecture insights
  - [ ] correct metrics
  - [ ] explainable risk scoring
  - [ ] useful pattern detection
  - [ ] structured outputs

### Phase 30 — Optional Advanced Features

#### 30.1 Multi-repo

Registry-first design, not raw path merge. Current recursive submodule scan, owner identity, and cross-package edges give base. Extend that into first-class multi-repo federation so each repo keeps its own identity, git lifecycle, and provenance while Atlas can answer cross-repo questions.

##### 30.1.1 Goals and scope

- [ ] treat root repo, initialized git submodules, and manually registered sibling repos as one logical analysis scope
- [ ] keep per-repo identity explicit in storage, output, and cache keys
- [ ] support query, review, impact, and context flows across repo boundaries
- [ ] keep single-repo UX fast and unchanged by default
- [ ] fail closed when registry entries are missing, stale, detached, or unauthorized

##### 30.1.2 Multi-repo registry

- [ ] define `RepoRegistry` model
- [ ] define `RepoRegistration` entry with:
  - [ ] stable `repo_id`
  - [ ] canonical absolute root
  - [ ] repo-relative display alias
  - [ ] VCS metadata: `HEAD`, default branch, remote URL when available
  - [ ] relationship kind: `root`, `submodule`, `workspace_member`, `manual`
  - [ ] trust state and enabled/disabled flag
  - [ ] optional include/exclude globs
  - [ ] optional dependency metadata to other registered repos
- [ ] persist registry metadata under `.atlas/` instead of inferring everything from transient process state
- [ ] keep registry format human-editable
- [ ] version registry schema for future migrations

##### 30.1.3 Discovery and bootstrap

- [ ] auto-register root repo on `atlas init`
- [ ] auto-discover initialized git submodules as first-class repo entries
- [ ] record parent-to-submodule linkage instead of flattening submodule identity into root only
- [ ] support manual `atlas repo add <path>` for sibling repos outside root tree
- [ ] support `atlas repo remove <repo-id>` without deleting graph data for unrelated repos
- [ ] support `atlas repo sync` to refresh refs, remotes, enabled state, and missing paths
- [ ] surface uninitialized or missing submodules as registry warnings, not hard failures

##### 30.1.4 Identity and storage model

- [ ] extend path identity invariant from repo-relative path to `(repo_id, canonical_repo_relative_path)`
- [ ] prevent qualified-name collisions for same file names across different repos
- [ ] keep per-repo synthetic owner/workspace nodes and add synthetic repo nodes
- [ ] add repo-membership edges:
  - [ ] `repo contains package`
  - [ ] `repo contains workspace`
  - [ ] `registry contains repo`
  - [ ] `repo depends_on repo`
  - [ ] `repo submodule_of repo`
- [ ] store repo provenance on nodes, edges, files, saved context, and diagnostics output
- [ ] preserve existing single-db deployment when practical, but partition rows by `repo_id`
- [ ] avoid shared-graph writes that cannot be traced back to one source repo

##### 30.1.5 Build and update flows

- [ ] build each registered repo as independent parse/update unit
- [ ] reuse existing submodule-safe git invocation rules for child repos
- [ ] detect changes per repo using each repo's own git root and diff state
- [ ] let root-repo `detect-changes` expand into registered sub-repo changes when requested
- [ ] support targeted update:
  - [ ] one repo
  - [ ] all enabled repos
  - [ ] affected repos only
- [ ] cache per-repo build status, indexed revision, and stale markers
- [ ] report partial success when some repos update and others fail

##### 30.1.6 Cross-repo resolution and graph semantics

- [ ] resolve imports/calls across repos only when registry relationship or dependency evidence exists
- [ ] treat submodule boundaries as repo boundaries first, directory prefixes second
- [ ] let package-owner and workspace-owner metadata bridge repo boundaries when manifests point across repos
- [ ] add cross-repo edge metadata:
  - [ ] source repo
  - [ ] target repo
  - [ ] relationship reason: import, dependency, submodule, workspace link
  - [ ] confidence tier
- [ ] keep unresolved cross-repo references explicit so review/impact can explain missing evidence
- [ ] support cross-repo impact radius and removal analysis without hiding repo hops

##### 30.1.7 CLI and MCP surface

- [ ] CLI:
  - [ ] `atlas repo list`
  - [ ] `atlas repo add <path>`
  - [ ] `atlas repo remove <repo-id>`
  - [ ] `atlas repo sync`
  - [ ] `atlas build --all-repos`
  - [ ] `atlas update --all-repos`
  - [ ] `atlas query --repo <repo-id>|--all-repos`
  - [ ] `atlas impact --all-repos`
- [ ] MCP:
  - [ ] expose registry inspection tool
  - [ ] add optional repo scoping to graph/context tools
  - [ ] return repo identity in ambiguity candidates and provenance payloads
- [ ] human-readable output must show repo labels anywhere same symbol exists in multiple repos
- [ ] JSON output must include repo metadata in stable fields, not ad hoc strings

##### 30.1.8 Review, context, and saved artifacts

- [ ] let review context summarize changed repos before changed files
- [ ] include cross-repo boundary violations in impact and review summaries
- [ ] allow `get_context` to follow caller/callee edges across repos when enabled
- [ ] store session artifacts with repo-set ownership, not single repo only
- [ ] block saved-context reads when session repo scope does not overlap requested repo scope

##### 30.1.9 Safety, performance, and rollout

- [ ] keep single-repo default path zero-config and zero-regression
- [ ] gate multi-repo federation behind explicit registry presence or `--all-repos`
- [ ] bound fan-out so one command cannot accidentally parse every nearby checkout
- [ ] add per-repo and aggregate budget reporting
- [ ] degrade cleanly when one repo is unavailable, corrupted, or on unsupported filesystem
- [ ] start with submodules as phase-1 supported multi-repo source, then add manual sibling repos

##### 30.1.10 Tests and completion criteria

- [ ] tests:
  - [ ] submodule auto-registration
  - [ ] manual sibling repo registration
  - [ ] repo-id stability across rebuilds
  - [ ] qualified-name collision handling across repos
  - [ ] cross-repo query ranking and ambiguity output
  - [ ] cross-repo impact/review context
  - [ ] partial update failure reporting
  - [ ] saved-context repo-scope isolation
- [ ] completion criteria:
  - [ ] Atlas can index at least root repo plus one submodule as separate repo identities
  - [ ] cross-repo query output is deterministic and provenance-rich
  - [ ] impact/review tools can explain repo hops
  - [ ] default single-repo behavior remains unchanged

#### 30.2 Remaining code intelligence

- [ ] similar-function detection beyond graph-shape heuristics
- [ ] duplicate detection beyond exact structural patterns
- [ ] infer modules
- [ ] label components

### Phase 31 — Lowest Priority

#### 31.1 Docs generation (CLI command)

- [ ] generate Markdown docs
- [ ] visualization/export

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

#### Phase CM14 is Shipped

See SHIPPED.md for details on decision memory implementation.

---

##### Completion Criteria for Part IV

Both **Phase CM14 (Decision Memory)** and **Phase CM15 (Agent-Aware Context)** are shipped. See SHIPPED.md for implementation details.

The memory system is undergoing continuous improvement:

- [x] decisions persist and are reused
- [x] agent partitioning is implemented
- [ ] memory is curated, not just stored
- [ ] retrieval is semantic-aware
- [ ] system can recall past sessions
- [ ] context selection is optimized
- [ ] system improves over time

---

### ICM-Inspired Memory Follow-On Roadmap

Use this section to merge compatible parts of `atlas-icm-inspired-memory-roadmap.md` into the existing continuity architecture.

Priority order below is implementation order. Extend shipped Phase CM14 and Phase CM15 behavior. Do not introduce a parallel memory stack that conflicts with current `session.db` / `context.db` / `worldtree.db` boundaries.

This grouped roadmap covers the full source document at theme level, except shell-first simplifications requested here: no slash-command track, no skill-install track, and no web dashboard track.

#### ICM-A — Shared Memory Surface Over Existing Storage

What to do:

- [ ] add one shared memory service layer over existing continuity crates so CLI and MCP reuse identical validation, visibility, and storage behavior
- [ ] restore detailed subphase structure here so `ISSUES.md` can replace source roadmap file without losing implementation guidance

What not to do:

- [ ] do not create a separate memory architecture that bypasses shipped decision-memory and agent-partition services
- [ ] do not store memory bodies or runtime artifacts in `worldtree.db`
- [ ] do not require an active session for `project` or `global` writes
- [ ] do not let CLI and MCP drift on record shape, defaults, or visibility rules

Implementation structure:

##### ICM-A1 — Memory model and storage schema

- [ ] define `MemoryImportance` enum with exact values `critical`, `high`, `normal`, and `low`
- [ ] add `importance` field to stored memory records and default manual writes to `normal`
- [ ] define `MemoryScope` enum with exact values `project`, `session`, `frontend`, and `global`
- [ ] add `scope` field to memory records and make `project` default
- [ ] require `frontend` identifier when scope is `frontend`
- [ ] add memory tables to continuity-owned storage, preferably existing session-side persistence unless a dedicated memory DB is justified later
- [ ] create `memories` table with `id`, `repo_root`, `session_id`, `frontend`, `scope`, `topic`, `title`, `body`, `importance`, `created_at`, `updated_at`, `last_accessed_at`, `decay_score`, `source_id`, and `metadata_json`
- [ ] add indexes for `topic`, `importance`, `scope`, `session_id`, and `last_accessed_at`
- [ ] reject unknown importance and scope values at CLI, MCP, and storage boundaries
- [ ] validate memory schema through `atlas db check` and golden schema tests

##### ICM-A2 — CLI memory CRUD

- [ ] add `atlas memory store <text>` with flags `--topic`, `--title`, `--importance`, `--scope`, `--frontend`, `--source-id`, and `--json`
- [ ] store memory text exactly as provided unless central redaction policy strips sensitive content
- [ ] add `atlas memory recall <query>` with flags `--topic`, `--importance`, `--scope`, `--shared`, `--limit`, and `--json`
- [ ] use lexical search first for recall and rank exact topic matches above broad text matches
- [ ] add `atlas memory list` with filters `--topic`, `--importance`, `--scope`, `--older-than`, `--newer-than`, and `--json`
- [ ] sort memory list by `updated_at DESC` by default
- [ ] add `atlas memory delete <memory_id>` with `--dry-run` and `--json`
- [ ] require exact memory id for delete and keep linked saved-context artifacts unless explicit delete-source behavior is added later

##### ICM-A3 — Frontend-aware visibility rules

- [ ] normalize frontend identities to `claude`, `codex`, `copilot`, `cli`, and `mcp`
- [ ] reject unknown frontend names unless config explicitly allows custom frontends
- [ ] enforce visibility rules: `global` visible everywhere, `project` visible to all frontends in repo, `session` visible only to same session, `frontend` visible only to same repo plus same frontend
- [ ] make `atlas memory recall --shared` return only `global` and `project` memories
- [ ] ensure project-scoped writes work without an active session

##### ICM-A4 — MCP parity

- [ ] add MCP `memory_store` with same fields and validation as CLI
- [ ] add MCP `memory_recall` with same visibility rules and bounded default output
- [ ] keep source ids and retrieval hints available in compact MCP output
- [ ] add CLI/MCP parity tests so stored record shape, errors, and defaults match

##### ICM-A completion criteria

- [ ] `atlas memory store --importance critical` persists `importance = critical`
- [ ] `atlas memory store --scope frontend --frontend codex` stores frontend-private memory with correct visibility
- [ ] `atlas memory recall --shared` excludes frontend-private memories
- [ ] `atlas memory list --importance critical` filters correctly and emits stable JSON
- [ ] invalid importance/scope/frontend values fail with clear validation errors
- [ ] CLI and MCP memory store/recall paths produce equivalent record shapes

#### ICM-B — Memory Curation, Decay, Health, and Consolidation

What to do:

- [ ] add memory decay config with safe defaults and explicit critical-memory protection
- [ ] preserve deterministic maintenance structure from source roadmap so cleanup work stays implementation-ready after source file deletion

What not to do:

- [ ] do not auto-prune `critical` memories by default
- [ ] do not hard-delete linked saved-context artifacts unless explicitly requested
- [ ] do not make health scoring or consolidation depend on opaque LLM behavior
- [ ] do not mutate state during `--dry-run`

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

What to do:

- [ ] add feedback storage and search for predicted vs actual outcomes, correction text, related symbol/file, and `source_id`
- [ ] keep feedback as first-class deterministic correction memory rather than loose comments or opaque notes

What not to do:

- [ ] do not let feedback override deterministic graph evidence silently
- [ ] do not lower confidence without explicit matching evidence
- [ ] do not couple feedback storage to graph tables or graph-node lifecycle

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

What to do:

- [ ] define a bounded wake-up pack that summarizes current focus, critical memories, recent decisions, recent feedback, graph readiness, changed files, and retrieval hints
- [ ] keep wake-up path compact, retrieval-backed, and consistent with resume architecture already shipped in continuity work

What not to do:

- [ ] do not inline raw large artifacts into wake-up or resume payloads
- [ ] do not block session start on wake-up generation failure
- [ ] do not replay raw command history as wake-up context

Implementation structure:

##### ICM-D1 — Wake-up pack model

- [ ] define `WakePack` model with `repo_root`, `session_id`, `frontend`, `current_focus`, `recent_decisions`, `critical_memories`, `recent_feedback`, `active_memoir_concepts`, `changed_files`, `graph_readiness`, `retrieval_hints`, and `generated_at`
- [ ] bound wake-up pack size through config and central budget policy
- [ ] serialize wake-up packs to stable JSON

##### ICM-D2 — CLI and MCP wake-up

- [ ] add `atlas wake-up` with flags `--topic`, `--session`, `--frontend`, `--max-items`, and `--json`
- [ ] pull wake-up content from memory, feedback, session resume, and graph readiness services
- [ ] add MCP `wake_up` with compact default output, retrieval hints, and source ids instead of raw artifact bodies

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

What to do:

- [ ] improve recall ranking with topic match, importance, recency, scope visibility, and source-backed evidence
- [ ] preserve lexical-first default and make cross-session recall quality measurable before adding vector complexity

What not to do:

- [ ] do not make embeddings required for baseline memory recall
- [ ] do not let vector scores outrank exact lexical or stronger structural evidence by default
- [ ] do not widen frontend-private or session-private recall unless caller explicitly asks for it

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

What to do:

- [ ] add separate memoir tables, concepts, relations, and graph ids outside the code graph schema
- [ ] keep memoir path explicit and bounded so semantic memory does not leak into code graph semantics

What not to do:

- [ ] do not merge memoir concepts into code graph `nodes` and `edges`
- [ ] do not allow unbounded custom relation types by default
- [ ] do not auto-create missing concepts unless caller explicitly opts in

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

#### ICM-G — Shell-First Install Modes, TUI, Docs, and Release Gates

What to do:

- [ ] add install/init mode split for `mcp`, `hook`, `cli`, and `all`, with idempotent generation and dry-run preview
- [ ] keep shell-first and TUI-first operational structure from source roadmap while dropping slash-command, skill, and dashboard work

What not to do:

- [ ] do not add slash-command generators or skill-install surfaces for this track
- [ ] do not add web dashboard routes for memory inspection in this track
- [ ] do not build TUI surfaces before core service contracts and tests stabilize
- [ ] do not introduce host-specific command generators that bypass shared service logic

Implementation structure:

##### ICM-G1 — Shell-first install and init modes

- [ ] add supported `atlas init --mode` values `mcp`, `hook`, `cli`, and `all`
- [ ] make each mode idempotent and emit files to be created during `--dry-run`
- [ ] ensure `--mode all` installs only MCP config, hooks, and CLI config relevant to shell-first memory workflows

##### ICM-G2 — TUI only, read-only first

- [ ] add `atlas memory tui` with read-only browsing for memories, topics, feedback, memoir concepts, health findings, and saved artifacts
- [ ] add filters for topic, scope, importance, and frontend
- [ ] keep first version non-mutating and smoke-testable without panic

##### ICM-G3 — Tests, docs, and release gates

- [ ] create reusable fixtures for critical decision memory, low-priority stale memory, dead-code false-positive feedback, memoir dependency graph, wake-up pack with saved artifact references, and frontend-private memory
- [ ] snapshot JSON output for `atlas memory store --json`, `atlas memory recall --json`, `atlas memory health --json`, `atlas feedback record --json`, `atlas feedback search --json`, `atlas memoir inspect --json`, and `atlas wake-up --json`
- [ ] add `wiki/memory-architecture.md` documenting memory DB ownership, importance and decay policy, scope and visibility rules, feedback integration, memoir graph separation, wake-up behavior, and CLI/MCP mapping
- [ ] define release gate `ICM Memory Layer Complete`
- [ ] require for release gate: CLI and MCP memory store/recall parity, importance and decay policies, feedback-adjusted analysis, memoir typed relations, wake-up packs without raw large content, health audit coverage, shared/private visibility rules, complete docs, and JSON snapshot coverage

##### ICM-G completion criteria

- [ ] every new shell-first memory command has CLI smoke coverage
- [ ] every MCP memory tool has handler tests and parity assertions where applicable
- [ ] `cargo test --workspace` passes with fixtures and JSON snapshots committed
- [ ] no memory feature writes directly to graph DB
- [ ] no large artifact is inlined into wake-up or resume output by default

---

## Part V — Follow-Up Patches

Use these patch sections for focused improvements that cut across existing roadmap phases without rewriting phase scope.

### Retrieval Follow-Up Patch

These are the high-value retrieval/indexing improvements still missing or only partially specified after the current v3 plan.

They are meant to strengthen Atlas’s retrieval/content sidecar without changing the graph-first core.

#### Patch R1 — Retrieval index lifecycle state

Atlas already has strong graph build/update state and separate content/session stores, but retrieval/content indexing should also have an explicit lifecycle model so “built”, “indexed”, “searchable”, and “failed” do not drift.

- [x] add explicit retrieval index state model
- [x] create retrieval/content index status table or snapshot
- [x] track per repo / per source states:
  - [x] `indexing`
  - [x] `indexed`
  - [x] `index_failed`
- [x] persist:
  - [x] total files discovered
  - [x] files indexed
  - [x] total chunks written
  - [x] chunks reused
  - [x] last successful index time
  - [x] last error
- [x] expose retrieval index status through CLI
- [x] expose retrieval index status through MCP
- [x] ensure one source of truth for “searchable now”
- [x] ensure interrupted indexing can recover cleanly without manual cleanup

Why:
- prevents state drift between stored content, searchable content, and agent-visible status
- improves crash recovery and diagnostics

#### Patch R2 — Retrieval batching and chunk explosion guardrails

Current plan has chunking and retrieval, but operational safety limits should be explicit.

- [x] add configurable `retrieval_batch_size`
- [x] add configurable `embedding_batch_size`
- [x] add hard `max_chunks_per_index_run`
- [x] add hard `max_chunks_per_file`
- [x] add policy for oversized indexing runs:
  - [x] fail fast
  - [x] partial index with warning
  - [x] skip pathological file with error entry
- [x] measure and log:
  - [x] buffered chunk count
  - [x] buffered bytes
  - [x] staged vector bytes
  - [x] batch flush count
- [x] add tests for:
  - [x] chunk explosion from large file
  - [x] recursive fallback chunk explosion
  - [x] partial indexing recovery after hard cap hit

Why:
- protects retrieval layer from pathological files and runaway indexing cost
- makes retrieval/index behavior predictable under load

#### Patch R3 — Embedding dimension registry and freeze rules

Atlas already has optional embeddings and hybrid retrieval roadmap, but dimension handling should be explicit and deterministic.

- [x] create embedding provider registry metadata
- [x] persist:
  - [x] provider name
  - [x] model name
  - [x] embedding dimension
  - [x] discovered_at
  - [x] index schema version
- [x] require dimension to be frozen at index creation time
- [x] reject insert/search if dimension does not match active retrieval index
- [x] cache discovered dimensions per provider/model
- [x] add CLI / diagnostics surface for current embedding config
- [x] add tests for:
  - [x] dimension mismatch on insert
  - [x] dimension mismatch on query
  - [x] provider switch with incompatible existing index
  - [x] explicit rebuild requirement after dimension change

Why:
- avoids one of the most common hybrid/vector indexing failure modes
- keeps retrieval layer deterministic and debuggable

#### Patch R4 — Retrieval backend capability flags

Atlas should make backend capability checks explicit instead of assuming all retrieval backends support all modes.

- [x] define retrieval backend capability model
- [x] support capability flags for:
  - [x] lexical FTS
  - [x] dense vector search
  - [x] hybrid lexical + vector fusion
  - [x] sparse / BM25-native retrieval
  - [x] metadata filtering
- [x] validate requested retrieval mode against backend capabilities before query/index
- [x] disable unsupported hybrid mode automatically with explicit warning
- [x] ensure MCP/CLI surfaces report active retrieval mode clearly
- [ ] add tests for:
  - [x] lexical-only backend
  - [x] dense-only backend
  - [x] hybrid-capable backend
  - [x] unsupported mode request fails cleanly

Why:
- makes future retrieval backends or storage variants safe to introduce
- avoids silent degradation and confusing behavior

#### Patch R5 — Stable content-derived chunk identity

Current chunk storage should have a true stable identity separate from display order.

- [x] add stable `chunk_id`
- [x] define `chunk_id` from content-derived hash over:
  - [x] source/file path
  - [x] line span or chunk boundary
  - [x] normalized content
- [x] keep `chunk_index` or display order separately
- [x] use `chunk_id` for:
  - [x] dedupe
  - [x] chunk reuse
  - [x] retrieval cache keys
  - [x] saved-context references
- [x] add tests for:
  - [x] same content same `chunk_id`
  - [x] moved chunk with changed path policy documented
  - [x] changed line span/content produces new `chunk_id`

Why:
- improves deduplication and retrieval consistency across rebuilds
- helps saved-context and future historical retrieval features

#### Patch R6 — Retrieval/token-efficiency evaluation

Atlas already measures correctness and performance in many places, but retrieval should also be evaluated as a context-efficiency system.

- [x] add retrieval benchmark metrics:
  - [x] `recall_at_k`
  - [x] `mrr`
  - [x] exact target hit rate
  - [x] retrieved tokens per query
  - [x] emitted tokens per query
  - [x] tool calls per task
- [x] benchmark:
  - [x] graph-only context
  - [x] lexical retrieval only
  - [x] hybrid retrieval
  - [x] hybrid retrieval + graph expansion
- [x] add fixed-budget evaluation:
  - [x] quality under small context budget
  - [x] quality under medium context budget
- [x] track whether retrieval actually reduces:
  - [x] payload size
  - [x] repeated search calls
  - [x] context noise
- [x] add acceptance thresholds before enabling hybrid retrieval by default

Why:
- keeps retrieval improvements aligned with actual user value
- validates that the retrieval layer improves token efficiency, not just ranking complexity

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

- [ ] retrieval/content index has explicit searchable state
- [ ] retrieval indexing has batch and chunk guardrails
- [ ] embedding dimension rules are explicit and enforced
- [ ] retrieval backend capabilities are validated, not assumed
- [ ] stable `chunk_id` exists and is used for dedupe/reuse
- [ ] retrieval/token-efficiency benchmarks are in place
- [ ] optional post-retrieval compaction is tracked as a late experiment only

---

### Retrieval Ranking Evidence Patch

Retrieval Ranking Evidence Patch is shipped. See SHIPPED.md for details.

---

### Graph/Content Companion Patch

Atlas already has graph search for symbols and relationships plus file/content/template/text-asset search for prompts, docs, config, SQL, and templates. The missing design rule is that these are coordinated retrieval surfaces, not separate universes or a simple fallback chain. Graph answers code structure questions; content lookup answers non-code and context-adjacent questions; the context engine should merge both under one bounded selection, ranking, evidence, and truncation policy.

#### Patch N1 — Declare graph/content lookup contract

- [x] document canonical responsibility split:
  - [x] graph search answers symbols, ownership, callers, callees, tests, imports, and structural relationships
  - [x] content lookup answers prompts, docs, config, SQL, templates, logs, and embedded text assets
  - [x] saved-context lookup answers prior Atlas outputs and session artifacts
  - [x] context engine decides how these surfaces combine for a task
- [x] define graph/content lookup as companion systems, not fallback-only systems
- [x] define when both should be queried for one request:
  - [x] review changes touching config or templates
  - [x] symbols whose behavior depends on prompts or SQL
  - [x] docs/spec questions tied to implementation files
  - [x] agent/task questions needing saved context plus graph facts
- [x] document anti-patterns:
  - [x] broad file search before graph resolution for symbol questions
  - [x] graph-only review when changed files include config/templates/prompts
  - [x] content-only answers for structural dependency questions
  - [x] separate unbounded result lists from graph and content tools

Why:
- non-code artifacts are first-class context when they affect behavior
- graph-first should not mean content-blind

#### Patch N2 — Unified bounded selection policy

- [x] define one context selection policy for mixed graph/content results:
  - [x] direct graph targets first
  - [x] changed files and changed symbols next
  - [x] adjacent config/templates/prompts/SQL tied to changed files next
  - [x] caller/callee/test evidence next
  - [x] saved-session artifacts only when relevant to current task
- [x] apply shared budgets across mixed results:
  - [x] max graph nodes
  - [x] max graph edges
  - [x] max content assets
  - [x] max saved artifacts
  - [x] max total payload bytes/tokens
- [x] ensure truncation reports mixed omissions:
  - [x] omitted graph nodes
  - [x] omitted graph edges
  - [x] omitted content assets
  - [x] omitted saved artifacts
  - [x] omitted bytes/tokens
- [x] add deterministic tie-breakers when graph and content scores compete
- [x] add tests for mixed graph/content truncation order

Why:
- separate bounded lists can still create an unbounded combined context
- agents need one budget story for the final answer context

#### Patch N3 — Coordinated ranking and evidence

- [x] define a mixed-result ranking envelope with source kind:
  - [x] `graph_node`
  - [x] `graph_edge`
  - [x] `file_asset`
  - [x] `content_match`
  - [x] `template`
  - [x] `text_asset`
  - [x] `saved_context`
- [x] normalize ranking signals across surfaces:
  - [x] exact symbol match
  - [x] graph distance
  - [x] changed-file boost
  - [x] same package/directory boost
  - [x] BM25/content match score
  - [x] trigram/fuzzy correction
  - [x] proximity/title/path rerank
  - [x] session recency/relevance
- [x] expose why each mixed item was selected through ranking evidence
- [x] include `selection_reason` for both graph and content assets
- [x] add tests proving config/template/prompt matches can be selected with graph evidence when relevant

Why:
- mixed context should be explainable, not an opaque concatenation of tool outputs
- ranking evidence must work for content assets as well as graph nodes

#### Patch N4 — MCP and prompt workflow integration

- [x] update MCP tool descriptions to describe graph/content companion rules
- [x] improve `search_content` invalid-regex guidance:
  - [x] keep `is_regex=true` strict; invalid regex returns error instead of fallback search
  - [x] include escaped-regex suggestion for literal metacharacters, for example `Context \\{`
  - [x] suggest `is_regex=false` when caller wants literal text search
  - [x] add MCP regression test for invalid pattern like `Command::Context|Context {`
- [x] update `review_change` prompt to query content assets when changed files include docs/config/templates/prompts/SQL
- [x] update `inspect_symbol` prompt to look for context-adjacent assets only when graph evidence suggests dependency
- [x] update installed AGENTS instructions:
  - [x] graph tools first for structure
  - [x] content tools as companion lookup for non-code assets
  - [x] context engine should merge both under bounded policy
- [x] add prompt/registry snapshot tests for companion-contract wording

Why:
- agents follow surface contracts more reliably than implicit architecture
- prompt and install docs should not describe content lookup as mere fallback

#### Patch N completion criteria

- [x] graph/content companion contract is documented as a design rule
- [x] mixed graph/content context has one bounded selection policy
- [x] mixed results expose source kind, selection reason, ranking evidence, and truncation metadata
- [x] MCP prompts, tool descriptions, README, and installed AGENTS instructions agree
- [x] tests cover mixed code + config/template/prompt/doc context assembly

---

### Parity Surface Patch

Atlas already has pieces of the upstream parity surface: Markdown heading graph nodes, content search over docs, large-function risk flags in review summaries, and explicit build/update plus flows/communities commands. Missing work is to turn those pieces into first-class CLI/MCP surfaces with shared service logic, compact output, and parity tests.

#### Patch PS1 — Docs section lookup parity

Patch PS1 is shipped. See SHIPPED.md for details.

#### Patch PS2 — Large-function finder parity

- [ ] add large-function analysis service:
  - [ ] scan function/method graph nodes by line span
  - [ ] configurable threshold with default matching review risk summary
  - [ ] rank by line count, fan-in/fan-out, changed-file relevance, and package/module boundary when available
  - [ ] return file path, qualified name, kind, line range, LOC, and ranking reason
  - [ ] support repo-wide and file-scoped modes
- [ ] add CLI surface:
  - [ ] `atlas find-large-functions`
  - [ ] `atlas find-large-functions --files ...`
  - [ ] `--threshold`, `--limit`, `--include-tests`, and `--json`
- [ ] add MCP `find_large_functions`:
  - [ ] same inputs and defaults as CLI JSON
  - [ ] compact defaults suitable for agent review
  - [ ] provenance and freshness metadata
- [ ] add CLI/MCP parity tests:
  - [ ] default threshold matches review large-function flag
  - [ ] file-scoped filtering
  - [ ] threshold and limit behavior
  - [ ] test-node include/exclude behavior
  - [ ] stable sort ties

Why:
- current review code only flags large changed functions; agents need direct repo/file discovery and ranked evidence
- one service prevents review, CLI, and MCP thresholds from drifting

#### Patch PS3 — Explicit postprocess command parity

Patch PS3 is shipped. See SHIPPED.md for details.

#### Patch PS completion criteria

- [ ] `find_large_functions` exists as an MCP tool with matching CLI surface
- [ ] large-function service shares ranking, truncation, and error rules with review summaries instead of drifting thresholds
- [ ] CLI JSON and MCP JSON are parity-tested for representative large-function fixtures
- [ ] README, MCP reference, installed AGENTS instructions, and prompt workflows document the remaining large-function surface consistently

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
  - [ ] `tool_or_command`
  - [ ] `repo_root`
  - [ ] `file_paths`
  - [ ] `symbols`
  - [ ] `classification`
- [x] ensure secrets are redacted before persistence and previews
- [ ] add tests for small, medium, large, oversized, and secret-bearing outputs

Why:
- `SessionStore::append_event` already rejects oversized inline payloads
- content store is the correct place for searchable runtime text

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
- [ ] event-to-graph links use stable identifiers and treat row IDs as optional cache hints
- [ ] graph linking obeys readiness state and budget policy
- [ ] context engine can merge runtime events/artifacts with graph and saved context under one bounded ranking policy
- [ ] CLI, MCP, and hook flows feed enrichment best-effort
- [ ] resume snapshots include compact enriched runtime signals
- [ ] tests cover classification, artifact routing, graph linking, context integration, and resume enrichment

---

### Graph Readiness Source-of-Truth Patch

Graph Readiness Source-of-Truth Patch is shipped. See SHIPPED.md for details.

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

### Fuzz Patches

Atlas now has initial `cargo-fuzz` coverage for parser registry dispatch, direct language handlers, and SQLite regex UDF execution. That is a good base, but tree-sitter-heavy paths still have integration and invariant gaps. Add focused fuzz patches to cover cache lifecycle, engine update flow, parser output invariants, AST helper safety, refactor validation reuse, and seed corpora quality.

#### Patch F1 — Stateful `TreeCache` incremental-reparse fuzz target

- [x] add `tree_cache_stateful` cargo-fuzz target under `fuzz/fuzz_targets/`
- [x] define stateful fuzz input model:
  - [x] sequence of operations over `TreeCache`: `parse`, `reparse_with_old_tree`, `insert`, `remove`, `evict`, `rename_key`
  - [x] path selector over supported parser paths (`.rs`, `.go`, `.py`, `.js`, `.ts`, `.json`, `.toml`, `.html`, `.css`, `.sh`, `.md`, `.java`, `.cs`, `.php`, `.c`, `.cpp`, `.scala`, `.rb`)
  - [x] source bytes per operation
- [x] drive real `ParserRegistry::with_defaults()` plus real `TreeCache` from `atlas-parser`
- [x] fuzz remove/reinsert path used by incremental update worker handoff
- [x] fuzz delete/rename path transitions:
  - [x] parse old path
  - [x] remove cached tree
  - [x] reinsert under new path
  - [x] evict old path
- [x] assert no panic when cached old tree is reused with changed bytes for same path
- [x] add deterministic regression test alongside `TreeCache` tests for minimal cache rename/remove round-trip discovered during fuzzing

Why:
- current fuzz targets pass `old_tree`, but do not exercise actual `TreeCache` lifecycle semantics
- tree ownership and path-key transitions are core to safe incremental reparse

#### Patch F2 — `atlas-engine` update-flow fuzz target

- [x] add `update_graph_sequence` cargo-fuzz target under `fuzz/fuzz_targets/`
- [x] create temp repo fixture per fuzz case with minimal `.git` init and tracked files
- [x] define fuzz input model:
  - [x] initial file set
  - [x] sequence of file mutations (`add`, `modify`, `delete`, `rename`)
  - [x] path kinds spanning supported parser extensions plus unsupported files
  - [x] base content bytes and mutated content bytes
- [x] run real `atlas_engine::update::update_graph` against temp repo and temp SQLite db
- [x] cover both:
  - [x] working-tree diff mode
  - [x] explicit file-list mode
- [x] ensure fuzz sequence exercises:
  - [x] old-tree reuse through `TreeCache`
  - [x] unsupported-file skip path
  - [x] deleted-file cleanup path
  - [x] renamed-file path
- [x] assert no panic and no hard error from benign malformed source in supported files
- [x] add targeted regression tests for any crashers found in update pipeline

Why:
- parser-only fuzz misses actual engine integration path that moves trees through work items and persistence logic
- update flow is where tree-sitter parse reuse meets repo state and file churn

#### Patch F3 — Parser output invariant fuzz target

- [x] add `parser_invariants` cargo-fuzz target under `fuzz/fuzz_targets/`
- [x] feed all built-in language handlers through `ParserRegistry::parse`
- [x] define invariant checks on returned `ParsedFile`:
  - [x] exactly one `file` node exists for supported parse result
  - [x] `ParsedFile.path` equals input relative path
  - [x] every node `file_path` equals input relative path
  - [x] every node `qualified_name` is non-empty
  - [x] every edge `source_qn` is non-empty
  - [x] every edge `target_qn` is non-empty
  - [x] `line_start >= 1`
  - [x] `line_end >= line_start`
  - [x] `size` matches source length when populated
- [x] document any intentional invariant exceptions in comments near the checks
- [x] add optional duplicate-qualified-name detector:
  - [x] fail only if duplicate QNs are invalid for that language/model
  - [x] otherwise record as advisory and do not assert
- [x] add regression tests for each new invariant that catches a real bug found by fuzzing

Why:
- current fuzz verifies “no crash” only
- malformed tree-sitter outputs can still silently create invalid graph state

#### Patch F4 — AST helper safety fuzz target

- [x] add `ast_helpers_walk` cargo-fuzz target under `fuzz/fuzz_targets/`
- [x] parse fuzz bytes with each built-in language grammar
- [x] if parse returns a tree:
  - [x] walk all nodes recursively
  - [x] call `node_text`, `start_line`, `end_line`, and `has_ancestor_kind` on each node
  - [x] call `field_text` across a bounded list of common field names (`name`, `parameters`, `return_type`, `body`, `value`, `type`, `result`, `object`, `function`)
  - [x] call `find_all` for bounded common kinds relevant to each grammar
- [x] assert helpers never panic on malformed parse trees or invalid UTF-8 source bytes
- [x] add direct unit tests in `ast_helpers.rs` for any helper edge case discovered by fuzzing

Why:
- helper panics would affect every language parser
- current language-handler fuzz only covers helper usage that happens to be reached by parser-specific traversal

#### Patch F5 — Refactor validation parser-reuse fuzz target

- [x] add `refactor_parse_validation` cargo-fuzz target under `fuzz/fuzz_targets/`
- [x] expose minimal public or test-only harness in `atlas-refactor` for `parse_file_content` path without requiring full refactor scenario setup
- [x] define fuzz input model:
  - [x] file path
  - [x] content bytes
  - [x] supported and unsupported file extensions
- [x] run parser revalidation path used by refactor engine
- [x] assert behavior stays bounded:
  - [x] unsupported files return `None`
  - [x] empty files do not panic
  - [x] malformed supported-language content does not panic
  - [x] validation warnings/errors remain UTF-8 safe
- [x] add regression tests for any parser-validation crash discovered

Why:
- refactor engine reuses parser stack through a different caller path with different assumptions about content and file support
- parser safety should cover both graph build and refactor validation paths

#### Patch F6 — Seed corpus and dictionary patch

- [x] add initial corpora under `fuzz/corpus/` for all parser-centric targets
- [x] seed `parser_handlers`, `language_parsers`, `tree_cache_stateful`, `parser_invariants`, and `ast_helpers_walk` from existing parser fixtures:
  - [x] `packages/atlas-parser/tests/fixtures/*/core.*`
  - [x] `packages/atlas-parser/tests/fixtures/*/bad_syntax.*`
- [x] add regex corpus for `regex_sql_udf`:
  - [x] literals
  - [x] anchors
  - [x] alternation
  - [x] character classes
  - [x] invalid patterns
  - [x] Unicode-heavy samples
- [x] add optional `regex.dict` with common regex metacharacters and flags
- [x] add `README` commands for refreshing corpora from fixture files
- [x] document nightly/toolchain and `cargo fuzz` setup in `fuzz/README.md`
- [x] update gitignore for the seed corpus

Why:
- harnesses without corpora start colder and discover structural paths more slowly
- existing parser fixtures already provide valid and invalid syntax seeds across languages

#### Patch F completion criteria

- [x] `tree_cache_stateful` fuzzes real `TreeCache` lifecycle operations with parser reuse
- [x] `update_graph_sequence` fuzzes `atlas-engine` incremental update flow on temp repos
- [x] `parser_invariants` asserts graph-shape invariants for every built-in language parser
- [x] `ast_helpers_walk` stress-tests `ast_helpers` against arbitrary parse trees and byte input
- [x] `refactor_parse_validation` fuzzes parser reuse through `atlas-refactor`
- [x] corpora exist under `fuzz/corpus/` and seed parser and regex targets from real fixtures
- [x] every new fuzz-discovered crash adds a deterministic regression test near affected code

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

#### Patch T1 — Canonical connection ownership contract

- [x] define one canonical SQLite thread/ownership policy in `atlas-db-utils`:
  - [x] each Atlas store owns exactly one `rusqlite::Connection`
  - [x] store structs are thread-confined and must not cross Rayon or worker-thread boundaries
  - [x] concurrent DB access, when needed, must use separate connections; never shared ownership of one connection
  - [x] current architecture is single-writer per store instance; no read pool exists yet
- [x] align wording across `atlas-store-sqlite`, `atlas-contentstore`, `atlas-session`, and `atlas-db-utils`:
  - [x] replace ambiguous `serialized-reader` wording with `single-connection per store instance`
  - [x] document that WAL permits concurrent reads during writes only across separate connections
  - [x] document why `worldtree.db` opens with `SQLITE_OPEN_NO_MUTEX` under thread-confined ownership
- [x] add succinct comments near engine build/update Rayon phases stating DB work is intentionally outside parallel closures
- [x] ensure parser docs stay explicit that parser crate has no SQLite access and is not part of DB-sharing risk

Why:
- avoids reviewer confusion from `rayon` presence in workspace dependencies
- makes current concurrency guarantees explicit and consistent across crates

#### Patch T2 — Engine boundary enforcement and regression tests

- [x] keep engine parallel parse phases structurally separated from SQLite write phases:
  - [x] Rayon closures receive only parse inputs such as paths, hashes, bytes, and optional tree-cache entries
  - [x] `Store` access stays in explicit sequential write/update phases after parallel collection completes
- [x] add regression tests for current architecture:
  - [x] full build path proves parallel parse completes before store write phase
  - [x] incremental update path proves changed/dependent file parse phases complete before store write phase
  - [x] existing WAL lock test continues to model concurrency with a second connection on a second thread, not a shared connection
- [x] add compile-fail or equivalent trait-bound tests proving `Store`, `ContentStore`, and `SessionStore` cannot satisfy APIs that require `Send` or `Sync`
- [x] reject any new abstraction that wraps one `Connection` in `Arc<Mutex<_>>`, `RwLock<_>`, or similar cross-thread sharing helper

Why:
- turns architecture intent into an enforceable boundary
- catches refactors that accidentally move store access into worker threads

#### Patch T3 — Future separate-connection read concurrency contract

- [x] document explicit non-goal for this patch: do not add `r2d2_sqlite` or any read pool yet
- [x] define future upgrade rule:
  - [x] if read concurrency is added later, use separate checked-out connections
  - [x] do not share one `Connection` across threads behind a lock
  - [x] keep write ownership/policy explicit before introducing mixed read/write pooling
- [x] surface current mode in docs or diagnostics:
  - [x] parallel parse plus sequential persistence
  - [x] single-connection per store instance
  - [x] separate-connection concurrency only
  - [x] read-pool layer reserved for future measured need
- [x] add note to `doctor`/`status` or crate-level docs that pooled graph reads are not implemented today

Why:
- answers pool question without adding premature complexity
- preserves clean path for future measured read-parallel improvements

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

- [ ] one canonical SQLite connection/thread policy exists and all Atlas stores reference it
- [ ] engine Rayon parse code is explicitly separated from SQLite access
- [ ] tests fail if store types become cross-thread sharable
- [ ] docs say current mode is single-connection per store instance with separate-connection concurrency only
- [ ] future pool direction is documented as separate-connection only, not shared-connection wrappers
- [ ] any future read pool remains evidence-driven and preserves explicit writer ownership

---

## Additional Backlog

- [ ] add canonical `docs/error_codes.md` file and make README, MCP responses, and tests reference that single error-code catalog
- [ ] add generated `MCP_TOOLS.md` from tool registry and test/docs check that catches drift from hand-maintained tool tables
- [ ] add build/query/MCP metrics counters and histograms for build duration, parsed file count, parser cache reuse ratio, query latency by mode, and MCP tool call counts
- [ ] add informational `cargo-llvm-cov` coverage task and CI job that reports coverage without gating merge
- [ ] add `criterion` bench suites per crate for build, incremental update, query modes, context engine, and history reconstruction workloads
- [ ] add CI regression harness for `cargo bench --message-format=json` and store benchmark output as comparable artifact
- [ ] add CI-visible parser cache hit-ratio metric and fail when cache reuse drops below configured threshold
- [ ] add thin LSP shim that maps Atlas query/context/impact/reference flows onto standard LSP requests
- [ ] add documented `budget_policy` block to `.atlas/config.toml` with defaults, environment overrides, and `--budget-profile` selection
- [ ] add configurable layer-rules file surface for Phase 29.1 so architecture rules can change without recompiling
- [ ] add configurable redaction-rules file surface for Patch X4 so sanitization policy can change without recompiling
- [x] add `.atlas/config.toml` embedding config block for `atlas-search` URL/model settings instead of reading `ATLAS_EMBED_URL`. Move the ATLAS_EMBED_* envs to config, remove the env getters for this
- [ ] add issue items for tokenizer-backed budget accounting using real token counts instead of byte/char heuristics
- [ ] add `proptest` coverage for ranking/trimming, canonical-path normalization, and FTS query escaping

---
