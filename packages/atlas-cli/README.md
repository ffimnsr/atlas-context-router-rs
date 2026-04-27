# atlas-cli

Command-line interface for building, updating, and querying Atlas code graphs. Provides the `atlas` binary with commands for graph initialization, incremental updates, change detection, status reporting, and graph-backed analysis.

## Public Surface

**Binary**: `atlas`

**Commands**
- `atlas init` — initialize graph in repository
- `atlas build` — full graph build from source
- `atlas update` — incremental update from git diffs
- `atlas detect-changes` — list changed files from git
- `atlas status` — health check and graph stats
- `atlas query` — search graph by symbol name or regex
- `atlas impact` — impact radius from changed files
- `atlas review-context` — review bundle for changes
- `atlas serve` — MCP server (JSON-RPC or stdio)

Integrates all atlas crates to provide unified command dispatch and user output formatting.
