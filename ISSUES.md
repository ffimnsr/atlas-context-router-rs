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

Product Name and CLI baseline is shipped. See SHIPPED.md for details.

---

## Roadmap Layout

- Part III. Remaining product expansion roadmap: Phases 29 through 31
- Part IV. Remaining context continuity roadmap: Phases CM12, CM14, and CM15, plus ICM-inspired memory follow-on roadmap
- Part V. Remaining focused follow-up patches: Retrieval Follow-Up Patch, Runtime Event Enrichment and Graph Linking Patch, Context Escalation Contract Patch, Graph Store Corruption Recovery Patch, SQLite Connection Concurrency Policy Patch

## Cross-Cutting Track Map

- Historical and analytics work: Phase 17, Phase 29, Phase 30, Phase 31
- Retrieval and search follow-ups: Retrieval Follow-Up Patch
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

Implement Phase 29 in patch order. Do not start later patches until the preceding report types, metrics, and deterministic ranking helpers exist. Keep this layer read-only over graph/content/session stores; do not add new graph schema unless a later item explicitly says so.

#### 29.1 Insights engine foundation

- [x] create `InsightsEngine` service:
  - [x] place service in engine or reasoning crate only after checking existing analysis service boundaries
  - [x] accept read-only graph/store handles or already-loaded graph summaries
  - [x] return deterministic report structs without writing to SQLite
  - [x] reuse existing ranking/truncation helpers before adding new ones
  - [x] reuse existing freshness/provenance metadata shape from graph/context tools
- [x] define shared report primitives:
  - [x] `InsightSummary` with total findings, highest severity, and generated-at metadata
  - [x] `InsightFinding` with `id`, `title`, `severity`, `category`, `message`, `evidence`, and `ranking_reason`
  - [x] `InsightEvidence` with file path, qualified name, node kind, edge kind, line range, and confidence tier when available
  - [x] deterministic severity values: `info`, `low`, `medium`, `high`
  - [x] deterministic sort order: severity desc, score desc, file path asc, line asc, qualified name asc
- [x] define top-level reports:
  - [x] `ArchitectureReport`
  - [x] `MetricsReport`
  - [x] `RiskReport`
  - [x] `PatternReport`
  - [x] `LargeFunctionReport`
- [x] define config surface:
  - [x] add insights thresholds under `.atlas/config.toml`
  - [x] include defaults for large-function LOC, high fan-in, high fan-out, high coupling, deep chain length, and max findings
  - [x] include defaults for high cyclomatic complexity, high cognitive complexity, max nesting depth, and branch count
  - [x] include ignore lists for files, modules, and node kinds
  - [x] include optional layer rules for architecture validation
  - [ ] add configurable layer-rules file surface for architecture validation:
    - [ ] add config field for external layer-rules file path under `.atlas/config.toml`
    - [ ] load layer rules from referenced file at runtime so architecture rules can change without recompiling
    - [ ] validate missing, unreadable, or malformed layer-rules file with actionable config errors
  - [x] validate thresholds are positive and fail with actionable config errors
- [x] add foundation tests:
  - [x] report sorting is stable
  - [x] severity ordering is stable
  - [x] invalid threshold config fails clearly
  - [x] ignored files/modules are excluded from findings
  - [x] report JSON shape is stable

Why:
- every later insight needs same evidence, severity, ranking, and config contract
- shared primitives prevent one-off report formats across CLI and MCP

#### 29.2 Code health metrics engine

- [x] implement node-level metric collection:
  - [x] compute fan-in from inbound graph edges by node qualified name
  - [x] compute fan-out from outbound graph edges by node qualified name
  - [x] compute dependency depth as longest bounded outbound path, with cycle guard
  - [x] compute reference count from `References`, `Calls`, `Imports`, and language-specific relationship edges
  - [x] compute test adjacency from direct test edges, test nodes in same file, or existing test-adjacency helpers
  - [x] compute line count / LOC for function and method nodes from `line_start` and `line_end`
  - [x] mark large-function candidate when LOC is at or above configured threshold
- [x] implement function complexity metric collection:
  - [x] compute cyclomatic complexity for function and method nodes from parser-backed syntax where available
  - [x] compute cognitive complexity for nested control-flow constructs where parser data supports it
  - [x] compute branch/control-flow count for `if`, `else if`, `match`/`switch`, loops, boolean short-circuit branches, `catch`/exception branches, and early returns where language parser exposes them
  - [x] compute max nesting depth for conditional, loop, match/switch, closure/lambda, and block constructs
  - [x] mark high-complexity function candidate when any configured complexity threshold is exceeded
  - [x] include per-language unsupported metrics as `not_available` instead of guessing from raw text
- [x] implement file-level metric collection:
  - [x] compute node count per file
  - [x] compute edge count per file
  - [x] compute average fan-in and fan-out for nodes in file
  - [x] compute import count from import nodes or import edges
  - [x] compute test coverage ratio as test nodes divided by non-test callable nodes when available
  - [x] flag large/highly connected files using configured percentile or threshold
- [x] implement module-level metric collection:
  - [x] group files/nodes by existing package/module ownership where available
  - [x] compute internal edge count within module
  - [x] compute external dependency edge count leaving module
  - [x] compute inbound dependency edge count entering module
  - [x] compute coupling score from external dependencies and inbound dependencies
  - [x] compute cohesion approximation from internal edges divided by possible internal relationships
- [x] compute distribution statistics:
  - [x] compute min, max, average, p50, p90, and p95 for fan-in, fan-out, LOC, cyclomatic complexity, cognitive complexity, nesting depth, branch count, file node count, and coupling
  - [x] detect outliers using configured percentile cutoffs
  - [x] include metric names, raw values, threshold values, and ranking reason in findings
- [x] add metrics tests:
  - [x] fan-in and fan-out on a known fixture graph
  - [x] dependency depth with cycle guard
  - [x] LOC from line ranges
  - [x] cyclomatic complexity from a fixture with branches and loops
  - [x] cognitive complexity increases with nested control flow
  - [x] max nesting depth from nested branch fixture
  - [x] unsupported language complexity metric reports `not_available`
  - [x] file import count
  - [x] module coupling score
  - [x] percentile/outlier detection

Why:
- risk, large-function, architecture, and pattern reports depend on these metrics
- metric definitions must be explicit before scoring uses them

#### 29.3 Large function finder

- [x] implement `find_large_functions()`:
  - [x] scan `Function`, `Method`, and test callable nodes by line span
  - [x] exclude test nodes by default
  - [x] include test nodes only when `include_tests = true`
  - [x] support repo-wide mode
  - [x] support file-scoped mode with one or more repo-relative file paths
  - [x] apply configured LOC threshold unless request overrides threshold
  - [x] cap results by configured or requested limit
- [x] implement high-complexity function filtering in same service:
  - [x] support `--complexity-threshold` for cyclomatic complexity
  - [x] support `--cognitive-threshold` for cognitive complexity
  - [x] support `--nesting-threshold` for max nesting depth
  - [x] include functions that exceed either size or complexity threshold
  - [x] allow `--mode large`, `--mode complex`, and `--mode large-or-complex`
- [x] rank large-function findings:
  - [x] primary sort by LOC descending
  - [x] boost changed-file relevance when changed-file input is provided
  - [x] boost high fan-in and high fan-out using metrics from 29.2
  - [x] boost package/module boundary crossings when module ownership is available
  - [x] tie-break by file path, line_start, and qualified name
  - [x] include ranking reason with LOC, complexity values, thresholds, fan-in, fan-out, and changed-file boost
- [x] return complete finding payload:
  - [x] file path
  - [x] qualified name
  - [x] display name
  - [x] node kind
  - [x] line_start and line_end
  - [x] LOC
  - [x] cyclomatic complexity when available
  - [x] cognitive complexity when available
  - [x] max nesting depth when available
  - [x] branch count when available
  - [x] threshold
  - [x] ranking reason
  - [x] provenance/freshness metadata
- [x] add surfaces:
  - [x] CLI `atlas insights large-functions`
  - [x] CLI `atlas insights large-functions --files ...`
  - [x] CLI flags `--threshold`, `--complexity-threshold`, `--cognitive-threshold`, `--nesting-threshold`, `--mode`, `--limit`, `--include-tests`, and `--json`
  - [x] MCP `find_large_functions` with same inputs and defaults as CLI JSON
  - [x] compact MCP default output suitable for agent review
- [x] add large-function tests:
  - [x] default threshold matches review risk summary threshold
  - [x] file-scoped filtering returns only requested files
  - [x] threshold override changes result set
  - [x] complexity threshold includes short but complex functions
  - [x] `--mode large` excludes short complex functions
  - [x] `--mode complex` excludes large simple functions
  - [x] `--mode large-or-complex` includes either category
  - [x] limit caps result count after ranking
  - [x] test-node include/exclude behavior
  - [x] stable sort ties
  - [x] CLI JSON and MCP JSON parity

Why:
- current review code only flags large changed functions; agents need direct repo/file discovery and ranked evidence
- one service prevents review, CLI, MCP, and insights thresholds from drifting

#### 29.4 Architecture analysis

- [x] build module-level graph:
  - [x] create module nodes from existing package/module/file ownership data
  - [x] aggregate file/node edges into module-to-module edges
  - [x] preserve source evidence edges that caused each module edge
  - [x] exclude ignored files/modules from config
  - [x] keep deterministic module IDs based on canonical repo paths or existing owner IDs
- [x] detect cycles:
  - [x] compute strongly connected components (SCC)
  - [x] identify cyclic dependencies from SCCs with more than one module or explicit self-cycle
  - [x] classify cycles as `local` when all modules share package/root and `cross-module` otherwise
  - [x] output at least one deterministic cycle path per finding
  - [x] include source file/node evidence for each cycle edge
- [x] enforce layer rules:
  - [x] parse configured layer names and path/module matchers
  - [x] map files and modules to layers
  - [x] reject invalid layer configs with clear diagnostics
  - [x] detect invalid dependency edges from lower/higher layers based on configured order
  - [x] output layer violation findings with source and target layer names
- [x] compute architecture health:
  - [x] compute coupling score per module using metrics from 29.2
  - [x] detect high-coupling modules using configured threshold
  - [x] detect tightly coupled clusters using SCC size and coupling score
  - [x] flag large/highly connected files using file metrics from 29.2
- [x] add architecture tests:
  - [x] SCC cycle detection
  - [x] local versus cross-module cycle classification
  - [x] deterministic cycle path output
  - [x] valid layer rule allows dependency
  - [x] invalid layer rule reports violation
  - [x] high-coupling module detection
  - [x] ignored module excluded

Why:
- architecture findings need module graph and metric foundation before risk and pattern analysis
- layer rules must be deterministic and config-driven

#### 29.5 Risk assessment engine

- [x] implement `assess_risk()`:
  - [x] accept symbol qualified name or resolved node target
  - [x] resolve ambiguous symbols using existing query/resolve-symbol behavior
  - [x] fail clearly when target cannot be resolved
  - [x] reuse metrics from 29.2 and architecture data from 29.4
  - [x] return one `RiskReport` for target node plus related evidence
- [x] score risk inputs:
  - [x] public API exposure
  - [x] fan-in
  - [x] fan-out
  - [x] cross-module dependency count
  - [x] test adjacency
  - [x] dependency depth
  - [x] unresolved edge count
  - [x] large-function flag, LOC, and complexity metrics when target is callable
  - [x] cycle participation when available
- [x] implement weighted formula:
  - [x] define default weights in config
  - [x] normalize final score to `0-100`
  - [x] classify `low`, `medium`, and `high` with configurable thresholds
  - [x] include factor contribution for each input
  - [x] include evidence nodes/edges for each non-zero factor
- [x] add risk tests:
  - [x] high fan-in increases score
  - [x] test adjacency lowers or mitigates score
  - [x] public API increases score
  - [x] unresolved edges increase score
  - [x] large function increases callable risk
  - [x] high cyclomatic complexity increases callable risk
  - [x] high cognitive complexity increases callable risk
  - [x] high nesting depth increases callable risk
  - [x] cycle participation increases score
  - [x] score normalization stays inside `0-100`
  - [x] low/medium/high boundaries are stable

Why:
- risk scoring should be explainable from deterministic graph and metric factors
- factor-level evidence lets users challenge or trust the score

#### 29.6 Pattern detection

- [x] detect duplicate or repeated graph patterns:
  - [x] find repeated call chains with same ordered simple-name sequence
  - [x] require minimum chain length from config
  - [x] group repeated chains by normalized sequence
  - [x] output files, qualified names, and edge evidence for each repeated chain
  - [x] skip chains crossing ignored modules/files
- [x] detect unused or isolated structures:
  - [x] find unused modules with no inbound edges outside their own module
  - [x] find isolated graph components with no incoming or outgoing external edges
  - [x] find orphan nodes with no meaningful inbound references and no test adjacency
  - [x] exclude entrypoints, tests, public APIs, and configured ignore patterns
  - [x] include blockers that prevent safe removal
- [x] detect high-centrality nodes:
  - [x] compute degree centrality from fan-in and fan-out
  - [x] identify hubs using percentile threshold
  - [x] identify bottlenecks with high fan-in and high fan-out
  - [x] include package/module context for each hub
- [x] detect deep chains:
  - [x] find call/dependency chains longer than configured depth
  - [x] cap traversal depth and node count
  - [x] avoid infinite loops through cycle guard
  - [x] output deterministic chain path and complexity reason
- [x] add pattern tests:
  - [x] repeated chain grouping
  - [x] unused module candidate with blockers
  - [x] isolated component detection
  - [x] hub and bottleneck detection
  - [x] deep-chain detection with cycle guard

Why:
- pattern findings must separate actionable candidates from graph noise
- blockers and evidence reduce false positives in dead-code and complexity reports

#### 29.7 Public surfaces and documentation

- [x] add CLI commands:
  - [x] `atlas insights architecture`
  - [x] `atlas insights metrics`
  - [x] `atlas insights risk <symbol>`
  - [x] `atlas insights patterns`
  - [x] `atlas insights large-functions`
  - [x] `atlas insights complex-functions`
  - [x] support `--json` on every insights command
  - [x] support `--limit` where findings can be large
  - [x] support `--config <path>` if existing commands support config override (not applicable; no existing config override on these surfaces)
- [x] add MCP tools or extend existing tool registry:
  - [x] expose architecture insights
  - [x] expose metrics insights
  - [x] expose risk assessment
  - [x] expose pattern detection
  - [x] expose `find_large_functions`
  - [x] expose complex-function filtering through the same tool or a dedicated `find_complex_functions` alias
  - [x] include freshness/provenance metadata in every response
  - [x] use compact default output with optional verbose details
- [x] document usage:
  - [x] update README or command reference for `atlas insights ...`
  - [x] update MCP tool docs
  - [x] update installed AGENTS instructions only if workflow changes (not needed; workflow unchanged)
  - [x] document threshold config and layer config examples
- [x] add surface tests:
  - [x] CLI JSON schema snapshots
  - [x] MCP response snapshots
  - [x] CLI/MCP parity for representative reports
  - [x] config override behavior
  - [x] freshness/provenance included

Why:
- service logic is only useful when CLI and MCP expose the same deterministic behavior
- parity tests prevent command/tool drift

#### 29.8 Phase 29 completion criteria

- [x] `InsightsEngine` exists with shared report primitives and deterministic sorting
- [x] metrics engine computes node, file, module, percentile, and outlier metrics
- [x] large-function finder works through service, CLI, and MCP with parity tests
- [x] function complexity metrics compute cyclomatic complexity, cognitive complexity, branch count, and nesting depth where parser support exists
- [x] high-complexity function discovery works through service, CLI, and MCP with parity tests
- [x] architecture analysis detects cycles, layer violations, coupling, and high-connectivity files
- [x] risk assessment returns explainable `0-100` scores with factor evidence
- [x] pattern detection reports repeated chains, unused/isolated structures, hubs, bottlenecks, and deep chains
- [x] config supports thresholds, inline layer rules, and ignore lists with validation
- [ ] config supports runtime-loaded external layer-rules files with validation
- [x] every insights report includes summary, findings, evidence, ranking reason, freshness, and provenance
- [x] tests cover cycle detection, coupling detection, layer violations, unused-node detection, large/complex-function ranking/filtering, risk scoring, outlier detection, and CLI/MCP parity
- [x] `cargo test -p atlas-engine` or owning insights crate test target passes
- [x] `cargo test -p atlas-cli` passes for insights commands
- [x] `cargo test -p atlas-mcp` passes for insights tools
- [x] `./scripts/test-workspace-summary.sh` passes

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

Before implementing any ICM checklist item:

- read the parent ICM section `Rules:` block
- treat `Rules:` bullets as mandatory constraints, not tasks
- do not mark `Rules:` bullets done; they are never checklist items
- implement checklist items only under `Implementation structure` and `completion criteria`
- if a checklist item conflicts with `Rules:`, follow `Rules:` and update the checklist wording before implementation

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

#### Patch R1 — Retrieval index lifecycle state

Patch R1 is shipped. See SHIPPED.md for details.

#### Patch R2 — Retrieval batching and chunk explosion guardrails

Patch R2 is shipped. See SHIPPED.md for details.

#### Patch R3 — Embedding dimension registry and freeze rules

Patch R3 is shipped. See SHIPPED.md for details.

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

Patch R5 is shipped. See SHIPPED.md for details.

#### Patch R6 — Retrieval/token-efficiency evaluation

Patch R6 is shipped. See SHIPPED.md for details.

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
- [ ] retrieval backend capabilities are validated, not assumed
- [x] stable `chunk_id` exists and is used for dedupe/reuse
- [x] retrieval/token-efficiency benchmarks are in place
- [ ] optional post-retrieval compaction is tracked as a late experiment only

---

### Retrieval Ranking Evidence Patch

Retrieval Ranking Evidence Patch is shipped. See SHIPPED.md for details.

---

### Graph/Content Companion Patch

Graph/Content Companion Patch is shipped. See SHIPPED.md for details.

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
- [ ] add configurable redaction-rules file surface for sanitization policy:
  - [ ] add config field for external redaction-rules file path under `.atlas/config.toml`
  - [ ] load redaction rules from referenced file at runtime so sanitization policy can change without recompiling
  - [ ] validate missing, unreadable, or malformed redaction-rules file with actionable config errors
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
- [ ] redaction policy supports runtime-loaded external rule files with validation
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

### Rust Parser Query-Backed Extraction Patch

Atlas Rust parser currently uses manual `node.kind()` AST walking for definition extraction, scope traversal, attribute detection, call extraction, and reference extraction. This works, but it makes grammar drift harder to audit and mixes syntax matching with Atlas graph semantics.

This patch moves Rust syntax extraction to Atlas-owned tree-sitter query files while keeping graph semantics in Rust code. `.scm` queries identify syntax facts; Rust code still builds qualified names, parent scopes, `Contains`, `Calls`, `References`, `Implements`, confidence tiers, and Atlas-specific metadata.

Use Helix Rust queries only as grammar reference for tree-sitter node names and scope patterns, especially `runtime/queries/rust/tags.scm` and `runtime/queries/rust/locals.scm`. Do not copy Helix query files verbatim unless license handling is added, because Helix is MPL-2.0. Atlas query files must be authored for Atlas captures.

#### Patch Q1 — Query infrastructure and behavior-preserving Rust extraction

- [x] add `packages/atlas-parser/queries/rust.scm` with Atlas-owned captures:
  - [x] capture `function_item` as `@atlas.definition.function`
  - [x] capture `function_signature_item` as `@atlas.definition.function_signature`
  - [x] capture `mod_item` as `@atlas.definition.module`
  - [x] capture `struct_item` as `@atlas.definition.struct`
  - [x] capture `enum_item` as `@atlas.definition.enum`
  - [x] capture `trait_item` as `@atlas.definition.trait`
  - [x] capture `const_item` as `@atlas.definition.const`
  - [x] capture `static_item` as `@atlas.definition.static`
  - [x] capture `impl_item` as `@atlas.definition.impl`
  - [x] capture impl `type` field as `@atlas.impl.type`
  - [x] capture impl `trait` field as `@atlas.impl.trait`
  - [x] capture item `name` fields with stable capture names such as `@atlas.name`
- [x] add shared query helper module in `packages/atlas-parser/src/query_helpers.rs`:
  - [x] expose helper to compile `tree_sitter::Query` from static query text and language
  - [x] expose helper to run `tree_sitter::QueryCursor` against a root node and source bytes
  - [x] expose helper to group captures by query match without losing capture order
  - [x] expose helper to read capture text using existing `ast_helpers::node_text`
  - [x] return parse/query errors as test-visible failures, not silent empty capture sets
- [x] wire `query_helpers` into `packages/atlas-parser/src/lib.rs`
- [x] refactor `packages/atlas-parser/src/lang/rust.rs` definition extraction:
  - [x] replace manual top-level `Walker::visit` `node.kind()` matching for definitions with query capture processing
  - [x] keep `parse_runtime::parse_tree` unchanged
  - [x] keep `LangParser`, `ParseContext`, `ParserRegistry`, and `ParsedFile` public interfaces unchanged
  - [x] keep file node creation unchanged
  - [x] keep existing qualified-name strings unchanged
  - [x] keep existing `NodeKind` choices unchanged
  - [x] keep existing `Contains` edge behavior unchanged
  - [x] keep existing same-file `Implements` edge behavior unchanged
  - [x] keep current same-file call resolver unchanged
  - [x] keep current same-file reference resolver unchanged
  - [x] keep current test-module and test-function detection behavior unchanged in Q1
- [x] add Rust-only internal syntax fact structs:
  - [x] `RustSyntaxFacts`
  - [x] `RustItem`
  - [x] `RustItemKind`
  - [x] `RustImpl`
  - [x] store source byte ranges or `tree_sitter::Node` handles needed to assign parent scopes
- [x] preserve scope semantics:
  - [x] root scope starts at repo-relative file path
  - [x] inline `mod foo { ... }` pushes module qualified name
  - [x] `impl Type { ... }` pushes impl qualified name
  - [x] methods inside impl remain `NodeKind::Method`
  - [x] nested module suffixes remain compatible with current `qualified_suffix`
- [x] add tests proving behavior preservation:
  - [x] existing `lang::rust` unit tests pass without expectation changes
  - [x] `packages/atlas-parser/tests/fixtures/rust/core.golden.json` does not change
  - [x] `packages/atlas-parser/tests/fixtures/rust/bad_syntax.golden.json` does not change
  - [x] malformed Rust source still returns file node and best-effort symbols
  - [x] query helper test fails clearly on invalid query text
  - [x] query helper test captures at least one Rust function from a small fixture

Why:
- separates syntax matching from Atlas graph semantics
- makes grammar drift easier to audit through one Rust query file
- preserves graph output before semantic changes
- creates shared query infrastructure for future parser migrations

#### Patch Q2 — Rust semantic extraction fixes on query foundation

- [x] improve trait body extraction:
  - [x] capture methods declared in `trait_item` bodies via `function_signature_item`
  - [x] emit trait method declarations using the existing `NodeKind` that best matches current graph model
  - [x] set trait method parent to the trait qualified name
  - [x] add `Contains` edge from trait node to trait method node
  - [x] keep trait methods distinct from free functions with same name
- [x] replace substring-based attribute detection:
  - [x] parse `attribute_item` structure or query captures instead of using `text.contains("test")`
  - [x] detect exact `#[test]` attribute for test functions
  - [x] detect exact `#[cfg(test)]` attribute for test modules
  - [x] do not treat `#[cfg(not(test))]` as test
  - [x] do not treat custom attributes containing the word `test` as test
- [x] improve impl target handling:
  - [x] normalize local type name from simple and scoped impl type paths
  - [x] normalize local trait name from simple and scoped trait paths
  - [x] keep same-file `Implements` edge only when local type and local trait targets resolve uniquely
  - [x] do not emit dangling `Implements` edges for external traits or external types
  - [x] keep existing confidence tier for same-file implements edges
- [x] move call syntax extraction to queries while preserving resolver semantics:
  - [x] capture `call_expression` function target
  - [x] capture method-call receiver and method name from Rust call target field expressions
  - [x] keep current same-file callee resolution rules
  - [x] keep unresolved call text target behavior
  - [x] keep current confidence values unless tests justify change
- [x] move reference syntax extraction to queries while preserving resolver semantics:
  - [x] capture `use_declaration` argument syntax
  - [x] capture type references from `type_identifier` and `scoped_type_identifier`
  - [x] ignore definition-name captures when producing references
  - [x] keep unique same-file target requirement for `References` edges
- [x] add focused semantic tests:
  - [x] trait method declaration is emitted and contained by trait
  - [x] free function and trait method with same name produce distinct qualified names
  - [x] `#[test] fn it_works()` emits `NodeKind::Test`
  - [x] `#[cfg(test)] mod tests { fn helper() {} }` marks nested function as test
  - [x] `#[cfg(not(test))] mod tests { fn helper() {} }` does not mark nested function as test
  - [x] custom attribute containing `test` does not mark function as test
  - [x] `impl local::Trait for local::Type` only emits `Implements` if local targets resolve uniquely
  - [x] `impl std::fmt::Display for Local` does not emit same-file `Implements` edge unless local trait exists
  - [x] generic function calls still resolve to same-file functions
  - [x] method calls still resolve to same-file methods when current resolver can disambiguate
  - [x] unresolved scoped calls still keep text target and `text` confidence tier

Why:
- query-backed extraction enables safer fixes after behavior-preserving migration
- trait method, attribute, and impl handling are current correctness gaps
- call/reference query captures reduce manual traversal without changing Atlas resolver policy

#### Patch Q completion criteria

- [x] Rust parser uses Atlas-owned `.scm` query captures for definition extraction
- [x] Rust parser public API and `ParsedFile` schema remain unchanged
- [x] Q1 preserves existing Rust golden outputs
- [x] Q2 adds semantic fixes with targeted regression tests
- [x] Helix Rust queries are referenced only for grammar guidance unless MPL-2.0 compliance is explicitly added
- [x] `cargo test -p atlas-parser lang::rust` passes
- [x] `cargo test -p atlas-parser --test parser_golden` passes
- [x] `cargo test -p atlas-parser` passes
- [x] `cargo clippy --workspace --all-targets --all-features -- -D warnings` passes
- [x] `cargo fmt --all` has been run

---

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

### Fuzz Patches

Fuzz Patches are shipped. See SHIPPED.md for details.

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

Patch T1 is shipped. See SHIPPED.md for details.

#### Patch T2 — Engine boundary enforcement and regression tests

Patch T2 is shipped. See SHIPPED.md for details.

#### Patch T3 — Future separate-connection read concurrency contract

Patch T3 is shipped. See SHIPPED.md for details.

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

Use this part to move Atlas MCP from hardcoded `2024-11-05` behavior to `2025-11-25` behavior across stdio, HTTP transport, schema metadata, auth, resources, tasks, and conformance.

Implementation order below is required. Do not start later phases until earlier phases land with tests.

Rules:

- treat Atlas as MCP server implementation first; do not add client-only `roots/*` or `sampling/*` reverse-RPC in this roadmap
- replace legacy HTTP+SSE behavior instead of preserving dual protocol paths
- keep stdio and HTTP transport behavior driven by one shared initialize, dispatch, schema, and error-classification layer
- keep one canonical protocol-version constant and derive metadata files, initialize responses, headers, and tests from it
- keep one canonical descriptor layer for tools, prompts, resources, completions, icons, and schemas; do not hand-maintain duplicate JSON blobs per transport
- classify malformed tool arguments as tool-execution failures when `tools/call` reached dispatch; reserve protocol errors for JSON-RPC envelope and method contract failures
- add automated coverage for every new capability before enabling it by default

### Phase MCP1 — Version baseline and initialize contract

Implement protocol-version upgrade first so all later phases share one spec baseline.

#### MCP1.1 Canonical protocol version source

- [x] create `packages/atlas-mcp/src/spec.rs` as single source of MCP protocol constants:
  - [x] add `pub const MCP_PROTOCOL_VERSION: &str = "2025-11-25"`
  - [x] add shared server identity builder returning `name`, `version`, and `description`
  - [x] add shared capability builder used by stdio and HTTP initialize handlers
- [x] remove duplicate hardcoded protocol-version strings from:
  - [x] `packages/atlas-mcp/src/transport.rs`
  - [x] `packages/atlas-mcp/src/transport_http.rs`
  - [x] `packages/atlas-cli/src/mcp_instance.rs`
  - [x] `packages/atlas-cli/tests/cli_quality_gates/core/serve.rs`
- [x] make instance metadata writers and readers use shared spec constant instead of inline string literals
- [x] add unit tests proving protocol version is emitted from one constant in transport and instance metadata paths

#### MCP1.2 Initialize request parsing and version negotiation

- [x] replace ad-hoc `initialize` param handling with typed request parsing in shared transport code:
  - [x] require `protocolVersion`
  - [x] require `capabilities`
  - [x] require `clientInfo.name`
  - [x] require `clientInfo.version`
- [x] reject initialize requests missing required fields with JSON-RPC `invalid_params` response and stable error body
- [x] negotiate exact protocol version `2025-11-25` only:
  - [x] reject older protocol versions instead of silently downgrading
  - [x] return clear unsupported-version error listing exact supported version
- [x] add `serverInfo.description` to initialize result
- [x] add `_meta` passthrough support on initialize result where latest schema allows it
- [x] add tests for:
  - [x] successful stdio initialize with full 2025-11-25 payload
  - [x] successful HTTP initialize with full 2025-11-25 payload
  - [x] missing `clientInfo` rejection
  - [x] unsupported protocol version rejection
  - [x] `serverInfo.description` presence

#### MCP1.3 Capability contract cleanup

- [x] replace inline initialize capability maps with typed capability structs serialized by shared builder
- [x] advertise only capabilities actually implemented after each later phase lands:
  - [x] remove capability claims that lack concrete method handlers
  - [x] gate future capability fields behind implementation-ready tests
- [x] add snapshot tests for stdio and HTTP initialize results so capability drift fails CI

#### MCP1 completion criteria

- [x] every initialize response and instance metadata file reports `2025-11-25`
- [x] empty `{}` initialize payload no longer passes quality gates
- [x] stdio and HTTP initialize payloads serialize from same shared builder
- [x] unsupported protocol-version requests fail deterministically with stable error JSON

### Phase MCP2 — Transport architecture migration to Streamable HTTP

Replace legacy HTTP+SSE transport with latest-spec Streamable HTTP semantics before adding newer server features.

#### MCP2.1 Route and session architecture

- [x] replace legacy `/` + `/sse` transport in `packages/atlas-mcp/src/transport_http.rs` with Streamable HTTP routes:
  - [x] `POST /mcp` for client requests
  - [x] `GET /mcp` for server event stream and polling reconnect
  - [x] `DELETE /mcp` for session termination
  - [x] keep `GET /health` as non-protocol liveness probe
- [x] create `packages/atlas-mcp/src/http_sessions.rs` for negotiated-session state:
  - [x] store negotiated protocol version
  - [x] store client info
  - [x] store per-session outbound event queue
  - [x] store stream identity and last event id
  - [x] store expiration timestamp and closed state
- [x] issue `Mcp-Session-Id` header on successful HTTP initialize response
- [x] require valid `Mcp-Session-Id` on all non-initialize HTTP requests
- [x] add tests for session creation, reuse, missing-session rejection, and session delete behavior

#### MCP2.2 Stream delivery, polling, and resumption

- [x] replace global broadcast stream with per-session outbound stream routing
- [x] assign deterministic event ids that encode session identity and per-session sequence number
- [x] support polling reconnect by honoring `Last-Event-ID` on `GET /mcp`
- [x] allow server-initiated stream closure without losing resumable state still inside retention window
- [x] add configurable event retention window and bounded queue size for resumed polling
- [x] drop expired retained events with deterministic `410 Gone` or equivalent latest-spec error path once resume window is exceeded
- [x] add tests for:
  - [x] resumed poll receives only missed events
  - [x] one session never receives another session's tool responses
  - [x] server-initiated disconnect can be resumed through `GET /mcp`
  - [x] expired `Last-Event-ID` fails with stable response

#### MCP2.3 Header, origin, and JSON-RPC envelope compliance

- [x] require `MCP-Protocol-Version: 2025-11-25` on all non-initialize HTTP protocol requests
- [x] reject mismatched `MCP-Protocol-Version` headers with deterministic protocol error response
- [x] remove permissive `CorsLayer::allow_origin(Any)` behavior
- [x] add explicit origin validation:
  - [x] allow absent `Origin` for trusted non-browser clients
  - [x] allow configured exact origins only when browser-origin requests are enabled
  - [x] return HTTP `403 Forbidden` for invalid origin per latest spec guidance
- [x] reject JSON-RPC batch arrays in stdio and HTTP transports
- [x] add regression tests for:
  - [x] missing version header on post-initialize HTTP request
  - [x] mismatched version header
  - [x] invalid origin returns `403`
  - [x] JSON-RPC batch request rejected on stdio
  - [x] JSON-RPC batch request rejected on HTTP

#### MCP2 completion criteria

- [x] HTTP transport no longer depends on legacy `/sse` route
- [x] every HTTP session is isolated by `Mcp-Session-Id`
- [x] resumed polling works with deterministic event ids
- [x] invalid origin and invalid version-header paths are covered by tests

### Phase MCP3 — Canonical descriptor, schema, and registry layer

Move tool and prompt metadata to one typed descriptor system before adding output schemas, icons, resources, and completions.

#### MCP3.1 Shared descriptor model

- [x] create `packages/atlas-mcp/src/descriptors.rs` holding typed descriptor structs for:
  - [x] tools
  - [x] prompts
  - [x] resources
  - [x] resource templates
  - [x] completions
- [x] include descriptor fields required by 2025-11-25 metadata surfaces:
  - [x] `name`
  - [x] `title`
  - [x] `description`
  - [x] `inputSchema`
  - [x] `outputSchema`
  - [x] `annotations`
  - [x] `icons`
  - [x] `_meta`
- [x] move `packages/atlas-mcp/src/tools/registry.rs` off hand-built JSON and onto descriptor serialization
- [x] add tests proving descriptor serialization is stable across stdio and HTTP `tools/list`

#### MCP3.2 JSON Schema 2020-12 upgrade

- [x] upgrade all MCP-advertised schemas to JSON Schema 2020-12:
  - [x] add `$schema: "https://json-schema.org/draft/2020-12/schema"`
  - [x] replace legacy schema patterns that rely on older draft assumptions
  - [x] validate every exported schema with test-time schema validation
- [x] decouple request parameter schemas from RPC method wiring:
  - [x] create standalone schema builders per method
  - [x] reuse same schema builder in registry output and request validation tests
- [x] add tests for:
  - [x] every `tools/list` entry includes valid 2020-12 `inputSchema`
  - [x] every `tools/list` entry includes valid 2020-12 `outputSchema`
  - [x] schema builder output matches registry snapshot

#### MCP3.3 Tool naming, titles, annotations, and icons

- [x] validate all exported tool names against latest tool-name guidance before registry emission
- [x] add human-readable `title` for each tool and prompt so identifiers stay machine-focused
- [x] add deterministic tool annotations where behavior is already known:
  - [x] mark read-only graph/query tools as read-only
  - [x] mark state-mutating tools like `build_or_update_graph`, `postprocess_graph`, `compact_session`, and `purge_saved_context` as state-changing
  - [x] mark destructive tools with destructive annotation when they delete persisted state
- [x] add static icon metadata constants for tools, prompts, resources, and resource templates; do not fetch icons at runtime
- [x] add tests for name validation, title presence, annotation presence, and icon metadata serialization

#### MCP3 completion criteria

- [x] descriptor JSON is generated from typed structs, not hand-built ad-hoc maps
- [x] all exported schemas validate as JSON Schema 2020-12
- [x] every tool and prompt has `title`
- [x] every tool has deterministic annotations and output schema coverage

### Phase MCP4 — Server features: resources, completions, structured output, and logging

Fill latest server-feature gaps using existing Atlas data and services instead of placeholder endpoints.

#### MCP4.1 Resource model and handlers

- [ ] create `packages/atlas-mcp/src/resources.rs` with read-only resource registry backed by existing Atlas data
- [ ] implement `resources/list` with deterministic ordering and cursor pagination
- [ ] implement `resources/read` for concrete Atlas resource families:
  - [ ] `atlas://health/status`
  - [ ] `atlas://graph/provenance`
  - [ ] `atlas://saved-context/{source_id}`
  - [ ] `atlas://docs/{file}#{heading}`
- [ ] implement `resources/templates/list` for URI templates matching supported dynamic resources:
  - [ ] saved-context resource template
  - [ ] docs-section resource template
- [ ] add MIME type, title, description, icons, and `_meta` for every resource and template entry
- [ ] add tests for `resources/list`, `resources/read`, template listing, pagination cursor stability, and not-found behavior

#### MCP4.2 Completion handlers

- [ ] implement `completion/complete` using descriptor-backed completion providers
- [ ] add completion providers for currently structured inputs:
  - [ ] `output_format`
  - [ ] review/context `intent`
  - [ ] known tool names in dispatcher-driven fields
  - [ ] docs-section resource template variables
- [ ] plumb `CompletionRequest.context` into provider logic where latest spec allows prior resolved variables
- [ ] add tests for exact-match completions, context-sensitive completions, empty-result stability, and cursor-less deterministic ordering

#### MCP4.3 Structured tool output and resource links

- [ ] create shared `ToolResultBuilder` used by stdio and HTTP tool-call paths
- [ ] emit `structuredContent` whenever tool output is native JSON object or array
- [ ] keep human-readable `content` summary alongside `structuredContent`
- [ ] add `resourceLinks` when tool output points at saved artifacts or docs sections already addressable through resource URIs
- [ ] route tool-argument validation failures through tool-execution error bodies once `tools/call` dispatch has started
- [ ] add tests for:
  - [ ] `structuredContent` presence on JSON-producing tools
  - [ ] content + structured-content parity on representative tools
  - [ ] resource links on saved-context-producing flows
  - [ ] invalid tool arguments return tool-execution classification, not protocol classification

#### MCP4.4 Logging capability implementation

- [ ] implement `logging/setLevel` and shared server log-level state
- [ ] emit MCP log notifications only after client initialization and only to subscribed session streams
- [ ] route stdio transport logging to `stderr` while preserving MCP log notifications for protocol-aware consumers
- [ ] add tests for log-level changes, stderr-only stdio diagnostics, and per-session log isolation on HTTP

#### MCP4 completion criteria

- [ ] `resources/list`, `resources/read`, and `resources/templates/list` all work with stable pagination
- [ ] `completion/complete` returns deterministic suggestions for supported inputs
- [ ] JSON-producing tools expose `structuredContent`
- [ ] logging capability is implemented, not only advertised

### Phase MCP5 — Authorization and protected resource metadata

Upgrade HTTP auth from static bearer gate to latest-spec protected-resource behavior.

#### MCP5.1 Auth config and validation module

- [ ] create `packages/atlas-mcp/src/auth.rs` for HTTP auth policy and token validation
- [ ] replace `ATLAS_HTTP_AUTH_TOKEN`-only runtime gate with config-driven protected-resource auth:
  - [ ] issuer URL
  - [ ] JWKS URL or OIDC discovery URL
  - [ ] audience/resource indicator
  - [ ] required scopes per route family
  - [ ] optional allowed origins list for browser callers
- [ ] validate config at startup and fail closed on inconsistent issuer/JWKS/resource settings
- [ ] add tests for invalid auth config, missing auth config under protected mode, and exact config parsing

#### MCP5.2 Protected resource metadata and discovery

- [ ] expose OAuth protected resource metadata endpoint at `/.well-known/oauth-protected-resource`
- [ ] publish metadata fields required for Atlas protected-resource discovery:
  - [ ] resource identifier
  - [ ] authorization server issuer URL
  - [ ] supported bearer methods
  - [ ] scope hints
- [ ] support OIDC discovery input so auth-server metadata can be resolved from standard discovery document when only issuer is configured
- [ ] add tests for protected-resource metadata body shape, issuer discovery, and startup failure on invalid discovery response

#### MCP5.3 WWW-Authenticate and incremental scope consent

- [ ] return `WWW-Authenticate` on unauthorized or insufficient-scope HTTP responses
- [ ] include resource-indicator and required-scope hints in `WWW-Authenticate`
- [ ] distinguish `401` unauthenticated from `403` authenticated-but-forbidden
- [ ] implement incremental scope challenge path for methods requiring stronger scopes than current token grants
- [ ] add tests for:
  - [ ] missing bearer token
  - [ ] invalid token
  - [ ] insufficient scope
  - [ ] forbidden origin with valid token
  - [ ] incremental scope challenge header contents

#### MCP5 completion criteria

- [ ] Atlas HTTP transport exposes protected-resource metadata endpoint
- [ ] bearer validation uses configured issuer/resource metadata, not static string equality
- [ ] unauthorized and insufficient-scope responses emit latest-spec `WWW-Authenticate` guidance
- [ ] origin rejection and auth rejection paths are covered independently

### Phase MCP6 — Elicitation and durable tasks

Add server-side advanced interaction only where Atlas already has long-running or destructive flows.

#### MCP6.1 Reverse-request plumbing for server-initiated interactions

- [ ] add shared reverse-request broker in `packages/atlas-mcp/src/transport.rs` for server-initiated requests tied to active client request scope
- [ ] enforce correlation so every server-initiated request is associated with triggering client request context
- [ ] add timeout, cancellation, and cleanup behavior for abandoned reverse requests
- [ ] add tests for correlation, timeout cleanup, and transport parity

#### MCP6.2 Elicitation support

- [ ] implement latest `elicitation/create` request and response schema handling in reverse-request broker
- [ ] support latest enum/result model:
  - [ ] titled enum values
  - [ ] untitled enum values
  - [ ] single-select enums
  - [ ] multi-select enums
  - [ ] default values on primitive fields
  - [ ] URL mode elicitation
- [ ] use elicitation for one concrete Atlas destructive flow:
  - [ ] require explicit elicitation confirmation before `purge_saved_context` runs without `session_id`
- [ ] add tests for single-select, multi-select, URL-mode, default-value, and confirmation-flow elicitation paths with mock client responses

#### MCP6.3 Durable tasks for long-running operations

- [ ] create `packages/atlas-mcp/src/tasks.rs` and persist task state in continuity-owned SQLite storage instead of process-only memory
- [ ] implement latest tasks extension methods and notifications exactly once through shared task service
- [ ] register long-running Atlas operations on task path:
  - [ ] `build_or_update_graph`
  - [ ] `postprocess_graph`
  - [ ] `doctor`
  - [ ] high-cost analysis operations when runtime exceeds configured defer threshold
- [ ] store task lifecycle fields:
  - [ ] task id
  - [ ] originating method
  - [ ] created time
  - [ ] updated time
  - [ ] status
  - [ ] progress snapshot
  - [ ] final result or final error
- [ ] support polling for task status, deferred result retrieval, and cancellation where underlying job is cancellable
- [ ] add tests for task creation, polling, completion, cancellation, restart-safe persisted task lookup, and task/result parity with synchronous tool output

#### MCP6 completion criteria

- [ ] server can issue latest-spec elicitation requests and validate typed responses
- [ ] destructive purge flow can require elicited confirmation
- [ ] long-running graph operations can return durable task handles and later final results
- [ ] reverse-request and task flows are covered on stdio and HTTP transports

### Phase MCP7 — Conformance, parity, and regression gates

Land broad regression coverage last so future MCP work cannot drift from 2025-11-25 behavior.

#### MCP7.1 Shared spec fixtures

- [ ] create `packages/atlas-mcp/tests/spec_2025_11_25/` integration suite with shared fixtures for stdio and HTTP
- [ ] add golden request/response fixtures for:
  - [ ] initialize success
  - [ ] initialize rejection
  - [ ] tools/list
  - [ ] tools/call structured output
  - [ ] resources/list
  - [ ] resources/read
  - [ ] logging/setLevel
  - [ ] protected-resource metadata
  - [ ] task lifecycle
  - [ ] elicitation round-trip
- [ ] make fixture harness assert exact protocol version, capability surface, error classification, and header behavior

#### MCP7.2 CLI and runtime parity gates

- [ ] update `packages/atlas-cli/tests/cli_quality_gates/core/serve.rs` to use full 2025-11-25 initialize payloads
- [ ] add CLI quality-gate checks for metadata-file protocol version, Streamable HTTP session headers, and removal of legacy SSE route assumptions
- [ ] add parity tests proving stdio and HTTP return equivalent bodies for same tool calls after removing transport-only envelope differences
- [ ] add negative tests proving advertised-but-unimplemented methods are absent from capability and descriptor output

#### MCP7.3 Drift-prevention checks

- [ ] add test that every advertised capability has method handlers and every method handler has descriptor coverage when required
- [ ] add test that every tool descriptor name resolves through dispatcher
- [ ] add test that every JSON-producing tool with `outputSchema` emits schema-compatible `structuredContent`
- [ ] add test that auth-protected HTTP routes all share same version-header, origin, and `WWW-Authenticate` enforcement
- [ ] add test that protocol-version constant, instance metadata, and initialize responses stay identical

#### MCP7 completion criteria

- [ ] integration suite covers stdio and HTTP for latest-spec happy path and failure path behavior
- [ ] legacy SSE-only assumptions are removed from tests and code
- [ ] descriptor, dispatcher, schema, and capability drift fail CI automatically
- [ ] MCP server behavior is locked to 2025-11-25 across versioning, transport, auth, metadata, resources, logging, elicitation, and tasks

---
