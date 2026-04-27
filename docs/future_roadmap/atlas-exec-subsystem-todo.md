# Atlas Execution / Compression Subsystem Todo

## Goal

Add a bounded sandbox execution layer that:

- runs selected commands/tools safely
- captures raw stdout/stderr
- compresses noisy output for agent consumption
- stores full output as retrievable artifacts when large
- emits session events
- optionally links execution results back to graph/package/symbol context

This should **not** replace Atlas graph/query/review behavior. It should sit beside it as a third subsystem, consistent with Atlas’s CLI-first, SQLite-first, deterministic architecture. Atlas is already graph-first with CLI and MCP continuity surfaces, while context-mode demonstrates the value of keeping large outputs out of context and retrieving them later via local storage/search. fileciteturn0file3 fileciteturn0file2

---

## Phase EX0 — Scope and design freeze

### EX0.1 Freeze subsystem purpose

- [ ] define subsystem as **execution-for-context-control**, not general shell orchestration
- [ ] primary behavior:
  - [ ] run command
  - [ ] capture stdout/stderr/exit code
  - [ ] classify output type
  - [ ] compress output into bounded result
  - [ ] store raw artifact if output is large
  - [ ] return compact summary + retrieval handle
- [ ] explicitly out of scope for v1:
  - [ ] background jobs
  - [ ] full PTY/interactive shell
  - [ ] arbitrary workflow engine
  - [ ] remote execution
  - [ ] runtime tracing/profiling
  - [ ] generalized tool wrapping for every CLI on day one

### EX0.2 Freeze architecture rule

- [ ] keep execution as separate subsystem from:
  - [ ] graph engine
  - [ ] parser/indexer
  - [ ] session reducer
- [ ] graph layer may consume execution metadata, but execution must not depend on graph correctness
- [ ] session layer may ingest execution events, but execution must remain usable standalone
- [ ] MCP wrappers must stay thin over execution service, matching Atlas’s existing transport-independent pattern fileciteturn0file3

### EX0.3 Add crate

- [ ] create `packages/atlas-exec`
- [ ] keep public API narrow
- [ ] add to workspace and CI

---

## Phase EX1 — Core types and crate skeleton

### EX1.1 Crate layout

- [ ] create:
  - [ ] `packages/atlas-exec/src/lib.rs`
  - [ ] `types.rs`
  - [ ] `runner.rs`
  - [ ] `policy.rs`
  - [ ] `capture.rs`
  - [ ] `classify.rs`
  - [ ] `compress.rs`
  - [ ] `artifact.rs`
  - [ ] `format.rs`

### EX1.2 Core request/response types

- [ ] define `ExecRequest`
- [ ] define `ExecResult`
- [ ] define `ExecStatus`
- [ ] define `ExecMode`
- [ ] define `OutputKind`
- [ ] define `CompressionResult`
- [ ] define `ArtifactRef`

### EX1.3 Suggested core fields

#### `ExecRequest`
- [ ] `command: Vec<String>`
- [ ] `cwd: Utf8PathBuf`
- [ ] `timeout_secs: u64`
- [ ] `mode: ExecMode`
- [ ] `capture_stderr: bool`
- [ ] `max_inline_bytes: usize`
- [ ] `env_policy: EnvPolicy`
- [ ] `store_large_output: bool`

#### `ExecResult`
- [ ] `exit_code: i32`
- [ ] `status: ExecStatus`
- [ ] `detected_kind: OutputKind`
- [ ] `summary: String`
- [ ] `stdout_preview: String`
- [ ] `stderr_preview: String`
- [ ] `artifact_source_id: Option<String>`
- [ ] `truncated: bool`
- [ ] `bytes_stdout: usize`
- [ ] `bytes_stderr: usize`
- [ ] `duration_ms: u64`

### EX1.4 JSON contract

- [ ] add serde support for all public structs
- [ ] add snapshot tests for stable JSON
- [ ] keep JSON output aligned with Atlas’s stable machine-readable contracts and MCP metadata patterns fileciteturn0file3

---

## Phase EX2 — Safe command runner

### EX2.1 Basic command execution

- [ ] implement subprocess runner using `std::process::Command`
- [ ] support cwd override
- [ ] support explicit env allowlist/blocklist
- [ ] capture stdout/stderr separately
- [ ] preserve exit code
- [ ] return timeout status cleanly

### EX2.2 Timeouts and bounds

- [ ] enforce wall-clock timeout
- [ ] enforce max captured bytes for stdout
- [ ] enforce max captured bytes for stderr
- [ ] stop capture when bounds exceeded
- [ ] mark result truncated when capture clipped

### EX2.3 Failure semantics

- [ ] distinguish:
  - [ ] spawn failure
  - [ ] timeout
  - [ ] non-zero exit
  - [ ] capture overflow
- [ ] keep failures machine-readable
- [ ] do not panic on malformed command input

### EX2.4 Platform behavior

- [ ] Linux tests
- [ ] macOS tests
- [ ] Windows tests
- [ ] shell-free argv path first
- [ ] shell-based execution only as explicit mode later

---

## Phase EX3 — Policy and sandbox rules

### EX3.1 Policy model

- [ ] define `ExecutionPolicy`
- [ ] define allow/deny rules for:
  - [ ] executable names
  - [ ] cwd scope
  - [ ] path globs
  - [ ] write-sensitive commands
  - [ ] destructive commands
- [ ] separate policy decision from execution logic

### EX3.2 Default policy

- [ ] default to repo-scoped cwd only
- [ ] deny dangerous destructive commands by default
- [ ] deny inherited secret-heavy env by default
- [ ] allow opt-in safe commands first:
  - [ ] `git`
  - [ ] `cargo`
  - [ ] `go`
  - [ ] `npm`
  - [ ] `pnpm`
  - [ ] `pytest`
  - [ ] `ruff`
  - [ ] `rg`
  - [ ] `ls`

### EX3.3 Policy result shape

- [ ] return:
  - [ ] `allowed`
  - [ ] `denied`
  - [ ] `needs_confirmation` later if needed
- [ ] include denial reason
- [ ] add structured error code for denied execution

### EX3.4 Tests

- [ ] deny dangerous command fixtures
- [ ] deny outside-repo cwd
- [ ] redact blocked env vars
- [ ] allow safe bounded commands

---

## Phase EX4 — Output capture and storage thresholds

### EX4.1 Capture policy

- [ ] small output: return inline
- [ ] medium output: summarize + preview
- [ ] large output: summarize + store artifact
- [ ] failed commands still eligible for artifact save

### EX4.2 Threshold config

- [ ] add config fields:
  - [ ] `exec.max_inline_bytes`
  - [ ] `exec.max_capture_bytes`
  - [ ] `exec.auto_store_threshold_bytes`
  - [ ] `exec.timeout_secs`
- [ ] expose via `.atlas/config.toml`
- [ ] surface active thresholds in diagnostics

### EX4.3 Byte accounting

- [ ] count stdout bytes
- [ ] count stderr bytes
- [ ] count bytes omitted from inline response
- [ ] expose avoided bytes in result metadata

This is directly aligned with the “large output stays out of context; compact result goes to model” philosophy seen in context-mode. fileciteturn0file2

---

## Phase EX5 — Output classification

### EX5.1 Output kind detection

- [ ] classify output as:
  - [ ] `PlainText`
  - [ ] `Json`
  - [ ] `GitDiff`
  - [ ] `GitStatus`
  - [ ] `TestOutput`
  - [ ] `LintOutput`
  - [ ] `BuildOutput`
  - [ ] `SearchOutput`
  - [ ] `ListOutput`
  - [ ] `Unknown`

### EX5.2 Detection signals

- [ ] detect by requested mode first
- [ ] fallback to command name
- [ ] fallback to content heuristics
- [ ] detect valid JSON early
- [ ] detect unified diff shape
- [ ] detect common test/lint/build markers

### EX5.3 Confidence model

- [ ] return classification confidence
- [ ] fall back to generic compression on low confidence
- [ ] never pretend a specialized parser succeeded when it did not

---

## Phase EX6 — Generic compression pipeline

### EX6.1 Generic compressor

- [ ] implement default plain-text compressor
- [ ] keep:
  - [ ] first useful lines
  - [ ] last useful lines
  - [ ] error lines
  - [ ] warning counts
  - [ ] summary counts
- [ ] strip obvious noise:
  - [ ] repeated progress lines
  - [ ] empty line floods
  - [ ] duplicated stack lines where safe

### EX6.2 Generic summary contract

- [ ] return:
  - [ ] one-paragraph summary
  - [ ] bounded preview
  - [ ] truncation metadata
  - [ ] artifact pointer if stored

### EX6.3 JSON-aware compression

- [ ] if output is valid JSON:
  - [ ] avoid returning raw blob by default
  - [ ] summarize top-level keys
  - [ ] preview representative entries
  - [ ] store full JSON artifact when large

---

## Phase EX7 — Tool-aware compressors (high-value first wave)

### EX7.1 Git status

- [ ] summarize:
  - [ ] staged count
  - [ ] modified count
  - [ ] untracked count
  - [ ] deleted count
- [ ] show bounded file list
- [ ] classify result as `GitStatus`

### EX7.2 Git diff

- [ ] summarize:
  - [ ] changed files
  - [ ] insertions/deletions
  - [ ] file-level status
- [ ] keep preview bounded
- [ ] store full diff artifact when large
- [ ] optional later: extract touched symbols using Atlas graph/file helpers

### EX7.3 Test output

- [ ] summarize:
  - [ ] pass/fail counts
  - [ ] failing tests
  - [ ] failing packages/files
  - [ ] first significant error
- [ ] support first wave for:
  - [ ] `cargo test`
  - [ ] `go test`
  - [ ] `pytest`
  - [ ] `npm test` / `pnpm test` generic
- [ ] store full failure log as artifact when large

### EX7.4 Linter output

- [ ] summarize:
  - [ ] issue counts by file
  - [ ] issue counts by rule when detectable
  - [ ] top offending files
- [ ] support first wave for:
  - [ ] `ruff`
  - [ ] `golangci-lint`
  - [ ] `eslint` generic
  - [ ] `cargo clippy` generic
- [ ] store full raw lint output when large

### EX7.5 Build/compiler output

- [ ] summarize:
  - [ ] success/failure
  - [ ] main error locations
  - [ ] affected files
  - [ ] error count / warning count
- [ ] support first wave for:
  - [ ] `cargo build`
  - [ ] `go build`
  - [ ] TS/JS build tools generic
- [ ] preserve raw output as artifact if large

### EX7.6 Search/list output

- [ ] `rg` / grep-like output:
  - [ ] group by file
  - [ ] cap matches per file
  - [ ] cap total files
- [ ] `ls` / list-like output:
  - [ ] show concise listing
  - [ ] summarize counts
  - [ ] suppress repetitive metadata unless useful

---

## Phase EX8 — Artifact storage

### EX8.1 Artifact model

- [ ] reuse Atlas saved-context artifact model where possible
- [ ] execution artifacts should record:
  - [ ] `source_id`
  - [ ] command
  - [ ] cwd
  - [ ] output kind
  - [ ] created time
  - [ ] byte count
  - [ ] exit code
  - [ ] session id if present

### EX8.2 Artifact save behavior

- [ ] save full stdout/stderr when output exceeds threshold
- [ ] save raw combined output optionally
- [ ] keep preview stored with artifact metadata
- [ ] support read-by-id through existing saved-context retrieval surface in MCP flows fileciteturn0file3

### EX8.3 Artifact retrieval

- [ ] add or reuse:
  - [ ] `read_saved_context`
  - [ ] `search_saved_context`
  - [ ] `get_context_stats`
- [ ] ensure execution artifacts participate in the same artifact ecosystem, not a parallel incompatible system

---

## Phase EX9 — Session integration

### EX9.1 Emit session events

- [ ] on execution start emit:
  - [ ] `tool_started`
- [ ] on success emit:
  - [ ] `tool_succeeded`
- [ ] on non-zero exit emit:
  - [ ] `tool_failed`
- [ ] on artifact save emit:
  - [ ] `artifact_saved`

### EX9.2 Event payloads

- [ ] include:
  - [ ] command name
  - [ ] mode
  - [ ] cwd
  - [ ] exit code
  - [ ] output kind
  - [ ] artifact `source_id` when present
  - [ ] bounded summary only
- [ ] never inline huge stdout/stderr into session event payloads

### EX9.3 Reducer integration later

- [ ] mark recent failed command in session state
- [ ] mark generated artifacts available for resume
- [ ] include last execution summary in resume snapshot if relevant

This fits Atlas’s continuity roadmap cleanly, where saved artifacts and session continuity are already first-class ideas. fileciteturn0file3

---

## Phase EX10 — Graph linkage

### EX10.1 File extraction from outputs

- [ ] detect file paths from:
  - [ ] diffs
  - [ ] compiler errors
  - [ ] test failures
  - [ ] lint messages
- [ ] normalize to repo-relative paths

### EX10.2 Symbol/package linking

- [ ] where practical, map execution-referenced files to:
  - [ ] owning package/workspace
  - [ ] relevant symbols
  - [ ] graph freshness state
- [ ] keep this best-effort and bounded
- [ ] do not make execution fail if graph lookup fails

### EX10.3 Freshness integration

- [ ] after write/edit/build/test flows, optionally:
  - [ ] mark graph stale
  - [ ] suggest `build_or_update_graph`
  - [ ] trigger bounded refresh in host hooks/MCP flows later

This is where Atlas can become stronger than a plain RTK-like layer: execution results can be linked back into graph context. fileciteturn0file3

---

## Phase EX11 — CLI surface

### EX11.1 Add CLI command group

- [ ] `atlas exec run`
- [ ] `atlas exec read-artifact`
- [ ] `atlas exec search-artifacts`
- [ ] `atlas exec stats`

### EX11.2 `atlas exec run`

- [ ] accept:
  - [ ] `--cwd`
  - [ ] `--timeout`
  - [ ] `--mode`
  - [ ] `--json`
  - [ ] `--store-large-output`
- [ ] output compact human-readable result by default
- [ ] output stable JSON when requested

### EX11.3 Diagnostics

- [ ] `atlas exec doctor`
- [ ] verify:
  - [ ] subprocess execution works
  - [ ] artifact store writable
  - [ ] thresholds loaded
  - [ ] policy config valid

Keep this aligned with Atlas’s current CLI-first and doctor/debug approach. fileciteturn0file3

---

## Phase EX12 — MCP surface

### EX12.1 First MCP tools

- [ ] add MCP `run_command`
- [ ] add MCP `read_command_output`
- [ ] add MCP `search_command_outputs`

or, if you want Atlas naming parity:

- [ ] `execute_tool`
- [ ] `read_execution_artifact`
- [ ] `search_execution_artifacts`

### EX12.2 MCP response contract

- [ ] compact summary only by default
- [ ] include:
  - [ ] exit code
  - [ ] output kind
  - [ ] truncation metadata
  - [ ] `source_id`
  - [ ] avoided bytes
- [ ] avoid raw large output in default MCP response

### EX12.3 Tool parity and metadata

- [ ] include repo/index/session provenance envelope, matching Atlas MCP conventions fileciteturn0file3
- [ ] keep adapter thin over execution service
- [ ] add tool-call validation tests
- [ ] add truncation behavior tests
- [ ] add denial/policy tests

---

## Phase EX13 — Config, policy, and install surface

### EX13.1 Config file

- [ ] extend `.atlas/config.toml` with:
  - [ ] `[exec]`
  - [ ] `enabled`
  - [ ] `timeout_secs`
  - [ ] `max_inline_bytes`
  - [ ] `max_capture_bytes`
  - [ ] `auto_store_threshold_bytes`
  - [ ] `allowed_commands`
  - [ ] `denied_commands`
  - [ ] `allow_shell`
  - [ ] `redacted_env`

### EX13.2 Policy docs

- [ ] document safe default behavior
- [ ] document command allowlist model
- [ ] document artifact retention behavior
- [ ] document how exec interacts with sessions and graph freshness

---

## Phase EX14 — Tests and benchmarks

### EX14.1 Unit tests

- [ ] command request validation
- [ ] timeout handling
- [ ] capture truncation
- [ ] output classifier
- [ ] generic compressor
- [ ] specialized compressors

### EX14.2 Integration tests

- [ ] `git status` compression
- [ ] `git diff` compression
- [ ] `cargo test` failure compression
- [ ] `go test` failure compression
- [ ] JSON output handling
- [ ] artifact creation + retrieval
- [ ] policy denial
- [ ] session event emission

### EX14.3 Benchmarks

- [ ] capture overhead
- [ ] compression latency
- [ ] artifact write latency
- [ ] large-output handling
- [ ] MCP response size reduction

---

## Phase EX15 — Rollout strategy

### EX15.1 First-wave supported commands

- [ ] `git status`
- [ ] `git diff`
- [ ] `cargo test`
- [ ] `go test`
- [ ] `pytest`
- [ ] `ruff`
- [ ] `rg`
- [ ] `ls`

### EX15.2 Later-wave commands

- [ ] `cargo build`
- [ ] `go build`
- [ ] `golangci-lint`
- [ ] `eslint`
- [ ] `pnpm test`
- [ ] `npm test`

### EX15.3 Explicit non-goals for v1

- [ ] do not attempt full command coverage
- [ ] do not attempt command rewrite hooks yet
- [ ] do not intercept every shell command globally
- [ ] do not couple execution to graph build pipeline correctness

---

## Recommended insertion into Atlas v6 roadmap

Add this under **Part III — Post-MVP Product Expansion**, after continuity work, as a new phase such as:

### Phase 22.5 — Execution and Output Compression

Why there:
- Atlas core graph functionality is already established
- saved-context/session surfaces already exist in roadmap form
- MCP/tool-facing payload shaping is already an active concern
- execution output compression naturally extends the context-efficiency work, not the parser/store core fileciteturn0file3 fileciteturn0file2

---

## Completion criteria

This subsystem is done when all of these are true:

- [ ] Atlas can run bounded commands safely
- [ ] large tool output no longer needs to be returned inline
- [ ] large outputs are stored as artifacts with `source_id`
- [ ] agents receive compact, useful summaries by default
- [ ] execution emits session events
- [ ] execution artifacts are retrievable through saved-context flows
- [ ] first-wave tool-aware compressors work on real fixtures
- [ ] execution can optionally enrich graph/session state without coupling failures across subsystems
