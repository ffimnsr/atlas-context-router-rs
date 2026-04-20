# Atlas Context Router

Atlas builds a local code graph for your repository, stores it in SQLite, and gives you fast graph-aware commands for search, impact analysis, review context, and MCP-based AI tooling.

## What Atlas Does

- scans tracked files in your repository
- parses supported languages into graph nodes and edges
- stores graph data in `.atlas/worldtree.db`
- updates incrementally from git changes
- answers graph-aware queries from CLI or MCP clients

Supported languages in current build:

- Rust
- Go
- Python
- JavaScript
- TypeScript

## Install

Build from source:

```bash
cargo install --path packages/atlas-cli
```

Or run from workspace during development:

```bash
cargo run -p atlas-cli -- --help
```

## Quick Start

Inside a git repository:

```bash
atlas init
atlas build
atlas status
atlas query "symbol_name"
```

Review recent work against a base ref:

```bash
atlas update --base origin/main
atlas detect-changes --base origin/main
atlas impact --base origin/main
atlas review-context --base origin/main
```

## Install For AI Tools

Atlas can install MCP configuration for supported coding tools and add repository hooks.

Configure all detected tools:

```bash
atlas install
```

Configure one tool explicitly:

```bash
atlas install --platform copilot
atlas install --platform claude
atlas install --platform codex
```

Preview without writing files:

```bash
atlas install --dry-run
```

What `atlas install` does:

- writes MCP server config for GitHub Copilot, Claude Code, or Codex
- installs git hooks for `pre-commit`, `post-checkout`, `post-merge`, and `post-rewrite`
- injects graph-first instructions into `AGENTS.md` and `CLAUDE.md`

Hook behavior:

- `pre-commit` prints full change summary
- `post-checkout`, `post-merge`, and `post-rewrite` use brief output

After install, restart your editor or coding tool, then run:

```bash
atlas build
```

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

Build bounded, machine-readable context around any symbol, file, or change-set.

**Symbol or free-text query** (auto-classified):

```bash
atlas context "AuthService"
atlas context "who calls handle_request"
atlas context "safe to remove helper"
```

**Explicit file target:**

```bash
atlas context --file src/auth.rs
```

**Changed-file review context:**

```bash
atlas context --files src/auth.rs src/session.rs
atlas context --files src/auth.rs --intent impact
```

**Machine-readable JSON:**

```bash
atlas --json context "AuthService"
atlas --json context --file src/auth.rs
atlas --json context --files src/auth.rs src/session.rs
```

**Supported `--intent` values:** `symbol` (default), `file`, `review`, `impact`,
`usage_lookup`, `refactor_safety`, `dead_code_check`, `rename_preview`,
`dependency_removal`.

**Limit flags:** `--max-nodes`, `--max-edges`, `--max-files`, `--depth`.
Extra flags: `--code-spans`, `--tests`, `--imports`, `--neighbors`.

**Default limits:** 100 nodes, 100 edges, 20 files, depth 2.

**JSON contract** (`atlas --json context ...`):

```json
{
  "schema_version": "atlas_cli.v1",
  "command": "context",
  "data": {
    "intent": "symbol",
    "request": { "intent": "symbol", "target": { "kind": "symbol_name", "name": "..." }, ... },
    "nodes": [ { "distance": 0, "selection_reason": "direct_target", "relevance_score": 121.0, "node": { ... } } ],
    "edges": [ { "depth": 1, "selection_reason": "callee", "edge": { ... } } ],
    "files": [ { "path": "src/lib.rs", "selection_reason": "direct_target", "line_ranges": [] } ],
    "truncation": { "truncated": false, "nodes_dropped": 0, "edges_dropped": 0, "files_dropped": 0 },
    "ambiguity": null
  }
}
```

When the target is ambiguous, `ambiguity` contains `{ "query": "...", "candidates": [...] }` instead of nodes.
When the target is not found, `nodes` is empty and `ambiguity` is null.

`atlas review-context` remains available as a focused shortcut for change-set review during transition.

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
| `query_graph` | Keyword search, returns compact symbol list |
| `get_impact_radius` | Graph traversal from changed files |
| `get_review_context` | Review bundle: symbols, neighbors, risk summary |
| `get_context` | General context engine: symbol, file, review, impact |
| `detect_changes` | Git diff → changed-file list with node counts |
| `build_or_update_graph` | Full scan or incremental graph update |
| `traverse_graph` | Bi-directional graph traversal from a qualified name |
| `get_minimal_context` | Auto-detect changes and return compact impact bundle |
| `explain_change` | Advanced impact: risk, change kinds, boundary/test gaps |

All MCP tools accept optional `output_format` with `json` or `toon`. `get_context`, `get_review_context`, `get_impact_radius`, and `explain_change` default to TOON. Other tools default to JSON. Explicit `output_format=json` still overrides TOON-first defaults. TOON uses official `toon-format` crate, stays limited to MCP response bodies, validates encode/decode round-trip, sorts object keys for deterministic output, and falls back to JSON when TOON output would be empty or invalid.

**`get_context` schema:**

```json
{
  "query":     "free-text or symbol name (alternative to file/files)",
  "file":      "repo-relative file path",
  "files":     ["list", "of", "changed", "paths"],
  "intent":    "symbol|file|review|impact|usage_lookup|refactor_safety|dead_code_check|rename_preview|dependency_removal",
  "max_nodes": 100,
  "max_edges": 100,
  "max_depth": 2,
  "output_format": "json|toon"
}
```

Response is a compact `PackagedContextResult` with `intent`, `node_count`, `nodes`, `edge_count`, `edges`,
`file_count`, `files`, `truncated`, `nodes_dropped`, `edges_dropped`, and optional `ambiguity_candidates`.

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
