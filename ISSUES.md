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

- Part I. Remaining core delivery roadmap: Phase 17
- Part III. Remaining product expansion roadmap: Phases 29 through 31
- Part IV. Remaining context continuity roadmap: Phases CM12, CM14, and CM15
- Part V. Remaining focused follow-up patches: Retrieval Follow-Up Patch, Retrieval Ranking Evidence Patch, Graph/Content Companion Patch, Parity Surface Patch, Runtime Event Enrichment and Graph Linking Patch, Graph Readiness Source-of-Truth Patch, Context Escalation Contract Patch, Graph Store Corruption Recovery Patch

## Cross-Cutting Track Map

- Historical and analytics work: Phase 17, Phase 29, Phase 30, Phase 31
- Retrieval and search follow-ups: Retrieval Follow-Up Patch, Retrieval Ranking Evidence Patch, Graph/Content Companion Patch, Parity Surface Patch
- Context continuity and runtime memory: Phase CM12, Phase CM14, Phase CM15, Runtime Event Enrichment and Graph Linking Patch
- Graph safety and workflow: Graph Readiness Source-of-Truth Patch, Context Escalation Contract Patch, Graph Store Corruption Recovery Patch

---

## Part I — Core Delivery Roadmap

This part now tracks the remaining core delivery work: historical graph planning and implementation.

### Phase 17 — Historical Graphs (Atlas v3 / Phase 1.1)

Implement commit-linked historical graph storage and querying. Atlas should index selected git commits, persist graph snapshots for those commits, reuse unchanged file graph state, diff any two snapshots, and answer deterministic history questions about symbols, files, modules, dependencies, churn, and architectural evolution.

This phase must answer:

- when was this symbol introduced?
- how did this function evolve?
- when did this dependency appear or disappear?
- which commits changed this module most often?
- what architectural edges were added or removed between two points in time?
- what did the graph look like at a given commit?

Core design rules:

- correctness before optimization
- no LLM dependency
- deterministic, machine-parseable git operations
- exact commit SHA evidence in every historical result
- no branch name as durable identity
- reuse unchanged file graphs across history
- prefer add/remove over false continuity when identity is uncertain
- keep storage and indexing costs bounded by measurable policy

Scope:

- graph snapshotting per commit
- commit-linked graph metadata
- compact snapshot membership using reusable file graph state
- graph diffing across commits
- symbol, file, module, and dependency history queries
- architectural evolution and churn analysis
- CLI surfaces for build, update, status, diff, query, and prune

Non-goals:

- full git hosting integration
- PR review UI
- wiki/history summarizer
- blame replacement
- cloud history service
- rename-aware symbol lineage in first pass

For storage model use a hybrid model:

- current graph remains optimized for live queries
- historical layer stores snapshot metadata and compact graph state references
- file graph state is keyed by canonical repo path + file hash
- unchanged file graphs are reused across commits
- snapshot membership records which file hashes, nodes, and edges are active at each commit

#### 17.4 Slice 1 — Metadata foundation

Implement schema, deterministic git metadata ingestion, and `atlas history status` first. This slice creates durable history identity before parsing historical file contents.

Schema:

- [x] `repos` if not already present
- [x] `commits`
- [x] `graph_snapshots`
- [x] `snapshot_files`
- [x] indexes and uniqueness constraints for repo, commit, snapshot, and file membership lookups

`commits` columns:

- [x] `commit_sha`
- [x] `repo_id`
- [x] `parent_sha`
- [x] `author_name`
- [x] `author_email`
- [x] `author_time`
- [x] `committer_time`
- [x] `subject`
- [x] `message`
- [x] `indexed_at`

`graph_snapshots` columns:

- [x] `snapshot_id`
- [x] `repo_id`
- [x] `commit_sha`
- [x] `root_tree_hash` if available
- [x] `node_count`
- [x] `edge_count`
- [x] `file_count`
- [x] `created_at`
- [x] `completeness`
- [x] `parse_error_count`

`snapshot_files` columns:

- [x] `snapshot_id`
- [x] `file_path`
- [x] `file_hash`
- [x] `language`
- [x] `size`
- [x] enforce uniqueness on `(snapshot_id, file_path)`

Git wrappers:

- [x] `git rev-parse`
- [x] `git log`
- [x] `git show`
- [x] `git ls-tree`
- [x] `git diff-tree`
- [x] `git cat-file`

Commit selection:

- [x] latest commit only
- [x] bounded history window
- [x] explicit commit list
- [x] commit range
- [x] `HEAD`
- [x] branch ref
- [x] commit SHA
- [x] tag
- [x] merge base ranges later

Commit metadata:

- [x] commit SHA
- [x] parent SHA(s)
- [x] author name
- [x] author email
- [x] author time
- [x] committer time
- [x] commit message subject
- [x] full message later
- [x] branch/ref used during indexing
- [x] normalized timestamps
- [x] canonical repo-relative commit identity

Status command:

- [x] implement `atlas history status`
- [x] report indexed commit count
- [x] report latest indexed commit
- [x] report snapshot count
- [x] report shallow clone or missing ref warnings

Tests:

- [x] commit metadata stored correctly
- [x] deterministic parsing of git metadata
- [x] missing refs produce clear error

Safeguards:

- [x] shallow clones
- [x] detached HEAD
- [x] missing refs
- [x] rewritten history
- [x] submodules later

#### 17.5 Slice 2 — File graph reuse and historical build

Implement checkout-free file reconstruction, reusable file graph storage, and `atlas history build` for bounded commit ranges.

Historical graph storage:

- [x] persist file graph state keyed by file hash
- [x] persist or reuse content-addressed historical node state
- [x] persist or reuse content-addressed historical edge state
- [x] create `snapshot_nodes` if membership is stored separately
- [x] create `snapshot_edges` if membership is stored separately
- [x] map each commit snapshot to active file hashes
- [x] map each commit snapshot to active node/edge ids
- [x] avoid duplicating unchanged file graphs across commits
- [x] duplicate per-snapshot nodes/edges only if required for correctness, then optimize later

Checkout-free source access:

- [x] read file contents from commit using `git show <sha>:<path>` first
- [x] allow tree/blob plumbing optimization later
- [x] keep binary detection behavior
- [x] handle deleted paths correctly
- [x] canonicalize repo paths before hashing or identity use

Tracked file reconstruction:

- [x] reconstruct tracked file list for each commit
- [x] use `git ls-tree` for first version
- [x] use `git diff-tree` for incremental file-set replay later
- [x] prefer full enumeration correctness before diff replay speed

Build command:

- [x] accept `--since`
- [x] accept `--until`
- [x] accept `--max-commits`
- [x] accept `--branch`
- [x] accept `--commits`
- [x] enumerate selected commits
- [x] enumerate tracked files at each commit
- [x] compute file hash
- [x] reuse parsed file graph if identical hash already indexed
- [x] parse only new file hashes
- [x] write commit and snapshot metadata
- [x] attach file hash membership
- [x] attach node/edge membership
- [x] summarize commits processed, files reused, files parsed, nodes reused, elapsed time

Tests:

- [x] snapshot membership stored correctly
- [x] unchanged file graph reused across commits
- [x] modified file graph creates new membership state
- [x] binary file handling matches live indexing behavior
- [x] deleted path handled correctly

#### 17.6 Slice 3 — Incremental update, identity, lifecycle, reconstruction, and diff

Implement missing-commit updates, first-pass symbol/edge identity, lifecycle tables, graph reconstruction, and `atlas history diff`.

Update command:

- [x] detect commits not yet indexed
- [x] process only missing commits
- [x] support appending new commits on branch
- [x] detect rewritten history
- [x] detect force-push divergence
- [x] require explicit repair mode for divergent history

Identity:

- [x] use qualified name as primary identity key
- [x] pair with canonical file path and symbol kind
- [x] include signature hash where helpful
- [x] treat changed qualified name as remove + add
- [x] document lack of rename continuity
- [x] edge kind
- [x] source qualified name
- [x] target qualified name
- [x] canonical file path
- [x] optional line bucket or metadata hash

Lifecycle tables:

- [x] create `node_history`
- [x] create `edge_history`
- [x] `node_history` supports first/last snapshot, first/last commit, introduction commit, removal commit
- [x] `edge_history` supports first/last snapshot, first/last commit, introduction commit, removal commit

Lifecycle computation:

- [x] compute first/last seen snapshots
- [x] compute first/last seen commits
- [x] compute introduction/removal commits
- [x] record confidence/evidence for changed nodes and edges

Snapshot reconstruction:

- [x] reconstruct graph state for any indexed commit
- [x] reconstruct from file-hash references and snapshot membership
- [x] expose partial snapshot completeness and parse error counts

Diff command:

- [x] implement `atlas history diff <commit-a> <commit-b>`

Diff scopes:

- [x] file diff
- [x] node diff
- [x] edge diff
- [x] module diff
- [x] architecture diff

Node diff detects:

- [x] added nodes
- [x] removed nodes
- [x] changed nodes
- [x] line span changes
- [x] signature changes
- [x] modifier changes
- [x] test status changes
- [x] extra metadata changes

Edge diff detects:

- [x] added edges
- [x] removed edges
- [x] changed confidence tier
- [x] changed metadata

File diff detects:

- [x] added files
- [x] removed files
- [x] modified files
- [x] renamed files if git reports them
- [x] language and size changes

Architecture diff detects:

- [x] new dependency paths
- [x] removed dependency paths
- [x] new cycles
- [x] broken cycles
- [x] changed central hubs
- [x] changed coupling between modules

Tests:

- [x] node add/remove/change diff
- [x] edge add/remove diff
- [x] architecture diff detects new cycle
- [x] architecture diff detects broken cycle
- [x] rewritten history requires explicit repair mode

#### 17.7 Slice 4 — History queries and output contracts

Implement symbol, file, dependency, and module history query commands after snapshot reconstruction and lifecycle data exist.

Commands:

- [x] `atlas history symbol <qualified-name>`
- [x] `atlas history file <path>`
- [x] `atlas history dependency <source> <target>`
- [x] `atlas history module <module>`

Symbol history query:

- [x] show first/last appearance
- [x] show commits where changed
- [x] show signature evolution
- [x] show file path changes

File history query:

- [x] show all commits touching file
- [x] show node count over time
- [x] show edge count over time
- [x] show symbol additions/removals

Dependency history query:

- [x] show when edge first appeared
- [x] show when edge disappeared
- [x] show commits that added/removed dependency
- [x] show persistence duration

Module history query:

- [x] show node growth over time
- [x] show dependency growth over time
- [x] show coupling trend over time
- [x] show test adjacency over time later

Output structures:

- [x] `HistoricalSnapshot`
- [x] `GraphDiffReport`
- [x] `NodeHistoryReport`
- [x] `EdgeHistoryReport`
- [x] `FileHistoryReport`
- [x] `ModuleHistoryReport`

Every output includes:

- [x] summary fields
- [x] detailed findings
- [x] evidence snapshot ids
- [x] evidence commit SHAs
- [x] evidence node/edge identifiers
- [x] evidence canonical file paths

Tests:

- [x] symbol history query
- [x] file history query
- [x] dependency history query
- [x] module history trend query
- [x] JSON outputs include required evidence fields

#### 17.8 Slice 5 — Analytics, retention, and diagnostics

Implement churn/stability/trend reports, storage diagnostics, and pruning after core history queries work.

Churn metrics:

- [x] compute per-symbol change count, lifetime, add/remove frequency
- [x] compute per-file commits touched and graph delta size
- [x] compute per-module dependency churn and symbol churn

Stability indicators:

- [x] stable symbols
- [x] unstable symbols
- [x] frequently changing dependencies
- [x] architectural hotspots

Trend metrics:

- [x] file count growth
- [x] node count growth
- [x] edge count growth
- [x] module coupling trend
- [x] cycle count trend

Output structure:

- [x] `ChurnReport`

Retention controls:

- [x] keep all commits
- [x] keep latest N
- [x] keep tagged releases only
- [x] keep weekly snapshots
- [x] implement `atlas history prune`

Storage diagnostics:

- [x] commits stored
- [x] unique file hashes
- [x] deduplication ratio
- [x] DB size
- [x] snapshot density
- [x] storage growth with and without deduplication

Commands:

- [x] `atlas history prune`

Tests:

- [x] prune latest N
- [x] prune by age
- [x] prune by release/tag policy later
- [x] storage diagnostics report deduplication ratio

#### 17.9 CLI flags

- [x] `--repo`
- [x] `--db`
- [x] `--since`
- [x] `--until`
- [x] `--branch`
- [x] `--max-commits`
- [x] `--json`
- [x] `--stat-only`
- [x] `--full`
- [x] `--follow-renames` later

#### 17.10 Correctness and safeguards

- [x] keep historical indexing reproducible for same commit range
- [x] preserve exact commit SHA references
- [x] never rely on branch name as identity
- [x] make rewritten-history behavior explicit
- [x] prefer add/remove over false continuity when symbol identity is uncertain
- [x] handle missing commits in shallow clones
- [x] handle corrupted snapshot membership rows
- [x] handle parser failures at historical commits without aborting full run
- [x] track partial snapshot completeness
- [x] mark snapshots with parse errors
- [x] allow reindex/rebuild of individual snapshots

#### 17.11 Git history fixtures

- [x] symbol introduced
- [x] symbol removed
- [x] symbol modified
- [x] dependency introduced
- [x] dependency removed
- [x] file renamed
- [x] module split/merge later

#### 17.12 Performance checks

- [x] commits/sec
- [x] snapshot reconstruction speed
- [x] graph diff speed
- [x] symbol history query latency
- [x] storage growth with and without deduplication

#### 17.13 Later optimization backlog

- [x] commit-to-commit diff replay instead of full file enumeration
- [x] blob-level cache
- [x] parser result cache keyed by blob hash
- [x] compressed membership encoding
- [x] partial snapshot materialization

#### 17.14 Completion criteria

Phase 17 is complete when all of these are true:

- [x] Atlas can persist commit-linked graph snapshots
- [x] unchanged file graphs are reused across commits
- [x] Atlas can reconstruct graph state for any indexed commit
- [x] Atlas can diff two snapshots structurally
- [x] Atlas can answer symbol/file/module/dependency history queries
- [x] Atlas can report churn/stability/trend metrics
- [x] storage growth is measurable and bounded by policy
- [x] all historical outputs are deterministic and evidence-backed

---

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

#### 31.1 Wiki / docs generation (CLI command)

- [ ] generate Markdown docs
- [ ] module pages
- [ ] function pages
- [ ] static site export
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

#### Phase CM12 — Predictive Context

##### Goal

Make context proactive instead of reactive.

##### Tasks

- [ ] predict next likely user action
- [ ] prefetch relevant artifacts
- [ ] preload context based on recent activity
- [ ] cache frequently accessed context

##### Output

- faster, smarter responses
- reduced latency for common workflows

##### CLI and MCP rollout follow-up

- [ ] wire predictive prefetch into `atlas context`, `query_graph`, and resume flows rather than leaving it as background-only logic
- [ ] expose debug or metadata fields showing what was prefetched and why in CLI JSON and MCP responses
- [ ] ensure predictive caches respect existing session and saved-context boundaries

---

#### Phase CM14 — Decision Memory

##### Goal

Persist and reuse decisions.

##### Tasks

- [x] create decision event types
- [x] link decisions to artifacts
- [x] store reasoning behind decisions
- [x] retrieve decisions for future tasks
- [x] avoid recomputing prior conclusions

##### Output

- system remembers why decisions were made

##### CLI and MCP rollout follow-up

- [x] emit decision events from CLI, context, reasoning, and MCP adapter flows
- [x] route `atlas context` and saved-context retrieval through decision lookup when relevant prior conclusions exist
- [x] expose decision retrieval through CLI or MCP surface with linked evidence and artifact references

---

#### Phase CM15 — Agent-Aware Context (Optional)

##### Goal

Support multi-agent workflows.

##### Tasks

- [ ] implement per-agent memory partitions
- [ ] track delegated tasks
- [ ] merge outputs across agents
- [ ] track agent responsibilities

##### Output

- scalable multi-agent memory system

##### CLI and MCP rollout follow-up

- [ ] add agent partition identifiers to session, context, and saved-context APIs
- [ ] extend MCP tools to read/write per-agent memory partitions and merged views intentionally
- [ ] expose delegated-task and responsibility summaries through CLI or MCP status/context surfaces

---

##### Completion Criteria

- [ ] memory is curated, not just stored
- [ ] retrieval is semantic-aware
- [ ] system can recall past sessions
- [ ] context selection is optimized
- [ ] decisions persist and are reused
- [ ] system improves over time

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

- [ ] create embedding provider registry metadata
- [ ] persist:
  - [ ] provider name
  - [ ] model name
  - [ ] embedding dimension
  - [ ] discovered_at
  - [ ] index schema version
- [ ] require dimension to be frozen at index creation time
- [ ] reject insert/search if dimension does not match active retrieval index
- [ ] cache discovered dimensions per provider/model
- [ ] add CLI / diagnostics surface for current embedding config
- [ ] add tests for:
  - [ ] dimension mismatch on insert
  - [ ] dimension mismatch on query
  - [ ] provider switch with incompatible existing index
  - [ ] explicit rebuild requirement after dimension change

Why:
- avoids one of the most common hybrid/vector indexing failure modes
- keeps retrieval layer deterministic and debuggable

#### Patch R4 — Retrieval backend capability flags

Atlas should make backend capability checks explicit instead of assuming all retrieval backends support all modes.

- [ ] define retrieval backend capability model
- [ ] support capability flags for:
  - [ ] lexical FTS
  - [ ] dense vector search
  - [ ] hybrid lexical + vector fusion
  - [ ] sparse / BM25-native retrieval
  - [ ] metadata filtering
- [ ] validate requested retrieval mode against backend capabilities before query/index
- [ ] disable unsupported hybrid mode automatically with explicit warning
- [ ] ensure MCP/CLI surfaces report active retrieval mode clearly
- [ ] add tests for:
  - [ ] lexical-only backend
  - [ ] dense-only backend
  - [ ] hybrid-capable backend
  - [ ] unsupported mode request fails cleanly

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

Atlas already exposes query scores, active query mode, global `explain_query` ranking factors, provenance, and truncation metadata. What is still missing is a first-class retrieval contract that explains why each returned result ranked where it did. A result-level score alone is not enough for agents to distinguish exact matches, fuzzy repairs, package/path boosts, changed-file boosts, graph expansion, and hybrid/vector fusion.

#### Patch Q1 — Result-level ranking evidence model

- [x] add compact `RankingEvidence` / `ScoreEvidence` model for ranked retrieval results
- [x] attach evidence to graph/search result structs without replacing numeric score
- [ ] include fields for:
  - [x] base retrieval mode (`fts5`, `regex_structural_scan`, `vector`, `hybrid`, `graph_expand`)
  - [x] raw score before boosts when available
  - [x] final score
  - [x] matched fields (`name`, `qualified_name`, `file_path`, `content`, `embedding`)
  - [x] exact name match
  - [x] exact qualified-name match
  - [x] prefix match
  - [x] fuzzy correction and edit distance
  - [x] kind boost
  - [x] public/exported boost
  - [x] same-directory boost
  - [x] same-language boost
  - [x] recent-file boost
  - [x] changed-file boost
  - [x] graph expansion hop distance
  - [x] hybrid/RRF contributing sources and ranks
- [x] keep evidence compact and stable for MCP JSON output
- [x] add serde round-trip tests for evidence schema

Why:
- agents need to know why a result won, not only that it scored higher
- global `ranking_factors` explain query mode, but not individual result ranking

#### Patch Q2 — Capture evidence during ranking

- [x] update `apply_ranking_boosts` to record which boosts fired per result
- [x] update fuzzy relaxed-candidate path to record:
  - [x] corrected/matched term
  - [x] edit distance
  - [x] fuzzy threshold
- [x] update exact-hit merge path to preserve exact-match evidence
- [x] update graph expansion to record hop distance and seed source
- [x] update hybrid/RRF merge to record:
  - [x] FTS rank contribution
  - [x] vector rank contribution
  - [x] RRF score contribution
- [x] ensure evidence survives result merging and deduplication
- [x] add tests for each evidence source and merge precedence

Why:
- evidence must be produced at scoring time while the ranking decision is known
- reconstructing explanation after sorting is lossy and easy to get wrong

#### Patch Q3 — Surface evidence in CLI and MCP retrieval outputs

- [x] include ranking evidence in MCP `query_graph` results
- [x] include ranking evidence in MCP `batch_query_graph` per-query results
- [x] include ranking evidence in `explain_query` matches
- [x] include ranking evidence in CLI `atlas query --json`
- [x] keep human CLI output compact:
  - [x] show score as today
  - [x] optionally show top evidence labels when verbose/debug mode is enabled
- [x] document stable evidence labels and meanings
- [x] add snapshot tests for MCP output shape

Why:
- query-mode observability should be part of normal retrieval output, not only debug output
- downstream tools can make better escalation and trust decisions from structured evidence

#### Patch Q4 — Evidence contract for context and review ranking

- [x] decide whether review/context `relevance_score` also gets evidence
- [x] if yes, add context-ranking evidence for:
  - [x] direct target
  - [x] changed symbol
  - [x] caller/callee neighbor
  - [x] test adjacency
  - [x] impact-score contribution
  - [x] saved-context/session boost
- [x] surface context-ranking evidence only where payload budget allows
- [x] document whether graph search evidence and context relevance evidence are separate contracts
- [x] add tests for direct target and changed-file evidence in context results

Why:
- search ranking and context ranking are related but not identical
- review flows need evidence for why context was included, not only why a symbol matched search

#### Patch Q completion criteria

- [x] every ranked graph/search result can include compact structured ranking evidence
- [x] query boosts, fuzzy correction, graph expansion, and hybrid/RRF all record evidence
- [x] MCP `query_graph`, `batch_query_graph`, and `explain_query` expose evidence
- [x] CLI JSON exposes evidence without bloating human output
- [x] evidence labels are documented and covered by tests
- [x] context/review relevance evidence is explicitly included or deferred with documented rationale

---

### Graph/Content Companion Patch

Atlas already has graph search for symbols and relationships plus file/content/template/text-asset search for prompts, docs, config, SQL, and templates. The missing design rule is that these are coordinated retrieval surfaces, not separate universes or a simple fallback chain. Graph answers code structure questions; content lookup answers non-code and context-adjacent questions; the context engine should merge both under one bounded selection, ranking, evidence, and truncation policy.

#### Patch N1 — Declare graph/content lookup contract

- [ ] document canonical responsibility split:
  - [ ] graph search answers symbols, ownership, callers, callees, tests, imports, and structural relationships
  - [ ] content lookup answers prompts, docs, config, SQL, templates, logs, and embedded text assets
  - [ ] saved-context lookup answers prior Atlas outputs and session artifacts
  - [ ] context engine decides how these surfaces combine for a task
- [ ] define graph/content lookup as companion systems, not fallback-only systems
- [ ] define when both should be queried for one request:
  - [ ] review changes touching config or templates
  - [ ] symbols whose behavior depends on prompts or SQL
  - [ ] docs/spec questions tied to implementation files
  - [ ] agent/task questions needing saved context plus graph facts
- [ ] document anti-patterns:
  - [ ] broad file search before graph resolution for symbol questions
  - [ ] graph-only review when changed files include config/templates/prompts
  - [ ] content-only answers for structural dependency questions
  - [ ] separate unbounded result lists from graph and content tools

Why:
- non-code artifacts are first-class context when they affect behavior
- graph-first should not mean content-blind

#### Patch N2 — Unified bounded selection policy

- [ ] define one context selection policy for mixed graph/content results:
  - [ ] direct graph targets first
  - [ ] changed files and changed symbols next
  - [ ] adjacent config/templates/prompts/SQL tied to changed files next
  - [ ] caller/callee/test evidence next
  - [ ] saved-session artifacts only when relevant to current task
- [ ] apply shared budgets across mixed results:
  - [ ] max graph nodes
  - [ ] max graph edges
  - [ ] max content assets
  - [ ] max saved artifacts
  - [ ] max total payload bytes/tokens
- [ ] ensure truncation reports mixed omissions:
  - [ ] omitted graph nodes
  - [ ] omitted graph edges
  - [ ] omitted content assets
  - [ ] omitted saved artifacts
  - [ ] omitted bytes/tokens
- [ ] add deterministic tie-breakers when graph and content scores compete
- [ ] add tests for mixed graph/content truncation order

Why:
- separate bounded lists can still create an unbounded combined context
- agents need one budget story for the final answer context

#### Patch N3 — Coordinated ranking and evidence

- [ ] define a mixed-result ranking envelope with source kind:
  - [ ] `graph_node`
  - [ ] `graph_edge`
  - [ ] `file_asset`
  - [ ] `content_match`
  - [ ] `template`
  - [ ] `text_asset`
  - [ ] `saved_context`
- [ ] normalize ranking signals across surfaces:
  - [ ] exact symbol match
  - [ ] graph distance
  - [ ] changed-file boost
  - [ ] same package/directory boost
  - [ ] BM25/content match score
  - [ ] trigram/fuzzy correction
  - [ ] proximity/title/path rerank
  - [ ] session recency/relevance
- [ ] expose why each mixed item was selected through ranking evidence
- [ ] include `selection_reason` for both graph and content assets
- [ ] add tests proving config/template/prompt matches can be selected with graph evidence when relevant

Why:
- mixed context should be explainable, not an opaque concatenation of tool outputs
- ranking evidence must work for content assets as well as graph nodes

#### Patch N4 — MCP and prompt workflow integration

- [ ] update MCP tool descriptions to describe graph/content companion rules
- [x] improve `search_content` invalid-regex guidance:
  - [x] keep `is_regex=true` strict; invalid regex returns error instead of fallback search
  - [x] include escaped-regex suggestion for literal metacharacters, for example `Context \\{`
  - [x] suggest `is_regex=false` when caller wants literal text search
  - [x] add MCP regression test for invalid pattern like `Command::Context|Context {`
- [ ] update `review_change` prompt to query content assets when changed files include docs/config/templates/prompts/SQL
- [ ] update `inspect_symbol` prompt to look for context-adjacent assets only when graph evidence suggests dependency
- [ ] update installed AGENTS instructions:
  - [ ] graph tools first for structure
  - [ ] content tools as companion lookup for non-code assets
  - [ ] context engine should merge both under bounded policy
- [ ] add prompt/registry snapshot tests for companion-contract wording

Why:
- agents follow surface contracts more reliably than implicit architecture
- prompt and install docs should not describe content lookup as mere fallback

#### Patch N completion criteria

- [ ] graph/content companion contract is documented as a design rule
- [ ] mixed graph/content context has one bounded selection policy
- [ ] mixed results expose source kind, selection reason, ranking evidence, and truncation metadata
- [ ] MCP prompts, tool descriptions, README, and installed AGENTS instructions agree
- [ ] tests cover mixed code + config/template/prompt/doc context assembly

---

### Parity Surface Patch

Atlas already has pieces of the upstream parity surface: Markdown heading graph nodes, content search over docs, large-function risk flags in review summaries, and explicit build/update plus flows/communities commands. Missing work is to turn those pieces into first-class CLI/MCP surfaces with shared service logic, compact output, and parity tests.

#### Patch PS1 — Docs section lookup parity

- [ ] add docs-section lookup service over indexed project docs:
  - [ ] resolve doc by canonical repo path
  - [ ] resolve section by Markdown heading path / slug
  - [ ] return section body with bounded child-heading context
  - [ ] include heading level, line range, file hash, and truncation metadata
  - [ ] reuse existing Markdown parser heading nodes and content-store/file reads where possible
- [ ] add CLI surface:
  - [ ] `atlas docs-section <path> --heading <heading-path-or-slug>`
  - [ ] `atlas docs-section <path> --line <line>`
  - [ ] `--json`, `--max-bytes`, and stable not-found errors
- [ ] add MCP `get_docs_section`:
  - [ ] same inputs and defaults as CLI JSON
  - [ ] TOON/JSON output parity
  - [ ] provenance and freshness metadata
- [ ] add CLI/MCP parity tests:
  - [ ] nested headings
  - [ ] duplicate heading slugs
  - [ ] missing file / missing heading
  - [ ] max-byte truncation
  - [ ] stale graph warning when doc file changed

Why:
- current docs support can find files and headings, but cannot fetch one section as a stable agent-facing unit
- review/query workflows need precise docs excerpts without broad file scans

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

- [x] define postprocess orchestration service for derived graph analytics:
  - [x] run after build/update without reparsing source files
  - [x] refresh derived analytics such as flows, communities, architecture metrics, query hints, and large-function summaries
  - [x] support full and changed-only modes where data dependencies allow
  - [x] record started/finished/failed state and per-stage counts/durations
  - [x] keep failures bounded and machine-readable
- [x] add CLI surface:
  - [x] `atlas postprocess`
  - [x] `atlas postprocess --changed-only`
  - [x] `atlas postprocess --stage <name>`
  - [x] `--json`, `--dry-run`, and stable error contract
- [x] add MCP `postprocess_graph`:
  - [x] same stage/mode controls as CLI JSON
  - [x] compact stage summary by default
  - [x] provenance, readiness, and freshness metadata
- [x] add CLI/MCP parity tests:
  - [x] no-op repo with no graph
  - [x] full postprocess after build
  - [x] changed-only postprocess after update
  - [x] single-stage execution
  - [x] stage failure surfaces same error code in CLI JSON and MCP

Why:
- build/update should stay focused on scan, parse, and persistence
- derived analytics need explicit orchestration instead of hidden side effects or ad hoc commands

#### Patch PS completion criteria

- [ ] `get_docs_section`, `find_large_functions`, and `postprocess_graph` exist as MCP tools with matching CLI surfaces
- [ ] all three surfaces share service-layer implementations with no duplicated ranking, truncation, or error rules
- [ ] CLI JSON and MCP JSON are parity-tested for representative fixtures
- [ ] README, MCP reference, installed AGENTS instructions, and prompt workflows document the new surfaces consistently
- [ ] graph freshness/readiness metadata appears on every new graph-backed MCP response

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

Atlas has persisted build state, graph freshness checks, health/debug tools, provenance, and adapter metadata, but there is no explicit invariant that one subsystem owns the answer to: "is the graph ready, searchable, and current enough to use?" That decision must not drift across CLI status, MCP status, query tools, impact analysis, review context, and adapters.

#### Patch S1 — Canonical graph readiness record

- [ ] define a canonical `GraphReadiness` / `GraphState` model in shared core or graph service code
- [ ] include fields:
  - [ ] `repo_root`
  - [ ] `db_path`
  - [ ] `db_exists`
  - [ ] `db_open_error`
  - [ ] `build_state`
  - [ ] `build_last_error`
  - [ ] `graph_built`
  - [ ] `graph_queryable`
  - [ ] `graph_current`
  - [ ] `stale_index`
  - [ ] `pending_graph_changes`
  - [ ] `integrity_state`
  - [ ] `error_code`
  - [ ] `message`
  - [ ] `suggestions`
  - [ ] `last_indexed_at`
  - [ ] `indexed_file_count`
- [ ] distinguish readiness dimensions:
  - [ ] built versus missing
  - [ ] queryable versus blocked
  - [ ] current versus stale
  - [ ] corrupt/inconsistent versus merely stale
  - [ ] graph readiness versus retrieval/content index readiness
- [ ] make this record the only source allowed to decide graph readiness
- [ ] add tests for every readiness class and field derivation

Why:
- prevents drift between build lifecycle state, status output, query behavior, and adapter metadata
- makes readiness a contract instead of scattered boolean logic

#### Patch S1.5 — Graph execution safety states

- [ ] define canonical graph execution states:
  - [ ] `fresh` — graph is built, queryable, current, and integrity-clean
  - [ ] `stale` — graph is queryable but behind graph-relevant working-tree changes
  - [ ] `partial` — graph is queryable but build/update/indexing stopped early or degraded
  - [ ] `corrupt` — graph has SQLite integrity errors, schema mismatch, orphan nodes, or dangling edges
- [ ] define feature behavior by state:
  - [ ] `fresh` -> full graph-backed features enabled
  - [ ] `stale` -> warn and allow graph-backed answers with freshness metadata
  - [ ] `partial` -> allow limited features only; block answers requiring complete graph facts
  - [ ] `corrupt` -> block graph-backed answers and require rebuild/quarantine flow
- [ ] define explicit override policy:
  - [ ] stale graph may run only when default policy allows stale reads or caller passes `--allow-stale` / MCP `allow_stale=true`
  - [ ] partial graph may run only for tools with documented degraded behavior or caller passes `--allow-partial` / MCP `allow_partial=true`
  - [ ] corrupt graph has no override for graph-backed answers
  - [ ] every allowed stale/partial response must include `safe_to_answer`, execution state, and freshness/degraded metadata
- [ ] define which tools are allowed in `partial` state:
  - [ ] status/debug/doctor allowed
  - [ ] direct symbol lookup allowed only when result provenance is complete enough
  - [ ] impact/review/analyze flows blocked or degraded unless completeness requirements are met
  - [ ] traversal blocked when missing edges could make answer unsafe
- [ ] expose execution state in CLI/MCP readiness output
- [ ] make query, impact, review, context, and analyze tools consume execution state before graph reads
- [ ] add tests for each state and allowed/blocked feature behavior

Why:
- agents need one simple safety state before deciding whether graph facts are usable
- stale, partial, and corrupt graphs require different behavior

#### Patch S2 — Route CLI graph tools through canonical readiness

- [ ] update `atlas status` to emit canonical readiness directly
- [ ] update `atlas doctor` to reference canonical readiness instead of partially recomputing it
- [ ] update `atlas query` to consult readiness before search
- [ ] update `atlas impact` to consult readiness before impact traversal
- [ ] update `atlas review-context` to consult readiness before context assembly
- [ ] update reasoning/refactor graph-backed commands to consult readiness before graph reads
- [ ] define command behavior per readiness state:
  - [ ] fresh graph: full features enabled
  - [ ] missing graph: fail with build suggestion
  - [ ] interrupted/failed build: fail with lifecycle suggestion
  - [ ] stale graph: warn + allow only by configured policy or explicit stale override
  - [ ] partial graph: allow limited features only by documented degraded policy or explicit partial override
  - [ ] corrupt/inconsistent graph: fail closed
- [ ] add CLI tests proving all graph-backed commands consume same readiness decision

Why:
- query, impact, and review must not infer readiness from `Store::open` alone
- status output and command behavior must agree

#### Patch S3 — Route MCP and adapters through canonical readiness

- [ ] update MCP `status` to surface canonical readiness, not redefine it
- [ ] add readiness block to graph-backed MCP responses:
  - [ ] `query_graph`
  - [ ] `get_context`
  - [ ] `get_impact_radius`
  - [ ] `get_review_context`
  - [ ] `get_minimal_context`
  - [ ] `symbol_neighbors`
  - [ ] `traverse_graph`
  - [ ] reasoning/refactor analysis tools
- [ ] replace ad hoc provenance/freshness readiness inference with canonical readiness fields
- [ ] keep provenance as identity metadata only:
  - [ ] `repo_root`
  - [ ] `db_path`
  - [ ] `indexed_file_count`
  - [ ] `last_indexed_at`
- [ ] ensure adapters never decide graph readiness independently
- [ ] add MCP tests proving graph-backed tools surface identical readiness for same repo/db

Why:
- MCP should surface graph readiness, not become another readiness authority
- provenance and readiness are related but not the same contract

#### Patch S completion criteria

- [ ] one canonical graph readiness model exists
- [ ] CLI status, doctor, query, impact, and review consume that model
- [ ] MCP graph-backed tools surface that model and do not redefine readiness
- [ ] adapters only report or forward readiness, never compute their own
- [ ] stale/queryable and corrupt/blocked states are distinct
- [ ] fresh/stale/partial/corrupt execution states map to explicit allowed/blocked features
- [ ] tests prove all graph-backed paths agree on readiness for same repo and DB

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

---
