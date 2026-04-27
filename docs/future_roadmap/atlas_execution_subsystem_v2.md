# Atlas Execution Subsystem — V2

## V2 goal

Move from:

- safe command runner
- generic output compression
- artifact storage

to:

- **high-value practical parity**
- **tool-aware summaries**
- **first-class artifact retrieval**
- **basic graph/session enrichment**

V2 should feel good for real workflows, not just technically functional.

---

# V2 definition

V2 is complete when Atlas execution can do all of these reliably:

- run bounded commands safely
- compress outputs in a tool-aware way
- store large outputs as artifacts
- retrieve artifacts by id and search them
- emit useful session events
- extract file/package hints from outputs
- provide compact MCP/CLI responses for:
  - test failures
  - lint failures
  - build failures
  - git status/diff
  - search/list results
  - JSON outputs

---

# V2 scope

## Included
- specialized compressors
- artifact retrieval/search
- improved classification
- graph-aware output hints
- stronger session integration
- better diagnostics/config

## Not included
- command rewriting
- global shell interception
- full command ecosystem coverage
- interactive shell / PTY
- background job orchestration
- hard dependency on graph freshness

---

# Phase V2.1 — Output classifiers become reliable

## Goal
Upgrade from “generic vs maybe JSON” into a real classifier pipeline.

### TODO
- [ ] Add command-name-based classifiers:
  - [ ] `git status`
  - [ ] `git diff`
  - [ ] `cargo test`
  - [ ] `go test`
  - [ ] `pytest`
  - [ ] `cargo build`
  - [ ] `go build`
  - [ ] `ruff`
  - [ ] `golangci-lint`
  - [ ] `eslint`
  - [ ] `rg`
  - [ ] `ls`
- [ ] Add content-shape fallback classifiers:
  - [ ] unified diff detection
  - [ ] JSON detection
  - [ ] compiler error detection
  - [ ] test failure detection
  - [ ] linter finding detection
- [ ] Add classifier confidence score
- [ ] Add generic fallback when confidence is low
- [ ] Record detected classifier in result metadata

### Acceptance
- [ ] Same command produces same output kind across repeated runs
- [ ] Unknown commands safely fall back to generic mode
- [ ] Invalid JSON never crashes classification

---

# Phase V2.2 — Tool-aware compression

## Goal
Turn raw outputs into summaries that are actually useful.

## Git status compressor
- [ ] summarize:
  - [ ] staged files
  - [ ] modified files
  - [ ] deleted files
  - [ ] untracked files
- [ ] show bounded file preview
- [ ] expose counts in JSON

## Git diff compressor
- [ ] summarize:
  - [ ] changed files
  - [ ] insertions/deletions
  - [ ] rename detection if present
- [ ] preview first changed hunks only
- [ ] store full diff artifact when large

## Test output compressor
- [ ] summarize:
  - [ ] passed count
  - [ ] failed count
  - [ ] skipped count where detectable
  - [ ] failing test names
  - [ ] failing package/module
  - [ ] first meaningful failure
- [ ] support:
  - [ ] `cargo test`
  - [ ] `go test`
  - [ ] `pytest`
  - [ ] generic JS test runners

## Lint compressor
- [ ] summarize:
  - [ ] issue count
  - [ ] files affected
  - [ ] rules when detectable
  - [ ] top offender files
- [ ] support:
  - [ ] `ruff`
  - [ ] `golangci-lint`
  - [ ] `eslint`
  - [ ] generic lint fallback

## Build/complier compressor
- [ ] summarize:
  - [ ] success/failure
  - [ ] error count
  - [ ] warning count
  - [ ] first significant errors
  - [ ] affected files
- [ ] support:
  - [ ] `cargo build`
  - [ ] `go build`
  - [ ] generic TS/JS build tools

## Search/list compressor
- [ ] `rg`:
  - [ ] group matches by file
  - [ ] cap matches per file
  - [ ] cap file count
- [ ] `ls`:
  - [ ] summarize directory entries
  - [ ] cap listing size
  - [ ] highlight likely relevant files

## JSON compressor
- [ ] detect valid JSON
- [ ] summarize top-level keys
- [ ] preview representative values
- [ ] store full artifact when large

### Acceptance
- [ ] Each supported tool family has stable JSON output
- [ ] Large outputs never flood MCP/CLI response
- [ ] Non-zero exit results still get useful summaries

---

# Phase V2.3 — Artifact retrieval becomes first-class

## Goal
Make large-output storage actually usable.

### TODO
- [ ] Reuse saved-context storage model
- [ ] Add execution artifact metadata:
  - [ ] `source_id`
  - [ ] command
  - [ ] cwd
  - [ ] output_kind
  - [ ] created_at
  - [ ] exit_code
  - [ ] byte_count
- [ ] Add CLI:
  - [ ] `atlas exec read-artifact <source_id>`
  - [ ] `atlas exec search-artifacts <query>`
- [ ] Add paging/truncation for artifact reads
- [ ] Add preview generation for stored outputs
- [ ] Add search over artifact previews/full text where practical

### MCP
- [ ] `read_command_output`
- [ ] `search_command_outputs`

### Acceptance
- [ ] Any large execution output returns a `source_id`
- [ ] Artifact can be read later by id
- [ ] Artifact search returns compact previews, not full blobs

---

# Phase V2.4 — Session integration improves

## Goal
Execution results should matter to continuity.

### TODO
- [ ] Emit normalized events:
  - [ ] `tool_started`
  - [ ] `tool_succeeded`
  - [ ] `tool_failed`
  - [ ] `artifact_saved`
- [ ] Include bounded metadata only:
  - [ ] command name
  - [ ] cwd
  - [ ] output kind
  - [ ] exit code
  - [ ] source_id
- [ ] Reducer updates:
  - [ ] track last failed command
  - [ ] track recent execution artifacts
  - [ ] track recent execution mode
- [ ] Include last relevant execution in resume snapshots when useful

### Acceptance
- [ ] Large raw stdout/stderr never stored inline as session event
- [ ] Resume can mention recent failed build/test with artifact pointer

---

# Phase V2.5 — Graph-aware hints

## Goal
Make Atlas stronger than a plain output compressor.

### TODO
- [ ] Extract repo-relative file paths from:
  - [ ] build errors
  - [ ] lint results
  - [ ] test failures
  - [ ] diffs
  - [ ] search results
- [ ] Normalize extracted paths
- [ ] Resolve owning package/workspace for extracted files
- [ ] Best-effort symbol linking for obvious cases
- [ ] Mark graph stale when execution strongly suggests edited/generated files changed
- [ ] Include graph hints in result metadata:
  - [ ] `related_files`
  - [ ] `related_packages`
  - [ ] `graph_stale_hint`

### Acceptance
- [ ] Build/lint/test outputs can surface related files in normalized form
- [ ] Graph linkage failure never breaks execution

---

# Phase V2.6 — MCP/CLI ergonomics

## Goal
Make the subsystem easy to consume.

## CLI
- [ ] `atlas exec run --mode test -- cargo test`
- [ ] `atlas exec run --mode lint -- ruff check .`
- [ ] `atlas exec run --mode diff -- git diff --stat`
- [ ] `atlas exec run --json ...`
- [ ] `atlas exec doctor`

## MCP
- [ ] `run_command`
- [ ] returns:
  - [ ] summary
  - [ ] stdout_preview
  - [ ] stderr_preview
  - [ ] exit_code
  - [ ] output_kind
  - [ ] truncated
  - [ ] source_id
  - [ ] related_files
  - [ ] related_packages

### Acceptance
- [ ] MCP responses are compact by default
- [ ] CLI JSON and MCP JSON stay aligned

---

# Phase V2.7 — Diagnostics and config

## Goal
Make it operable.

### TODO
- [ ] Add config:
  - [ ] `exec.enabled`
  - [ ] `exec.timeout_secs`
  - [ ] `exec.max_inline_bytes`
  - [ ] `exec.max_capture_bytes`
  - [ ] `exec.auto_store_threshold_bytes`
  - [ ] `exec.allowed_commands`
  - [ ] `exec.denied_commands`
- [ ] Add `atlas exec doctor` checks:
  - [ ] subprocess execution works
  - [ ] policy config valid
  - [ ] artifact store writable
  - [ ] thresholds loaded
- [ ] Add metrics:
  - [ ] executions run
  - [ ] artifacts stored
  - [ ] bytes avoided inline
  - [ ] compression ratio estimate

### Acceptance
- [ ] Misconfigurations produce clear errors
- [ ] Doctor can verify subsystem readiness

---

# Phase V2.8 — Test and benchmark gates

## Unit tests
- [ ] classifier tests by command family
- [ ] compressor tests by fixture
- [ ] truncation tests
- [ ] artifact metadata tests

## Integration tests
- [ ] `git status`
- [ ] `git diff`
- [ ] `cargo test` failing case
- [ ] `go test` failing case
- [ ] `pytest` failing case
- [ ] `ruff` findings
- [ ] JSON command output
- [ ] denied command policy
- [ ] artifact read/search

## Benchmarks
- [ ] execution overhead
- [ ] compression latency
- [ ] artifact write latency
- [ ] bytes avoided inline
- [ ] MCP response size reduction

### Acceptance
- [ ] No supported fixture floods the response
- [ ] Compression stays fast enough for interactive use

---

# V2 first-wave supported commands

## Strong support
- [ ] `git status`
- [ ] `git diff`
- [ ] `cargo test`
- [ ] `go test`
- [ ] `pytest`
- [ ] `ruff`
- [ ] `rg`
- [ ] `ls`

## Good support
- [ ] `cargo build`
- [ ] `go build`
- [ ] `golangci-lint`
- [ ] `eslint`

## Generic fallback only
- [ ] everything else allowed by policy

---

# V2 non-goals

- [ ] no command rewriting
- [ ] no global shell interception
- [ ] no interactive shell
- [ ] no full RTK command parity
- [ ] no mandatory graph freshness dependency
- [ ] no orchestration of multiple commands

---

# V2 completion criteria

V2 is complete when all of these are true:

- [ ] Supported commands return tool-aware compressed summaries
- [ ] Large outputs are artifacted automatically
- [ ] Artifacts are readable/searchable
- [ ] Session layer receives compact execution events
- [ ] Graph/package/file hints can be extracted from major outputs
- [ ] MCP and CLI responses are stable and compact
- [ ] Test/build/lint/git workflows feel good in real use
