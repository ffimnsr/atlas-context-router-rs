# Atlas Execution Subsystem — V4

## V4 goal

Move from V3:

- searchable execution artifacts
- execution-aware sessions
- graph-linked execution hints
- strong MCP execution workflows

to:

- **broad command-family coverage**
- **ecosystem breadth**
- **more advanced execution ergonomics**
- **higher-fidelity reductions**
- **operator-grade polish**
- **long-tail parity across real developer workflows**

V4 is the version where the subsystem feels **mature and broad**, not just solid and integrated.

---

# V4 definition

V4 is complete when Atlas execution can do all of these well:

- support a much broader set of command families
- provide high-quality compression across multiple ecosystems
- handle more edge-case outputs reliably
- support richer retrieval and follow-up workflows
- provide high-confidence next-step guidance from command results
- integrate deeply with hook-driven agent workflows
- expose polished CLI/MCP behavior for execution-heavy sessions
- scale to longer, noisier, and more varied real-world outputs

---

# V4 scope

## Included
- broader ecosystem support
- advanced output shaping
- richer retrieval/ranking over execution artifacts
- stronger hook/agent integration
- more advanced next-step reasoning
- better ops/observability/admin controls
- more configurable execution modes

## Still excluded
- full system-wide shell replacement
- arbitrary hidden command rewriting
- PTY/interactive terminal emulation
- unbounded orchestration engine
- executing anything without policy boundaries

---

# Phase V4.1 — Broader command-family coverage

## Goal
Expand beyond the first-wave commands into a genuinely broad developer surface.

### Rust
- [ ] `cargo test`
- [ ] `cargo build`
- [ ] `cargo check`
- [ ] `cargo clippy`
- [ ] `cargo bench`
- [ ] `cargo metadata` summary mode
- [ ] `cargo tree` compact dependency summarization

### Go
- [ ] `go test`
- [ ] `go build`
- [ ] `go vet`
- [ ] `go list`
- [ ] `go mod tidy` summary mode
- [ ] `golangci-lint`

### Python
- [ ] `pytest`
- [ ] `pytest -q`
- [ ] `ruff`
- [ ] `mypy`
- [ ] `uv` / `pip` install/update summaries where safe

### JS / TS
- [ ] `npm test`
- [ ] `pnpm test`
- [ ] `yarn test`
- [ ] `eslint`
- [ ] `tsc`
- [ ] common build tools:
  - [ ] Vite
  - [ ] Next.js build output
  - [ ] generic Webpack summaries

### Git / repo tools
- [ ] `git status`
- [ ] `git diff`
- [ ] `git log`
- [ ] `git show`
- [ ] `git grep`
- [ ] `git blame` summary mode later if useful

### Generic utilities
- [ ] `rg`
- [ ] `fd`
- [ ] `find`
- [ ] `ls`
- [ ] `tree`
- [ ] structured JSON-producing CLIs

### Acceptance
- [ ] Atlas supports major Rust, Go, Python, and JS/TS workflows at a specialized level
- [ ] unsupported commands still degrade gracefully to generic mode

---

# Phase V4.2 — Higher-fidelity output reducers

## Goal
Make reducers smarter and more accurate on messy real-world output.

### TODO
- [ ] improve diff summarization:
  - [ ] file-level grouping
  - [ ] rename detection
  - [ ] likely touched symbol extraction
  - [ ] test adjacency hints
- [ ] improve compiler/build parsing:
  - [ ] dedupe repeated errors
  - [ ] collapse template/backtrace noise
  - [ ] rank root-cause errors above cascades
- [ ] improve test parsing:
  - [ ] collapse repeated failure sections
  - [ ] group by failing suite/module/package
  - [ ] identify flaky-looking patterns later
- [ ] improve lint parsing:
  - [ ] rule-based grouping
  - [ ] file severity rollups
  - [ ] autofix hint extraction where safe
- [ ] improve JSON shaping:
  - [ ] structural summarization
  - [ ] schema-like summaries
  - [ ] top-k object previews

### Acceptance
- [ ] reducers reliably surface the most important lines first
- [ ] noisy outputs become compact without hiding root causes

---

# Phase V4.3 — Rich retrieval and ranking over execution artifacts

## Goal
Turn execution outputs into a genuinely strong searchable corpus.

### TODO
- [ ] improve ranking over stored artifacts using:
  - [ ] command-family boosts
  - [ ] recency boosts
  - [ ] session-local boosts
  - [ ] file/package overlap boosts
  - [ ] failure-first boosts
- [ ] add multiple retrieval views:
  - [ ] by session
  - [ ] by command family
  - [ ] by failure class
  - [ ] by file/package/symbol
- [ ] add query expansion for artifact search:
  - [ ] symbol names
  - [ ] package names
  - [ ] common compiler/test/lint error aliases
- [ ] add bounded snippet selection that prefers root-cause regions
- [ ] support retrieval metadata for omitted sections and rank reasons

### Acceptance
- [ ] artifact search feels purpose-built, not like a raw log grep
- [ ] common follow-up questions can be answered from stored outputs quickly

---

# Phase V4.4 — Execution-aware next-step reasoning

## Goal
Use execution outcomes to suggest the most useful next Atlas action.

### TODO
- [ ] infer suggested next steps from execution results:
  - [ ] build failed → inspect related files/package, maybe query graph neighbors
  - [ ] lint failed → group by files, maybe regenerate review context
  - [ ] diff changed files → run graph update or impact
  - [ ] tests failed → inspect failing package, impacted symbols, related tests
- [ ] assign confidence to next-step suggestions
- [ ] expose reasoning hints in MCP responses
- [ ] keep suggestions bounded and transparent
- [ ] never auto-execute next actions silently

### Acceptance
- [ ] agents regularly get useful next actions after command execution
- [ ] suggestions are obviously tied to evidence from output

---

# Phase V4.5 — Strong hook and agent workflow integration

## Goal
Make execution first-class in Atlas’s hook and session model.

### TODO
- [ ] let hook flows store execution outputs automatically when large
- [ ] attach execution summaries to hook-generated session events
- [ ] incorporate recent execution artifacts into:
  - [ ] resume snapshots
  - [ ] post-compact handoff
  - [ ] stale-graph warnings
  - [ ] review refresh hints
- [ ] improve host-facing behavior for:
  - [ ] Copilot
  - [ ] Claude
  - [ ] Codex
- [ ] add execution-aware hook policies:
  - [ ] safe command allow
  - [ ] deny reasons
  - [ ] refresh triggers after build/test/edit flows

### Acceptance
- [ ] execution-heavy sessions survive compaction cleanly
- [ ] host integrations can use execution results without flooding chat context

---

# Phase V4.6 — Better CLI/MCP execution ergonomics

## Goal
Make execution flows easy enough that users and agents do not need custom glue.

### CLI
- [ ] add polished command modes:
  - [ ] `atlas exec run --mode build`
  - [ ] `atlas exec run --mode test`
  - [ ] `atlas exec run --mode lint`
  - [ ] `atlas exec run --mode diff`
  - [ ] `atlas exec summarize <source_id>`
  - [ ] `atlas exec recent`
- [ ] support richer output profiles:
  - [ ] compact
  - [ ] review
  - [ ] debug
  - [ ] diagnostics

### MCP
- [ ] `run_command`
- [ ] `read_command_output`
- [ ] `search_command_outputs`
- [ ] `summarize_command_output`
- [ ] `list_recent_command_outputs`
- [ ] maybe `suggest_next_execution_step`

### Acceptance
- [ ] common execution workflows need fewer round trips
- [ ] CLI and MCP stay aligned in schema and flags where applicable

---

# Phase V4.7 — Operator-grade diagnostics and controls

## Goal
Make the subsystem maintainable in large real-world use.

### TODO
- [ ] add retention controls for execution artifacts:
  - [ ] by age
  - [ ] by size budget
  - [ ] by session count
- [ ] add pruning and stats:
  - [ ] artifact count
  - [ ] DB/storage size
  - [ ] hottest command families
  - [ ] bytes avoided inline
  - [ ] search hit rates
- [ ] add subsystem health checks:
  - [ ] artifact store
  - [ ] retrieval index
  - [ ] policy rules
  - [ ] classifier coverage
- [ ] add corruption/rebuild handling if execution artifact DB/index goes bad
- [ ] improve observability:
  - [ ] structured tracing
  - [ ] compression metrics
  - [ ] artifacting thresholds and reasons

### Acceptance
- [ ] operators can manage storage growth
- [ ] failures can be diagnosed without guesswork

---

# Phase V4.8 — Broader policy and trust controls

## Goal
Make execution safer and more configurable without making it opaque.

### TODO
- [ ] improve command-family policy model
- [ ] support per-mode allowlists:
  - [ ] test-only
  - [ ] build-only
  - [ ] search-only
- [ ] support configurable deny patterns with good diagnostics
- [ ] improve env filtering and redaction rules
- [ ] surface policy decisions in execution provenance
- [ ] ensure command transparency: always show what actually ran

### Acceptance
- [ ] users can tighten or relax execution safely
- [ ] policy behavior remains explainable

---

# Phase V4.9 — Long-tail edge-case quality

## Goal
Handle awkward outputs and less-common workflows better.

### TODO
- [ ] nested multi-package monorepo build outputs
- [ ] deeply repeated compiler cascades
- [ ] mixed stdout/stderr tool formatting
- [ ] commands with huge JSON outputs
- [ ] commands with partial structured + partial freeform output
- [ ] noisy progress bars/spinners
- [ ] tools that emit repeated stack traces
- [ ] more resilient fallback for unknown commands

### Acceptance
- [ ] subsystem stays useful outside the happy path
- [ ] generic fallback remains safe and bounded

---

# Phase V4.10 — Test and benchmark gates

## Unit tests
- [ ] long-tail classifier fixtures
- [ ] reducer root-cause prioritization tests
- [ ] artifact ranking tests
- [ ] policy provenance tests
- [ ] retention/prune tests

## Integration tests
- [ ] Rust, Go, Python, JS/TS workflows end-to-end
- [ ] compaction resume with multiple execution artifacts
- [ ] graph-linked next-step hints from build/test/lint flows
- [ ] large artifact search and retrieval
- [ ] hook-driven execution-heavy sessions

## Benchmarks
- [ ] broad command-family latency
- [ ] retrieval quality over stored outputs
- [ ] storage growth under repeated execution sessions
- [ ] compaction survival quality
- [ ] MCP round-trip count reduction

### Acceptance
- [ ] V4 remains interactive and bounded
- [ ] broader parity does not regress V1–V3 reliability

---

# V4 strong-support workflows

By the end of V4, these should feel polished:

- [ ] run build/test/lint across major ecosystems and get useful summaries
- [ ] inspect/retrieve/search stored outputs without raw-log overload
- [ ] survive compaction during debugging-heavy sessions
- [ ] connect execution failures back into Atlas graph and review flows
- [ ] get evidence-backed next-step guidance after command execution
- [ ] use execution tools naturally through MCP and hooks

---

# V4 non-goals

- [ ] no global shell replacement
- [ ] no hidden command rewriting engine
- [ ] no interactive terminal emulator
- [ ] no arbitrary workflow DAG/orchestrator
- [ ] no attempt to support every niche command perfectly
- [ ] no removal of policy boundaries for convenience

---

# V4 completion criteria

V4 is complete when all of these are true:

- [ ] Atlas execution supports a broad set of high-value command families
- [ ] reducers and retrieval stay high quality on noisy real-world outputs
- [ ] execution artifacts behave like a searchable knowledge layer
- [ ] session continuity and hook workflows use execution state well
- [ ] graph-linked hints and next actions are useful and evidence-backed
- [ ] subsystem is operable, diagnosable, and storage-manageable

---

# Simple version mapping

- **V1** = safe execution + generic compression + artifact save
- **V2** = practical parity for major command families
- **V3** = integrated parity with session, retrieval, and graph context
- **V4** = broad parity, long-tail command quality, and mature platform polish

So the simplest way to say it is:

**V3 makes it fully integrated.  
V4 makes it broadly mature.**
