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

## Quick Start

Inside any git repository:

```bash
atlas init
atlas build
atlas status
atlas query "symbol_name"
```

Review recent work against mainline:

```bash
atlas update --base origin/main
atlas detect-changes --base origin/main
atlas impact --base origin/main
atlas review-context --base origin/main
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

## LLM Agent Setup

Atlas can install MCP configuration for popular AI coding tools and add repo hooks so agents start with graph-aware context.

`atlas serve` remains a stdio MCP entrypoint for editors and agents, but on Linux it now acts as a repo-scoped broker: it attaches to one live daemon per canonical repo root plus DB path or starts one under lock when absent. Generated editor config stays `type = "stdio"`, `command = "atlas"`, `args = ["--repo", ..., "--db", ..., "serve"]`.

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

## MCP Tools

The MCP server (`atlas serve`) exposes these tools to agents:

| Tool | Description |
|------|-------------|
| `list_graph_stats` | Node/edge counts and language breakdown |
| `query_graph` | Keyword search with optional `regex` SQL-UDF filter; returns compact symbol list |
| `batch_query_graph` | Run up to 20 `query_graph` searches in a single round-trip |
| `search_files` | File-path discovery for config, templates, SQL, Markdown, and other non-code assets |
| `search_content` | Literal or regex content search outside graph-symbol lookup |
| `search_templates` | Discover HTML, Jinja, Handlebars, Tera, and related template files |
| `search_text_assets` | Discover SQL, config, env, and prompt files |
| `status` | Compact graph health summary with machine-readable failure state |
| `doctor` | Full repo health checks: git, config, DB, build, and retrieval index |
| `db_check` | SQLite integrity plus orphan-node and dangling-edge scan |
| `debug_graph` | Graph internals: node/edge kinds, top files, and anomalies |
| `explain_query` | Explain how `query_graph` will tokenize and execute a request |
| `resolve_symbol` | Resolve a symbol or QN alias to canonical `qualified_name` |
| `analyze_safety` | Refactor-safety analysis with callers, fan-out, and test adjacency |
| `analyze_remove` | Removal-impact analysis with bounded evidence |
| `analyze_dead_code` | Dead-code candidate detection with certainty tiers and blockers |
| `analyze_dependency` | Dependency-removal validation for a symbol |
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
| `compact_session` | Compact session event ledger: merge, decay, dedup, promote |
| `resume_session` | Retrieve and consume current session snapshot |
| `search_saved_context` | Search saved artifacts from prior tool outputs |
| `read_saved_context` | Retrieve full artifact content by source_id with optional paging |
| `save_context_artifact` | Store large context payloads for later retrieval |
| `get_context_stats` | Session/content-store stats and DB paths |
| `purge_saved_context` | Delete saved artifacts by session or age |
| `cross_session_search` | CM11: search saved context across all sessions for this repo |
| `get_global_memory` | CM11: frequent symbols/files/workflows and related past sessions |
| `symbol_neighbors` | Immediate callers, callees, tests, and nearby graph nodes |
| `cross_file_links` | Files semantically linked to a file by shared symbol references |
| `concept_clusters` | Related file groups around seed files by coupling density |

Search tool selection rules:

1. `query_graph`: use for symbol names, definitions, and graph-native relationships.
2. `search_files`: use when filename, extension, or path pattern is known but content is not.
3. `search_content`: use for literal or regex text such as error strings, comments, config keys, SQL fragments, and embedded constants. Enable `rich_snippets=true` only when grouped before/match/after context is worth extra payload.
4. `search_templates`: use when looking specifically for template files by engine or extension.
5. `search_text_assets`: use for SQL, config, `.env`, and prompt files outside graph-symbol lookup.

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

- `get_context`, `get_review_context`, `get_impact_radius`, and `explain_change` default to `toon`
- all other tools default to `json`
- explicit `output_format=json` overrides TOON-first behavior

`get_context` accepts free-text query, file, or changed-file list plus intent and limit controls. Response is compact `PackagedContextResult` with counts, selected nodes and edges, files, truncation fields, and optional ambiguity candidates.

Recommended agent workflow:

1. `detect_changes`
2. `get_minimal_context` or `get_review_context`
3. `get_impact_radius` or `explain_change`
4. `query_graph` or `get_context`
5. fall back to file search only when graph lacks needed fact

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
