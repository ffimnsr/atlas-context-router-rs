# atlas-cli

Command-line interface for building, updating, and querying Atlas code graphs. Provides the `atlas` binary with commands for graph initialization, incremental updates, change detection, status reporting, and graph-backed analysis.

## Public Surface

**Binary**: `atlas`

**Commands**
- `atlas init --profile minimal|standard|full` — initialize repo-local databases and write config template
- `atlas build` — full graph build from source
- `atlas update` — incremental update from git diffs
- `atlas migrate` — run explicit schema migrations for graph/content/session stores
- `atlas detect-changes` — list changed files from git
- `atlas status` — health check and graph stats
- `atlas debug-config` / `atlas config show` — print resolved config values with source metadata
- `atlas query` — search graph by symbol name or regex
- `atlas impact` — impact radius from changed files
- `atlas review-context` — review bundle for changes, with `--format markdown` for PR comments
- `atlas selfupdate` — explicit refusal with reinstall guidance
- `atlas serve` — MCP server (JSON-RPC or stdio)

Integrates all atlas crates to provide unified command dispatch and user output formatting.
