# Atlas Execution Subsystem — V3

## V3 goal

Move from V2:

- tool-aware compression
- artifact retrieval
- basic graph/session enrichment

to:

- **execution as a first-class Atlas signal source**
- **execution-aware continuity**
- **retrieval over execution artifacts**
- **graph-linked execution intelligence**
- **agent workflows that use execution results without flooding context**

V3 should make Atlas execution feel like part of the platform, not an add-on.

---

# V3 definition

V3 is complete when Atlas can do all of these reliably:

- execute bounded commands safely
- compress outputs with tool-aware logic
- store and retrieve large outputs as artifacts
- search across execution artifacts intelligently
- attach execution outputs to files/packages/symbols when possible
- use execution history in session resume/handoff
- give agents execution-aware next-step guidance
- surface execution-derived freshness and review hints
- support multi-step but still bounded “analyze output then retrieve details” workflows

---

# V3 scope

## Included
- execution-aware session continuity
- artifact search/ranking
- graph-linked file/package/symbol extraction
- execution-derived freshness/review hints
- better MCP workflows around execution artifacts
- improved result shaping for agents

## Not included
- full system-wide shell interception
- arbitrary command rewriting
- PTY / interactive shells
- unbounded orchestration engine
- full RTK command parity across every ecosystem

---

# Phase V3.1 — Execution artifacts become searchable knowledge

## Goal
Make stored command outputs act like a searchable Atlas knowledge layer, not just blobs.

### TODO
- [ ] index execution artifacts into a local retrieval store
- [ ] support artifact chunking strategies by content kind:
  - [ ] plain text by line windows/sections
  - [ ] diff by file/hunk
  - [ ] JSON by object path / array batches
  - [ ] test output by test case / failure block
  - [ ] build/lint output by file/error block
- [ ] attach artifact metadata:
  - [ ] `source_id`
  - [ ] `session_id`
  - [ ] `command`
  - [ ] `cwd`
  - [ ] `output_kind`
  - [ ] `exit_code`
  - [ ] `created_at`
  - [ ] `related_files`
  - [ ] `related_packages`
  - [ ] `related_symbols`
- [ ] add retrieval modes:
  - [ ] exact source-id fetch
  - [ ] search by query
  - [ ] search by session
  - [ ] search by file/package
  - [ ] search by command family
- [ ] support bounded previews by default

### Acceptance
- [ ] large execution outputs can be searched meaningfully later
- [ ] artifact search is better than raw grep over stored logs
- [ ] search results return previews and pointers, not raw blobs

---

# Phase V3.2 — Execution-aware session continuity

## Goal
Make execution history a first-class input to resume state.

### TODO
- [ ] extend session reducer to track:
  - [ ] last successful command
  - [ ] last failed command
  - [ ] recent execution artifacts
  - [ ] recent command family
  - [ ] unresolved build/test/lint failures
  - [ ] whether next step likely requires artifact retrieval
- [ ] include execution summaries in resume snapshots when relevant
- [ ] persist execution-derived handoff hints:
  - [ ] “last build failed in package X”
  - [ ] “test failure log stored under source_id Y”
  - [ ] “graph likely stale after command Z”
- [ ] add artifact refs to session resume
- [ ] support “resume by retrieval” rather than replaying raw logs

### Acceptance
- [ ] resume snapshots can mention execution failures/useful outputs without inlining them
- [ ] agents can continue a build/test/debug workflow after compaction using artifact refs

---

# Phase V3.3 — Graph-linked execution intelligence

## Goal
Turn command outputs into graph-aware Atlas signals.

### TODO
- [ ] improve file extraction from outputs:
  - [ ] compiler errors
  - [ ] lint messages
  - [ ] test failures
  - [ ] diffs
  - [ ] grep/search outputs
- [ ] normalize extracted paths to repo-relative form
- [ ] resolve owning package/workspace for extracted files
- [ ] best-effort symbol linking:
  - [ ] parse obvious symbol names from errors
  - [ ] map error/function/test names to graph nodes where possible
- [ ] attach graph context to execution results:
  - [ ] `related_files`
  - [ ] `related_packages`
  - [ ] `related_symbols`
  - [ ] `neighbor_counts`
- [ ] keep graph linking bounded and optional
- [ ] graph linking failure must never block execution

### Acceptance
- [ ] build/lint/test outputs frequently surface useful package/file/symbol links
- [ ] graph metadata improves downstream review/debug context quality

---

# Phase V3.4 — Execution-derived freshness and follow-up guidance

## Goal
Use execution results to tell the agent what to do next.

### TODO
- [ ] mark graph freshness hints from execution:
  - [ ] build/test after edits
  - [ ] commands that likely modified/generated files
  - [ ] diffs showing changed code files
- [ ] generate follow-up suggestions:
  - [ ] rerun `build_or_update_graph`
  - [ ] inspect stored failure artifact
  - [ ] regenerate review-context
  - [ ] inspect impacted tests
- [ ] add MCP metadata:
  - [ ] `graph_stale_hint`
  - [ ] `suggested_next_actions`
  - [ ] `requires_artifact_read`
- [ ] allow hook/MCP workflows to consume these hints without duplicating logic

### Acceptance
- [ ] execution results can advise the next Atlas action without forcing it
- [ ] agents get bounded next-step guidance instead of raw log spam

---

# Phase V3.5 — Multi-step execution workflows, still bounded

## Goal
Support small execution flows without becoming a workflow engine.

### TODO
- [ ] add bounded “run + artifact + query” pattern:
  - [ ] execute command
  - [ ] store large output if needed
  - [ ] optionally run focused search over artifact
  - [ ] return compact answer + refs
- [ ] support useful built-ins:
  - [ ] “run test command and summarize failures”
  - [ ] “run diff and summarize touched files/packages”
  - [ ] “run search and group by file”
- [ ] allow execution result formatting profiles by mode:
  - [ ] debug
  - [ ] review
  - [ ] diagnostics
- [ ] preserve transparency: command actually run must remain explicit

### Acceptance
- [ ] agents can ask for execution plus summary without manual follow-up every time
- [ ] output remains bounded
- [ ] subsystem still does not become an unbounded orchestrator

---

# Phase V3.6 — Better MCP execution surface

## Goal
Make execution tools agent-native, not just shell wrappers.

### MCP tools
- [ ] `run_command`
- [ ] `read_command_output`
- [ ] `search_command_outputs`
- [ ] `summarize_command_output`
- [ ] `list_recent_command_outputs`
- [ ] `get_execution_status` later if needed

### Response contract
- [ ] compact summary by default
- [ ] previews only
- [ ] artifact refs for large outputs
- [ ] graph/session metadata when available
- [ ] suggested next actions when confidence is high
- [ ] stable truncation metadata

### Acceptance
- [ ] execution tools feel aligned with the rest of Atlas MCP surfaces
- [ ] agents do not need to manually stitch command run + artifact read + search every time

---

# Phase V3.7 — Diagnostics, observability, and trust

## Goal
Make execution outputs trustworthy and debuggable.

### TODO
- [ ] add execution provenance metadata:
  - [ ] command
  - [ ] cwd
  - [ ] policy decision
  - [ ] time started
  - [ ] duration
  - [ ] truncation reason
- [ ] add subsystem diagnostics:
  - [ ] artifact store health
  - [ ] retrieval index health
  - [ ] policy config validity
  - [ ] classifier coverage report
- [ ] add metrics:
  - [ ] execution count
  - [ ] artifact count
  - [ ] bytes avoided inline
  - [ ] most common command families
  - [ ] compression ratio estimate
  - [ ] retrieval hit rate over artifacts

### Acceptance
- [ ] failures are explainable
- [ ] operators can tell whether compression/retrieval is helping

---

# Phase V3.8 — Broader command-family parity

## Goal
Expand beyond the first-wave commands, but still selectively.

### Add stronger support for
- [ ] `cargo clippy`
- [ ] `cargo build`
- [ ] `cargo check`
- [ ] `go build`
- [ ] `go vet`
- [ ] `golangci-lint`
- [ ] `eslint`
- [ ] `pnpm test`
- [ ] `npm test`
- [ ] `pytest -q` style variants
- [ ] common package-manager install/update outputs later if safe

### Keep fallback for
- [ ] unknown but policy-allowed commands

### Acceptance
- [ ] major Rust/Go/Python/JS workflows have at least decent specialized support
- [ ] unsupported commands still degrade gracefully

---

# Phase V3.9 — Test and benchmark gates

## Unit tests
- [ ] artifact chunking tests
- [ ] execution-to-file extraction tests
- [ ] symbol-linking heuristics tests
- [ ] session reducer tests for execution events
- [ ] next-action hint tests

## Integration tests
- [ ] failing test → artifact → search → summary
- [ ] failing build → file extraction → package resolution
- [ ] diff → file/package enrichment
- [ ] resume after compaction with execution artifacts
- [ ] MCP execution workflow end-to-end

## Benchmarks
- [ ] artifact indexing latency
- [ ] artifact search latency
- [ ] bytes avoided inline
- [ ] search quality over stored outputs
- [ ] end-to-end execution workflow latency

### Acceptance
- [ ] V3 workflows remain interactive enough for agent use
- [ ] artifact retrieval meaningfully reduces raw-context usage

---

# V3 strong-support workflows

By the end of V3, these should feel good:

- [ ] run tests and summarize failures
- [ ] read/search full failure output later
- [ ] run build and identify likely affected files/packages
- [ ] run lint and group issues by file/rule
- [ ] run diff and connect touched files back to graph/package context
- [ ] resume a failed execution-heavy session after compaction
- [ ] let agents follow artifact pointers instead of replaying raw logs

---

# V3 non-goals

- [ ] no full system shell interception
- [ ] no universal command rewrite engine
- [ ] no interactive shell UX
- [ ] no full-blown workflow/orchestration DAG system
- [ ] no guarantee of perfect symbol extraction from arbitrary logs
- [ ] no dependency on graph availability for execution correctness

---

# V3 completion criteria

V3 is complete when all of these are true:

- [ ] execution artifacts are searchable and useful
- [ ] session resumes can carry execution-derived state cleanly
- [ ] graph-linked execution hints are available for major workflows
- [ ] agents can use execution results without reading raw large outputs
- [ ] MCP execution tools feel native to Atlas
- [ ] build/test/lint/debug flows can survive compaction and continue via artifact retrieval

---

# Simple version mapping

- **V1** = safe execution + generic compression + artifact save
- **V2** = practical parity for major command families
- **V3** = integrated parity with session, retrieval, and graph context
- **V4** = long-tail breadth and more advanced shell behavior
