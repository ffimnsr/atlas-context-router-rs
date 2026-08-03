# Atlas Context Router

[![CI](https://github.com/ffimnsr/atlas-context-router-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/ffimnsr/atlas-context-router-rs/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/ffimnsr/atlas-context-router-rs)](https://github.com/ffimnsr/atlas-context-router-rs/releases)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

```text
     _  _____ _      _   ___
    / \|_   _| |    / \ / __|
   / _ \ | | | |__ / _ \\__ \
  /_/ \_\|_| |____/_/ \_\___/

  graph-aware code context for CLI and MCP workflows
```

Atlas scans repository code, builds graph structure, stores it in SQLite, and serves graph-aware context to both CLI workflows and MCP-based coding agents.

| Area | What Atlas gives you |
|------|----------------------|
| Graph build | tracked-file scan, parse, persist into `.atlas/worldtree.db` |
| Incremental update | rebuild only from git or working-tree changes |
| Search and impact | symbol lookup, call/risk traversal, review context |
| Agent tooling | MCP broker/daemon server, install helpers, repo hooks, editor integration |

Supported languages in current build:

- Rust
- Go
- Python
- JavaScript
- TypeScript

## Install

### Latest release

Download installer from latest GitHub release assets:

```bash
curl -fsSLO https://github.com/ffimnsr/atlas-context-router-rs/releases/latest/download/install.sh
curl -fsSLO https://github.com/ffimnsr/atlas-context-router-rs/releases/latest/download/install.sh.sha256

sha256sum -c install.sh.sha256 2>/dev/null || shasum -a 256 -c install.sh.sha256
sh install.sh
```

Release notes:

- `install.sh.sha256` is not stored in repo root; release workflow generates it and publishes it as a release asset
- `install.sh.sha256` ships with each release
- `install.sh` verifies downloaded Atlas archive checksums before install
- release archives are published for Linux musl, macOS x86_64, and macOS arm64

If latest-release URLs are not available yet, use one of these fallback paths:

```bash
cargo install --path packages/atlas-cli
```

Maintainer-only raw installer fallback:

```bash
curl -fsSLO https://raw.githubusercontent.com/ffimnsr/atlas-context-router-rs/main/install.sh
sh install.sh
```

Raw `main` installer is not release-pinned. Prefer release assets once first tagged release exists.

### Build from source

```bash
cargo install --path packages/atlas-cli
```

### Run from workspace

```bash
cargo run -p atlas-cli -- --help
```

### Build container image

Build the multi-stage container image from the repository root:

```bash
podman build -f Containerfile -t atlas .
```

Docker works as well:

```bash
docker build -f Containerfile -t atlas .
```

The default builder and runtime images in [Containerfile](Containerfile) are pinned to immutable Chainguard digests.
Override them explicitly if you need newer base images during maintenance:

```bash
podman build \
  --build-arg RUST_IMAGE=cgr.dev/chainguard/rust:latest-dev \
  --build-arg GIT_RUNTIME_IMAGE=cgr.dev/chainguard/git:latest-glibc \
  -f Containerfile \
  -t atlas .
```

Refresh the pinned digests in [Containerfile](Containerfile) to the latest upstream tags:

```bash
scripts/update-containerfile-images.sh
```

Preview the resolved references without editing files:

```bash
scripts/update-containerfile-images.sh --dry-run
```

Run Atlas against the current repository by mounting the workspace into the container:

```bash
podman run --rm -it \
  -v "$PWD":/work \
  -w /work \
  atlas status
```

Atlas shells out to `git`, so the runtime stage uses a minimal Git-capable base image instead of a shell-free static runtime.

## Quick Start

Inside any git repository:

```bash
atlas init --profile standard
atlas build
atlas status
atlas debug-config
atlas query "symbol_name"
```

Repair local Atlas state without rebuilding graph data:

```bash
atlas migrate
atlas config show
atlas selfupdate
```

Review recent work against mainline:

```bash
atlas update --base origin/main
atlas detect-changes --base origin/main
atlas impact --base origin/main
atlas review-context --base origin/main
atlas review-context --base origin/main --format markdown
```

## Benchmarks

Local smoke benchmark on this repository, using `target/release/atlas` with a
prebuilt `.atlas/worldtree.db` graph. Host: Linux 6.19, AMD Ryzen 5 5600X,
12 logical CPUs. Numbers are best read as order-of-magnitude guidance, not a
portable guarantee.

Symbol lookup for `WatchRunner`, 30 warm runs:

| Tool | Command shape | Avg wall time | Output size |
|------|---------------|---------------|-------------|
| `grep` | `grep -RIn --include='*.rs' WatchRunner .` with build dirs excluded | 6.8 ms | 1.3 KiB |
| `rg` | `rg -n WatchRunner --glob '*.rs'` | 6.8 ms | 1.3 KiB |
| `atlas query` | `atlas --json query WatchRunner --limit 20` | 9.1 ms | 20.4 KiB |

Context gathering for `WatchRunner`, comparing raw nearby text against bounded
graph context:

| Tool | Command shape | Avg wall time | Context payload | Compression vs raw text |
|------|---------------|---------------|-----------------|-------------------------|
| `grep` | related-symbol regex with `-C 8` over known watch files | 3.3 ms | 47.8 KiB | baseline |
| `rg` | related-symbol regex with `-C 8` over known watch files | 5.6 ms | 47.8 KiB | baseline |
| `atlas context` | `atlas --json context WatchRunner --max-nodes 20 --max-edges 20 --max-files 10` | 35.5 ms | 3.1 KiB | 93.5% smaller |

Use `grep` or `rg` for raw text search. Use Atlas when caller/callee links,
impact, review context, or MCP token budget matter more than the fastest line
scan.

## Project Policy and Maintenance

Repository workflow and support docs live at the root:

- [CONTRIBUTING.md](CONTRIBUTING.md)
- [SECURITY.md](SECURITY.md)
- [SUPPORT.md](SUPPORT.md)
- [CODE_OF_CONDUCT.md](CODE_OF_CONDUCT.md)
- [CHANGELOG.md](CHANGELOG.md)
- [docs/error_codes.md](docs/error_codes.md)

GitHub triage defaults also live in `.github/`:

- `.github/CODEOWNERS`
- `.github/PULL_REQUEST_TEMPLATE.md`
- `.github/ISSUE_TEMPLATE/`

## LLM Agent Setup

Atlas can install MCP configuration for popular AI coding tools and add repo hooks so agents start with graph-aware context.

`atlas serve` remains a stdio MCP entrypoint for editors and agents. Repo-scoped installs now pin only `--repo`, while user/global installs stay portable with `type = "stdio"`, `command = "atlas"`, `args = ["serve"]`. Atlas now follows MCP `2026-07-28`: use `server/discover` for negotiation, then send per-request `params._meta` with protocol version and client capabilities. When no repo is pinned, Atlas resolves repo context from explicit `arguments.repo_root` or launch `cwd`; it no longer depends on MCP Roots or protocol-level HTTP sessions.

If an editor MCP client is incompatible with broker/daemon indirection, use `atlas serve --direct-stdio` to run MCP directly in launched process with no relay layer.

### GitHub Copilot

```bash
atlas install --platform copilot
atlas build
```

Writes MCP config under `.vscode/mcp.json`.

### Claude Code

```bash
atlas install --platform claude
atlas build
```

Writes MCP config under `.mcp.json`.

### Codex

```bash
atlas install --platform codex
atlas build
```

Writes Codex MCP entry into `.codex/config.toml`.

### Auto-detect installed tools

```bash
atlas install
```

### Preview without writing files

```bash
atlas install --dry-run
```

### Replace full instruction file

```bash
atlas install --platform codex --instructions-mode replace-file
```

Default mode is `refresh`: Atlas replaces only its managed instruction block when markers already exist, or appends that block when `AGENTS.md` / `CLAUDE.md` already contains user-managed text. Use `--instructions-mode replace-file` to overwrite the full instruction file with Atlas-managed content.

### Update instructions only

```bash
atlas install --platform codex --instructions-only
```

`--instructions-only` is alias for `--no-platform-config --no-hooks`. It updates only `AGENTS.md` / `CLAUDE.md`.

### Install modes

`--mode` selects which surfaces `atlas install` sets up for the target platform. It composes with the explicit `--no-*` flags (which add further skips):

| Mode | Installs | Skips |
| ---- | -------- | ----- |
| `mcp` | platform MCP config only | hooks + instruction fallback files |
| `hook` | git hooks + platform agent hooks | MCP config + instruction fallback files |
| `cli` | instruction fallback text (`AGENTS.md` / `CLAUDE.md`) | MCP config + hooks |
| `all` (default) | all three surfaces | — |

Examples:

```bash
atlas install --platform claude --mode mcp    # MCP config only
atlas install --platform codex --mode cli     # instruction fallback only
atlas install --mode hook                     # hooks only, auto-detected platform
```

`atlas install --dry-run` prints every file it would write or refresh, including which instruction fallback files would be created, appended, or refreshed, without touching anything.

### Native hooks vs MCP fallback

| surface | works_with | automatic | token_cost | best_for |
| ------- | ---------- | --------- | ---------- | -------- |
| native `atlas hook` (installed via `atlas install`) | Claude Code, Codex, GitHub Copilot | yes — zero agent effort | low (one background call per event) | hosts with LLM hook support; hands-off continuity |
| MCP fallback (`wake_up` + `record_session_event` + instruction block) | any MCP-capable host: Zed, Cursor, Cline, VS Code Copilot Chat, JetBrains AI, others | no — agent follows the installed instruction block | low–medium (one tool call per trigger) | hookless hosts; universal compatibility |
| instruction block alone (`AGENTS.md` / `CLAUDE.md`) | any host that reads repository instructions | no | none | hosts without MCP or hooks |
| CLI (`atlas hook`, `atlas session resume`, `atlas session compact`) | shell scripts and manual workflows | no | none | debugging, CI, and scripted flows |

Both capture paths record the same events into the same `session.db` / `context.db` storage through one shared event service, so event semantics, redaction, lifecycle actions, graph refresh, and review refresh never drift. See [Session Memory Fallback for Hookless Hosts](wiki/session-memory-fallback.md) for the hookless protocol.

`atlas install` will:

- write MCP server config for GitHub Copilot, Claude Code, or Codex
- install git hooks for `pre-commit`, `post-checkout`, `post-merge`, and `post-rewrite`
- inject graph-first instructions into platform-relevant agent files (`AGENTS.md` for Copilot/Codex, `CLAUDE.md` for Claude)

Hook behavior:

- `pre-commit` prints full change summary
- `post-checkout`, `post-merge`, and `post-rewrite` use brief output

After install, restart your editor or coding tool, then run:

```bash
atlas build
```

Repo-local MCP coordination state lives under `.atlas/mcp/<instance-id>/`:

- `mcp.instance.lock`
- `mcp.instance.json`
- `mcp.sock` on Unix

`instance-id` is derived from canonical repo root plus canonical DB path, so same repo with different DBs can run separate daemons while same repo plus same DB reuses one backend.

## Canonical Path Migration

Atlas treats repo-relative paths as canonical identity for persisted graph rows, content source IDs, chunk seeds, and session file references.

Shared path normalization now happens in `atlas-repo`: separators become `/`, Unicode folds to NFC, Windows identities fold case, and CLI/watch boundary absolute paths prefer filesystem-canonical spelling so case-insensitive filesystems do not drift. Repo scan skips symlinks and dedupes canonical submodule roots to guard loop topologies.

Audit coverage:

- graph store persistence and lookup keys go through canonical `files.path` / `nodes.file_path` identity before reuse or persistence
- file-hash reuse in full and incremental build paths uses the same canonical graph file keys
- content `source_id` and `chunk_id` seeds require canonical repo-path identity for file-backed artifacts
- session payload normalization and resume snapshot file references canonicalize repo-file paths before persistence
- MCP explicit `files` inputs canonicalize repo-relative paths before change-source resolution
- future sidecar/cache/index keys, including parser tree-cache entries, must reuse the same canonical repo-path spelling

If `atlas doctor` or `atlas db-check` reports `noncanonical_path_rows`, rebuild from clean canonical inputs instead of trying to rewrite stale rows in place:

```bash
atlas purge-noncanonical
atlas build
```

`atlas purge-noncanonical` removes repo-local `context.db` and `session.db` state, keeps `worldtree.db`, and leaves rebuild plus session bootstrap explicit.

## Shell Completion

Generate completion script to stdout:

```bash
atlas completions bash
atlas completions zsh
atlas completions fish
atlas completions powershell
```

Example for Bash:

```bash
atlas completions bash > ~/.local/share/bash-completion/completions/atlas
```

Example for Zsh:

```bash
mkdir -p ~/.zfunc
atlas completions zsh > ~/.zfunc/_atlas
```

## Typical Workflows

### Bootstrap graph for a repo

```bash
atlas init
atlas build
atlas status
```

### Review branch against main

```bash
atlas update --base origin/main
atlas detect-changes --base origin/main
atlas impact --base origin/main
atlas review-context --base origin/main
```

### Install editor and hook integration

```bash
atlas install --platform copilot
atlas build
```

## Common Commands

Initialize repository state:

```bash
atlas init
```

Full rebuild:

```bash
atlas build
```

Incremental update from working tree or git base:

```bash
atlas update
atlas update --base origin/main
atlas update --staged
```

Show graph status:

```bash
atlas status
atlas status --base origin/main
```

Search graph nodes:

```bash
atlas query "AuthService"
atlas query "login" --kind function --language rust
atlas query "router" --subpath packages/atlas-cli --expand --expand-hops 2

# regex filter: text is the pattern, matched against name/qualified_name via SQL UDF
atlas query "^handle_[a-z]+" --regex
atlas query "(body|node)" --regex --kind function
```

Inspect changed files:

```bash
atlas detect-changes
atlas detect-changes --base origin/main
atlas detect-changes --staged
```

Compute blast radius:

```bash
atlas impact --base origin/main
atlas impact --files packages/atlas-cli/src/commands.rs
```

Assemble review context:

```bash
atlas review-context --base origin/main
atlas review-context --base origin/main --format markdown
atlas review-context --files packages/atlas-cli/src/install.rs
```

Explain changed code with impact and risk summary:

```bash
atlas explain-change --base origin/main
atlas explain-change --files packages/atlas-cli/src/commands.rs
```

Dead-code scan and deterministic rename preview:

```bash
atlas analyze dead-code --subpath packages/atlas-cli
atlas refactor rename --symbol src/lib.rs::fn::helper --to helper_renamed --dry-run
```

Run MCP server over stdio:

```bash
atlas serve
```

Health and integrity checks:

```bash
atlas doctor
atlas db-check
```

## Context Engine (`atlas context`)

`atlas context` builds bounded, machine-readable context around a symbol, file, or change-set.

Common forms:

```bash
atlas context "AuthService"
atlas context "who calls handle_request"
atlas context --file src/auth.rs
atlas context --files src/auth.rs src/session.rs --intent impact
atlas --json context --files src/auth.rs src/session.rs
```

Key knobs:

- `--intent`: `symbol`, `file`, `review`, `impact`, `usage_lookup`, `refactor_safety`, `dead_code_check`, `rename_preview`, `dependency_removal`
- limits: `--max-nodes`, `--max-edges`, `--max-files`, `--depth`
- extra detail: `--code-spans`, `--tests`, `--imports`, `--neighbors`

Default limits: 100 nodes, 100 edges, 20 files, depth 2.

JSON output uses `atlas_cli.v1` and returns request metadata, selected nodes and edges, file spans, truncation info, and optional ambiguity candidates. If target is ambiguous, `ambiguity` is populated instead of direct node results. `atlas review-context` remains available as focused shortcut for review-heavy workflows.

Contract references:

- [Output stability policy](docs/contracts/output-stability.md)
- `schemas/atlas_cli.v1/*.schema.json`

## Output Modes

Most user-facing commands support machine-readable output:

```bash
atlas --json status
atlas --json detect-changes --base origin/main
atlas --json install --platform claude
```

## Files Atlas Writes

- `.atlas/config.toml`
- `.atlas/worldtree.db`
- `.mcp.json` for Claude Code installs
- `.vscode/mcp.json` for GitHub Copilot installs
- `.codex/config.toml` entry for Codex installs
- `.git/hooks/*` for installed git hooks

## Insights

Atlas also exposes deterministic insight reports through CLI:

```bash
atlas insights architecture
atlas insights metrics --limit 20
atlas insights risk src/service.rs::fn::compute
atlas insights patterns --limit 20
atlas insights large-functions --threshold 80 --mode large-or-complex
atlas insights complex-functions --complexity-threshold 15
atlas insights similar-functions src/service.rs::fn::compute --limit 10
atlas insights duplicates --min-score 0.7
atlas insights infer-modules
atlas insights label-components --files packages/atlas-cli/src/commands/changes.rs
atlas --json insights metrics
```

Thresholds and layer rules live under `.atlas/config.toml`:

```toml
[insights]
large_function_loc = 80
high_fan_in = 20
high_fan_out = 10
high_coupling = 15
deep_chain_length = 6
repeated_call_chain_min_length = 3
risk_medium_threshold = 35.0
risk_high_threshold = 70.0

[[insights.layer_rules]]
name = "api"
path_prefixes = ["src/api"]
module_prefixes = []

[[insights.layer_rules]]
name = "domain"
path_prefixes = ["src/domain"]
module_prefixes = []
```

Layer rules can also live in a separate `.atlas/layer-rules.toml` file so they
change without editing the main config. Point `insights.layer_rules_file` at
it (relative paths resolve from `.atlas/`); when set, it replaces inline
`[[insights.layer_rules]]` entries:

```toml
[insights]
layer_rules_file = "layer-rules.toml"
```

`.atlas/layer-rules.toml`:

```toml
[[layer_rules]]
name = "api"
path_prefixes = ["src/api"]
module_prefixes = []

[[layer_rules]]
name = "domain"
path_prefixes = ["src/domain"]
module_prefixes = []
```

Missing, unreadable, or malformed layer-rules files fail config load with
actionable errors naming `insights.layer_rules_file` and the resolved path.

## MCP Tools

The MCP server (`atlas serve`) exposes these tools to agents:

| Tool | Description |
|------|-------------|
| `list_graph_stats` | Node/edge counts and language breakdown |
| `tool_list` | Compact runtime inventory of visible exported MCP tools |
| `tool_search` | Search visible exported MCP tools by name/title/description with explicit score factors and typo-tolerant fuzzy name matching |
| `tool_help` | Runtime manual for one visible exported MCP tool by exact name |
| `man` | Namespace-aware manual alias for exported MCP tools |
| `query_graph` | Keyword search with optional `regex` SQL-UDF filter; returns compact symbol list |
| `batch_query_graph` | Run up to 20 `query_graph` searches in a single round-trip |
| `search_files` | File-path discovery for config, templates, SQL, Markdown, and other non-code assets |
| `search_content` | Literal or regex content search outside graph-symbol lookup |
| `read_file_excerpt` | Repo-scoped file excerpt reads by line range or line-with-context |
| `get_docs_section` | Markdown section lookup by heading path/slug or line number |
| `read_file_around_match` | Grouped snippets around literal or regex matches in one file |
| `search_templates` | Discover HTML, Jinja, Handlebars, Tera, and related template files |
| `search_text_assets` | Discover SQL, config, env, and prompt files |
| `broker_status` | Lightweight broker liveness probe: PID, uptime, version |
| `status` | Compact graph health summary with machine-readable failure state |
| `doctor` | Full repo health checks: git, config, DB, build, and retrieval index |
| `db_check` | SQLite integrity plus orphan-node and dangling-edge scan |
| `debug_graph` | Graph internals: node/edge kinds, top files, and anomalies |
| `explain_query` | Explain how `query_graph` will tokenize and execute a request |
| `resolve_symbol` | Resolve a symbol or QN alias to canonical `qualified_name` |
| `analyze_architecture` | Deterministic architecture report with cycles, layers, and coupling hotspots |
| `analyze_metrics` | Deterministic metrics report with outliers and complexity hotspots |
| `assess_risk` | Deterministic risk assessment for one symbol with factor evidence |
| `analyze_patterns` | Deterministic pattern detection for repeated chains and isolated structures |
| `analyze_safety` | Refactor-safety analysis with callers, fan-out, and test adjacency |
| `analyze_remove` | Removal-impact analysis with bounded evidence |
| `analyze_dead_code` | Dead-code candidate detection with certainty tiers and blockers |
| `analyze_dependency` | Dependency-removal validation for a symbol |
| `find_large_functions` | Deterministic large/complex function discovery with thresholds and ranked evidence |
| `find_complex_functions` | Deterministic complex-function discovery with complexity thresholds |
| `find_similar_functions` | Similar callable discovery with score breakdown |
| `find_duplicates` | Near-duplicate callable detection |
| `infer_modules` | Explainable module inference from paths and graph |
| `label_components` | Rule-based Atlas component labeling |
| `get_impact_radius` | Graph traversal from changed files |
| `get_review_context` | Review bundle: symbols, neighbors, risk summary |
| `get_context` | General context engine: symbol, file, review, impact |
| `detect_changes` | Git diff → changed-file list with node counts |
| `build_or_update_graph` | Full scan or incremental graph update |
| `postprocess_graph` | Refresh derived graph analytics after build/update without reparsing |
| `traverse_graph` | Bi-directional graph traversal from a qualified name |
| `get_minimal_context` | Auto-detect changes and return compact impact bundle |
| `explain_change` | Advanced impact: risk, change kinds, boundary/test gaps |
| `get_session_status` | Current session identity, event count, and resume state |
| `record_session_event` | Hook-equivalent MCP fallback capture for session/prompt/tool/stop events |
| `wake_up` | Bounded session-start recall: focus, decisions, memories, readiness |
| `repo_registry` | Persisted multi-repo registry inventory, relationships, trust, and warnings |
| `compact_session` | Compact session event ledger: merge, decay, dedup, promote |
| `resume_session` | Retrieve and consume current session snapshot |
| `search_saved_context` | Search saved artifacts from prior tool outputs |
| `search_decisions` | Search persisted decision memory for prior conclusions |
| `read_saved_context` | Retrieve full artifact content by source_id with optional paging |
| `save_context_artifact` | Store large context payloads for later retrieval |
| `get_context_stats` | Session/content-store stats and DB paths |
| `purge_saved_context` | Delete saved artifacts by session or age |
| `cross_session_search` | CM11: search saved context across all sessions for this repo |
| `get_global_memory` | CM11: frequent symbols/files/workflows and related past sessions |
| `memory_store` | ICM-A: store a memory record through the shared memory service |
| `memory_recall` | ICM-A: recall memories with visibility rules and retrieval hints |
| `symbol_neighbors` | Immediate callers, callees, tests, and nearby graph nodes |
| `cross_file_links` | Files semantically linked to a file by shared symbol references |
| `concept_clusters` | Related file groups around seed files by coupling density |

Graph tools answer code structure questions; content tools answer non-code context-adjacent questions (docs, config, templates, SQL, prompts). Use both as companion surfaces, not as a fallback chain.

Manual surface:

- MCP `tool_list` lists current visible exported tools in compact runtime form
- MCP `tool_search` ranks likely tool matches with explicit lexical/fuzzy score factors when exact name is unclear or misspelled
- MCP `tool_help` returns runtime docs for one exact exported tool name without executing it
- `atlas man mcp <mcp_tool_name>` prints same runtime docs through CLI
- MCP `man` remains namespace-aware alias for callers that already use `namespace` + `tool_name`

Resource docs surface:

- `resources/read { uri: "atlas://docs/index" }` returns Atlas docs index with discovery flow plus per-tool and per-prompt docs links
- `resources/read { uri: "atlas://tool-docs/query_graph" }` returns generated Markdown docs with examples and usage for one tool
- `resources/read { uri: "atlas://prompt-docs/review_change" }` returns generated Markdown docs and default body for one prompt
- `resources/templates/list` exposes `atlas://tool-docs/{name}`, `atlas://prompt-docs/{name}`, and `atlas://docs/{file}#{heading}` templates

Search tool selection rules:

1. `query_graph`: use for symbol names, definitions, and graph-native relationships.
2. `search_files`: companion lookup for config, template, SQL, Markdown, and other non-code assets not indexed as graph symbols.
3. `search_content`: companion lookup for text matches; use alongside graph results when changed files include embedded constants, config keys, SQL fragments, or error strings.
4. `read_file_excerpt`: use when file path is already known and you need precise line ranges or one line with bounded surrounding context.
5. `get_docs_section`: use for Markdown docs when section identity matters more than raw line ranges.
6. `read_file_around_match`: use when file path is known and you need grouped context around matched lines.
7. `search_templates`: companion lookup for HTML, Jinja, Handlebars, and related template files when changed files include templates.
8. `search_text_assets`: companion lookup for SQL, config, `.env`, and prompt files when changed files or graph evidence points to non-code assets. Pass results to `get_context` via `files=` to merge under bounded selection policy.

## MCP Prompts

The MCP server also exposes prompt templates for external LLM clients that support `prompts/list` and `prompts/get`:

| Prompt | Purpose |
|------|-------------|
| `review_change` | Guide review flow through `detect_changes`, `get_minimal_context`, `get_review_context`, `explain_change`, and `get_impact_radius` |
| `inspect_symbol` | Guide symbol lookup through `query_graph`, `symbol_neighbors`, `get_context`, and `traverse_graph` |
| `plan_refactor` | Guide refactor planning through context, impact, coupling, and safety checks |
| `resume_prior_session` | Guide continuity recovery through session status, resume snapshot, and saved-context retrieval |

These prompts are guidance only. Atlas still keeps graph, context, impact, and continuity logic in tools rather than hard-coding behavior into prompt text.

Output defaults:

- all MCP tools return JSON text content
- `structuredContent` is authoritative for object payloads
- do not pass legacy format-selector arguments; JSON is implicit

`get_context` accepts free-text query, file, or changed-file list plus intent and limit controls. Response is compact `PackagedContextResult` with counts, selected nodes and edges, files, truncation fields, and optional ambiguity candidates.

Recommended agent workflow:

1. `detect_changes`
2. `get_minimal_context` or `get_review_context`
3. `get_impact_radius` or `explain_change`
4. `query_graph` or `get_context`
5. when changed files include docs, config, templates, SQL, or prompts: call `search_text_assets` or `search_templates` as companion lookup, then pass results into `get_context` via `files=` to merge under bounded selection policy
6. do not use content search before graph tools for symbol questions

## Contributing

Contributions welcome.

Current repo expectations:

- keep diffs focused and avoid unrelated refactors
- prefer small safe changes over broad churn
- add or update tests for behavior changes
- run `cargo test` and `cargo clippy -- -D warnings` before sending changes
- check `ISSUES.md` for existing tracked work before starting

Useful commands:

```bash
cargo test
cargo clippy --workspace -- -D warnings
```

## License

See [LICENSE](LICENSE) and [LICENSE-APACHE](LICENSE-APACHE) for repository license texts.
