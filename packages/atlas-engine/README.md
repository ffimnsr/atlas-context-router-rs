# atlas-engine

Shared build and incremental update pipeline for Atlas graphs. Provides the core graph construction engine used by CLI and MCP, including parallel source parsing, graph assembly, and SQLite persistence.

## Public Surface

- **`build_graph()`** — full repository graph build
  - Parallel file collection and parsing via Rayon
  - Graph node/edge assembly
  - SQLite persistence with built-in budget controls
  - Returns `BuildSummary` with stats and diagnostics

- **`update_graph()`** — incremental update from git diffs
  - Changed-file detection and dependency computation
  - Selective reparse and graph diff
  - Idempotent persistence
  - Returns `UpdateSummary` with impact metrics

- **`Config`, `BuildRunBudget`** — operational limits
  - File count caps, byte budgets, timeout policies
  - Language maturity configuration

- **`WatchRunner`** — filesystem watcher for real-time updates

Parallel parse phases are explicitly separated from SQLite write phases per concurrency policy.
