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

Run MCP server over stdio:

```bash
atlas serve
```

Health and integrity checks:

```bash
atlas doctor
atlas db-check
```

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
- `~/.codex/config.toml` entry for Codex installs
- `.git/hooks/*` for installed git hooks

## Similar Open Source Projects

- `code-review-graph` for graph-assisted code review workflows
- Sourcegraph for large-scale code search and navigation
- Semgrep for rule-based code scanning and pattern matching
- ripgrep-based CLI workflows for fast text search without graph context

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
